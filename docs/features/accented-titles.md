# F050 — Títulos com acentos em português (BL141)

## Objetivo e limites

A camada de texto só desenhava ASCII: um título como "AÇÃO" virava PNG feito à mão. Agora um título aceita **ASCII
imprimível + as letras latinas acentuadas do português e demais línguas do oeste europeu + pontuação tipográfica**
(lista única em [`crates/core/src/text_coverage.txt`](../../crates/core/src/text_coverage.txt): `À Á Â Ã Ä Ç È É Ê Ë Ì Í Î Ï Ñ Ò Ó Ô Õ Ö Ù Ú Û Ü Ý`
e minúsculas, `¿ ¡ « » ‘ ’ “ ” – — … º ª ° ·`). O resultado é o mesmo no monitor (WebView), no quadro headless
(`render.frame`) e no vídeo exportado: é a mesma camada de texto do plano.

**Fonte, determinismo e licença.** Nenhum arquivo de fonte é carregado e nenhuma fonte do sistema é consultada: o runtime
desenha células de uma grade **5×7 própria do projeto** (`runtime/src/font.js`, MIT, a mesma [LICENSE](../../LICENSE) do
repositório; a tabela de glifos já era do projeto). Um acento é uma **marca composta** sobre a letra maiúscula (agudo,
grave, circunflexo, til e trema em 2 linhas acima; cedilha em 2 linhas abaixo). Como são células inteiras, o desenho é igual
em qualquer máquina, motor e escala (Chrome e WKWebView). Não há fonte de terceiros a atribuir ([THIRD_PARTY_NOTICES](../../THIRD_PARTY_NOTICES.md)
registra isso); uma fonte vetorial real, com métricas e minúsculas, continua [BL053](../backlog.md).

Limites que **não mudaram** (layout preservado): um título é **uma linha**, centrada no quadro, com `size` = altura do corpo da
letra em % da altura do quadro; **não há quebra de linha nem alinhamento**: texto mais largo que o quadro é cortado nas
bordas, como antes (quebra e multilinha: BL141.1). Minúsculas continuam desenhadas como maiúsculas. Aspas tipográficas e
travessões reusam os glifos retos (a célula de 5 colunas não os distingue; BL141.2). Fora da cobertura (`€`, `ß`, CJK, emoji,
`ÿ`) o texto é **recusado** antes de qualquer quadro (nunca desenhado como caixa).

**Layout dos acentos.** O corpo da letra mantém o tamanho e o lugar do texto sem acento: quando algum caractere do título tem
marca, a caixa do texto cresce 2 linhas **em cima e embaixo** (simétrico), então o centro e o `size` não mudam e "AÇÃO" fica
no mesmo lugar que "ACAO" (testado em pixels).

## Contrato do core / biblioteca

`tessel_editor_core::text::normalize(&str) -> Result<String, CommandError>` e `content::TextSpec`:

- **Normalização NFC.** `ã` e `a` + U+0303 são o mesmo texto. `TextSpec::new` (usado por `clip.text`, `asset.add`, `clip.add`,
  leitura de documento, `set clip_text:`) compõe `letra + marca` na letra precomposta da cobertura e guarda sempre a forma
  composta: plano, documento salvo, desfazer e export recebem uma única grafia. `TextSpec::validate` recusa um texto fora de NFC.
- **Recusas** (`unsupported_text`, com `text` e `code_points` nos detalhes; estado, revisão e histórico intactos): caractere fora
  da cobertura, controle/nova linha/tabulação, marca solta ou que não compõe (`a` + U+0307, `ã` + U+0301, duas marcas na mesma
  letra). Vazio e mais de 256 caracteres (depois de compor): `invalid_value`.
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
| Monitor / quadro / export (pixels reais) | `crates/render/tests/accented_titles.rs` (Chrome + FFmpeg): AÇÃO tem a largura de ACAO, til ≥ 4 px acima e cedilha ≥ 4 px abaixo do mesmo corpo (±2 px), maiúscula = minúscula, NFD = NFC byte a byte; export H.264 do quadro: mesma caixa e pixels de glifo iguais ao `render.frame` (965 vs 966 acesos, caixas iguais) | V |
| UI | `crates/desktop/src/workstation/title_text_tests.rs` (4 testes GPUI: seção/nó só para título, Enter = uma edição composta com undo, recusa restaura o campo e reporta, `set` externo chega ao campo; mutação sem `sync_title_text` derruba os 4) e `scripts/accented-titles-proof.py` na janela real própria (21 checks por `ui op/get/set/invoke`, `render.frame` PNG lido no script e captura nativa [`assets/bl141-accented-title-window.png`](assets/bl141-accented-title-window.png)) | V (sem digitar com teclado físico) |

Limites: não houve digitação física no campo nem clique de coordenada (o teste GPUI aciona a entidade `Input`; o script usa `set`);
a captura nativa mostra a janela, não substitui os testes de pixel; quebra de linha/alinhamento/multilinha não existem
(BL141.1); só a cobertura listada (BL141.3: ampliar com fonte real = BL053).
