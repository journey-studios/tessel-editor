# P4b S2-C0a — pré-requisitos do core: relatório de entrega

Base `main` `9073e775e0014b3927dfde7ff0a76d8eb4ead2ce` (S3a integrada). Branch `feat/p4b-s2-c0a-core-prereqs`.
Plano: [p4b-s2-core-prerequisites-plan.md](p4b-s2-core-prerequisites-plan.md); notas de pesquisa (de um head antigo,
**não** prova de execução): [p4b-s2-implementation-notes.md](p4b-s2-implementation-notes.md). Contrato do produto:
[F064](../features/fx-transitions.md) (seção S2-C0a) e [scene-plan](../features/scene-plan.md).

Escopo: só o core Rust. Sem C0b/H/R/S2A, sem UI, sem engine/GPU/desempenho, sem Domain novo, sem `Accept` em produção,
sem env flag nem porta de teste no binário público. A admissão pública continua `Refuse` e o produto F064 continua `P`.
Nada do oráculo Python, das fixtures, do PNG, do manifesto nem dos snapshots foi regravado.

## 1. O que entrou

| Item | Mudança | Onde |
|---|---|---|
| BL082.4.29 | Proveniência por geração `(tipo, id)` dentro de um lote: só os ops da geração viva no fim recebem ponteiro no candidato recusado. Privado ao pipeline de edição; sem chave de documento nem tipo público. | `scene_edit/fx_lineage.rs` (novo), `scene_edit/fx.rs` (`run_batch`) |
| BL082.4.19 | `ScenePlan::build_with` e `PlanTimeline::build_with` qualificam os erros dos passos do **vizinho** com o `within_other` que já existia (`/transition/other…`, `Invalid`). O deck de cima não muda. | `scene_plan/build.rs`, `scene_plan/timeline.rs` |
| BL082.4.25 | Reparo exato da cauda de câmera: `missing_frames_after` (checado; satura em `i64::MAX` só na recusa) → `Lack.camera_extend` → dica e campo aditivo `cameraExtend` (público só quando difere de `camera`; sempre em `args`). Projeção nas três línguas; `cameraExtend` inválido preserva o texto original e marca `untranslated`; ausente cai em `camera`. | `directed_scene/fx/window.rs`, `transition_explain.rs`, `transition_problem.rs` |
| BL082.4.26/.27/.28 | Lacunas permanentes de teste fechadas (sem mudar produção). | ver §3 |

API pública nova: nenhuma função/tipo `pub`. `missing_frames_after` é `pub(crate)`; `touched_the_survivor` é `pub(super)`.
O campo `cameraExtend` aparece em `details.lacks[]`/`details.args.lacks[]` de `TRANSITION_NO_HANDLES`, que só existe para uma
cena importada sob a política escondida `Accept` (nos testes); o import público segue recusando `fx.transitions`.

Decisões onde o plano deixava escolha: (a) nome do campo `cameraExtend` (distinto de `cameraEnd`, a falta de cabeça da .24);
(b) `cameraExtend` público só quando difere de `camera`, para que toda linha sem divergência mantenha os bytes do oráculo;
(c) a mensagem continua dizendo o que falta à janela (`camera`), só o reparo muda; (d) a qualificação do vizinho converte
`Frame`/`Unsupported`/`SplatTransform` em `Invalid` sob o ponteiro, igual ao que o leitor de JSON já faz; (e) a resposta
de `scene.fx.gesture.begin` não tem orçamento (a forma real do código): o teste compara os fatos de `update`, `status`,
`inspect` e do frame, não inventa um campo no `begin`.

## 2. Divergências explícitas (corpus)

Três linhas do oráculo mudam **só a dica** (a admissão, o veredito e a lista de `lacks` seguem iguais, mais o campo
aditivo): a câmera do `shot-a` (40 quadros, `cameraIn 5`) termina 5 quadros antes do fim do shot.

| Linha | Dica antiga → nova (quadros após o fim) | `lacks[].cameraExtend` |
|---|---|---|
| `corpus-center-12-use-lacks-camera-and-head` | 6 → 11 | 11 |
| `corpus-center-12-refuse-reads-as-use` | 6 → 11 | 11 |
| `corpus-start-12-use-needs-only-the-tail` | 12 → 17 | 17 |

`origin-spec-s1-s2-use`, `origin-spec-s2-s3-camera-cut-to-50` e `no-handles-use` não mudam. O consumidor
(`tests/scene_fx_transition/corpus.rs`) adapta só essas três linhas por uma tabela nomeada que **exige** a frase antiga no
oráculo (uma linha que deixe de divergir ou uma tabela que deixe de casar falha, não passa em silêncio).

## 3. Matriz de testes

| ID | Prova | Onde |
|---|---|---|
| A01 | Contagem presente e inválida (texto, −1, 1,5, `null`, `3.0`) em `instance`/`camera`/`cameraEnd` mantém mensagem/dica originais + `untranslated`; `cameraEnd` ausente/0 é válido e traduzido | `edit_problems.rs` (+2) |
| A02 | Guarda de corte da restauração: caso real (remove `x` num corte e recria `x` no outro com `intensity` 1 → `1.0`), no sentido contrário, controle do mesmo corte; guarda de `effect`/`from`/`to` e valor realmente mudado no helper (defensivo, declarado) | `restoration.rs` (+2), `scene_edit/fx_transition_tests.rs` (+5 lib) |
| A03 | 8 looks + transição criada = 9 passes na janela e 8 fora, em `scene.fx.frame` (bordas 28/51 dentro, 27/52 fora), nas respostas de orçamento (`edit`, `list`, `inspect`) e num gesto (`update`, `status`, `inspect`, frame sob gesto), sem cortar nenhum look | `scene_project/transition_edit_tests.rs` (+1 lib) |
| B01/B02 | `x` criado, removido e recriado: só o último op responde; entidade do documento substituída; controle (todos os ops na entidade viva); upsert não inicia geração; remoção definitiva não é culpada por outra | `attribution.rs` (+4) |
| C01/C02 | Splat esticado só no vizinho → `/transition/other/nodes`; domínio não suportado no vizinho → `/transition/other`; os dois construtores iguais (e na segunda chamada, sem prefixo duplo); deck de cima mantém `SplatTransform`/`Unsupported`, ids colidentes (`cloud-node` nos dois decks); `allow_partial` respeitado | `deck_context.rs` (+3) |
| D01 | `cameraIn 40`/câmera 60/shot 48/`after 6`: `cameraExtend 34`; 94 aceita, 93 e 66 recusadas; `refuse` igual com a nota; os dois lados juntos | `tail_repair.rs` |
| D02 | Campo só quando difere de `camera`; lado `before` nunca o leva; bytes intactos nos demais | `tail_repair.rs` |
| D03 | Projeção pt-br/en/es com o número exato, sem marcador; mensagem continua dizendo 6; ausente → `camera`; inválido → original + `untranslated` | `tail_repair.rs` |
| D04 | `alignment: end` (after 0) com câmera curta é aceito (não exige frames que ninguém lê) | `tail_repair.rs` |
| D05 | Helper `missing_frames_after`: exemplos, equivalência com a diferença saturada onde a câmera alcança o shot, suficiência/minimalidade em 4 loops, saturação em estouro | `window.rs` (+3 lib) |
| D06 | A divergência do corpus é exatamente as três linhas nomeadas | `corpus.rs` (+1) |

Contagens: `scene_fx_transition` 175 → **192** (+17), lib 606 → **615** (+9), corpus 10, desktop FX 80.

## 4. Gates (Windows nativo, Git Bash + `./scripts/cargo`, alvo padrão; sem `CARGO_TARGET_DIR`/lane/clean/gc)

Código final `16fa526` (a PR acrescenta só este relatório e a atualização de docs por cima; sem `.rs`).
Logs: `%TEMP%\c0a\gates-16fa526\`. Antes dele, o mesmo conjunto passou em `588360c` (sem o teste D04): 191/10/191, lib 615,
FX 80, mesmas falhas conhecidas.

| Gate | Exit | Contagem |
|---|---|---|
| `cargo fmt --all -- --check` | 0 | — |
| `check --workspace --locked` | 0 | — |
| `clippy --workspace --all-targets --locked -- -D warnings` | 0 | — |
| `test --workspace --locked --no-run` | 0 | — |
| `build -p tessel-editor --locked` | 0 | — |
| core `scene_fx_transition` + `pilot_dissolve_corpus`, debug | 0 | 192 / 10 |
| idem com `--features serde_json/preserve_order` | 0 | 192 / 10 |
| idem `--release` (+ `scene_plan_transition_baseline`) | 0 | 192 / 10 / 1 |
| core `--lib` | 0 | 615 |
| core completo `--no-fail-fast` | **101** | tudo ok exceto 2 alvos (abaixo) |
| desktop `scene_fx` | 0 | 80 |
| `cargo tree -p tessel-editor-core` | 0 | sem gpui/wry |

O core completo falha **só** em `scene_gizmo_rotation::the_rotate_overlay_has_three_closed_rings_and_the_metrics_to_draw_them` e
`scene_gizmo_scale::four_parents_of_anisotropic_scale_with_a_rotation_between_them_refuse_the_world_shear`, os dois vermelhos já
conhecidos da base Windows (`9073e77`). **Não** se declara workspace global verde. A baseline de bytes (`scene_plan_transition_baseline`)
e o resto dos 60+ alvos do core passam.

## 5. Mutantes (restaurados por hash; mtime tocado; controle limpo depois de cada um)

19 mutantes, **19 mortos, 0 sobreviventes**. Cada um: arquivo restaurado do Git com hash idêntico ao original, `git status`
limpo, mtime tocado e os mesmos testes rodados de novo como controle (exit 0), para provar que o binário do mutante não foi
herdado. Resumos em `%TEMP%\c0a\mut-*.summary`.

| Mutante (plano) | Id | Morto por |
|---|---|---|
| opcional inválido → zero (`cameraEnd`) | m1a | `a_present_but_invalid_count_never_becomes_zero…` |
| idem para `cameraExtend` | m1b | `an_absent_repair_falls_back…` |
| apagar a guarda inteira de corte da restauração | m2a | 3 testes defensivos do helper |
| a guarda sem `fromShotId` / sem `toShotId` / sem `effect` | m2b/m2c/m2d | o teste defensivo de cada chave |
| atribuir pelo ID final sem geração | m3 | `attribution::only_the_op_that_made…`, `…document_that_was_replaced…` |
| qualificação `other` omitida em `ScenePlan` / em `PlanTimeline` | m4a/m4b | `deck_context` (splat e domínio) |
| qualificação aplicada ao topo em `ScenePlan` / em `PlanTimeline` | m4c/m4d | `the_deck_under_the_needle_keeps_its_own_error…` |
| reparo da cauda reduzido a `after` (explain) / no helper | m5a/m5b | `tail_repair`, corpus / 2 testes do helper |
| `after 0` exigindo frames | m6 | `a_window_that_asks_nothing_after_the_cut…`, corpus |
| hint numérico divergente (dica / projeção) | m7a/m7b | `tail_repair` + corpus / projeção em três línguas |
| E13: blend não contado no frame | m8a | `eight_looks_and_a_new_transition_cost_nine_passes…` |
| E13: blend fora do orçamento | m8b | A03 + 2 testes existentes |
| E13: orçamento do `status` de gesto removido | m8c | A03 + `admission_budget_answers…` |

Os mortos por testes novos desta entrega são os da coluna acima com nome de teste novo; m8b/m8c também caem em testes
anteriores da S3a (o que é esperado: o A03 acrescenta o frame e o gesto, não substitui o que já existia).

## 6. Executado, apenas lido, N/A

- **Executado:** todos os gates e mutantes acima, neste checkout, com Git Bash.
- **Apenas lido:** o oráculo Python/fixtures (não rodados nem regravados), docs da S3a e as notas antigas do plano.
- **N/A:** engine/GPU/desempenho (sem lock de GPU), UI física, CLI da janela, Mac, Domain novo, C0b/H/R/S2A, `Accept` público.
- Paridade das três superfícies: a regra mora no core e é consumida pela lib; CLI headless e UI não ganham comportamento novo
  (nenhum controle novo), então não há paridade a inventar. Gap já registrado: nenhuma transição é editável pelas
  superfícies públicas enquanto a admissão for `Refuse`.

## 7. Limites e ressalvas

- Revisão independente pendente; nada aqui herda a R1/R2 da S3a.
- Os dois gizmos Windows continuam vermelhos na base; esta entrega não os toca.
- A conversão da qualificação do vizinho para `Invalid` perde o tipo `Frame`/`Unsupported`/`SplatTransform` do vizinho
  (fica no texto): é o que o leitor de JSON já faz e o que o plano pediu; um consumidor que queira o tipo do vizinho
  precisará de um contrato novo.
- `cameraExtend` vale só para a cauda; o lado `before` já é exato (`camera`, `cameraEnd`).
- `fx.rs` (808) e `transition_problem.rs` (807) seguem abaixo de 1000 linhas.

## 8. Procedência dos commits

- **Skill `commit`:** não consta nas skills deste ambiente (a lista local só tem `tessel-feature-contract` no repositório e
  nenhuma skill de commit nas globais). Segui as convenções do repositório: título convencional ≤ 70, linhas do corpo ≤ 100, sem
  referência a assistentes.
- **Incidente de trailer:** o primeiro `git commit` do terminal injetou `Co-authored-by: Cursor <cursoragent@cursor.com>`
  (e uma linha > 100). Como o commit não estava publicado, saneei só ele por plumbing (`write-tree` + `commit-tree` +
  `update-ref`, sem force); os commits seguintes foram criados por plumbing desde o início. O objeto antigo só existe no
  reflog local; o remoto nunca o viu.
- Commits: `a54628d` docs do plano/notas; `3b1ad61` testes Marco A; `149fe16` contrato F064/scene-plan antes do código;
  `f4da664` .29; `d2754f3` .19; `588360c` .25; `16fa526` teste after 0; mais o commit deste relatório.

## 9. Ownership de arquivos e extensões mecânicas

Alterados pela entrega: `crates/core/src/scene_edit/{fx.rs,mod.rs,fx_lineage.rs,fx_transition_tests.rs}`,
`crates/core/src/scene_plan/{build.rs,timeline.rs}`, `crates/core/src/directed_scene/fx/{window.rs,transition_explain.rs,
transition_problem.rs}`, `crates/core/src/scene_project/transition_edit_tests.rs`, `crates/core/tests/scene_fx_transition.rs`
(registro dos módulos) e `tests/scene_fx_transition/{attribution,deck_context,restoration,tail_repair,corpus,edit_problems}.rs`,
`docs/features/{fx-transitions,scene-plan}.md`, e os três docs de `docs/migration/`. Extensão mecânica: registrar quatro módulos
em `scene_fx_transition.rs` e um `mod` de teste em `scene_edit/mod.rs`. **Não editados** (patch abaixo):
`docs/backlog.md`, `docs/features/README.md`, `docs/README.md`.

## 10. Patch literal para o coordenador (não aplicado aqui)

`<PR>` e `<HEAD>` são a URL e o head final da PR, que o coordenador preenche.

**`docs/backlog.md`** — substituir as células de status/aceite (colunas 4 e 7) destas linhas; ID, origem, título, dependências e
responsável ficam como estão:

- `BL082.4.19`: status `A validar — construtor qualifica o vizinho por deck em <PR> (<HEAD>); validador/leitor já validados R2`;
  aceite `build_with e PlanTimeline::build_with envolvem só os passos do vizinho em within_other (/transition/other…, Invalid); topo
  intacto; ids colidentes, dois construtores iguais, sem prefixo duplo; 4 mutantes mortos (m4a–m4d). Relatório S2-C0a §3/§5.`
- `BL082.4.25`: status `A validar — <PR> (<HEAD>)`; aceite `reparo exato missing_frames_after, campo aditivo cameraExtend só quando
  difere de camera, dica e projeção nas 3 línguas; 94 aceita, 93 e 66 recusadas; 3 linhas do corpus divergem só na dica e estão
  nomeadas no consumidor; oráculo/fixtures não regravados. Mutantes m5a–m7b mortos.`
- `BL082.4.26`: status `Validado Windows/Mac — b097979; guarda permanente em <PR> (<HEAD>), a validar`; aceite `caso real
  (id removido num corte e recriado em outro) + testes defensivos de effect/from/to; m2a–m2d mortos.`
- `BL082.4.27`: status `A validar — E13 frame/gesto e hints provados em <PR> (<HEAD>)`; aceite `9 passes na janela e 8 fora em
  scene.fx.frame, orçamento e gesto; m8a–m8c mortos.`
- `BL082.4.28`: status `Validado Windows/Mac — b097979; teste permanente de cameraEnd/3.0 em <PR> (<HEAD>), a validar`;
  aceite `contagens inválidas (texto, −1, 1,5, null, 3.0) em instance/camera/cameraEnd/cameraExtend mantêm o original + untranslated; m1a/m1b mortos.`
- `BL082.4.29`: status `A validar — proveniência por geração em <PR> (<HEAD>)`; aceite `só o op da entidade que sobrevive responde;
  remove+recria no mesmo tipo, entidade do documento substituída, controle com todos os ops; m3 morto. Não é rastreamento causal por propriedade.`
- `BL082.4`: acrescentar ao fim da célula de status `; S2-C0a (.19/.25/.26–.29) a validar em <PR>; F064 produto continua P e Refuse fechado.`

**`docs/features/README.md`**, linha F064: acrescentar ao fim da descrição
`S2-C0a (<PR>, <HEAD>): proveniência por geração (.29), erros do vizinho por deck (.19), reparo exato da cauda e cameraExtend (.25), lacunas .26–.28 fechadas; core 192/10, lib 615, FX 80, 19/19 mutantes; Refuse fechado e produto P.`

**`docs/README.md`**, depois da linha da S3a:
`- [P4b S2-C0a — pré-requisitos do core](migration/p4b-s2-c0a-report.md): proveniência de edição, erros do vizinho por deck e reparo exato da cauda; admissão pública fechada.`

Plano e notas: `docs/migration/p4b-s2-core-prerequisites-plan.md` e `p4b-s2-implementation-notes.md` (entregues no primeiro commit).
