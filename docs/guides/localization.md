# Um projeto, vários idiomas

1. Na timeline, use **Ativar localização** e informe o idioma do material original, por exemplo `pt-BR`.
2. Use **+ Idioma** para adicionar `en`, `es` ou outro código. O seletor carrega a variante escolhida no monitor e na timeline.
3. Edite manualmente o conteúdo de títulos, textos de grupos e legendas no inspetor. Mover e aparar clipes na mesma trilha altera os cortes só nesse idioma. Volte à base para editar a composição compartilhada.
4. Importe a narração pela biblioteca. Em **Ajustes**, troque a mídia do clipe de voz e ajuste sua duração. A janela mostra o JSON da sequência atual para poder reaproveitar os ajustes já existentes.
5. Revise as pendências e use **Aprovar idioma** quando o vídeo estiver pronto. Uma revisão fica vencida quando a composição ou os dados daquele idioma mudam.

## CLI

Envie os comandos pela mesma sessão de `./scripts/tessel headless` depois de abrir seu projeto.

```jsonl
{"op":"localization.add","locale":"pt-BR"}
{"op":"localization.add","locale":"en"}
{"op":"localization.add","locale":"es"}
{"op":"localization.select","locale":"en"}
{"op":"localization.get","locale":"en","sequence":"sequence"}
{"op":"localization.set","locale":"en","sequence":"sequence","overrides":{"length":960,"clips":{"abertura":{"text":"The third torch…"},"voz":{"audio":"voz-en","start":12,"duration":910},"musica":{"audio":null},"chat":{"group_texts":{"pergunta":"Where was the third torch?","marca":null}},"legendas":{"cues":[{"start_ms":0,"end_ms":2000,"text":"The third torch…"}]}}}}
{"op":"localization.status"}
{"op":"localization.approve","locale":"en"}
{"op":"project.save"}
```

Substitua os IDs pelos da sua timeline e biblioteca. O código do idioma é salvo em minúsculas.
`localization.get` devolve o ajuste salvo, inclusive quando a versão tem conflitos.
`localization.set` troca o bloco inteiro da sequência de forma atômica. Para editar um bloco que
já existe, consulte-o e envie a versão atualizada. Envie `{}` para limpar o bloco.
Se `sequence` for omitida, usa a sequência atual. Os tempos `start`, `duration` e `length` usam
quadros. `source_start` é o deslocamento na mídia em segundos racionais (`{"num":1,"den":2}`
para meio segundo). Os tempos das legendas usam milissegundos relativos ao clipe.

`null` em `text`, `audio`, `cues` ou em um filho de `group_texts` confirma que o conteúdo da
base pode ser usado, por exemplo para música ou nomes próprios. Um campo ausente permite
prévia com a base e conta como pendência para aprovação. `localization.status` lista as pendências
com seus IDs e informa conflitos de posição, mídia e IDs que deixaram de existir.
`localization.remove` remove uma tradução e seus ajustes em uma operação desfeita pelo histórico.

## Resolver uma variante para composição externa

```jsonl
{"op":"localization.resolve","locale":"es","sequence":"main"}
```

A resposta tem `locale` e `document`. O documento inclui a sequência com os
ajustes aplicados, propriedades dos clipes e caminhos de mídia resolvidos. A
seleção atual e o arquivo editável ficam como estavam.

Além dos ajustes de conteúdo, um bloco de clipe pode conter `media` para imagem,
vídeo ou cena do mesmo tipo, `group_def` para uma definição de grupo da biblioteca,
`title_style` para ajustes de estilo, `shape` para a geometria, `keys` ou
`camera_keys` para substituir trilhas de animação. `properties` substitui o bloco
de propriedades de um clipe naquele idioma. Envie `{}` em `properties` para
repor os valores padrão naquele clipe.

## Exportar

**Exportar vídeo** na janela e o export integral pelo CLI geram um MP4 por idioma habilitado.
Com a base `pt-br`, mais `en` e `es`, este comando gera `filme.pt-br.mp4`, `filme.en.mp4`
e `filme.es.mp4` na pasta informada.

```sh
./scripts/tessel export filme.tessel.json out/filme.mp4 --preset fullhd
./scripts/tessel export filme.tessel.json out/teste-es.mp4 --locale es --range 0 90
```

Para exportar só uma versão, use `--locale`. No JSONL de render, use `all_locales:true`
ou `locale:"es"` em `export.start` e `export.preflight`. Saídas que usam legendas externas
seguem a configuração da faixa e recebem o arquivo SRT junto com seu MP4.
Versões pendentes podem ser exportadas para revisão. Um conflito bloqueia a exportação em lote.
Em caso de falha de render no meio do lote, os vídeos já concluídos ficam disponíveis.
