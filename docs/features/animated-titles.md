# F066 — Títulos animados na timeline (BL169.4)

O clipe de título guarda conteúdo, tamanho, cor, fonte, alinhamento e preset de entrada. A camada de texto é avaliada no plano da timeline e composta junto com os demais clipes. Ao substituir o texto ou o preset, o monitor e o export usam a mesma configuração.

## Presets

| Valor | Entrada |
|---|---|
| `none` | Texto visível desde o início |
| `rise` | Deslocamento de baixo para cima, revelação e ganho de opacidade |
| `fade` | Dissolver, com ganho gradual de opacidade |
| `pop` | Salto, com entrada em escala e acomodação |

`animation_ms` ajusta a duração de 1 a 3.000 ms, com padrão de 450 ms. O tempo é local ao clipe e vem do relógio da sequência. O plano é independente da ordem dos seeks. A entrada termina dentro do clipe mesmo quando ele é menor que a duração pedida.

`font:"sans"` usa Inter empacotada no runtime. `align:"left"` ancora a caixa no topo esquerdo, relativo à posição do clipe com origem no centro da tela. `align:"center"` mantém a âncora central. Documentos que omitem os novos campos preservam o título bitmap de [F050](accented-titles.md).

## Editar

Use **Adicionar título** na barra da timeline para inserir um clipe no cursor e selecioná-lo. No inspetor **Vídeo**, edite a frase e escolha o preset. Enter confirma e Shift+Enter insere uma quebra de linha. Para assistir à entrada, reproduza a partir do início do clipe. As ações chamam `Command::AddTitle` e `Command::SetClipTextField`, as mesmas rotas semânticas usadas pela CLI. Cada escolha é reversível pelo histórico.

```json
{"op":"track.add","id":"titles","kind":"video","name":"Títulos"}
{"op":"clip.add","id":"intro","track":"titles","start":0,"duration":90,"properties":{"position_x":-45,"position_y":-20},"text":{"content":"A história fica.","size":10,"color":"#ffd090","font":"sans","align":"left","animation":"rise","animation_ms":530}}
{"op":"set","target":"clip_text:intro","field":"animation","value":"pop"}
{"op":"set","target":"clip_text:intro","field":"content","value":"A aventura continua."}
{"op":"plan","frame":8}
```

`clip.text` substitui a especificação completa. Um campo inválido é recusado antes de alterar o documento. `project.save` grava os valores e `project.open` os restaura.

`title.add` é o atalho de criação. Na CLI da janela, use `invoke timeline.add_title invoke`. O atalho insere até três segundos na trilha visual superior se ela comportar o título. Quando necessário, a mesma operação cria uma trilha.

## Escopo

As animações são entradas predefinidas do clipe inteiro. Uma composição que usa cor ou ritmo diferentes por linha pode usar dois clipes. Ilustrações, texto dentro de cartões de demonstração e transições podem permanecer em camadas gráficas independentes. Legendas externas seguem como SRT.

## Evidência

No Windows, o build dos binários passou. Os testes focados passaram no core (17),
no runtime Chromium (10) e no inspetor GPUI (9). A janela real abriu a montagem com
116 títulos, mostrou o inspetor do clipe selecionado e restaurou o preset com Desfazer.
Na CLI de render, a comparação de quadros confirmou mudança visual ao trocar o preset
e estabilidade ao revisitar o quadro. O export nativo da faixa de 36 quadros em 1080p
com áudio foi integralmente decodificado. Evidência local em
`projects/tessel-apresentacao/out/native-title-proof/`.
