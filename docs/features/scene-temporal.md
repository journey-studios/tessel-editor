# F033 — Cena dirigida: tempo, câmera e gate temporal (BB03)

Registrada na matriz como F033; integração e limites no relatório da frente D.

## Status

Lib **Parcial**: avaliação pura de edit frame → shot → frame de instância → frame do clipe de câmera →
pose/velocidade da câmera (receitas, keys, easings, camadas) e gate temporal das junções contínuas, validados
contra o corpus do oráculo. **Fora desta fatia, e declarado no retorno:** animação de nós (canais de
performance). CLI headless **P** e UI **P**: nenhum comando, nenhum controle, nenhum nó em `tree.rs`. Nada
daqui afirma paridade de adapters.

## Objetivo e limites

Dado um bundle importado (`DirectedScene`, F031), responder **sem GPUI, sem relógio e sem playback**:

1. Que shot toca um frame da edição e quais frames de instância e de clipe de câmera ele mostra.
2. Onde está a câmera (posição, alvo, FOV) e com que velocidade, em qualquer frame, em qualquer ordem.
3. Se os cortes contínuos entre shots são suaves (o `CONTINUOUS_*_JUMP` da origem).

Não é render, não é áudio, não é o catálogo de propriedades (BB04). A lógica de tempo vive aqui, em Rust; um
adapter (comando `scene.*`, headless, UI, runtime) só traduz.

Fonte lida (somente leitura, snapshot `d6adac15…`): `camera/resolve-camera-clip.ts`, `camera/easing.ts`,
`keyframes/resolve-keyframes.ts`, `edit/resolve-shot-at-edit-frame.ts`, `evaluator/evaluate-edit-frame.ts`,
`validate/edit-joins.ts`, `validate/bundle-integrity.ts` (`validateCameraLayers`), `performance/resolve-performance.ts`
(só para delimitar o que **não** é avaliado). Nada foi instalado, compilado ou escrito na origem.

## Contrato do core / biblioteca

```rust
use tessel_editor_core::directed_scene::{DirectedScene, NodeAnimation};
use tessel_editor_core::time::Ratio;

let scene = DirectedScene::import_str(text)?;
let bundle = scene.bundle();

// 1) mapeamento exato: só inteiros e Ratio, nada de f64
let map = bundle.resolve_frame_mapping(24)?;        // shot, instance_frame, camera_clip_frame, edit_time ...
let map = bundle.resolve_time_mapping(Ratio::new(1001, 1000).unwrap())?; // tempo -> frame (piso)

// 2) câmera (completa) em um frame da edição
let cam = bundle.evaluate_camera(24)?;              // cam.mapping + cam.camera (pose + velocidade)

// 3) a verdade inteira do frame, com a fronteira dita
let frame = bundle.evaluate_frame(24)?;
match &frame.node_animation {
    NodeAnimation::Static => { /* nenhum canal de performance se aplica */ }
    NodeAnimation::Unsupported(what) => { /* canais existem e NÃO foram avaliados */ }
}
let frame = frame.require_complete()?;              // Err(TemporalError::Unsupported) se parcial

// 4) um clipe, muitos frames, qualquer ordem
let timeline = bundle.camera_timeline("wide")?;
let state = timeline.state_at(17)?;

// 5) gates
let structural = scene.edit_join_issues();          // F031, inalterado
let temporal   = scene.temporal_join_issues();      // CONTINUOUS_*_JUMP
let all        = scene.temporal_issues();           // + FOV empilhado das camadas
```

| Item | Contrato |
|---|---|
| `Bundle::resolve_frame_mapping(i64)` | `FrameMapping { edit_frame, shot, instance, instance_frame, camera_clip_frame, edit_time, instance_time, camera_clip_time }`. Fora de `[0, editDurationFrames)` → `EditFrameOutOfRange` (texto idêntico ao da origem) |
| `Bundle::frame_at_time(Ratio)` / `resolve_time_mapping` | Piso exato: um limite pertence ao frame que começa nele; antes de 0 dá frame negativo (fora da edição) |
| `Bundle::evaluate_camera(i64)` | Mapeamento + `instanceFrame < duração da instância` + câmera. **Completo para a câmera**; não diz nada sobre nós |
| `Bundle::evaluate_frame(i64)` | O mesmo mais `node_animation`. `Static` ou `Unsupported(PerformanceChannels { instance_id, track_ids, channels })`. Nunca inventa estado de nó |
| `FrameEvaluation::is_complete` / `require_complete` | Quem precisa da verdade inteira pergunta; recebe `TemporalError::Unsupported` tipado |
| `Bundle::camera_timeline(id)` → `CameraTimeline` | Clipe disposto uma vez (`O(segmentos)`); `state_at(i64)`, `pose_at(i64)`, `frames()`, `main_track_poses()` respondem por busca binária |
| `Bundle::camera_at_clip_frame`, `camera_clip_frames`, `main_track_poses` | Atalhos de uma chamada |
| `CameraState` | `position`, `target`, `fov_deg`, `velocity` (unidades/frame do clipe), `fov_velocity_deg_per_frame`; `.pose()`; `CameraPose::to_world_matrix()` (`Mat4`, convenção `lookAt` do Three.js) |
| `DirectedScene::temporal_join_issues()` | `CONTINUOUS_CAMERA_JUMP`, `_TARGET_JUMP`, `_FOV_JUMP`, `_VELOCITY_JUMP`, `_FOV_VELOCITY_JUMP` com as tolerâncias da origem (`JOIN_*`, exportadas) |
| `DirectedScene::temporal_issues()` | `FOV_OUT_OF_RANGE` das camadas somadas + `temporal_join_issues()` |
| `TemporalError` | Tipado, sem pânico; `code()`; `From<TemporalError> for CommandError` (`frame_out_of_range`, `unsupported`, `invalid_camera`, `invalid_time`) |

### Semântica herdada da origem (e preservada)

- Clipe antes de 0 → erro; **depois do fim o clipe segura a última pose com velocidade 0**.
- A velocidade da trilha principal é medida **dentro** do movimento: no último frame de um movimento ela é 0. Por
  isso dois dollies seguidos têm `CONTINUOUS_VELOCITY_JUMP` mesmo com a pose contínua. Com camadas, a velocidade
  é a da pose composta (frame+1 menos frame).
- `from: "initial"` no meio do clipe reinicia a pose (`CAMERA_SEGMENT_RESET` no gate estrutural, salto de pose no temporal).
- Keyframes: a pose de partida é uma key implícita no frame 0 (linear); no frame de uma key a resposta é a key exata.
- Camadas somam offsets (giro, depois lateral+frente, depois zoom); cada camada continua da anterior.
- A origem percorre os movimentos desde o início a cada frame; `CameraTimeline` os dispõe uma vez e usa as mesmas
  poses carregadas, então o resultado é idêntico. Um clipe que falha no meio **avalia os frames anteriores**
  (preguiçoso como a origem) e devolve o erro tipado a partir do movimento que falha.

### Gate temporal

O gate estrutural (`edit_join_issues`, F031) continua com o mesmo significado: é barato, não avalia câmera e tem
chamadores que dependem dele. O temporal é **aditivo e separado**; `validateEditJoins` da origem = os dois juntos.

- Compara, em cada junção contínua **do mesmo clipe**, o último frame do shot anterior com o primeiro do seguinte.
  Pula (como a origem): buracos, cortes, transição ausente e clipes diferentes. Compara mesmo com `cameraInFrame`
  errado ou instâncias diferentes (como a origem; o estrutural os nomeia).
- `CAMERA_EVALUATION_FAILED` (código **próprio**): a origem lança exceção onde aqui há achado tipado.
- `TEMPORAL_CHECK_BUDGET_EXCEEDED` (código **próprio**): clipe com camadas e mais de `LAYER_FOV_SWEEP_MAX_FRAMES`
  (1 000 000) frames não é varrido; é **reportado**, não ignorado (o importador limita contagens, não números de frame).
- `FOV_OUT_OF_RANGE` das camadas somadas: a origem checa no import/schema; aqui o importador não avalia câmera
  (decisão de A), então é do gate temporal.
- Verificado: o oráculo `finalizeBundle` aceita ⇔ nenhum dos dois gates acha algo, nos 55 casos do corpus e em 9
  variantes sintéticas executadas na origem. Isso vale para a parte do bundle que este módulo interpreta;
  regras adiadas (`DEFERRED_RULES`: luz, clima, rig, narração, performance...) continuam não rodando.

### Tempo exato

Frames são inteiros; só o **tempo** é racional (`Rate`/`Ratio`): `frame * den / num`. 30000/1001: o frame 30000 começa
em exatamente 1001 s. `frame_at_time` é piso em `i128` com estreitamento verificado; `Rate::frame_to_time` devolveria 0
em overflow, então o mapeamento usa a variante verificada (`FrameOverflow`).

## Tolerâncias numéricas (decididas **antes** de ver as diferenças Rust×origem)

| Grandeza | Regra |
|---|---|
| frames, ids, contagens, `ok`, códigos de junção | exato |
| mensagens de erro de fora do intervalo (`editFrame … out of range`, `clipFrame … before the start`) | texto exato |
| posição, alvo, `fovDeg`, velocidade, `fovVelocityDegPerFrame`, poses do movimento principal | `abs <= 1e-9 + 1e-12·abs(esperado)` |

Motivo: uma pose é uma cadeia de poucas dezenas de operações em `f64` mais `sin/cos/asin/atan2`, que a origem toma do V8
(fdlibm) e o Rust do libm do sistema: podem diferir 1 ulp. Valores O(1…100) → erro honesto < 1e-12. 1e-9 deixa três
ordens de margem e fica seis ordens abaixo do menor efeito de uma receita/easing/key errada (≥ 1e-3). `Math.hypot`
foi reproduzido com o algoritmo do V8 (escala pelo maior + soma de Kahan) para que o comprimento arredonde igual.
A tolerância **não foi apertada** depois de ver o resultado. Controle negativo: trocar os easings do clipe rico por
`linear` move posições em >1e-3 e o mesmo critério falharia (`the_comparison_would_catch_a_wrong_easing`).

Erro observado (máximo): 2.2e-16 em 330 números de frames da edição; 7.1e-15 em 855 números de clipes/poses;
1.8e-15 em 528 números do bundle em camadas.

## CLI headless

**P.** Não existe comando. O contrato é só a biblioteca; um adapter futuro deve chamar `evaluate_camera` /
`temporal_issues` e traduzir `TemporalError` por `CommandError` (já implementado), sem reavaliar nada.

## UI e CLI da janela

**P.** Nenhum controle, scrub ou nó de `tree.rs` usa isto ainda. O viewport segue sem consumir a câmera avaliada.

## Evidências e status

| Superfície | Estado |
|---|---|
| Lib | **V** para o escopo: `MBX_GC_MAX_TOTAL_SIZE=64GiB ./scripts/cargo test -p tessel-editor-core --locked` (19 testes de integração em `directed_scene_temporal.rs` + 7 unitários de `temporal/camera.rs`, entre eles seek fora de ordem/repetido por bits, 30000/1001, bordas, positivos/negativos de junção, `Unsupported`) |
| Contra a origem | corpus de 4 válidos: 30 frames de edição + 8 erros, 5 clipes (durações, poses de movimento, 70 frames de clipe, 5 deles erros em -1, e frames depois do fim); `editJoins` de **todos os 55** casos (1 com `CONTINUOUS_*`); 9 variantes sintéticas executadas na origem (`crates/core/tests/directed_scene_temporal_fixtures/`, regeneradas por `generate.mjs`) |
| Qualidade | `clippy` limpo nos alvos de `directed_scene` (`--lib` + 3 testes); `cargo fmt --all -- --check` limpo |
| CLI headless / UI | **P** |

### Gate da fixture própria

`crates/core/tests/directed_scene_fixtures/layered_scene.json` passa import e `edit_join_issues()` (vazio), e
`temporal_join_issues()` devolve `CONTINUOUS_VELOCITY_JUMP` em `/shots/1` (`0.0880` unidades/frame > `0.08`), o mesmo
valor e o mesmo código que a origem (`finalizeBundle`: "Camera velocity jump at join s1|s2: 0.0880") nesta execução.
Importar e não ter achado estrutural **não** certifica o bundle: uma UI/CLI que habilite cenas deve chamar
`temporal_issues()` antes.

## Limitações explícitas

- **Animação de nós não é avaliada.** `evaluate_frame` devolve `NodeAnimation::Unsupported` com as trilhas; nada
  de estado parcial passado por completo. O estado estático dos nós está em `SceneGraph`, e **não** vale nesses frames.
- Sem FPS não inteiro no corpus do oráculo (lacuna dele); 24000/1001, 30000/1001, 60000/1001, 25 e 30 são testados
  pelo mapeamento exato com bundles sintéticos, não contra a origem.
- `CameraPose::to_world_matrix` segue o `lookAt` do Three.js mas **não** foi comparada com o oráculo (ele não grava matriz de câmera).
- Mapeamento de nós/camadas de luz, clima e rigs por frame: fora (M2/M3/M6).
- O gate varre FOV de camadas até 1 000 000 frames por clipe (acima, achado de orçamento).

## Revisão do coordenador

- Repetidos os 19 testes públicos temporais, a suíte do core e clippy.
- Teste unitário adicional verifica abaixo/no/acima de cada um dos cinco limiares
  `JOIN_*`; é teste da regra Rust, não comparação de corpus nesses limiares.
- Regeneração das 9 variantes repetida byte a byte. O gerador agora recusa fonte
  incompatível com o commit fixado; `origin.json` registra hashes de todos os arquivos
  executados, entradas, gerador e versões das dependências. Sem fallback de proveniência.
