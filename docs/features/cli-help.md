# F076 — Ajuda local no CLI

O comando `help` lista os [guias de uso](../guides/README.md). `help <tópico>`
imprime os passos pela janela e exemplos copiáveis para os tópicos listados no índice, incluindo [export, setup e pacotes](cli-workflows.md).
O binário incorpora os mesmos arquivos Markdown do repositório.

## Terminal e JSONL

```sh
./scripts/tessel help
./scripts/tessel help splat
./scripts/tessel help camera
./scripts/tessel help preview
./scripts/tessel help export
./scripts/tessel help setup
./scripts/tessel help package
./scripts/tessel help restore
```

`./scripts/tessel` sem argumentos, `--help` e `-h` mostram o índice. A ajuda roda
sem projeto, janela ou renderer e retorna código 2 para tópico desconhecido ou
excesso de argumentos.

O comando central abre o app com `editor`, controla a janela com
`ui` e encaminha o JSONL em stdin com `headless`, `media` e `render`. Os quatro
scripts anteriores encaminham os mesmos argumentos para os respectivos subcomandos.

Dentro de uma sessão JSONL, envie `{"op":"help"}` para receber `topics` e
`markdown`. Envie `{"op":"help","topic":"preview"}` para receber `topic` e
`markdown`. Na sessão da janela, use `./scripts/tessel ui op '{"op":"help","topic":"preview"}'`.
Um tópico desconhecido retorna `unknown_topic` com os tópicos válidos. A operação
é de leitura e não altera projeto, histórico ou preview.

## Biblioteca

`tessel_editor_core::help::guide(None)` retorna o índice. Passe o ID com
`Some("preview")` para retornar um guia. Para adicionar um tópico, inclua o arquivo em
`docs/guides/`, a entrada em `core::help` e o link no índice de guias.

O recurso fica no terminal. Na janela, os guias indicam onde estão os controles das features.

## Evidências locais

Os testes de `crates/core/tests/headless.rs` conferem o índice, o texto de cada
guia, a saída do binário, a recusa de tópico desconhecido e a preservação do projeto.
Os quatro scripts foram executados sem janela, com saída igual ao guia em disco.
O mesmo pedido JSONL passou em headless, media, render e numa janela nativa do Mac
acessada pelo socket semântico.

O ponto de entrada central foi validado a partir de fora do repositório. Os três
tópicos coincidiram com os guias em disco. As rotas JSONL receberam stdin e
argumentos e devolveram as mesmas respostas dos aliases. No Mac, `tessel editor`
abriu uma janela isolada e `tessel ui` consultou o status e o guia no socket dela.
