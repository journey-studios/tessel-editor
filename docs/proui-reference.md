# Referência ProUI — Splice

- Demo: https://pro-ui.dev/examples/video-editor
- Guia GPUI: https://pro-ui.dev/guide/gpui
- Biblioteca oficial: https://pro-ui.dev/library

A demo pública orienta a composição visual: biblioteca de mídia, monitores de origem
 e programa, inspetor e timeline multifaixa. A implementação local reproduz a composição com estado demonstrativo em memória.

## Disponibilidade do kit oficial

O kit GPUI e os exemplos são distribuídos sob licença ProUI, com download autenticado.
Não há cópia licenciada instalada neste projeto. Não confundir a referência pública
com disponibilidade do código-fonte oficial. O shell local usa GPUI diretamente.

Quando o kit autorizado estiver disponível, o guia oficial orienta colocar o crate
completo em `vendor/proui` e usar uma dependência por caminho. Não adicionar uma
dependência inexistente ao Cargo.toml nem armazenar tokens neste repositório.

## Origem dos assets de demonstração

`assets/demo/highlands.png` e `assets/demo/coast.png` foram baixados dos assets da demo
pública ProUI (https://pro-ui.dev/examples/video-editor) apenas como referência de
protótipo. A licença de redistribuição comercial não foi verificada e **não deve ser
presumida**: trocar por mídia própria ou licenciada antes de qualquer distribuição. O kit
pago do ProUI não foi baixado nem adicionado, e continua não sendo dependência.
