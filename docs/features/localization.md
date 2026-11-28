# F078 — Localização de projetos

Um projeto pode ter um idioma-base e traduções independentes. A composição, as
trilhas, as definições de grupo e a biblioteca são compartilhadas. Cada tradução
guarda só os ajustes por ID de sequência, clipe e filho de grupo.

## Documento e revisão

O schema 4 adiciona `localization`, com o idioma-fonte, os ajustes das traduções e
as assinaturas dos conteúdos aprovados. Projetos sem localização continuam sendo
gravados no menor schema que comporta seus dados. O documento guarda a timeline-base
uma vez. Ao abrir, o editor mostra a base e permite selecionar outro idioma.

Os ajustes podem trocar o conteúdo e estilo de títulos, filhos de texto de grupos,
cues de legendas, início, duração, deslocamento na fonte e duração da sequência.
Uma tradução também pode trocar a mídia por outra do mesmo tipo, apontar para
uma definição de grupo diferente e ajustar forma, quadros-chave, câmera animada
e propriedades de clipe, como volume e posição. Os tempos usam os relógios que o editor já usa para cada objeto. A seleção
aplica os ajustes à timeline usada pelo monitor, pelo mixer e pelo renderer.

A revisão de cada idioma tem uma situação calculada pelo core.

| Situação | Significado |
| --- | --- |
| `source` | Conteúdo original da composição |
| `incomplete` | Há texto, legendas ou áudio para traduzir ou confirmar como compartilhado |
| `draft` | Conteúdo completo que ainda precisa de aprovação |
| `approved` | Base e ajustes correspondem à revisão aprovada |
| `stale` | A revisão aprovada é anterior à edição da base ou dos dados daquele idioma |
| `conflict` | O ajuste conflita com a base, por exemplo um corte fora da timeline ou um clipe removido |

`null` confirma o uso do conteúdo-base. A omissão mantém uma pendência para revisão.
A assinatura é independente da ordem das chaves JSON e ignora seleção, zoom, idioma
ativo e assets sem referência. Adicionar uma voz de outro idioma só afeta a revisão
quando ela passa a ser usada ali. As edições e aprovações entram no histórico.
A seleção de idioma é uma ação de visualização.

## Biblioteca e interfaces

- `EditorState::execute(Command::Localization(Edit))` adiciona, seleciona, remove,
  consulta, altera e aprova idiomas. `SequencePatch::from_json` lê os campos
  permitidos. `EditorState::localization_status` devolve pendências e conflitos por ID.
- Um bloco é aplicado por inteiro, após validar a timeline resultante. Rejeição de
  campos compartilhados, mídia ausente, intervalo de áudio ou clipes sobrepostos
  preserva a edição anterior. O seletor recusa uma projeção em conflito.
- O [guia de localização](../guides/localization.md), também disponível em
  `./scripts/tessel help localization`, documenta
  `localization.add|select|get|set|status|resolve|approve|remove`.
- `EditorState::resolve_locale` e `localization.resolve` devolvem o documento de
  uma sequência resolvida, com propriedades efetivas e caminhos absolutos. O
  comando mantém a seleção e o documento editável. Um compositor externo pode
  usar a sequência e a biblioteca para entregar formatos próprios.
- Na janela, a faixa de idioma fica abaixo da barra da timeline. Os títulos, grupos,
  cues e cortes editáveis usam os comandos dos controles existentes. A ação **Ajustes**
  aceita um bloco JSON para alterar, por exemplo, a mídia da narração. A automação usa
  `timeline.language`, `flow.language_add`, `flow.language_overrides` e
  `timeline.language.approve`.
- **Exportar vídeo** e `tessel export` integral exportam um MP4 por idioma. O
  exportador nativo verifica todos os destinos antes do primeiro render e usa a
  mesma composição capturada para o lote. Cada MP4 e suas legendas externas usam
  o código no nome. Uma falha de render posterior preserva as entregas concluídas.
  JSONL aceita `all_locales:true` ou `locale` para uma versão específica.
  A configuração sempre inclui o idioma-base. O diálogo de exportação integral
  oferece só destino e preset. Para faixa, clipe ou idioma específico, use a CLI.

## Limites deste incremento

O pacote completo inclui os assets de todos os idiomas. Quando o pacote calcula
um hash novo para mídia sem hash de origem, a versão passa por nova revisão. Pacotes de subset e
importação de sequências de um projeto localizado exigem suporte a remapeamento de
IDs. Ajustes conflitantes podem ser corrigidos na CLI com `localization.get` e
`localization.set`. Texto já rasterizado faz parte do asset e fica fora da
detecção de pendências.

## Evidências

- `./scripts/cargo test -p tessel-editor-core --locked` cobre reabertura, edição
  de título, grupo e legenda, seleção, duração, áudio, undo, conflitos e aprovação.
- Prova local com uma composição retrato e três idiomas. Um único `tessel export`
  produziu H.264/AAC com 24, 27 e 30 quadros. O vídeo e o SRT tinham o texto de cada
  variante, as três faixas tinham frequências distintas, e o FFmpeg decodificou os
  arquivos completos. Aprovações sobreviveram de headless a render.
- Na janela nativa do Mac, o socket semântico confirmou troca de idioma,
  plano localizado, adição, ajustes, aprovação e exportação em lote pelo diálogo.
  A prova acionou os controles por ID.
- A suíte do workspace passou com 3.272 testes e 17 ignorados. Os testes focados
  passaram também após a validação final do tamanho dos textos de grupo. O Clippy
  passou com a exceção de `large_enum_variant` no enum `Op`.

A migração da montagem de demonstração em BL170.1 comparou os 226 clipes de cada
idioma com os originais, incluindo propriedades de clipe e definições de grupo.
A sequência de tomadas verticais está na mesma biblioteca. O compositor externo
usa `localization.resolve` e `render.audio` ao entregar as duas proporções.

Acompanhamento em [BL170](../backlog.md).
