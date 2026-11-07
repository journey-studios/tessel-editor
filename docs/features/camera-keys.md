# F054 — Keyframes da câmera de clipe de cena (BL140)

## Objetivo e limites

Um clipe `tessel-scene@1` (ou de splat) tinha só uma câmera fixa ou em órbita (`clip.camera`: posição, alvo, FOV,
`orbit`). Esta feature acrescenta **keyframes de posição, alvo e FOV** com easing por trecho: dolly, push-in, pull-out e
movimentos compostos, sempre como função pura do tempo de cena. É processamento real: o plano de render (`RenderPlan`)
entrega ao preview, ao `render.frame` e ao export a câmera **já avaliada** por quadro; o runtime só desenha.

Fora do escopo: curvas de Bézier, interpolação esférica/órbita entre chaves, chaves de outras propriedades (cor,
opacidade), editor gráfico de curvas e o motor de câmera da cena dirigida (receitas, camadas — F033/F035). O easing
reutiliza o avaliador dirigido (`directed_scene::apply_easing`, `applyEasing` da origem).

## Contrato do core / biblioteca

`tessel_editor_core::camera_keys` (puro, sem GPUI/wry) e `state::camera_edit`:

- `CameraKey { time: Ratio, position, target, fov, easing }`; `Easing::{Linear, EaseInOutCubic, EaseOutCubic}` (nomes da
  origem: `linear`, `easeInOutCubic`, `easeOutCubic`). O easing de uma chave vale para o trecho **que começa nela**.
- `Clip::camera_keys: Vec<CameraKey>` (ordenado por tempo). Vazio = comportamento anterior (`Camera::at`, `orbit`
  contado do início do clipe), bit a bit.
- `Command::CameraKeys(CameraKeysCommand)`: `Set` (insere ou substitui a chave do mesmo tempo), `Remove` (por tempo
  exato), `Replace` (lista inteira; vazia limpa), `Capture` (grava a câmera mostrada no playhead como chave no tempo
  de cena daquele quadro), `Field` (edita um campo da n-ésima chave: `time`, `easing`, `fov`, `position_x|y|z`,
  `target_x|y|z`). Cada edição monta a lista candidata **inteira**, valida e só então troca: rejeitada = nada muda
  (documento, revisão, histórico). Uma entrada de undo ("Edit camera keys") por comando que mudou algo.
- `EditorState::clip_camera_at(clip, frame)` é a **única** avaliação: o plano a usa por quadro (preview e export
  concordam) e `Capture` a usa para gravar o que está na tela. `clip_scene_time(clip, frame)` expõe o tempo.

**Tempo das chaves.** `time` é *tempo de cena* em segundos exatos (`Ratio`): o mesmo `scene_time` do plano (`source_start`
mais o tempo decorrido na velocidade do clipe). Não é contado do primeiro quadro do clipe. Consequências, todas testadas:
mover o clipe não muda a câmera de nenhum instante da cena; aparar o início (que avança `source_start`) esconde as chaves
anteriores sem deslocar as demais; aparar o fim esconde as posteriores; **nenhuma aparagem apaga chaves**.

**Avaliação.** Antes da primeira chave a câmera segura a primeira pose; depois da última, a última. Entre duas chaves,
posição, alvo e FOV interpolam linearmente com o easing da chave anterior aplicado ao progresso do trecho.

**Validade (tudo antes de gravar).** Cada chave é uma `Camera` válida (finita, FOV 1–170°, posição ≠ alvo, não olhar
exatamente ao longo de Y); tempos estritamente crescentes entre 0 e 86 400 s; no máximo 256 chaves por clipe. Como o
trecho é linear, o vetor alvo→posição é linear no progresso: uma câmera que **atravessaria o próprio alvo** (ou olharia
reto ao longo de Y) *entre* duas chaves válidas é detectada analiticamente (mínimo da distância horizontal ao quadrado em
forma fechada) e recusada ao gravar. `orbit` e chaves são excludentes: gravar chaves num clipe que orbita, ou pedir `orbit`
num clipe com chaves, é `camera_orbit_conflict`.

Erros: `invalid_value` (campo/faixa/ordem/atravessa o alvo), `not_applicable` (clipe que não é de cena),
`unknown_element` (clipe, chave ou índice), `unknown_field`, `invalid_time` (`Capture` com o playhead fora do clipe),
`camera_orbit_conflict`.

**Persistência.** `camera_keys` no JSON do clipe só existe quando há chaves; nesse caso o documento é gravado em
`schema_version` 2 (leitores antigos recusam em vez de descartar as chaves em silêncio). Projetos sem chaves continuam
byte a byte iguais e em schema 1; projetos antigos abrem sem chaves. Documento com chave inválida ou em clipe que não é
de cena é recusado inteiro.

```rust
use serde_json::json;
use tessel_editor_core::{EditorState, headless};

// Num projeto com um clipe de cena `c-scene` (sem `orbit`): push-in de z = 10 para z = 4 em 2 s de cena.
let mut state = EditorState::demo();
for (time, z, fov) in [(0, 10.0, 60.0), (2, 4.0, 40.0)] {
    headless::execute(&mut state, &json!({"op":"clip.camera_key","clip":"c-scene","time":time,
        "position":[0, 1.2, z],"target":[0, 0, 0],"fov":fov,"easing":"easeOutCubic"}))?;
}
let plan = headless::execute(&mut state, &json!({"op":"plan","frame":24}))?;
```

## CLI headless

Vocabulário de operações (`ops::parse_op`, o mesmo em `./scripts/headless`, `tessel-render` e `ui op`):

| Operação | Campos |
|---|---|
| `clip.camera_key` | `clip`, `time` (segundos inteiros ou `{"num":1,"den":24}`), `position`, `target`, `fov`, `easing` |
| `clip.camera_key.remove` | `clip`, `time` |
| `clip.camera_key.capture` | `clip`, `easing` (padrão `linear`); usa o playhead |
| `clip.camera_keys` | `clip`, `keys: [...]` (substitui tudo; `[]` limpa) |

Alvos para `set`/`invoke` (também por `./scripts/headless`): `set camera_key:<clip>:<n> <campo> <valor>` e
`invoke camera_key:<clip>:<n> remove`; `invoke camera_keys.capture` e `invoke camera_keys.clear` operam no clipe de cena
selecionado. A câmera avaliada aparece em `plan` (`layers[].camera`).

```sh
printf '%s\n' \
  '{"op":"clip.camera_key","clip":"c-scene","time":0,"position":[0,1.2,10],"target":[0,0,0],"fov":60}' \
  '{"op":"clip.camera_key","clip":"c-scene","time":2,"position":[0,1.2,4],"target":[0,0,0],"fov":40}' \
  '{"op":"plan","frame":24}' | ./scripts/headless
```

## UI e CLI da janela

Inspetor, aba Vídeo, seção **Câmera** (só para clipe de cena selecionado): botão **Chave no playhead** (desabilitado com
motivo se o playhead está fora do clipe), uma linha por chave (tempo, `z`/FOV, botão que alterna o easing do trecho,
remover) e **remover todas**. Redefinir a seção também limpa as chaves. Os campos numéricos de uma chave (`time`, `fov`,
posição, alvo) são editados por `./scripts/ui set camera_key:<clip>:<n> <campo> <valor>`; **não há widget numérico por chave
na janela** (BL140.1). Nós: `inspector.camera` (accordion), `camera_keys.capture`, `camera_keys.clear` e
`camera_key:<clip>:<n>` (nove campos, ação `remove`) em `./scripts/ui tree`/`get`. Todos os botões e `ui` passam por
`Workstation::execute` e a mesma `Command`.

## Evidências e status

| Superfície | Evidência | Estado |
|---|---|---|
| Lib | `crates/core/tests/camera_keys.rs` (12 testes: interpolação/hold, easing, trim/move/trim do fim, undo/redo, recusas com estado intacto, capture, `set`/`invoke`, salvar/reabrir/schema, processo `tessel-headless`) + 5 unitários em `camera_keys.rs` + 1 em `state/tree.rs` | V |
| CLI headless | mesmo fluxo em `tessel-headless` (processo real) e `scripts/camera-keys-proof.py` (19 checks, JSONL, sem janela) | V |
| UI | `crates/desktop/.../camera_keys_tests.rs` (3 testes GPUI: nós só para cena, capture/edição/remoção/undo, capture desabilitado com motivo) e `scripts/camera-keys-proof.py` na janela real própria (24 checks por `ui op/get/set/invoke`, 3 `render.frame` do monitor com hashes distintos, captura nativa [`assets/bl140-camera-keys-window.png`](assets/bl140-camera-keys-window.png)) | V (sem clique de ponteiro) |
| Preview = export | `crates/render/tests/export.rs::keyed_camera_export_matches_the_frames_the_monitor_draws` (runtime real, H.264): o quadro exportado fica a menos de 4/255 de diferença média do quadro do monitor do mesmo tempo e bem mais longe do de outro tempo | V |

Limites: sem ponteiro físico/clique de coordenada; a captura nativa mostra a janela, não substitui o teste de pixel do
export; sem curva além dos três easings; o preview nativo (WKWebView) e o export (Chrome) são pilhas GL diferentes (ver
[viewport](viewport.md)) e a igualdade provada é a do plano e a de pixels entre `render.frame` e export.
