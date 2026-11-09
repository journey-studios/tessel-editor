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
Nenhum modelo roda neste repositório; os PLYs reais do SHARP existem só fora do Git, na máquina do autor (ver a seção seguinte).

**Arquivo real (BL143.1 e BL143.8), executado depois da integração.** Os quatro PLYs locais do filme (`t1-teatro`…`t4-batalha`; só lidos,
nada foi copiado para o repositório e os quadros temporários foram apagados) foram lidos pelo leitor Rust (`asset.add` em `tessel-headless`):
1 179 648 gaussianas, `image_size` 1672×941, `fx=fy` 1330,3168, `cx` 836, `cy` 470,5, FOV vertical 38,9548°, `color_space` sRGB declarado, versão
1.5.0, eixos OpenCV → giro de 180° em X, câmera em (0,0,0) olhando −Z: o que o revisor leu no cabeçalho. O render real (`tessel-render`: Chrome +
SwiftShader + Spark) a 1672×941, com a câmera e os eixos só da captura, foi comparado com a imagem-fonte por FFmpeg:

| Arquivo | SSIM do render | SSIM com `capture: ignore` (controle) | Média RGB do render × da fonte | Maior diferença por canal (total e por quadrante) |
|---|---|---|---|---|
| `t1-teatro` | 0,9625 | 0,168 | 84/41/32 × 85/42/32 | 1 / 2 |
| `t2-estudio` | 0,9552 | 0,102 | 103/64/45 × 105/65/45 | 2 / 2 |
| `t3-chao` | 0,9530 | 0,069 | 122/81/58 × 124/81/58 | 2 / 2 |
| `t4-batalha` | 0,9241 | 0,016 | 115/99/88 × 117/100/88 | 2 / 3 |

Resultado: as métricas de enquadramento e cor ficaram dentro das tolerâncias declaradas. As médias diferem da fonte (até 2/255 por canal nos três casos SwiftShader; até 3/255 em `t4` na GPU). Médias globais e por quadrante não descartam escurecimento localizado. O controle sem captura cai para
SSIM ≤ 0,17, então a métrica distingue o certo do errado. A diferença residual de SSIM é a natural de Gaussianas contra os pixels originais.
`scripts/sharp-real-check.py` repete a conferência (só leitura, caminhos locais, tolerâncias declaradas no cabeçalho do script:
SSIM ≥ 0,90, cor ≤ 4/255, controle ≥ 0,5 abaixo). **Limites:** é a pilha do preview/export (Chrome headless), não a WKWebView nativa; o SwiftShader é
determinístico, uma GPU real pode diferir; o corpus é de um único modelo (quatro cenas com a mesma câmera) e não entra no CI por ser privado.

**`t4-batalha` e o tempo limite (BL143.12), 2026-11-08, `main` `30ac65b`.** A linha de `t4-batalha` acima é da GPU real (`TESSEL_GL=gpu`, Metal):
`scripts/sharp-real-check.py` passa nos quatro arquivos, com os mesmos números de SSIM e cor para `t1`–`t3` que o SwiftShader deu (0,962559, 0,95281,
0,955055), e o controle `capture: ignore` de `t4` cai para 0,016. O tempo limite **não é do arquivo**: com o SwiftShader (o padrão do `tessel-render`
e da janela) e a máquina com carga ~40, `render.frame` a 1280×720 respondeu em 33,5 s para `t3-chao` e **excedeu 60 s** para `t1-teatro`, `t2-estudio` e
`t4-batalha`, e a miniatura de 240×136 falhou (`render_failed`, `timeout`) nos quatro: o custo é por gaussiana (1 179 648 em cada arquivo), não por pixel, de um
rasterizador por CPU. Na GPU real um quadro leva 0,6–2 s. O prazo de 60 s do host não foi mudado. Consequência de produto, registrada em BL159: os jobs de miniatura e exportação pelo renderer Chrome/SwiftShader falharam nas condições medidas (o export falhou com `Frame 0: No frame within 120s`); `TESSEL_GL=gpu` contornou essas falhas. O preview nativo em WKWebView não foi validado para esses quadros e permanece não comprovado. O resultado de `render.frame` para `t3` em 33,5 s continua válido.

**Filme de 60 s a partir dos quatro arquivos crus (2026-11-08).** Em uma cópia derivada (`/Users/wallyssonnunes/www/tessel-video-splat-derived-20261108`, fora do
Git; a origem `~/www/tessel-video-splat` só foi lida) os quatro PLYs crus foram importados sem conversão (`asset.add … capture: auto`), a cena de cada take segura a
nuvem com `rotation [180,0,0]` e a câmera orbita (16:9) ou varre por chaves de câmera (9:16) em torno do ponto de interesse, com títulos acentuados, fades,
efeitos de runas e o áudio já existente. O projeto foi montado e exportado **pela janela nativa** (`tessel-editor ui op`, `HOME` próprio, sem coordenadas) com
`TESSEL_GL=gpu`: `work/tessel-splat-preview-60s-h.mp4` (1280×720, 24 fps, 60,000 s, H.264 + AAC, 23 812 564 bytes, sha256 `85de1ac392a48eb5…`; 1440 quadros em cerca de 4,5 min de
relógio com a máquina sob carga) e `work/tessel-splat-preview-60s-v.mp4` (720×1280, 22 102 333 bytes, sha256 `951eb9e67fafe0cf…`). Os dois passam pela
verificação do próprio export. Os quadros dos dois foram vistos (tomadas, fades, títulos, cartão final). Limites: **a trilha existente tem 25 s** (usada duas vezes
com fades, e a voz existente cobre só os títulos principais; ver BL160); os títulos finais são PNGs porque a camada de texto do editor é uma linha centrada em fonte 5×7
só de maiúsculas (BL141.1/BL053); o 9:16 mostra um terço da largura da imagem 16:9 (a lente é a mesma), então a câmera varre o assunto por chaves escritas à mão
(BL161) e a borda inferior de algumas tomadas mostra o fundo preto onde a nuvem acaba. A pasta `work/` contém só derivados locais.

**Revisão r2 do filme (2026-11-08; procedência conferida por leitura em 2026-11-09).** Numa cópia nova (`…-20261108-r2`, a r1 intacta) os mesmos quatro PLYs
crus (SHA-256 iguais aos da origem `~/www/tessel-video-splat`, só lida) foram exportados de novo pela janela nativa com `TESSEL_GL=gpu` por um binário release
construído de `5258e8c` (PR #36; não é a `main` atual): `…-60s-h-r2.mp4` (1280×720) e `…-60s-v-r2.mp4` (720×1280), cada um H.264 + AAC, 24 fps, 60,000 s, 1440 quadros.
A r2 repete a trilha de 25 s três vezes com emendas de **48 quadros (2 s)** em 23–25 s e 46–48 s (BL160, sem áudio novo) e dá à câmera vertical chaves de
manter-mover-manter com inclinação por take (BL161). Medido sobre o MP4: sem silêncio digital, trecho calado mais longo 0,5 s, queda de nível de 7,7 e 5,6 dB nas emendas,
108 quadros de varredura (um a cada 12, de 120; o cartão final fica de fora) sem faixa preta acima de 1 %. **Não é aceite humano:** ninguém ouviu as emendas nem
viu os 60 s inteiros, e 120 amostras não são o filme. Os arquivos ficam fora do Git; nada foi copiado para este repositório.

**Cor (BL143.8).** As sondas de pixel antigas comparam a cor dominante sobre preto, o que **não detecta escurecimento**. O runtime
já traz a correção BL019-F1; não se repete correção de cor sem prova. O teste novo `colours_and_framing_match_an_independent_oracle_...`
mede erro de cor por canal e erro de enquadramento em pixels contra um leitor e um pinhole independentes, com controles negativos
(escurecer, trocar R/B, espelhar, atrás da câmera). Ele cobre o contrato do **arquivo**, não renderiza. **Prova com o arquivo real:** feita, ver a seção anterior (BL143.8).

Limites: o FOV é vertical (proporção da sequência diferente da fonte muda o corte horizontal); `tessel-scene` que referencia um
splat com captura não herda câmera nem giro (BL143.2, mas `rotation` por objeto existe); sem seletor de `capture` na janela
(BL143.3); cores linearRGB só avisam (BL139.color); o preview nativo (WKWebView) e o export (Chrome) são pilhas GL
diferentes (ver [viewport](viewport.md)).

R2: rejeitados centros principais e extrínseca não finitos; replace_asset agora aplica a mesma recusa de captura em ativo não-splat. Regressões escritas para NaN/Inf e preservação de estado, sem execução Rust por disco. O teste de cor do contrato não prova fidelidade de render nem independência total do oráculo; BL143.8 permanece pendente de comparação contra fonte real por caminho independente. F052 está V para o contrato sintético; leitura de PLY real e cor/pixels contra a fonte seguem em BL143.1/BL143.8.

A provaSHARP também usa run_surface (criação/cleanup/relatório por superfície), saída absoluta e captura indisponívelSKIP. Helper canônico da câmera28/28 adotado por merge da base. Caller passou py_compile, sem prova real.
