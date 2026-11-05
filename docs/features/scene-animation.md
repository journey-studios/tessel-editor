# F035 — Cena dirigida: animação de nós (canais de performance, BB03)

ID reservado no plano (`parallel-implementation-plan.md`); a linha da matriz é proposta no relatório da
frente F e pertence ao coordenador.

## Status

Lib **Parcial**: avaliação pura dos canais de performance `cycle`, `event` e `static` e das sobreposições
de transformação do origin, por frame **da instância**, com hierarquia, sockets e matrizes de mundo
atualizadas, validada contra as funções reais da origem. Completa a fatia que a F033 deixava como
`Unsupported`. **Fora desta fatia, e declarado no retorno:** luz, clima, efeitos, rigs, caminhos, `rigBind`
e `reveal` dos nós (nada disso é avaliado nem inventado). CLI headless **P**, socket **P**, UI **P**: nenhum
comando, nenhum controle e nenhum nó em `tree.rs`; o viewport não consome este estado. Nada aqui afirma
paridade de adapters.

| Superfície | Estado |
|---|---|
| Biblioteca (`tessel_editor_core::directed_scene`) | Implementada e validada (Cargo, abaixo) |
| CLI headless (`tessel-headless`) | **P** — sem comando; a leitura é a lib |
| Socket / `./scripts/ui` | **P** — sem comando `scene.*` (frente G é dona dos adapters) |
| UI da janela | **P** — nenhum controle |

## Objetivo e limites

Dado um bundle importado (`DirectedScene`, F031) e um frame inteiro de uma instância, responder **sem GPUI,
sem relógio e sem playback**: que canais a instância aplica, o que cada um escreve, qual é o estado de
cada nó (posição, rotação Euler XYZ em graus, escala, `renderOrder`) e onde ele está no mundo. O resultado é
um valor próprio (`NodeFrame`), consultável e serializável, para um renderer futuro.

Não é render, não é luz/clima/rig/pós, não é o catálogo de propriedades (F034). O estado do grafo estático
**não** vale em frames animados: a origem recompõe as matrizes a cada frame, e aqui também.

Fonte lida (somente leitura, snapshot `d6adac15…`): `performance/resolve-performance.ts`,
`evaluator/evaluate-edit-frame.ts`, `scene/scene-graph.ts`, `schema/render-bundle.schema.ts` (canais).
Nada foi instalado, compilado ou escrito na origem.

### Semântica reproduzida (a da origem)

- Canais aplicáveis a uma instância: as tracks listadas em `performanceTrackIds` com `instanceId` igual, na ordem do bundle.
  O schema importado exige essa lista; vazia significa nenhuma track.
- `cycle`: `sin(((f + phaseOffsetFrames) % periodFrames) / periodFrames · 2π) · amplitude · axis`.
- `event`: progresso `clamp((f − start) / duration, 0, 1)`, `holdFinal` obrigatoriamente `true`; valor =
  `delta · progresso`.
- `static`: `value`.
- Mescla com o estado do nó: `localRotation` e `localPosition` **somam**; `localScale` **substitui**.
- A rotação base vira Euler XYZ (`Euler.setFromQuaternion`, inclusive o ramo de gimbal `|m13| ≥ 0.9999999`),
  soma-se o deslocamento e a quaternion é refeita (`setFromEuler`): a mesma ida e volta da origem.
- Mundo = `mundo(pai) · local`; para um nó em socket, `mundo(pai) = mundo(dono) · local(socket)`.
- Duas escritas na mesma `(nó, propriedade)` na mesma instância são conflito. A origem **aceita** o bundle
  e só falha ao avaliar um frame (`Multiple performance channels write arm:localRotation on instance inst-a`);
  o mesmo vale aqui, como erro tipado `invalid_animation`. Tracks fora da lista permitida não entram.

## Contrato do core / biblioteca

```rust
use tessel_editor_core::directed_scene::{ChannelProperty, DirectedScene, NodeAnimation};

let scene = DirectedScene::import_str(text)?;      // payload dos canais validado aqui (abaixo)
let bundle = scene.bundle();

// 1) edit frame -> câmera + nós (a verdade do frame, com a fronteira dita)
let frame = bundle.evaluate_frame(24)?;
let pen = frame.nodes.world_matrix("pen");          // Option<&Mat4>
let arm = frame.nodes.override_of("arm", ChannelProperty::LocalRotation);
match &frame.node_animation {
    NodeAnimation::Static => { /* nenhuma tarefa de performance se aplica */ }
    NodeAnimation::Animated { track_ids, channels } => { /* tracks e nº de canais usados */ }
}
frame.unevaluated;            // ["lighting", "weather", ...]: o que existe e NÃO foi avaliado
frame.is_complete();          // unevaluated vazio
let json = frame.to_json();   // camelCase; nós, matrizes, overrides, tempos, câmera, `complete`

// 2) uma instância, sem shot (qualquer ordem, qualquer repetição)
let timeline = bundle.node_timeline("inst-a")?;     // resolve plano e base uma vez
let state = timeline.state_at(17)?;                 // NodeFrame (próprio, sem empréstimo)
let once  = bundle.evaluate_instance_nodes("inst-a", 17)?;

// 3) o plano, sem frame
let plan = bundle.performance_plan("inst-a")?;      // Err no conflito de escrita
plan.track_ids(); plan.channel_count(); plan.overrides_at(17);
```

- `Bundle::evaluate_camera` e `FrameMapping`/`CameraEvaluation` são **inalterados**: a câmera não depende
  dos nós. `FrameEvaluation` ganhou `nodes: NodeFrame` e `unevaluated`; `NodeAnimation` agora é
  `Static | Animated { track_ids, channels }`, e `Unsupported` é `DeferredSections { scene_definition_id,
  sections }` (consumidores atualizados em `tests/directed_scene_temporal.rs`).
- `PerformanceChannel.motion: ChannelMotion` (`Cycle`/`Event`/`Static`) e `ChannelProperty`
  (`LocalRotation`/`LocalPosition`/`LocalScale`) são o modelo tipado; `SceneGraph::unevaluated_sections()`
  lista as seções declaradas sem avaliador.
- Erros (`TemporalError`, todos com `code()` e conversão para `CommandError`):

  | Situação | Código | Texto |
  |---|---|---|
  | edit frame fora de `[0, duração)` | `frame_out_of_range` | o da origem (`editFrame -1 out of range [0, 48)`) |
  | frame de instância fora de `[0, duração)` | `frame_out_of_range` | tipado (`InstanceFrameOutOfRange`) |
  | instância/cena inexistente | `invalid_camera` (código herdado da F033 para `Unresolved`) | `Unknown scene instance: …` |
  | conflito de escrita | `invalid_animation` | o da origem, literal |
  | nó com estado não finito (ex.: amplitude `1e308`, escala `1e200` em pai e filho) | `invalid_animation` | `NonFiniteNode` com o id |

  Divergência deliberada: com esses valores (aceitos pelo schema) a origem devolveria `Infinity`/`NaN`
  para um renderer; aqui é erro tipado, com o primeiro nó afetado (locais antes dos mundos, em ordem de pai).
- Falhas não alteram o bundle (avaliar é puro; testado por igualdade antes/depois).

### Payload dos canais no importador

`performanceTracks[].channels[]` deixou de ser "adiado": cada canal é validado ao importar, **todos** os
problemas de um canal são reportados (não só o primeiro) e um canal inválido recusa o bundle inteiro, como a
origem (`renderBundleSchema`). Campos: `kind` (`cycle|event|static`), `channelId` (não vazio), `nodeId`,
`property` (`cycle`/`event`: rotação ou posição; `static`: as três), `periodFrames` (inteiro ≥ 1),
`amplitude`, `axis{x,y,z}`, `phaseOffsetFrames` (inteiro ≥ 0), `startInstanceFrame` (inteiro ≥ 0),
`durationFrames` (inteiro ≥ 1), `delta`, `holdFinal` (`true`), `value`. Inteiros aceitam `24.0`, como um
número JS. Campos desconhecidos continuam no documento e geram o aviso `UNKNOWN_FIELD` (a origem os remove
em silêncio); campos de "outro tipo" de canal contam como desconhecidos. Os caminhos de erro são JSON
Pointers; um campo ausente é reportado no pai citando o campo (como em todo o importador).

## CLI headless

**P.** Nenhum comando. Quando a frente G expuser `scene.*`, o contrato é `evaluate_frame(...).to_json()`.

## UI e CLI da janela

**P.** Nenhum controle e nenhum equivalente `./scripts/ui`; o viewport não consome nós avaliados.
Apresentação N/A nesta fatia: é estado puro para o renderer (BB05/BB06).

## Evidências e status

Arquivos:

- Lib: `crates/core/src/directed_scene/animation/{mod,channels,nodes}.rs`; extensões em `model.rs`,
  `parse.rs`, `graph.rs`, `transform.rs` (Euler↔quaternion + 3 testes unitários), `temporal/{mod,frame}.rs`.
- Testes: `crates/core/tests/directed_scene_animation.rs` (29 testes públicos);
  `directed_scene_temporal.rs` atualizado (consumidor da extensão).
- Expectativas: `crates/core/tests/directed_scene_animation_fixtures/` — `generate.mjs`, `origin.json`,
  `bundles/*.bundle.json` (11 válidas, 20 recusadas). `generate.mjs` usa o harness do oráculo
  (`scripts/migration-oracle`), confere os 33 fontes executados contra os blobs do commit `d6adac15…`
  **antes** de gravar, falha se uma variante deixar de exercer o que o nome diz, e tem modo `verify`
  (`node …/generate.mjs verify`: reexecuta a origem e compara byte a byte, sem gravar).

Comparado com a origem (proveniência em `origin.json`; o teste Rust confere os hashes SHA-256 dos
bundles, do gerador e do caso-base, então editar uma fixture sem regerar quebra o Cargo):

- Os 4 bundles válidos do oráculo (`docs/migration/fixtures`): 30 frames + 8 erros de borda; o caso
  `valid-graph-sockets-performance` (3 canais) nos 12 frames sondados, todos os nós, overrides e matrizes de
  mundo.
- 11 variantes sintéticas: bordas de cycle (período, fase ≥ período, frames -1/0/fim/fim+1), bordas de event
  (antes, início, meio, fim, depois), static e combinações (soma/substituição), lista permitida e ordem de
  tracks, tracks todas fora da lista, conflito de escrita (mesma track e duas tracks), duas instâncias com
  offsets de shot e seek fora de ordem, rotações base em gimbal e grandes, campos desconhecidos:
  122 frames + 19 erros.
- 20 payloads recusados: mesmo número de problemas e mesmos caminhos que a origem.
- Total: **20 373 números** comparados; maior erro observado **7,1e-15**.

Tolerâncias (fixadas no cabeçalho do teste antes da comparação): frames, ids, propriedades, contagens,
`renderOrder`, ordem de nós/overrides e textos de erro **exatos**; posição/rotação/escala/overrides/matrizes/
câmera dentro de `1e-9 + 1e-12·|x|` (diferença de ulp entre libm e V8). O erro real é 5 ordens menor; a
margem fica 6 ordens abaixo do menor efeito de uma fase, progresso ou ordem de mescla errados.

Comportamento desta lib (não é equivalência com a origem): seno analítico e fase/offset, progresso do
event (antes/início/passos/fim/depois), soma × substituição, propagação por hierarquia e socket com
igualdade exata da cadeia de matrizes, estado sem canais = grafo estático (dentro de 1e-12), tempo local da
instância e offsets de shot, seek embaralhado/invertido/repetido com **bits idênticos**, linha do tempo
= chamadas avulsas, conflitos, erros tipados, lista de seções não avaliadas, forma e consistência do JSON,
`evaluate_camera` independente do conflito. Controles negativos: fase errada em 1 frame e duração do event
errada **falham** na mesma comparação (a da fase, por 18 graus), e a variante intacta passa.

Comandos realmente executados (checkout `feat/migration-foundation`): `MBX_GC_MAX_TOTAL_SIZE=64GiB
./scripts/cargo test -p tessel-editor-core --locked` (tudo verde), `./scripts/cargo clippy -p
tessel-editor-core --all-targets --locked -- -D warnings`, `rustfmt` nos arquivos da frente,
`node …/generate.mjs verify`, `node scripts/migration-oracle/oracle.mjs verify`, `node --test
scripts/migration-oracle/oracle.spec.mjs`. `cargo tree -p tessel-editor-core` sem gpui/wry.

## Limitações

- Sem luz, clima, efeitos, rigs, caminhos, `rigBind`, `reveal`: listados em `unevaluated`; um bundle que os
  use **não** é "completo" (`require_complete` falha). As regras adiadas de `finalizeBundle` para esses
  domínios seguem sem rodar (`DEFERRED_RULES`).
- O conflito de escrita só aparece ao avaliar (como na origem); `performance_plan` permite checar antes.
- JSON gerado e relido por `serde_json` sem `float_roundtrip` pode mudar 1 ulp ao reler; as matrizes e os
  números do estado são os `f64` exatos nos acessores.
- Rotação: a decomposição Euler XYZ reproduz a da origem, inclusive o gimbal (z = 0); a rotação descrita é
  preservada, o triplo Euler não.
- Sem relógio: converter tempo → frame é `resolve_time_mapping` (F033).

## Integração pelo coordenador

- Repetidos o gerador em modo `verify` (31 variantes, 33 fontes), a suíte do core e clippy.
- Acrescentado teste de round-trip JSON das matrizes por 48 frames. Ele falhava antes da
  opção `serde_json/float_roundtrip` e passou depois, preservando os bits dos `f64` no
  transporte. A opção foi habilitada no workspace, sem trocar versão ou adicionar dependência.
- São 29 testes públicos desta feature após a regressão adicional. A comparação numérica
  Rust/origem mantém sua tolerância; esta não promete igualdade bit a bit entre engines.
- `is_complete()` indica ausência das seções animadas/de iluminação que o avaliador declara
  adiadas; não é certificação de assets, mapas estáticos de material ou capacidade de render.
- Instância desconhecida ainda usa o código herdado `invalid_camera`; não foi alterado nesta
  entrega para evitar mudança de contrato incidental.
