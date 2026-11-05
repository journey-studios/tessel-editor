# Relatório da frente F — BB03 animação de nós

2026-11-05. Rodada 2 da base de migração ([plano](parallel-implementation-plan.md), [roadmap](phased-roadmap.md) BB03).
Checkout compartilhado `tessel-editor`, branch `feat/migration-foundation`, base A–E integrada em `52f3175` (HEAD
`574f88e`). Origem lida em somente leitura (`d6adac15c61f4ef582a7ec259397b2e025d1f059`, `packages/scene-engine`). Esta frente
não fez `git add`/commit/push/merge/PR, não tocou `Cargo.toml`/`Cargo.lock`, `lib.rs`, `state`, `ops`, `document`, headless, `scene_edit`,
runtime nem `media`, não adicionou dependências, não editou matriz/índices, não instalou nem compilou nada na origem, não copiou mídia
privada e não rodou GPU/Chrome. Disco livre durante a frente: ≥ 9,9 GiB; temporários próprios < 1 MiB (as fixtures somam 0,5 MiB).

Contrato público, semântica e tolerâncias: [`docs/features/scene-animation.md`](../features/scene-animation.md) (F035).

## Entrega

Avaliação **pura e determinística**, sem GPUI, relógio ou playback, dos canais de performance `cycle`/`event`/`static` e das
sobreposições de transformação da origem, por frame **da instância**, com hierarquia, sockets e matrizes de mundo recompostas.
O estado avaliado (`NodeFrame`) é próprio, consultável e serializável (`to_json`), para um renderer futuro.

| Pergunta | API |
|---|---|
| Edit frame → câmera + nós + fronteira dita | `Bundle::evaluate_frame` → `FrameEvaluation { mapping, camera, node_animation, nodes, unevaluated }`, `is_complete`, `require_complete`, `to_json` |
| Uma instância, qualquer ordem de frames | `Bundle::node_timeline(id)` → `NodeTimeline::state_at(frame)`; atalho `Bundle::evaluate_instance_nodes` |
| Que canais aplica, há conflito | `Bundle::performance_plan(id)` → `PerformancePlan::{track_ids, channel_count, overrides_at}` |
| Estado de um nó | `NodeFrame::{nodes, node, world_matrix, overrides, override_of, to_json}`; `NodeState` (posição, Euler XYZ graus, escala, `renderOrder`, matriz local e de mundo) |
| O que existe e não é avaliado | `FrameEvaluation::unevaluated`, `SceneGraph::unevaluated_sections()`, `NodeAnimation`/`Unsupported::DeferredSections` |
| Erros | `TemporalError::{PerformanceWriterConflict, NonFiniteNode, InstanceFrameOutOfRange}` (+ os existentes), `code()`, `CommandError` |

O payload `performanceTracks[].channels[]` deixou de ser "adiado": o importador o valida e tipa
(`ChannelMotion`, `ChannelProperty`) com os códigos e JSON Pointers já usados, reporta todos os problemas de um canal e recusa o bundle
com canal inválido; campos desconhecidos seguem no documento com `UNKNOWN_FIELD` e a sobreposição de `Eq` foi removida de
`PerformanceChannel`/`PerformanceTrack` (agora `PartialEq`, há `f64`).

**Não prometido:** luz, clima, efeitos, rigs, caminhos, `rigBind`, `reveal`. Se o bundle os declara, a avaliação continua e os lista em
`unevaluated`; nada é inventado.

## Distinção por superfície

| Superfície | O que há |
|---|---|
| **Lib** | Implementada e validada contra a origem (abaixo). |
| **CLI headless** | **P.** Nenhum comando; `tessel-headless` não foi tocado. |
| **Socket** | **P.** Nenhum comando `scene.*`; a frente G é dona dos adapters. Contrato proposto: `evaluate_frame(n).to_json()`. |
| **UI** | **P.** Nenhum controle, nenhum nó em `tree.rs`; o viewport não consome o estado. |

## Arquivos

Novos (todos no ownership):

- `crates/core/src/directed_scene/animation/{mod,channels,nodes}.rs`
- `crates/core/tests/directed_scene_animation.rs` — 28 testes públicos
- `crates/core/tests/directed_scene_animation_fixtures/` — `generate.mjs`, `origin.json`, `bundles/` (31 bundles: 11 válidos, 20 recusados)
- `docs/features/scene-animation.md`, `docs/migration/foundation-f-report.md`

Alterados (dentro de `directed_scene/` e do teste consumidor):

- `model.rs` (`ChannelProperty`, `ChannelMotion`, `PerformanceChannel.motion`), `parse.rs` (validação dos canais, seções não avaliadas),
  `graph.rs` (`order`, `compose_world`, `unevaluated`), `transform.rs` (`Quat::{to,from}_euler_xyz_degrees`, `Vec3::{to_json,plus}`, +3 testes unitários),
  `temporal/mod.rs` (2 variantes de erro), `temporal/frame.rs` (`FrameEvaluation`, `NodeAnimation`, `Unsupported`), `mod.rs` (módulo, exports, `DEFERRED_FIELDS`/`DEFERRED_RULES`).
- `crates/core/tests/directed_scene_temporal.rs` (de D): o consumidor de `NodeAnimation`/`Unsupported` foi atualizado para a nova forma; as outras asserções ficaram como estavam.

**Compatibilidade.** `evaluate_camera`, `resolve_frame_mapping`, `camera_timeline` e os gates não mudaram de assinatura nem de resultado.
Mudanças de forma, todas dentro da fatia declarada como provisória pela F033: `NodeAnimation::Unsupported(PerformanceChannels{..})` → `NodeAnimation::Animated{..}`;
`Unsupported::PerformanceChannels` → `Unsupported::DeferredSections`; `FrameEvaluation` ganhou `nodes` e `unevaluated`. A frente G (adapters) usa
`evaluate_frame`/`FrameEvaluation`: se ela já ramifica em `NodeAnimation::Unsupported`, precisa passar para `Animated`/`unevaluated`.

## Evidências (comandos realmente executados)

| Comando | Resultado |
|---|---|
| `MBX_GC_MAX_TOTAL_SIZE=64GiB ./scripts/cargo test -p tessel-editor-core --locked` | verde: 183 unitários (inclui 3 novos de `transform` e os de G que já compilam), `directed_scene` 29, **`directed_scene_animation` 28**, `directed_scene_oracle` 3, `directed_scene_temporal` 19, `headless` 2, `project` 8, `proof` 8, `scene_edit` 29, `scene_project` 18 (de G), `video_sound` 7, doctests 2 |
| `./scripts/cargo clippy -p tessel-editor-core --all-targets --locked -- -D warnings` | sem avisos |
| `rustfmt` nos arquivos da frente / `cargo fmt --all -- --check` | arquivos da F limpos; resta **um** diff em `crates/core/src/scene_project/mod.rs:191` (WIP da frente G, não corrigido por mim) |
| `cargo tree -p tessel-editor-core --locked` | nenhum `gpui`, `wry`, `tao` ou `winit` |
| `node …/directed_scene_animation_fixtures/generate.mjs` e `… verify` | 31 variantes, 33 fontes conferidas contra os blobs de `d6adac15…` antes de gravar; `verify` reexecuta a origem e confirma byte a byte |
| `node scripts/migration-oracle/oracle.mjs verify` e `node --test scripts/migration-oracle/oracle.spec.mjs` | 55 casos verificados; 21/21 testes |

Equivalência com a origem (as expectativas vêm de `renderBundleSchema`, `finalizeBundle`, `evaluateEditFrame` e `computeWorldMatrices` executados via oráculo):

- 4 bundles do oráculo: 30 frames + 8 erros; `valid-graph-sockets-performance` (B) nos 12 frames sondados — nós, overrides (ordem e valores) e matrizes de mundo.
- 11 variantes sintéticas: 122 frames + 19 erros (bordas de cycle/event, combinações, lista permitida, conflitos, 2 instâncias com offsets e seek fora de ordem, gimbal, campos desconhecidos).
- 20 payloads recusados: mesma contagem de problemas e mesmos caminhos.
- 20 373 números comparados; **maior erro 7,1e-15** contra tolerância `1e-9 + 1e-12·|x|` (fixada antes da comparação, no cabeçalho do teste).
- Controles negativos: fase errada em 1 frame e duração de event errada falham na mesma comparação; a variante intacta passa.
- Proveniência verificada em Rust: o teste confere os SHA-256 de cada bundle, do gerador e do caso-base com os registrados em `origin.json` (SHA-256 implementado no próprio teste, sem dependência nova, com teste de resposta conhecida).

## Divergências e limites

- **Não finito:** a origem devolveria `Infinity`/`NaN`; aqui é `TemporalError::NonFiniteNode` com o id (decisão deliberada, documentada).
- **Campos de "outro tipo" num canal** (ex.: `delta` num `cycle`): a origem (zod) os descarta em silêncio; aqui são preservados no documento com `UNKNOWN_FIELD`.
- **Conflito de escrita** só aparece ao avaliar (a origem aceita o bundle em `finalizeBundle`); `performance_plan` permite checar antes. Não foi adicionado gate mais estrito, para manter "origem aceita ⇔ gates não acham nada".
- **Rotação em gimbal** (`|m13| ≥ 0.9999999`): reproduz o Three.js (z = 0); a rotação é preservada, o triplo Euler não. Teste de preservação com tolerância `1e-6` porque `asin` perto de 1 amplifica 1e-16 em ~1e-8 rad.
- `serde_json` sem `float_roundtrip` pode mudar 1 ulp ao reler um JSON serializado; os acessores têm os `f64` exatos. Se um adapter precisar de round trip exato, a feature `float_roundtrip` é decisão do coordenador (mexe em `Cargo.toml`, fora do meu ownership).
- `Unresolved` (instância desconhecida) mantém o código herdado `invalid_camera` da F033, embora a causa não seja câmera; mudar é decisão do dono de `TemporalError` (compatibilidade).
- Sem luz/clima/efeitos/rigs/caminhos/`rigBind`/`reveal`; as `DEFERRED_RULES` correspondentes seguem sem rodar.
- `FrameEvaluation::to_json` e `NodeFrame::to_json` fixam um formato (camelCase). Nenhum adapter o consome ainda.

## Dependências e pontos para o coordenador

1. **Matriz** (proposta, número e nota a renumerar conforme as existentes):

   `| F035 | [Cena dirigida: animação de nós — canais de performance, hierarquia e matrizes de mundo (BB03)](scene-animation.md) | P | P | P | Parcial¹³ | `Bundle::{evaluate_frame, node_timeline, evaluate_instance_nodes, performance_plan}`, `NodeFrame`; adapters pendentes |`
   (mesmo formato da linha da F033; as colunas seguem a convenção vizinha da matriz.)

   Nota sugerida: ¹³ Avaliação pura dos canais `cycle`/`event`/`static` e de sobreposições de nó, com 28 testes públicos, comparada ao oráculo nos 4 casos
   válidos e em 11 variantes + 20 recusas geradas pelo código fixado da origem (20 373 números, tolerância `1e-9 + 1e-12·|x|`, frames/IDs/erros exatos). Sem
   luz, clima, efeitos, rigs, caminhos; sem comando headless/socket nem UI. [Contrato](scene-animation.md).
2. **F033** (`docs/features/scene-temporal.md`, nota ¹¹ da matriz/README e `foundation-d-report.md`) ainda dizem "animação de nós retorna `Unsupported`" e mostram
   `NodeAnimation::Unsupported(what)`; estão fora do meu ownership e **desatualizados** desde esta entrega. `docs/features/directed-scene.md` (de A) idem para
   "Avaliação de câmera/performance por frame (BB03)" em "Pendências".
3. **G** deve ler `FrameEvaluation { nodes, unevaluated }` em vez do antigo `Unsupported::PerformanceChannels`, e pode expor `to_json()` num comando de leitura sem
   duplicar regra de animação. Nenhum arquivo de G foi alterado; o único diff de formato que `cargo fmt --check` aponta hoje é em `scene_project/mod.rs`.
4. Renderer (BB05/BB06): o estado de nó está pronto para consumo, mas luz/clima/rig/pós continuam fora; um bundle que os use não é "completo" (`require_complete`).
5. Nenhuma dependência nova, nenhuma mudança em `Cargo.toml`/lock/matriz/índices.

## Revisão pelo coordenador

O gerador em modo verify passou para 31 variantes/33 fontes reais. O core passou na
execução oficial integrada. Acrescentei uma regressão para os bits das matrizes após
round-trip JSON: falhou antes e passou depois de habilitar `float_roundtrip` no
serde_json do workspace, sem mudança de versão/Cargo.lock. São 29 testes públicos
de animação agora. Documentação e consumidores foram atualizados para Animated/unevaluated.
O código herdado invalid_camera para instância desconhecida continua como lacuna de nomenclatura.
