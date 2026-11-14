# F050 — Títulos com acentos em português (BL141)

## Objetivo e limites

A camada de texto só desenhava ASCII: um título como "AÇÃO" virava PNG feito à mão. Agora um título aceita **ASCII
imprimível + as letras latinas acentuadas do português e demais línguas do oeste europeu + pontuação tipográfica**
(lista única em [`crates/core/src/text_coverage.txt`](../../crates/core/src/text_coverage.txt): `À Á Â Ã Ä Ç È É Ê Ë Ì Í Î Ï Ñ Ò Ó Ô Õ Ö Ù Ú Û Ü Ý`
e minúsculas, `¿ ¡ « » ‘ ’ “ ” – — … º ª ° ·`). O plano de texto é compartilhado pelo monitor (WebView) e pelo quadro headless
(`render.frame`) e pelo vídeo exportado: é a mesma camada de texto do plano.

**Fonte, determinismo e licença.** Nenhum arquivo de fonte é carregado e nenhuma fonte do sistema é consultada: o runtime
desenha células de uma grade **5×7 própria do projeto** (`runtime/src/font.js`, MIT, a mesma [LICENSE](../../LICENSE) do
repositório; a tabela de glifos já era do projeto). Um acento é uma **marca composta** sobre a letra maiúscula (agudo,
grave, circunflexo, til e trema em 2 linhas acima; cedilha em 2 linhas abaixo). Como são células inteiras, o desenho é igual
sem depender da fonte instalada; igualdade visual entre máquinas, escalas e Chrome/WKWebView não foi demonstrada. Não há fonte de terceiros a atribuir ([THIRD_PARTY_NOTICES](../../THIRD_PARTY_NOTICES.md)
registra isso); uma fonte vetorial real, com métricas e minúsculas, continua [BL053](../backlog.md).

**Quebra de linha e ajuste ao quadro (BL141.1, 2026-11-14).** `size` continua sendo a altura do corpo da letra em % da
altura do quadro, mas um título não sai mais cortado pelas bordas. O **core** faz o layout (`text::layout`), na proporção da
sequência:

- `\n` no texto sempre começa uma linha (linha vazia é mantida). É o único caractere de controle aceito;
- uma linha mais larga que a **área segura** (`SAFE_AREA` = 90 % da largura) quebra no último espaço que cabe (o espaço sai),
  de forma gulosa; uma palavra nunca é cortada;
- se uma palavra sozinha não cabe em 90 % da largura, ou se as linhas passam de 90 % da altura, o título inteiro **encolhe**
  (`scale < 1`) até caber e é quebrado de novo nesse tamanho. Nunca cresce;
- as linhas ficam **centradas** uma na outra e o bloco no quadro; a distância entre os corpos de duas linhas é fixa
  (`LINE_PITCH_ROWS` = 12 linhas de célula: espaço para acento em cima, cedilha embaixo e uma linha vazia), com ou sem acento.

O plano leva o resultado em `text.lines` e `text.scale` **só quando** ele não é a única linha em tamanho cheio de antes: um
título que já cabia continua com o mesmo plano e os mesmos pixels. O runtime só desenha o que o core decidiu
(`runtime/src/compositor.js`, `drawText`); monitor, `render.frame` e export usam o mesmo plano.

**Posição (decisão de 2026-11-14): o bloco cresce para longe da borda mais próxima.** Uma linha fica onde a posição do clipe
a põe, como sempre. A altura das outras linhas cresce para o espaço livre que essa linha deixa acima e abaixo dela dentro da
área segura, na proporção desse espaço (`text::anchor_offset`, somado ao `transform.y` da camada no plano): no terço inferior o
título cresce para cima, perto do topo para baixo, centrado para os dois lados igualmente, e a mudança é contínua com a posição
(um título animado nunca salta). O bloco nunca passa da borda que a linha única já tinha: se ela estava dentro da área segura,
o bloco também fica; se ela foi posta além, o bloco mantém essa borda e cresce para o outro lado. A posição horizontal não é
ajustada, e a rotação não entra na conta.

Limites que continuam: o alinhamento é sempre centralizado (esquerda/direita não existem); a área segura é fixa; o campo de
texto da janela é de uma linha (Enter confirma a edição), então a quebra **explícita** chega pela CLI, pela lib ou por
`./scripts/ui set clip_text:<id> content` (a quebra **automática** vale em qualquer superfície). Minúsculas continuam desenhadas
como maiúsculas. Aspas tipográficas e travessões reusam os glifos retos (a célula de 5 colunas não os distingue; BL141.2).
Fora da cobertura (`€`, `ß`, CJK, emoji, `ÿ`) o texto é **recusado** antes de qualquer quadro (nunca desenhado como caixa).

**Layout dos acentos.** O corpo da letra mantém o tamanho e o lugar do texto sem acento: quando algum caractere do título tem
marca, a caixa do texto cresce 2 linhas **em cima e embaixo** (simétrico), então o centro e o `size` não mudam e "AÇÃO" fica
no mesmo lugar que "ACAO" (testado em pixels).

## Contrato do core / biblioteca

`tessel_editor_core::text::normalize(&str) -> Result<String, CommandError>` e `content::TextSpec`:

- **Normalização NFC.** `ã` e `a` + U+0303 são o mesmo texto. `TextSpec::new` (usado por `clip.text`, `asset.add`, `clip.add`,
  leitura de documento, `set clip_text:`) compõe `letra + marca` na letra precomposta da cobertura e guarda sempre a forma
  composta: plano, documento salvo, desfazer e export recebem uma única grafia. `TextSpec::validate` recusa um texto fora de NFC.
- **Recusas** (`unsupported_text`, com `text` e `code_points` nos detalhes; estado, revisão e histórico intactos): caractere fora
  da cobertura, controle (tabulação, `\r`, inclusive em `\r\n`; o `\n` é a quebra de linha aceita, BL141.1), marca solta ou que
  não compõe (`a` + U+0307, `ã` + U+0301, duas marcas na mesma letra). Vazio e mais de 256 caracteres (depois de compor, `\n`
  incluído): `invalid_value`.
- **Layout** (BL141.1): `text::layout(content, size, width, height) -> TextLayout { lines, scale }`, constantes `GLYPH_ADVANCE`,
  `GLYPH_ROWS`, `MARK_ROWS`, `LINE_PITCH_ROWS` e `SAFE_AREA` (as do runtime em `runtime/src/font.js` têm de ser iguais);
  `TextLayout::is_single_line_of` diz quando o plano omite `lines`/`scale`.
- `Command::SetClipTextField { clip, field: content|size|color, value }` troca um campo mantendo os demais, valida o texto
  inteiro e passa pela mesma regra de `SetClipText`. Texto igual ao atual não é edição (`changed: false`, sem undo, sem revisão).
- Desfazer/refazer: uma edição de texto = uma entrada ("Edit text").

```rust
use tessel_editor_core::text::normalize;
assert_eq!(normalize("Ac\u{0327}a\u{0303}o").unwrap(), "Ação");
assert_eq!(normalize("preço €").unwrap_err().code, "unsupported_text");
```

## CLI headless

`./scripts/headless` (`tessel-headless`, JSONL, sem janela). Operações já existentes, agora com acentos:
`{"op":"clip.text","clip":"c-title","text":{"content":"Ação","size":14,"color":"#ffffff"}}`, `asset.add`/`clip.add`
com `text`, e o alvo novo `{"op":"set","target":"clip_text:c-title","field":"content","value":"Fe\u0301rias"}` (campos
`content`, `size`, `color`; campo desconhecido = `unknown_field`). `plan` mostra `layers[].text.content` já composto.
Quadros reais do mesmo título: `tessel-render`/`render.frame` com o runtime (`./scripts/runtime setup`).

## UI e CLI da janela

Inspetor, aba Vídeo, seção **Texto** (só para clipe de título selecionado): uma entrada (`Input` do Kit) com o texto do título.
**Enter** ou sair do campo confirma **uma** edição (`SetClipTextField`, um undo); texto recusado (caractere sem glifo) não
altera nada, aparece na barra de atividade e o campo volta ao texto do modelo; desfazer e `set` externo atualizam o campo.
Nós semânticos: `inspector.text` (accordion, sem `reset`: um texto não tem padrão) e `clip_text:<clip>` (campos `content`,
`size`, `color`; `state.normalization = "NFC"`), este só existe com o título selecionado. Equivalentes: `./scripts/ui invoke
clip:<id> select`, `./scripts/ui set clip_text:<id> content "Ação"`, `./scripts/ui op '{"op":"clip.text",…}'`. O monitor mostra o
resultado pelo mesmo plano.

## Evidências e status

| Superfície | Evidência | Estado |
|---|---|---|
| Lib | `crates/core/src/text.rs` (5 testes: NFC/NFD, cobertura inteira, marcas inválidas, controle, ASCII intacto), `content.rs`, `crates/core/tests/accented_titles.rs` (7: plano canônico, recusa com estado intacto, undo/repetição, `set clip_text:`, só título, salvar/reabrir NFD, processo `tessel-headless`) e `state/tree.rs` (nó só do título selecionado) | V |
| Runtime | `runtime/test/pure.test.mjs` (cobertura = a mesma lista do core, marca sobre o corpo, NFC/NFD) e `runtime/test/accented-text-browser.test.mjs` (Chrome real: NFD = NFC sem passar pelo core, acento ≠ sem acento, til ≠ agudo ≠ grave, caixa desconhecida ≠ acento coberto; só hashes, os pixels estão abaixo) | V |
| CLI headless | mesmo fluxo em `tessel-headless` (processo real) e `scripts/accented-titles-proof.py` (11 checks JSONL, sem janela) | V |
| Monitor / quadro / export (pixels reais) | `crates/render/tests/accented_titles.rs` (Chrome + FFmpeg): AÇÃO tem a largura de ACAO, til ≥ 4 px acima e cedilha ≥ 4 px abaixo do mesmo corpo (±2 px), maiúscula = minúscula, NFD = NFC byte a byte; export H.264 do quadro: mesma caixa e comparação histórica com `render.frame` em Chrome headless (965 vs 966 acesos, caixas iguais); limites mais estritos (foreground + linhas) executados na integração: `changed=1 union=966 worst_row_fraction=0.0238`, 3/3 | V |
| UI | `crates/desktop/src/workstation/title_text_tests.rs` (4 testes GPUI: seção/nó só para título, Enter = uma edição composta com undo, recusa restaura o campo e reporta, `set` externo chega ao campo; mutação sem `sync_title_text` derruba os 4) e `scripts/accented-titles-proof.py` na janela real própria (21 checks por `ui op/get/set/invoke`, `render.frame` PNG lido no script e captura nativa [`assets/bl141-accented-title-window.png`](assets/bl141-accented-title-window.png)) | V (sem digitar com teclado físico) |

Limites: não houve digitação física no campo nem clique de coordenada (o teste GPUI aciona a entidade `Input`; o script usa `set`);
a captura nativa mostra a janela, não substitui os testes de pixel; só a cobertura listada (BL141.2: ampliar com fonte real = BL053).

### BL141.1 — quebra e ajuste (2026-11-14, Windows 11, Chrome headless/SwiftShader)

| Superfície | Evidência | Estado |
|---|---|---|
| Lib | `crates/core/src/text.rs`: 5 testes novos de `layout` (cabe = uma linha; quebra em espaço dentro de 90 %; `\n` e linha vazia; palavra maior que o quadro encolhe até exatamente 90 %; muitas linhas encolhem até caber na altura) e a recusa de controle trocada (`\n` aceito, `\t`/`\r` recusados); `content.rs` e `tests/accented_titles.rs` ajustados ao novo contrato | V (executado) |
| CLI headless | `crates/core/tests/title_layout.rs` (4): plano sem `lines`/`scale` quando cabe; título longo em 1920×1080, 1080×1920 e 1080×1080 dentro de 90 % nas duas direções e sem palavra perdida; `\n` e tamanho 100 % com palavra gigante; processo `tessel-headless` real: plano com linhas, `set clip_text:` curto remove o layout, `history.undo` o devolve | V (executado) |
| Monitor / quadro / export (pixels) | `crates/render/tests/title_layout.rs` (3, Chrome + FFmpeg): título longo em 320×180 e 180×320 com ≥ 2 faixas de linha, tudo dentro da área segura, bloco e cada linha centrados (tolerância de uma célula de glifo na horizontal e do acolchoamento dos acentos na vertical); `\n` desenha 2 linhas e o mesmo texto com espaço volta a 1; export H.264 vertical com a mesma caixa (±2 px) e o mesmo número de linhas do `render.frame`. `accented_titles.rs` 3/3 sem mudança (título de uma linha igual). Conferência visual pela CLI (`render.frame` 1920×1080 e 1080×1920) | V (executado) |
| UI | sem teste novo: o monitor usa o mesmo plano e o mesmo runtime; a quebra automática vale para o texto digitado no campo, a explícita só por `set`/`op` | Parcial (não verificado na janela) |

Revisão B e R2 (histórico): a comparação histórica do export aceitava divergências em 1 % do quadro incluindo o fundo; o teste
passou a comparar foreground e linhas dos acentos, com controle negativo de til→agudo. Eventos tardios de input são descartados quando
o clipe exibido difere do selecionado e o reset Texto sem operação saiu da apresentação. Provas usam `run_surface`; captura indisponível
vira SKIP, nunca PASS.

**Integração com `main` (PR #21, após os merges das PRs #20, #23 e #22): gates reais executados** (15–17 GiB livres, piso de 12 GiB,
runtime real, Chrome, FFmpeg). `TESSEL_REQUIRE_RUNTIME=1 ./scripts/cargo test --workspace --locked --no-fail-fast`: **1668 passaram,
0 falharam, 12 ignorados**; `clippy -D warnings` e `fmt --check` limpos; `./scripts/runtime test` 181/181; `scripts/accented-titles-proof.py`
(com `with-gpu-lock.py`): **headless 11/11, janela 21/21**, 0 SKIP; todos os `scripts/*.spec.py` passam (helper normal e `-O`);
`cargo tree -p tessel-editor-core` sem gpui/wry. **Export de títulos, impresso antes de qualquer decisão** (`accented_titles.rs`, 3/3, uma
thread): `glyph comparison changed=1 union=966 worst_row_fraction=0.0238` (limites do teste mantidos: global ≤ 2 % e por linha ≤ 1 pixel ou ≤ 15 %;
nenhuma tolerância foi alterada) e o controle negativo (til→agudo) é rejeitado. Não houve digitação física no campo nem igualdade
entre escalas ou com o pixel da WKWebView nativa.
