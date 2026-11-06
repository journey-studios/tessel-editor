# F030 — Gaussian splats com Spark 2.4.0 (P04)

## Status

Lib/CLI headless **V** (Chrome real); UI **V** (WKWebView real, `scripts/live-proof.py`).
Spark **2.4.0** fixado em `runtime/package.json` com Three.js 0.187.0. Sem CDN: o pacote vem de
`node_modules` e o servidor de assets só entrega arquivos registrados.

## Contrato

- Formatos: `.splat`, PLY Gaussian e `.spz`. O core identifica **pelo conteúdo**
  (`crates/core/src/splat.rs`); `meta.format` (`splat|ply|spz`) vai ao runtime, que o converte no
  nome de arquivo que o Spark exige (`runtime/src/splat-format.js`). PLY de pontos comum é
  rejeitado (a mensagem lista as propriedades Gaussian ausentes), PLY comprimido/ASCII também; nada é aceito como splat por extensão.
- Estágio (`runtime/src/splat-stage.js`): um `SparkRenderer` por contexto WebGL
  (`minSortIntervalMs: 0`, `autoUpdate: false`). Um frame só é devolvido depois do **settle**: o
  loop chama `spark.update` até o sort terminar e o centro ordenado coincidir com a câmera do
  pedido (tolerância 1e-4, máx. 400 passes; esgotar falha com `render_failed`).
- Cada asset tem uma malha "emprestada" à primeira cena; outras cenas decodificam a sua
  (`SplatMesh.dispose()` descarta os dados compartilhados). Perda de contexto descarta o estágio
  e as malhas; elas voltam sob demanda dos bytes guardados.
- Load cancelado (`asset.release` durante o load) responde `cancelled`; decodificação inválida ou
  com 0 splats responde `decode_failed` com o nome do asset.
- `capabilities.runtime.spark` informa a versão carregada.

## Evidência

- `./scripts/runtime test`: `runtime/test/splat.test.mjs` (Chrome real) com encoders de
  `.splat`/`.ply`/`.spz` — contagem de splats e cor idêntica nos três contêineres, ordenação por
  câmera (frente/trás) com visita em ordens opostas e hash igual bit a bit, splats translúcidos
  (mistura dependente da ordem), movimento por tempo (0→2→1→0 repete o frame), oclusão por caixa
  opaca da cena, o mesmo asset duas vezes na cena, arquivo malformado (`decode_failed`), load
  cancelado (`cancelled`, nada fica para trás) e perda/restauração de contexto (mesmo frame).
- `cargo test -p tessel-editor-render --test frames` (7) e `crates/render/tests/export.rs` (2).
- `scripts/live-proof.py`: 56/56 na janela macOS real (splat incluso).

## Splat numa cena dirigida (F048)

O mesmo Spark, o mesmo estágio e o mesmo `settle` desenham um splat de um plano de cena dirigida:
o plano ([F037](scene-plan.md)) carrega a matriz de mundo do nó e a câmera avaliadas e o recurso
`spz` aprovado; o runtime só os desenha. Contrato e limites em [scene-splat.md](scene-splat.md).

## Limites

Medido em SwiftShader (CPU); ordem de composição entre splats e planos transparentes segue as
limitações documentadas do Spark (splats ordenados entre si, planos pela ordem do plano).
Outros formatos do Spark (`.ksplat`, `.sog`) não têm prova aqui e não são aceitos.

Cor (BL019-F1): o desenho vai a um render target e, nele, o Spark 2.4.0 usa `ColorManagement.workingColorSpace`
(linear) para ligar `encodeLinear`, o que escurecia a cor guardada. `keepStoredColour`
(`runtime/src/splat-stage.js`) mantém o uniform em `false` só no material do Spark, depois do `onBeforeRender` do
próprio Spark. **Isto também muda a cor do caminho de projeto (`scene`)**, não só o da cena dirigida: um splat do
projeto agora mostra a cor guardada. Detalhes e prova em [scene-splat.md](scene-splat.md#cor-bl019-f1).
