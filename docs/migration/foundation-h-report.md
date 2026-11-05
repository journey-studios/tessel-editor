# Relatório da frente H — plano avaliado de frame de cena dirigida (BL006 / BB05 / F037)

2026-11-05. Rodada H/I/J/K ([plano](next-round-plan.md)). Checkout compartilhado `tessel-editor`,
base `2bb036b`. Origem lida só para conferir o vocabulário do schema (`widthSceneUnits`,
`heightSceneUnits` obrigatórios em `imagePlane`); nada foi editado nela nem em `tesselapp`.
Contrato da feature: [`docs/features/scene-plan.md`](../features/scene-plan.md) (F037).

**Esta frente não compilou nada.** Não rodou Cargo, rustc nem clippy; só `rustfmt` nos próprios
arquivos (que também confirma que a sintaxe parseia). Nenhum teste foi executado. Não fez
`git add`/commit/push/PR, não criou branch/worktree, não tocou `core/src/lib.rs`, `media/src/lib.rs`,
`Cargo.toml/lock`, `docs/backlog.md`, matriz nem índices. Nenhum subagente.

## Arquivos (ownership exclusivo)

| Arquivo | Conteúdo |
|---|---|
| `crates/core/src/scene_plan/mod.rs` | Documentação do módulo, constantes `PLAN_KIND`/`PLAN_VERSION`/`OUTPUT_COLOR_SPACE`, reexports |
| `crates/core/src/scene_plan/model.rs` | `ScenePlan` e partes, `Capability`, `Domain`, `UnsupportedDomain`, `UninterpretedField`, `Capabilities`/`capabilities()` |
| `crates/core/src/scene_plan/build.rs` | `ScenePlan::build`/`build_with`, `PlanOptions`; domínios não avaliados, camadas, recursos, campos não interpretados |
| `crates/core/src/scene_plan/validate.rs` | `ScenePlan::validate` e `derive_requires` |
| `crates/core/src/scene_plan/json.rs` | `to_json*` e `from_json*`, escritos à mão |
| `crates/core/src/scene_plan/error.rs` | `PlanError`, `code()`, `details()`, `From<PlanError> for CommandError` |
| `crates/core/tests/scene_plan.rs` | 29 testes públicos |
| `docs/features/scene-plan.md`, este relatório | docs |

## Implementação

- `ScenePlan::build(&DirectedScene, i64)` chama `Bundle::evaluate_frame` e usa `SceneGraph` e
  `Bundle::paint_order`; **não há** câmera, animação, matriz nem ordenação reimplementada. Os tipos
  do avaliador (`CameraState`, `NodeAnimation`, `Vec3`, `Mat4`, `Fps`, `AssetAlpha`...) entram no plano sem
  cópia semântica, para o plano não divergir do que `evaluate_frame` devolveu.
- Domínios declarados e não avaliados são **erro tipado** (`PlanError::Unsupported`, ordenado, com o
  nó): a lista de cena vem de `unevaluated_sections()`; `rigBind`/`reveal` idem, com o nó lido do
  documento; mapas de dados (`surface`, `normalMap`, `sceneDepth`, `relief`) e nós `splat` são
  acrescentados porque mudam o aspecto do quadro mesmo sem variar no tempo. `allow_partial` devolve o
  plano do que é avaliado, marcado incompleto.
- Camadas de trás para frente, recursos por `assetId` em ordem bytewise, `requires` derivado do
  conteúdo, versão e capacidades. `validate()` roda em todo `build` e em toda leitura.
- Campos preservados e não interpretados (`UNKNOWN_FIELD`, seções cruas) vão para `uninterpreted`, com
  escopo limitado ao que o frame usa (outra cena do bundle não altera o plano).
- Serialização manual porque `serde` do core não habilita `derive`. Chave desconhecida, campo ausente
  e versão ≠ 1 são recusados com ponteiro; a versão é conferida antes dos demais campos.

## Testes: escritos × executados

| | |
|---|---|
| **Escritos** | 29 em `crates/core/tests/scene_plan.rs`: igualdade plano × `evaluate_frame` nos 4 bundles válidos do corpus (4 frames cada, bit a bit); hierarquia/socket/canais; tempo exato 24000/1001; camadas com passe de composição, empate e ordem; transparência; recursos estáveis independentes de caminho/ordem dos assets e compartilhados; sem caminho absoluto; recusa de domínios de cena e de nó (inclui `null` = não declarado); splat; plano parcial × completo; erro de frame antes dos domínios; campos não interpretados, escopo por cena (invariância); determinismo e independência da ordem de perguntas; round-trip e texto canônico; versão/kind/campo desconhecido/ausente; 13 adulterações do plano que violam o contrato (ordem, caminho, requires, pai, ciclo, id duplicado, `complete`) e 2 de plano parcial com splat; capacidades |
| **Executados** | **Nenhum.** Sem Cargo nesta rodada. Nada aqui é "verde" |
| **Revisão manual** | Passagem linha a linha por tipos, empréstimos e inferência de fechamentos (anotei o tipo de retorno dos que usam `?`); pode restar erro de compilação |

Comando do gate (coordenador): `python3 /Users/wallyssonnunes/.cache/tessel-editor/cargo-serial.py test -p tessel-editor-core --locked --test scene_plan` e `--doc scene_plan`; depois clippy `-D warnings` e `cargo tree -p tessel-editor-core` (sem GPUI).

Pontos que, se falharem, são **expectativas de fixture e não do plano** (verificar antes de mudar código):
`valid-cut-instance-offsets` com `shots[0]` começando em 0 e `instance_frame == 10`; o contador
`channels: 3` de `NodeAnimation::Animated` no corpus `sockets-performance`; o frame 3 da cena
`layered` com `cycle` de período 12 movendo `hero-plane`; `/metadata` como única seção crua do
`layered`; os valores crus aceitos pelo importador em `weatherKeys`/`effects`/`rigs`/`paths`.

## Limites

- Não é render; sem consumidor (BL007). Recursos não resolvidos nem verificados em disco (BL009).
- Os gates `temporal_issues()`/`edit_join_issues()` não rodam no plano; o `layered_scene.json` por
  exemplo passa no plano e é recusado pelo `finalizeBundle` da origem. O adapter deve chamá-los.
- Domínios recusados: luz, clima, efeitos, rig, paths, reveal, mapas de dados, splat (BL003, BL023–BL027).
- Desempate por profundidade de câmera entre camadas de mesma ordem não modelado (limite herdado de `paint_order`).
- Custo: cada `build` refaz `NodeTimeline::new` e a lista de recursos; JSON com 2 matrizes por nó.
- Áudio/narração/legendas fora do plano (BL016/BL017).

## Contratos para integrar

1. **Coordenador — `crates/core/src/lib.rs`:** `pub mod scene_plan;` (entre `scene_edit` e `scene_project`, ordem alfabética). Sem dependência nova; a frente não usa nenhum módulo de I/J/K.
2. **Frente I:** consultar o manifesto pelos mesmos `assetId` de `ScenePlan::required_asset_ids()`
   (ordem ascendente bytewise, sem repetição). Identidade = `assetId`; `relative_path`/`revision_id`/
   `content_hash` são metadado do plano. Não há `use scene_resources` aqui.
3. **Frente J:** `Command::Scene`/`scene.*` podem expor o plano sem alterar `scene_plan`: `ScenePlan::build(&attached.scene, frame)` e `PlanError → CommandError` já existe (`code`/`details`). Nada foi acrescentado a `ops.rs`.
4. **Frente K:** `scene_plan` não muda `evaluate_frame`; qualquer ajuste das tolerâncias do oráculo continua válido.
5. **Renderer (BL007):** executa só o que está no plano, confere `plan.require_capabilities(...)` e `is_complete()`, e rejeita `uninterpreted` apenas como informação.

## Patches propostos (o coordenador integra)

### `docs/features/README.md` — linha e nota

```markdown
| F037 | [Plano avaliado de frame de cena dirigida (BB05)](scene-plan.md) | P | P | P | I¹⁴ | `scene_plan::ScenePlan::{build, build_with, validate, to_json, from_json_str}`; domínios não avaliados recusados; sem renderer/CLI/UI |
```

```markdown
¹⁴ Plano tipado e serializável do frame (câmera, nós e matrizes, ordem de pintura, recursos por
`assetId`, versão e capacidades) sobre `Bundle::evaluate_frame`; luz, clima, efeitos, rig, paths,
`reveal`, mapas de dados e splat são recusados com erro tipado. 29 testes escritos, **não
executados** até o gate do coordenador. Sem consumidor (BL007), sem resolução de recursos (BL009).
```

(Atualizar para `V` na coluna Lib somente depois de `cargo test`, clippy e doctest passarem.)

### `docs/backlog.md`

BL006 (BB05) — trocar a linha por:

```markdown
| BL006 | BB05 | Definir plano avaliado de cena e contrato core → runtime, versão/capabilities | A validar | BB02/BB03 existentes | Frente H (F037, [relatório](migration/foundation-h-report.md)) | `scene_plan` implementado e testado só por escrita; falta gate Cargo. Core resolve o frame e recusa domínios não avaliados; renderer (BL007), `scene.plan` (BL063) e recursos (BL009) pendentes. Não encerra BB05 |
```

Novas linhas (IDs sugeridos; renumerar se houver colisão):

```markdown
| BL063 | BB05/Adapters | `scene.plan` na lib/JSONL (e depois UI) sobre `ScenePlan`; teste de integração plano × manifesto de recursos (todo `required_asset_ids()` é conhecido de I) | Pendente | BL006, BL009 | — | Mesmo parser e `PlanError → CommandError` nas superfícies; recusa de domínios no JSONL; recursos do plano ⊆ manifesto |
| BL064 | BB05 | Plano por frame sem refazer a linha do tempo (`PlanTimeline`: monta uma vez, pergunta muitos frames); medir custo e tamanho do JSON | Pendente | BL006, BL007/BL008 | — | Mesma saída bit a bit que `ScenePlan::build`; orçamento medido no export curto, não presumido |
| BL065 | BB05 | Estender o plano a domínios hoje recusados (mapas de dados, luz, clima, efeitos, rig/paths/reveal, splat em cena) e ao desempate de camadas por profundidade | Pendente | BL003, BL023–BL027 | — | Cada domínio entra com estado completo e prova; `unsupported` só encolhe junto com `PLAN_VERSION`/`Capability` novos |
```

BL003, BL023–BL027: sem mudança de estado; acrescentar à coluna Depende de/Critério que a recusa
está em `scene_plan` (`Domain`) e deve ser removida junto com a avaliação do domínio.
BL007 (renderer) passa a ter o contrato: "consome `ScenePlan`, confere `require_capabilities`".

### Índices

`docs/README.md`/índices de features, se listarem por feature: acrescentar F037 → `features/scene-plan.md`. Esta frente não editou nenhum.

## Recebimento pelo coordenador

Export scene_plan e F037 adicionados; BL006 permanece A validar até o gate conjunto. Nenhum teste Cargo executado neste recebimento porque os módulos compartilhados J/L ainda estão em edição. BL063–BL065 são os IDs reconciliados no backlog; BL062 é o bug de undo dos sliders.

Validação posterior: 29 testes públicos e doctest passaram; clippy lib limpo. F037 Lib V para o subconjunto documentado, sem consumidor renderer/CLI/UI.
