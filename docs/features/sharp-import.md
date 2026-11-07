# F052 — Importar o PLY do SHARP direto, com a câmera da captura (BL143)

## Objetivo e limites

O PLY de saída do SHARP (Apple `ml-sharp`) é um splat gaussiano comum nos eixos **OpenCV** (x à direita, **y para baixo**,
z para a frente), com metadados da imagem-fonte em elementos finais. Antes, importá-lo exigia um script externo que girava
os eixos e calculava a câmera. Agora o editor lê esses elementos, **recusa o que não consegue interpretar** e, quando
consegue, devolve ao clipe a câmera que tirou a foto: posição na origem, olhando para -z, com o FOV vertical de
`2·atan(H / (2·fy))`, e os eixos do arquivo girados 180° em X para os do editor (y para cima, câmera olhando para -z).

Fonte de cada regra (lidas no lugar, nada copiado): `sharp/utils/gaussians.py::save_ply` (elementos e tipos),
`sharp/utils/color_space.py::encode_color_space` (`0` sRGB, `1` linearRGB), `sharp/cli/predict.py` e o README (convenção
OpenCV). Nada é executado: sem modelo, sem pesos, sem mídia gerada, sem serviço remoto. Todos os arquivos dos testes são
sintéticos (`fixtures::write_sharp_ply`, aritmética pura, reproduzível byte a byte).

Fora do escopo: rodar o SHARP, múltiplas câmeras/cenas, `intrinsic` com pixel não quadrado ou ponto principal fora do centro
(recusados, ver abaixo), conversão de espaço de cor (BL139, só um aviso), aplicar a captura automaticamente a um `tessel-scene`
que referencia o splat (BL143.2).

## O que o arquivo diz e o que o editor deriva

| Elemento do PLY | Uso |
|---|---|
| `vertex` (`x y z f_dc_0..2 opacity scale_0..2 rot_0..3`) | O splat (já aceito por `splat::inspect`). Eixos OpenCV |
| `intrinsic` (9 `float`, matriz 3×3 por linha) | `fx`, `fy`, `cx`, `cy` em pixels da imagem-fonte |
| `image_size` (2 `uint`) | Largura e altura da imagem-fonte |
| `extrinsic` (16 `float`) | Deve ser a identidade (o SHARP escreve um placeholder); outra coisa = ambíguo |
| `frame`, `disparity` | Conferidos quando existem (contagem coerente com o `vertex`) |
| `color_space` (1 `uchar`) | `0` sRGB, `1` linearRGB; linear gera o aviso `capture_linear_colors` |
| `version` (3 `uchar`) | `1.x` é o lido; outra versão ou ausência gera `capture_version` |

Derivados (`sharp::Capture`): `vfov = 2·atan(H/(2·fy))`; `camera()` = `{position:[0,0,0], target:[0,0,-1], fov:vfov, orbit:0}`;
`orientation = [180,0,0]` (graus em X); `up` de origem `[0,-1,0]`, que a rotação leva a `+y`, o `up` fixo de toda câmera do
editor (por isso nenhuma câmera ganha um `up` configurável). O FOV é **vertical** e preservado; se a sequência tem outra
proporção que a imagem-fonte, o enquadramento horizontal muda (a lente é a mesma, o corte não) — limite honesto, sem
"ajustar à largura".

**Nada é adivinhado.** Sem nenhum elemento de captura o arquivo é um splat comum (`capture: null`, eixos não especificados,
câmera padrão). Com elementos, mas incompletos, é `incomplete_capture`; contraditórios (`fx` ≠ `fy` além de 0,1 %, ponto
principal a mais de 1 px do centro, `intrinsic` com skew, `extrinsic` ≠ identidade, `frame` que não é `[1, n]`, `color_space` desconhecido, elemento com forma errada, FOV fora de 1–170°) são
`ambiguous_capture`; bytes truncados são `invalid_asset`. A mensagem diz que `capture: ignore` importa o mesmo arquivo como
splat comum. O erro **não muda nada** (documento, revisão, histórico).

## Contrato do core / biblioteca

`tessel_editor_core::sharp` (puro, sem GPUI/wry): `read(path, Choice) -> Result<Option<Capture>, CommandError>`,
`Choice::{Auto, Ignore}`, `Capture::{camera, project, project_in_editor, warnings, label, to_json, to_document, from_document}`.

- `Meta::capture` (em `assets`) guarda a captura do ativo; `Declared::capture` é a escolha do chamador.
- `asset.add` (e `media.import`) aceitam `capture: "auto" | "ignore"` (padrão `auto`); valor inválido é `invalid_request`.
  O resultado traz `asset.capture` (inclui `vfov`, `camera`, `orientation`, `up`) e `warnings` quando há.
- `clip.add` de um clipe de splat com captura sem `camera` explícito **grava** a câmera da captura em `clip.camera` (visível,
  editável, desfazível). Uma `camera` explícita vence.
- `Command::SetClipCameraFromCapture` (`clip.camera_capture {clip}`): devolve a câmera da captura a um clipe de cena do splat.
  `no_capture` se o ativo não tem captura, `camera_orbit_conflict` se o clipe tem chaves de câmera (BL140), no-op sem
  entrada de undo, uma entrada de undo ("Use capture camera") quando muda.
- `RenderPlan`: a camada de cena de um splat **com captura** leva `orientation:[180,0,0]` (omitido nos demais); a visualização
  de fonte (`preview.set source`) de um splat com captura usa a câmera da captura. É a mesma camada para monitor, `render.frame`
  e export.
- **Persistência.** `meta.capture` no documento guarda só os dados do arquivo (`image`, `fx`, `fy`, `cx`, `cy`, `color_space`,
  `version`); o FOV é recalculado ao abrir, então uma edição manual nunca contradiz as intrínsecas. Projetos com captura são
  gravados em `schema_version` 2 (leitores antigos recusam em vez de descartar); sem captura continuam em schema 1. Captura
  armazenada que não pode ser câmera recusa o documento inteiro (`invalid_document`).
- **Pacote/relink.** A captura viaja no documento do pacote; `media.relink` para um arquivo com outras intrínsecas é recusado
  (`relink_mismatch`, a comparação inclui a captura).

Runtime (`runtime/src/scene.js`, `compositor.js`): uma camada `scene` cujo ativo é um splat vira, no runtime, uma cena de um
objeto `splat` girada por `orientation`; objetos `splat` de um `tessel-scene` aceitam `rotation:[x,y,z]` em graus (somado ao
`spin`). A chave de reuso do resultado inclui a rotação.

```rust
use serde_json::json;
use tessel_editor_core::{EditorState, headless};

let mut state = EditorState::demo();
for request in [
    json!({"op":"project.create","name":"Filme","sequence":"main","rate":24,"width":1920,"height":1080,"length":96}),
    json!({"op":"asset.add","kind":"splat","id":"shot","name":"shot","path":"cena.ply"}), // capture: auto
    json!({"op":"track.add","id":"v1","name":"V1","kind":"video"}),
    json!({"op":"clip.add","id":"c","track":"v1","asset":"shot","start":0,"duration":96}), // câmera da captura
] {
    headless::execute(&mut state, &request)?;
}
let plan = headless::execute(&mut state, &json!({"op":"plan","frame":0}))?; // layers[0].orientation == [180,0,0]
```

## CLI headless

| Operação | Campos |
|---|---|
| `asset.add` | `kind:"splat"`, `path`, `capture` (`auto`/`ignore`) |
| `media.import` | `paths`, `mode`, `capture` (job cancelável, fora da thread da UI; falha por arquivo, o lote segue) |
| `clip.camera_capture` | `clip` |

```sh
./scripts/fixtures /tmp/fx   # escreve /tmp/fx/sharp.ply (cinco gaussianas sobre pixels conhecidos)
printf '%s\n' \
  '{"op":"project.create","name":"x","sequence":"main","rate":24,"width":320,"height":180,"length":48}' \
  '{"op":"asset.add","kind":"splat","id":"shot","name":"shot","path":"/tmp/fx/sharp.ply"}' \
  '{"op":"track.add","id":"t","name":"t","kind":"video"}' \
  '{"op":"clip.add","id":"c","track":"t","asset":"shot","start":0,"duration":48}' \
  '{"op":"plan","frame":0}' | ./scripts/headless
```

## UI e CLI da janela

Biblioteca: o item mostra `SHARP 160×120 · FOV 50.0°` (`media:<id>` → `state.meta`, `state.capture`). Inspetor, seção
**Câmera** de um clipe de cena: linha da captura + botão **Câmera da captura** (nó `camera.capture`, `state.capture`,
`state.matches`; desabilitado com motivo se o ativo não tem captura ou o clipe tem chaves). `./scripts/ui invoke camera.capture`
e `ui op clip.camera_capture` passam por `Workstation::execute` e a mesma `Command`. O diálogo de importação nativo continua
sendo só uma entrada de caminhos; `capture` se escolhe por `media.import`/`asset.add` (BL143.3: sem seletor na janela).

## Evidências e status

| Superfície | Evidência | Estado |
|---|---|---|
| Lib | `crates/core/tests/sharp_import.rs` (6 testes: leitura, câmera e giro no plano, simples/ignore sem câmera, recusas com estado intacto + mesmo arquivo com `ignore`, restaurar/undo, salvar/reabrir/schema/FOV recalculado/documento inválido) + 6 unitários em `sharp.rs` + `runtime/test/pure.test.mjs` | V |
| CLI headless | `crates/media/tests/sharp_import.rs` (3: job de import com falha por arquivo, salvar/reabrir/empacotar/restaurar, relink recusado) e `scripts/sharp-import-proof.py` (24 checks JSONL, processo `tessel-headless`) | V |
| UI | `crates/desktop/.../sharp_capture_tests.rs` (2 GPUI) e `scripts/sharp-import-proof.py` na janela real própria (30 checks por `ui op/get/invoke`, 5 sondas de pixel nos lugares previstos, a importação simples do mesmo arquivo fora deles, captura nativa [`assets/bl143-sharp-import-window.png`](assets/bl143-sharp-import-window.png)) | V (sem clique de ponteiro) |
| Pixels reais | `crates/render/tests/sharp_frames.rs` (Chrome + SwiftShader + Spark): cada gaussiana cai no pixel previsto **só pelas intrínsecas do arquivo** (pinhole calculado fora do código sob teste), idem na visualização de fonte, idem no vídeo exportado (H.264); sem o giro de eixos ou com a câmera virada para outro lado a prova falha (mutação executada: `ORIENTATION = [0,0,0]` derruba os dois testes de quadro) | V |

Executado × atribuído: tudo acima foi executado nesta máquina com arquivos sintéticos. **Não** foi executado com uma saída real
do SHARP (nenhum modelo roda aqui): o formato segue a fonte primária e o espelho sintético; a conferência com um PLY real é
BL143.1. Sondas de pixel comparam cores dominantes sobre preto (cada gaussiana é um disco suave), não hashes.

Limites: o FOV é vertical (proporção da sequência diferente da fonte muda o corte horizontal); `tessel-scene` que referencia um
splat com captura não herda câmera nem giro (BL143.2, mas `rotation` por objeto existe); sem seletor de `capture` na janela
(BL143.3); cores linearRGB só avisam (BL139.color); o preview nativo (WKWebView) e o export (Chrome) são pilhas GL
diferentes (ver [viewport](viewport.md)).
