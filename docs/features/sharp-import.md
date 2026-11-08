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
`ambiguous_capture`; bytes truncados — ou um cabeçalho que promete mais bytes do que um `u64` conta — são `invalid_asset`. A
mensagem diz que `capture: ignore` importa o mesmo arquivo como splat comum. O erro **não muda nada** (documento, revisão,
histórico).

**Ausência antes de layout (BL143.5).** O cabeçalho é lido sem recusar nada: `list`, tipo desconhecido, contagem que não é
número ou linha com tokens a mais ficam só registrados. Se nenhum elemento de captura existe, o resultado é `Ok(None)` e o
arquivo vale pelas regras do leitor anterior (`splat::inspect`): o scanner nunca recusa um PLY comum por algo que ele aceitava.
Com captura presente, tudo falha fechado: formato que não é `binary_little_endian`, `property` solta, linha `element`/`property`
malformada (a `property` seguinte **não** cai no elemento anterior), elemento de captura ou `vertex` declarado duas vezes,
elemento sem tamanho conhecido antes de uma captura (`list`, tipo desconhecido) — tudo `ambiguous_capture` com `details.defect`
— e contagens/offsets que estouram `u64` ou passam do tamanho do arquivo — `invalid_asset`. A aritmética é `checked_*`; valores
decodificados só viram inteiro por `whole()` (exato, na faixa), nunca por `as`.

**`color_space` ausente (BL143.9).** Vale sRGB, e o aviso `capture_color_space` diz que foi suposto (`Capture.color_space_declared`,
gravado no documento apenas quando `false`).

## Contrato do core / biblioteca

`tessel_editor_core::sharp` (puro, sem GPUI/wry): `read(path, Choice) -> Result<Option<Capture>, CommandError>`,
`Choice::{Auto, Ignore}`, `Capture::{camera, project, project_in_editor, warnings, label, to_json, to_document, from_document}`.

- `Meta::capture` (em `assets`) guarda a captura do ativo; `Declared::capture` é a escolha do chamador.
- `asset.add` (e `media.import`) aceitam `capture: "auto" | "ignore"` (padrão `auto`); valor inválido é `invalid_request`.
  O resultado traz `asset.capture` (inclui `vfov`, `camera`, `orientation`, `up`) e `warnings` quando há. O `media.import` leva os
  mesmos avisos (`capture_linear_colors`, `capture_version`, `capture_color_space`) em `items[].warnings`, como `code: message`
  (BL143.7).
- **Duplicado com outra escolha de `capture` (BL143.6).** Mesmos bytes de um ativo já na biblioteca: se a captura resolvida é a
  mesma, o item é `duplicate`, como sempre. Se difere (`auto` sobre um ativo importado com `ignore`, ou o inverso), o item é
  recusado por arquivo com `capture_conflict` (`details`: `existing_asset`, `existing_has_capture`, `requested_has_capture`):
  nenhuma transação, o ativo, seus clipes, o projeto e o undo ficam como estavam, e o resto do lote segue. Para trocar a escolha,
  remova o ativo e importe de novo.
- `clip.add` de um clipe de splat com captura sem `camera` explícito **grava** a câmera da captura em `clip.camera` (visível,
  editável, desfazível). Uma `camera` explícita vence.
- `Command::SetClipCameraFromCapture` (`clip.camera_capture {clip}`): devolve a câmera da captura a um clipe de cena do splat.
  `no_capture` se o ativo não tem captura (só um splat tem; uma captura em ativo de outro tipo é recusada ao registrar e ao abrir
  o documento), `camera_keys_override` se o clipe tem chaves de câmera e a câmera da captura difere da do clipe (a captura tem
  órbita 0: não é conflito de órbita; sob chaves seria uma entrada de undo inerte, BL143.9), no-op sem entrada de undo, uma
  entrada de undo ("Use capture camera") quando muda.
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
| Lib (revisão, BL143.4–.9) | `sharp.rs`: 6 unitários novos (PLY comum, captura não localizável, duplicados/tokens extras, cabeçalho hostil, `color_space` ausente, `whole`); `sharp_import.rs`: 5 novos (PLY comum com `auto`/`ignore`, cabeçalho hostil, aviso de `color_space`, captura em ativo não splat, fidelidade de cor/enquadramento com oráculo independente e controles negativos) | V (executados na integração com `main`) |
| CLI headless | `crates/media/tests/sharp_import.rs` (3: job de import com falha por arquivo, salvar/reabrir/empacotar/restaurar, relink recusado) e `scripts/sharp-import-proof.py` (24 checks JSONL, processo `tessel-headless`) | V |
| CLI headless (revisão) | `crates/media/tests/sharp_import.rs`: 3 novos (avisos no relatório do `media.import`, duplicado com outra escolha de `capture`, cabeçalho hostil/truncado por arquivo no lote) | V (executados na integração com `main`) |
| UI | `crates/desktop/.../sharp_capture_tests.rs` (2 GPUI) e `scripts/sharp-import-proof.py` na janela real própria (30 checks por `ui op/get/invoke`, 5 sondas de pixel nos lugares previstos, a importação simples do mesmo arquivo fora deles, captura nativa [`assets/bl143-sharp-import-window.png`](assets/bl143-sharp-import-window.png)) | V (sem clique de ponteiro) |
| Pixels reais | `crates/render/tests/sharp_frames.rs` (Chrome + SwiftShader + Spark): cada gaussiana cai no pixel previsto **só pelas intrínsecas do arquivo** (pinhole calculado fora do código sob teste), idem na visualização de fonte, idem no vídeo exportado (H.264); sem o giro de eixos ou com a câmera virada para outro lado a prova falha (mutação executada: `ORIENTATION = [0,0,0]` derruba os dois testes de quadro) | V |

Executado × atribuído: **integração com `main` (PR #23, após o merge da PR #20): gates reais executados** em máquina com 18–21 GiB
livres (piso de 12 GiB), runtime real, Chrome, FFmpeg. `TESSEL_REQUIRE_RUNTIME=1 ./scripts/cargo test --workspace --locked --no-fail-fast`:
**1634 passaram, 0 falharam, 12 ignorados**, incluindo os 3 de `crates/render/tests/sharp_frames.rs` (pixels no quadro, na visualização e no MP4);
`clippy -D warnings` e `fmt --check` limpos; `./scripts/runtime test` 176/176; `scripts/sharp-import-proof.py` (com `with-gpu-lock.py`):
**headless 24/24, janela 30/30**, 0 SKIP; todos os `scripts/*.spec.py` passam (helper 29/29 normal e `-O`); `cargo tree -p tessel-editor-core`
sem gpui/wry. Nota de operação: o bundle `runtime/dist` não é versionado e precisa de `./scripts/runtime build` depois de trocar de branch
(com o bundle antigo os 3 testes de pixels falham com `Cannot read properties of undefined (reading 'objects')`, que **não** é defeito do código).
**Não** foi executado com uma saída real do SHARP (nenhum modelo roda aqui): o formato segue a fonte primária e o espelho sintético; a
conferência com um PLY real é BL143.1. Um revisor leu o cabeçalho primário de quatro PLYs reais
(1 179 648 pontos, 1672×941, fx=fy=1330,3168, cx 836, cy 470,5, `color_space` 0, versão 1.5.0, FOV vertical 38,955°): confirma o
formato em parte, não a execução em Rust nem os pixels.

**Cor (BL143.8).** As sondas de pixel antigas comparam a cor dominante sobre preto, o que **não detecta escurecimento**. O runtime
já traz a correção BL019-F1; não se repete correção de cor sem prova. O teste novo `colours_and_framing_match_an_independent_oracle_...`
mede erro de cor por canal e erro de enquadramento em pixels contra um leitor e um pinhole independentes, com controles negativos
(escurecer, trocar R/B, espelhar, atrás da câmera). Ele cobre o contrato do **arquivo**, não renderiza. **Prova ausente:** comparar a
leitura Rust do PLY real e um render com a imagem-fonte local (`input/t1-teatro.png`), por cor e enquadramento, só-leitura e fora do
Git, quando houver disco; até lá BL143.8 fica bloqueado.

Limites: o FOV é vertical (proporção da sequência diferente da fonte muda o corte horizontal); `tessel-scene` que referencia um
splat com captura não herda câmera nem giro (BL143.2, mas `rotation` por objeto existe); sem seletor de `capture` na janela
(BL143.3); cores linearRGB só avisam (BL139.color); o preview nativo (WKWebView) e o export (Chrome) são pilhas GL
diferentes (ver [viewport](viewport.md)).

R2: rejeitados centros principais e extrínseca não finitos; replace_asset agora aplica a mesma recusa de captura em ativo não-splat. Regressões escritas para NaN/Inf e preservação de estado, sem execução Rust por disco. O teste de cor do contrato não prova fidelidade de render nem independência total do oráculo; BL143.8 permanece pendente de comparação contra fonte real por caminho independente. F052 está V para o contrato sintético; leitura de PLY real e cor/pixels contra a fonte seguem em BL143.1/BL143.8.

A provaSHARP também usa run_surface (criação/cleanup/relatório por superfície), saída absoluta e captura indisponívelSKIP. Helper canônico da câmera28/28 adotado por merge da base. Caller passou py_compile, sem prova real.
