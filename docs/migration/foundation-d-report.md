# Relatório da frente D — BB03 tempo e câmera

2026-11-05. Rodada 2 da base de migração ([plano](parallel-implementation-plan.md), [roadmap](phased-roadmap.md) BB03).
Checkout compartilhado `tessel-editor` em `d7b405d` (origem `git@github.com:wallynm/tessel-editor.git`), base A/B integrada
`72fa715`, oráculo B `f18b1cf`. Origem lida em somente leitura (`d6adac15…`). Esta frente não fez `git add`/commit/push/
switch/reset/clean, não tocou `Cargo.toml`/`Cargo.lock`, `lib.rs`, `scene_edit`, runtime, matriz nem índices, e não rodou GPU/Chrome nem o workspace desktop.

Contrato público e tolerâncias: [`docs/features/scene-temporal.md`](../features/scene-temporal.md).

## Entrega

API pura e determinística, sem GPUI, que mapeia frame da edição → shot → frame da instância → frame do clipe de câmera →
pose e velocidade da câmera (receitas, keys, easings, camadas), com tempo exato (`Rate`/`Ratio`), e um **gate temporal aditivo**
das junções contínuas (`CONTINUOUS_*_JUMP`). Não há framework: `Bundle` ganhou consultas, `CameraTimeline` dispõe um clipe uma vez,
`DirectedScene` ganhou dois gates.

| Pergunta | API |
|---|---|
| Frame → shot/instância/clipe, tempo exato | `Bundle::resolve_frame_mapping(i64)`, `resolve_time_mapping(Ratio)`, `frame_at_time` |
| Câmera num frame da edição | `Bundle::evaluate_camera(i64)` |
| Frame inteiro com a fronteira dita | `Bundle::evaluate_frame(i64)` → `FrameEvaluation { mapping, camera, node_animation }`, `is_complete`, `require_complete` |
| Clipe, vários frames, qualquer ordem | `Bundle::camera_timeline(id)` → `CameraTimeline::{state_at, pose_at, frames, main_track_poses}`; atalhos `camera_at_clip_frame`, `camera_clip_frames`, `main_track_poses` |
| Gate | `DirectedScene::temporal_join_issues()`, `temporal_issues()` (+ FOV empilhado); constantes `JOIN_*` |
| Erros | `TemporalError` tipado, `code()`, `From<TemporalError> for CommandError` |

**Fronteira honesta:** animação de nós (canais de performance) **não** é avaliada. `evaluate_frame` retorna
`NodeAnimation::Unsupported(PerformanceChannels { instance_id, track_ids, channels })` quando um canal se aplica
(mesma seleção da origem: trilhas da instância listadas por ela) e `Static` caso contrário; `require_complete()` converte o parcial em
`TemporalError::Unsupported`. `evaluate_camera` é completo **só para a câmera** e diz isso.

### Gate temporal vs. estrutural

`edit_join_issues()` (F031) não mudou de comportamento nem de assinatura (a suíte de A e o oráculo continuam passando). O novo gate é
`joins::check_temporal`, em `joins.rs`, separado, e a união dos dois **é** o `validateEditJoins` da origem: conferido por
código e `shotId` nos 55 casos do corpus (1 com `CONTINUOUS_*`) e em 9 variantes executadas na origem.

`layered_scene.json`: `edit_join_issues()` vazio; `temporal_join_issues()` = `[CONTINUOUS_VELOCITY_JUMP]` em `/shots/1`, `0.0880`
unidades/frame (limite `0.08`) — o mesmo código e o mesmo valor que `finalizeBundle` da origem devolveu ao ser executado nesta frente
sobre essa fixture ("Camera velocity jump at join s1|s2: 0.0880").

## Arquivos (todos dentro do ownership)

Novos:

- `crates/core/src/directed_scene/temporal/mod.rs` — `TemporalError`, `CommandError`, exports
- `crates/core/src/directed_scene/temporal/camera.rs` — `CameraPose/State/Timeline`, receitas, easing, keys, camadas (+7 testes unitários)
- `crates/core/src/directed_scene/temporal/frame.rs` — mapeamento, tempo exato, `evaluate_*`, `Unsupported`
- `crates/core/src/directed_scene/temporal/gate.rs` — FOV das camadas somadas, orçamento
- `crates/core/tests/directed_scene_temporal.rs` — 19 testes
- `crates/core/tests/directed_scene_temporal_fixtures/` — `generate.mjs`, `origin.json`, 9 `*.bundle.json` (sintéticos, derivados de `layered_scene.json`)
- `docs/features/scene-temporal.md`, `docs/migration/foundation-d-report.md`

Alterados (aditivo): `crates/core/src/directed_scene/mod.rs` (módulo, exports, 2 métodos, `DEFERRED_RULES`/doc), `crates/core/src/directed_scene/joins.rs`
(constantes `JOIN_*`, `check_temporal`; `check` estrutural intacto). `model.rs`/`parse.rs`/`lib.rs`: **não tocados** (não foram necessários).

Alteração de comportamento do que A entregou: só `DEFERRED_RULES` (3 textos) para dizer que FOV empilhado e `CONTINUOUS_*` agora rodam em
`temporal_issues`/`temporal_join_issues`, não no import.

## Decisões de projeto

- **Mesma aritmética da origem, na mesma ordem**, porque o oráculo compara poses. `Math.hypot` foi reproduzido com o algoritmo do V8 (escala
  + Kahan). Onde a origem lança exceção, retorna-se `TemporalError`; a avaliação é **preguiçosa como a origem** (frames antes do movimento
  que falha avaliam).
- **`CameraTimeline`** dispõe o clipe uma vez (poses carregadas, somas de camadas) e responde por busca binária: a origem é `O(segmentos)`
  por frame; o gate varre milhões de frames sem custo quadrático. Resultado idêntico (conferido em bits no teste de seek).
- **Frames `i64` na API** para que -1 seja um erro tipado e não um saturamento; internamente `u64` com `checked_add`.
- **Tempo**: frames inteiros, tempo `Ratio`; `Rate::frame_to_time` devolve 0 em overflow, então o mapeamento usa a variante verificada
  (`FrameOverflow`); `frame_at_time` é piso em `i128` com estreitamento verificado (`Rate::time_to_frame_floor` usa `as`).
- **Códigos próprios** (a origem lança): `CAMERA_EVALUATION_FAILED`, `TEMPORAL_CHECK_BUDGET_EXCEEDED` (clipe com camadas > 1 000 000 frames não é
  varrido e **é reportado**; o importador limita contagens, não números de frame).
- FOV empilhado das camadas fica no gate temporal (a origem o checa no schema; o importador de A não avalia câmera).
- `to_world_matrix` (`Mat4`, `lookAt` do Three.js) existe para o consumo de render, **sem** comparação com o oráculo (ele não grava matriz de câmera).

## Tolerâncias (definidas antes de ver as diferenças)

`exato` para frames/ids/contagens/códigos e para o texto dos dois erros de intervalo; `abs <= 1e-9 + 1e-12·abs(esperado)` para posição, alvo, FOV,
velocidades e poses de movimento. Justificativa completa no teste e em `scene-temporal.md`. **Erro máximo observado**: 2.2e-16 (frames da edição,
330 números), 7.1e-15 (clipes e poses, 855 números), 1.8e-15 (bundle em camadas, 528 números). A tolerância não foi apertada depois.
Controle negativo: easings trocados por `linear` movem posições em >1e-3 e o mesmo critério falharia.

## Comandos e resultados

Todos com `MBX_GC_MAX_TOTAL_SIZE=64GiB ./scripts/cargo …` (sem `mbx clean/gc`, sem trocar HOME).

| Comando | Resultado |
|---|---|
| `cargo test -p tessel-editor-core --locked` | **verde**: 168 unitários, 29 `directed_scene`, 3 oráculo A, **19 temporal**, 2 headless, 8 project, 8 proof, 26 `scene_edit` (de E, no estado do momento), 7 video_sound, 2 doctests |
| `cargo test … --test directed_scene_temporal -- --nocapture` | 19/19; imprime o erro máximo e a mensagem da junção da fixture |
| `cargo clippy -p tessel-editor-core --lib --test directed_scene_temporal --test directed_scene --test directed_scene_oracle --locked -- -D warnings -A clippy::result_large_err` | limpo (alvos de D) |
| `cargo clippy -p tessel-editor-core --all-targets --locked -- -D warnings` | **limpo** na última execução (exit 0). Em execuções anteriores falhou por código de E (`scene_edit`: `result_large_err`, `approx_constant` em `tests/scene_edit.rs`) e, por instantes, por um `.cause` sem parênteses em `scene_edit/error.rs`; E corrigiu depois. Nenhuma ocorrência em arquivos de D (um `manual_clamp` em `apply_easing` foi tratado com `#[allow]` justificado: `clamp` mudaria o sinal do zero) |
| `cargo fmt --all -- --check` | limpo |
| `cargo tree -p tessel-editor-core` | só `flate2`, `serde`, `serde_json` (sem GPUI/wry) |
| `node crates/core/tests/directed_scene_temporal_fixtures/generate.mjs` | 9 variantes executadas na origem (33 fontes, Node 22.23.3); só lê a origem; usa o harness do oráculo e seu cache de dependências fora do repo |

Não executado: `./scripts/cargo build -p tessel-editor`, workspace desktop, GPU, Chrome, `oracle.mjs verify` (o oráculo não foi alterado).
Espaço livre ao final ≥ 9.5 GiB; temporários próprios (`target` é compartilhado) ≈ 0.3 MB de fixtures.

Aviso de coordenação: rodei `./scripts/cargo fmt --all` uma vez (formatação do workspace inteiro). Não alterou `src/scene_edit/*`; é possível que tenha
reformatado `crates/core/tests/scene_edit.rs` de E (só estilo). Daí em diante usei somente `--check`.

## Evidência contra a origem (resumo)

- 4 bundles válidos: 30 frames de edição + 8 erros de borda (-1, fim, fim+1…), com mapeamento (`instanceId`, `instanceFrame`, `cameraClipId`, `cameraClipFrame`) exato e
  câmera dentro da tolerância; 5 clipes (inclui zoom, orbit, keyframes, hold e 2 camadas): `durationFrames`, `mainTrackPoses`, 70 frames de clipe (5 erros em -1, frames
  após o fim).
- `editJoins` dos 55 casos: estrutural ∪ temporal = origem (multiset de `(código, shot)`); aceito pela origem ⇔ nenhum gate acha algo.
- 9 variantes sintéticas derivadas de `layered_scene.json` (suave, salto de velocidade, salto de FOV, reset de pose, FOV empilhado, corte após clipe com salto…): veredito e códigos iguais aos da origem; câmera do bundle em camadas dentro da tolerância (528 números).
- Seek: 3 sementes embaralhadas + repetição imediata, bits idênticos ao percurso sequencial.
- FPS: 24000/1001, 30000/1001, 60000/1001, 30, 25 (sintéticos, **sem** origem; o corpus só tem 24/1): tempo exato `frame·den/num`, piso nas bordas, 1 ns antes pertence ao frame anterior, frame 30000 a 30000/1001 = 1001 s exatos, 1,2 milhão de frames sem deriva.

## Lacunas

- Animação de nós (canais cycle/event/static): não avaliada, tipada como `Unsupported`. Estado estático do grafo não vale nesses frames.
- O corpus do oráculo não tem FPS não inteiro, nem salto isolado na fronteira das tolerâncias `JOIN_*` (a fixture em camadas fica em 0.0880 contra 0.08, ~10 % acima), nem seek aleatório; os dois
  últimos são cobertos por testes sintéticos próprios, o primeiro só pelo mapeamento (não contra a origem).
- `CameraPose::to_world_matrix` não comparada com a origem.
- CLI headless **P**; UI **P**; nenhum comando `scene.*`; viewport não consome a câmera avaliada; `finalize`/gravação atômica fora.
- Regras adiadas de A (`DEFERRED_RULES`: luz, clima, rig, narração, payload dos canais) seguem sem rodar: "aceito pelos dois gates" não é `finalizeBundle` completo para bundles que usem esses domínios.
- Aviso de custo: o gate de FOV varre até 1 000 000 frames por clipe com camadas; acima disso emite achado de orçamento.

## Dependências e pontos para o coordenador

- **E** não precisa mudar nada; para certificar um bundle antes de habilitar cenas, chamar `temporal_issues()` junto com `edit_join_issues()`.
- Consumidores de `DEFERRED_RULES` por texto (se houver) veem 3 strings novas.
- `docs/features/directed-scene.md` é de A e **não** foi editado; sugestão: trocar, em "Pendências", `CONTINUOUS_*_JUMP` e "Avaliação de câmera/performance por frame (BB03)" por
  um link para `scene-temporal.md` (câmera/tempo/gate entregues; performance de nós pendente) e, em "Evidências", registrar que `layered_scene.json` é rejeitada por `temporal_join_issues()`.
- `docs/features/migration-oracle.md`: a coluna Lib pode citar que o consumidor Rust agora compara câmera/tempo (`directed_scene_temporal.rs`).
- Matriz e rodapé propostos abaixo; `README.md`/`docs/README.md` podem listar `scene-temporal.md`.

## Linha exata da matriz (`docs/features/README.md`)

ID proposto `F033` (E pode propor outro; reconciliar a numeração):

```markdown
| F033 | [Cena dirigida: tempo, câmera e gate temporal (BB03)](scene-temporal.md) | P | P | P | Parcial¹¹ | `directed_scene::{Bundle::evaluate_camera, evaluate_frame, camera_timeline}`, `DirectedScene::temporal_issues`; adapters pendentes |
```

Rodapé sugerido:

```markdown
¹¹ Lib: avaliação pura de shot → instância → clipe → pose/velocidade da câmera (receitas, keys, easings, camadas), tempo racional exato e gate das
junções contínuas, igualados à origem `d6adac15` no corpus do oráculo (4 válidos, 55 vereditos de junção, 9 variantes), com tolerância
`1e-9 + 1e-12·|x|` fixada antes da comparação. Animação de nós (canais de performance) **não** é avaliada e é devolvida como
`Unsupported`; CLI headless e UI não existem. Validar o `layered_scene.json` exige `temporal_issues()`: ele passa o gate estrutural e a origem o rejeita
(`CONTINUOUS_VELOCITY_JUMP`).
```

## Reprodução

```sh
MBX_GC_MAX_TOTAL_SIZE=64GiB ./scripts/cargo test -p tessel-editor-core --locked --test directed_scene_temporal -- --nocapture
node scripts/migration-oracle/oracle.mjs setup   # uma vez
node crates/core/tests/directed_scene_temporal_fixtures/generate.mjs   # regenera as variantes e origin.json
```

## Revisão de integração pelo coordenador

A suíte completa do core passou na repetição, incluindo os 19 testes públicos temporais
e os testes de `scene_edit` que estavam no checkout compartilhado (isto não integra E).
Clippy também passou. Foram adicionados e executados testes abaixo/no/acima dos limiares
`JOIN_*`, sem mudar suas constantes nem a comparação estrita da origem.

O gerador de variantes passou a validar os blobs contra o snapshot fixado antes de escrever
qualquer fixture. O fallback que declarava o commit fixado quando Git falhasse foi removido.
`origin.json` inclui hashes das fontes, entradas/gerador e versões das dependências.
Duas gerações consecutivas produziram bytes idênticos; os 19 testes temporais passaram
novamente depois dessa regeneração. A matriz F033 permanece parcial, sem UI/CLI.

Atualização posterior: a fronteira `NodeAnimation::Unsupported(PerformanceChannels)` acima
foi substituída pela F035. Agora os canais são avaliados, `NodeAnimation` é Static/Animated,
e `unevaluated` identifica outros domínios pendentes. O registro da entrega D permanece
histórico; contrato atual em [scene-animation](../features/scene-animation.md).
