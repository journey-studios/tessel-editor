# P4b S1a (BL082.4.2) — fundação de domínio de `fx.transitions` atrás do portão de admissão

Contrato: [F064](../features/fx-transitions.md). Plano: [p4b-cross-dissolve-plan.md](p4b-cross-dissolve-plan.md)
(seções 5, 6, 8.4 e 13). Base: `origin/main` em `bb309a79ea54d396defb2aa8a62b0b496ebe496b` (S0 mesclada). Commit de
código: `5e21e0931932d1537cd213f4dd40d81a9cad4271` (árvore `c406aa20543d5b18070e67530e9b746070016dbd`).

**O que esta entrega é:** o modelo de documento, a leitura estrita, a validação completa (`explain`), a aritmética de
janela, os avisos e uma tabela de catálogo separada, todos no core, **sem habilitar nenhuma transição pública**.
**O que não é:** avaliação fora da janela e `ScenePlan.transition` (S1b), runtime/shader/export (S2), operações,
gestos e inspetor (S3), UI e gizmo (S4), dissolve legado (S6). Catálogo público = 3 looks, `catalogVersion` 1,
`PLAN_VERSION` 1, 13 capacidades.

## Invariante principal: o portão

Uma política única, `TransitionAdmission::{Refuse, Accept}` (`fx/admission.rs`), é guardada no `DirectedScene`
(`admission()`).

- Todo caminho público é `Refuse` e um `transitions` não vazio continua `FX_TRANSITION_NOT_YET` em `/fx/transitions`,
  com a mensagem de sempre (`transitions between shots are not implemented yet; leave `transitions` empty`).
- O candidato de toda edição herda a política da cena: `DirectedScene::reimport` (`directed_scene/mod.rs:222`) usado por
  `scene_edit/fx.rs:777` e `scene_edit/batch.rs:440`. Uma edição nunca abre nem fecha o portão.
- `Accept`, `import_str_admitting` e `import_value_admitting` são `#[doc(hidden)] pub`: servem aos testes e ao corpus do
  core. **Não são barreira de segurança** (qualquer crate pode chamá-los); a defesa é o teste `boundary`, que varre o
  texto de produção de todos os crates. Nenhuma flag, variável de ambiente, socket ou arquivo chega a `Accept`.
- Nenhum handler de produção (desktop, headless, ops, media, render) chama `Accept`.

### Mapa de caminhos que criam ou reidratam uma cena (todos `Refuse`)

| Caminho | Função e arquivo | Política |
|---|---|---|
| Anexar (JSON, texto, arquivo, bundle) | `SceneSource::import`, `scene_project/run.rs:99-101` (`AdmittedScene::prepare`) | `Refuse` |
| Importar mídia de cena / host | `AdmittedScene::prepare` (mesmo ponto) via `scene_resources/import.rs` e `host.rs` | `Refuse` |
| Abrir projeto, restaurar pacote | `parse_entry`, `scene_project/store.rs:66` (← `SceneBook::from_json` ← `document.rs`) | `Refuse` |
| Render headless / frames | `import_gated`, `crates/render/src/scene.rs:64` | `Refuse` |
| Candidato de `scene.edit` | `scene_edit/batch.rs:440` (`reimport`) | herda |
| Candidato de `scene.fx.edit` | `scene_edit/fx.rs:777` (`reimport`) | herda |
| Desfazer / refazer | restaura clones `Arc` da cena, sem reimportar | herda (mesmo objeto) |
| Pacote, intercâmbio, pack | carregam cenas dentro do documento; não importam cena | n/a |

O inventário é fechado pelo teste `boundary::every_creation_and_rehydration_of_a_scene_is_in_the_reviewed_inventory`
(5 pontos fora de `directed_scene/`: 3 em `run.rs`, 1 em `store.rs`, 1 em `render/src/scene.rs`); um ponto novo falha
até ser revisado e listado aqui.

## Mapa de arquivos

| Arquivo | Papel |
|---|---|
| `crates/core/src/directed_scene/fx/admission.rs` | A política (`Refuse` padrão, `Accept` oculto) |
| `fx/transition.rs` | `Alignment`, `Handles`, `FxTransition` |
| `fx/transition_catalog.rs` | `CROSS_DISSOLVE` (família `transition`, `curve` Choice, `intensity` 0..1), `transition_catalog()`, `transition_effect()` |
| `fx/window.rs` | `transition_window`, `check_duration`, `raw_window` (i64 verificada, `div_euclid`), `max_transition_frames`, `default_transition_duration`, `available_frames` |
| `fx/transition_read.rs` | Leitura estrita (passo 1 → sink; passos 2-3 → `early`), `FX_LIMIT` antes das entradas |
| `fx/transition_explain.rs` | Passos 4-11 pelo oráculo, sobreposição, avisos (`transition_warnings`) |
| `fx/mod.rs`, `fx/catalog.rs`, `fx/read.rs` | Códigos novos, `FxValue::Choice`, `ParamKind::Choice`, `Fx.transitions`, `WRONG_FAMILY` só sob `Accept`, ids compartilhados |
| `directed_scene/{mod,parse,read}.rs` | Campo `admission`, importadores `*_admitting`, `reimport`, estágio 3, leitores de inteiro estrito |
| `scene_edit/{fx,batch}.rs` | Candidato com `reimport` (`fx.rs` com delta líquido 0) |
| `scene_plan/json.rs`, `scene_edit/fx_animation.rs`, `desktop/.../scene_fx_keys.rs`, `tests/scene_fx.rs` | Só os braços novos dos `match` exaustivos (veja a extensão de ownership) |

## Evidência por superfície

| Superfície | Prova | Resultado |
|---|---|---|
| **L** (biblioteca) | `crates/core/tests/scene_fx_transition.rs` (`document`, `corpus`, `public`, `boundary`, `support`): 46 testes | 46 ok |
| **L** em crate | `scene_project/admission_tests.rs` (política mantida em `scene.fx.edit`, `scene.edit`, desfazer e refazer; `Refuse` nunca vira `Accept`) | 2 ok |
| **H** (headless real) | `public::a_real_headless_process_refuses_transitions_and_keeps_its_session`: processo `tessel-headless --empty`, saída **1**, respostas iguais às de `headless::run_session` (id do projeto e sessão normalizados); `scene.attach` com bundle e com texto → `invalid_bundle` / `FX_TRANSITION_NOT_YET`; `scene.list`, `status` (revisão) e `history.status` intactos; catálogo = 3 looks | ok |
| **G** (GPUI) | `workstation/scene_fx_transition_tests.rs`: anexar pelo handler compartilhado é recusado, cenas/revisão/histórico intactos, seção «Efeitos» igual, 3 ações `add:`, sem `crossDissolve`, sem controle novo | 2 ok |
| **P/R/E** | Nada novo: um bundle sem transição tem o mesmo `Fx` e o mesmo `to_json` (`a_scene_without_transitions_is_bit_identical_to_before`) | ok |

Células de F064 cobertas: **T01** (ida e volta em bytes e idempotência), **T02** (cada recusa, na ordem do contrato),
**T03** doc (avisos `ON_CONTINUOUS` e `SPLAT_COST`), **T05** (janela, bordas), **T28** doc (limites), **T32** (portão,
negativo) em L, H e G. O corpus `dissolve-cases.json` é lido por Rust (32 `explain`, 12 `handles`, 15 `bounds`,
15 `max_transition_frames`, 30 `raw`, `strict_inputs`, `ids`, `limits`, `keys_hidden_seed`), sem Python e sem tocar
oráculo, fixtures ou tolerâncias; nenhum golden derivado da implementação.

### Mutantes detectados (aplicar, rodar, reverter)

| Mutante | Testes que falham |
|---|---|
| Importador público com `Accept` | 5 (`attach_and_edit...`, `every_public_importer...`, processo real, `boundary::inside_the_importer...`, `a_family_mix_up...`) |
| `reimport` com `Refuse` fixo (candidato perde a política) | `the_edit_candidate_inherits_the_policy_of_its_scene` |
| Piso → teto (`div_euclid(2)` → `(D+1)/2`) | 4 (janela ímpar, raw, bounds, janela e corte) |
| Família confundida (`WRONG_FAMILY` → `UNKNOWN_EFFECT`) | 3 |
| `seed` flutuante coagido (`opt_strict_int` → `opt_int`) | `seeds_keep_their_exact_uint32_value_and_never_coerce` |
| Sem `check_duration` antes da conta | `window_bounds_agree_with_the_oracle` |
| Duração inválida segue para a janela (sem `continue`) | 2 |
| Sobreposição `<` → `<=` (tocar = sobrepor) | `windows_that_touch_do_not_overlap...` |
| `hidden: false` aceito | 4 |
| `FX_LIMIT` depois de ler as entradas | 3 |
| Alcance de câmera ignorado nos handles | 4 |

Não houve mutante sobrevivente entre os listados. O mutante «`#[default]` removido de `Refuse`» não compila (o
`derive(Default)` o exige), então a regressão de padrão é impedida pelo compilador e por `the_default_policy_is_refuse...`.

## Divergências conhecidas do oráculo (declaradas, cada uma com teste)

1. **Dissolve legado** (`spec+legacy-dissolve`): o oráculo reporta `TRANSITION_DUPLICATE`; o importador recusa o
   dissolve legado antes (`DISSOLVE_UNSUPPORTED`). Teste: `corpus::known_divergences` e `document`.
2. **Id reservado e duplicado** (`fx`): o oráculo lista `RESERVED_ID` e `DUPLICATE_ID`; o core dá só `RESERVED_ID`.
3. **`variant-only-the-camera-is-short`:** no Rust a instância `inst-b` ganha 70 quadros (o importador exige que ela
   cubra o plano com `instanceStartFrame` 10).
4. As recusas de efeito/família/parâmetro/easing/enums (passos 2-3) só aparecem depois que o bundle é estruturalmente
   válido. Um erro estrutural em qualquer transição também oculta as questões semânticas iniciais de transições
   anteriores. A lista/primeiro erro pode diferir do oráculo; o veredito de recusa é o mesmo.
5. `hidden: false` em um **look** já era recusado por `opt_true`; mantido. Nas transições é `INVALID_VALUE` explícito.
6. `fx` sem a chave `transitions` continua `MISSING_FIELD` (comportamento anterior, igual nas duas políticas).

## Extensão de ownership (justificativa)

Os `match` exaustivos sobre `ParamKind` e `FxValue` exigiram um braço novo para `Choice` fora de `fx/`:
`scene_plan/json.rs` (recusa tipada, inalcançável hoje: o plano ainda não carrega transição), `scene_edit/fx_animation.rs`
(`"choice"`), `desktop/.../scene_fx_keys.rs` (`value_text`) e `crates/core/tests/scene_fx.rs` (`unreachable!`). São
adaptações mínimas, sem habilitar controles novos. A auditoria do diff corrigiu a identificação
do arquivo: o crescimento de 1364 para 1365 linhas estava em crates/core/tests/scene_fx.rs,
não em scene_fx.rs da UI. scene_fx_keys.rs tinha 719 linhas, abaixo do teto. O coordenador
moveu o teste existente de contrato do catálogo/runtime para scene_fx/runtime_catalog.rs,
reduzindo o arquivo principal e preservando as mesmas asserções e o mesmo alvo Cargo.
Nenhuma fonte de produção mudou depois do head bb43b16; scene_edit/fx.rs mantém delta líquido 0.
A movimentação de teste é posterior aos gates do implementador e aguarda compilação Windows.

## Gates (Windows x64, Rust 1.99.1, wrapper `./scripts/cargo`; sem lock de GPU: nada mede desempenho)

| Gate | Saída |
|---|---|
| `cargo fmt --all -- --check` | 0 |
| `./scripts/cargo check --workspace --locked` | 0 |
| `./scripts/cargo clippy --workspace --all-targets --locked -- -D warnings` | 0 |
| `./scripts/cargo build -p tessel-editor --locked` | 0 |
| `./scripts/cargo tree -p tessel-editor-core --locked` | 0; **0** ocorrências de `gpui`/`wry` |
| `./scripts/cargo test -p tessel-editor-core --locked --no-fail-fast` | 101: 60 binários ok; **2 falhas externas** (abaixo) |
| `… --test scene_fx_transition --test scene_fx` (autor, antes da movimentação em72c9fe2) | 0 (46 e 25 ok) |
| `-p tessel-editor --bin tessel-editor scene_fx` (GPUI, 80 testes, 2 novos) | 0 |
| `./scripts/cargo test --workspace --locked --no-run` | 0 |

**Falhas externas, também na base** (conferido com `git stash` e `bb309a7`, mesmos testes, mesmas falhas):
`scene_gizmo_rotation::the_rotate_overlay_has_three_closed_rings_and_the_metrics_to_draw_them` (diferença de último
bit em `v`, 0,6491406061697664 contra …65) e `scene_gizmo_scale::four_parents_of_anisotropic_scale_with_a_rotation_between_them_refuse_the_world_shear`.
Não foram mascaradas nem corrigidas aqui.

## A validar / lacunas (mapeamento BL167.6)

- A suíte do workspace inteiro **não** foi executada (só `--no-run`): a suíte de `media-claims` e as de render/Chrome
  dependem de W1 (PR59) e W2 (PR60), que não estão em `main`; ficam **A validar** por suíte no CI.
- As provas vivas (`scripts/ui`, janela real) não se aplicam: a S1a não adiciona controle, nó nem diálogo.
- Nada aqui mede desempenho: nenhuma afirmação de latência, FPS ou custo.
- `TRANSITION_SPLAT_COST` usa um nó `splat` montado no teste (nenhum fixture do repositório tem splat).
- `T02` em H/G (entradas pelas superfícies), e todas as demais células, seguem como no F064.

## Reconciliação pelo coordenador

O implementador não entregou as linhas prometidas na resposta final. O coordenador integrou
BL082.4.2 e a nota de F064 nesta PR, com status **A validar** e todas as superfícies de produto
P: o fluxo novo só existe pela entrada interna de ensaio, enquanto o fluxo público recusa.
As evidências anteriores ficam atribuídas a 5e21e0/bb43b16; o patch do coordenador inclui
movimentação de teste e aguarda rechecagem no Windows, sem herdar aqueles gates.

Os dois commits publicados pelo implementador têm trailers de coautoria injetados pelo wrapper.
Não foram reescritos nem enviados com force. A integração, se aprovada, usará squash com mensagem
explícita sem atribuição a assistentes. Isso não é aprovação da PR nem remove o histórico da branch.


## Revisão independente de72c9fe2 e correções posteriores

A revisão **não aprovou** o head: o coordenador moveu o teste de catálogo mas esqueceu o
atributo path na raiz do teste de integração. E0583 impediu scene_fx, clippy all-targets e
workspace --no-run. A correção é #[path = "scene_fx/runtime_catalog.rs"] antes do mod.
A revisão aplicou essa linha temporariamente: scene_fx25 e clippy passaram, e o core completo
ficou só com os dois gizmos da base. Isso é prova com patch temporário, não gate do head limpo.

No72c9fe2, a revisão executou scene_fx_transition46, dois testes de admissão em crate e
GPUIscene_fx80 com sucesso, além de probes próprias de12.474 combinações de janela pori128,
reload/open/attach-file e preservação de Accept em no-op/dry-run/remover último look/undo/redo.
Não encontrou bypass de admissão nem erro real da janela/validação. Restam coberturas a tornar
permanentes: caminhos dinâmicos de abertura e arquivo, TOO_LONG assimétrico e ordem cronológica
inversa. A revisão matou mutantes adicionais e identificou dois sobreviventes nesses casos.

Outro achado válido: a ordem das mensagens de parâmetros dependia de serde_json/preserve_order,
ligada no workspace mas não no core isolado. O coordenador fixa, **somente nas transições**,
a ordem lexical das chaves de params, sem alterar o documento ou os valores no catálogo.
É divergência explícita da ordem de autoria da origem; evita variar diagnósticos conforme o
chamador sem ativar uma feature global de serde_json. O comportamento preexistente dos looks
fica em BL082.4.13. Novo teste cobre três ordens de autoria e importação Value/texto, a executar
nos dois perfis Cargo. Nenhum oráculo foi alterado para concordar com a implementação.

O scanner é auxiliar textual, não barreira. A revisão demonstrou dois pontos cegos: exclusão
por nome *_tests.rs sem verificar cfg(test), e chaves em strings de um item cfg(test) engolindo
código posterior. Os arquivos atuais não abrem o gate. Fortalecimento e testes negativos ficam
no aceite de BL082.4.2, junto aos testes dinâmicos permanentes, antes de encerrar a fase.
A pré-condição da variante only-the-camera-is-short do oráculo fica em BL082.4.14.

A correção de path e a ordem lexical são posteriores às provas acima. Mac só tem fmt e checagens
documentais; compilação e regressões requerem nova execução Windows. A PR continua A validar,
sem runtime/engine/produto público habilitado e sem merge.
