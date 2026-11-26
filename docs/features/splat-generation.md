# F074 — Gerar splats a partir de imagens

`media.splat.generate` chama o Apple SHARP local, importa o PLY calibrado na biblioteca do
projeto e pode criar um clipe em uma trilha indicada pelo chamador. O editor guarda o PLY
como mídia gerenciada. Ao salvar, a biblioteca e a timeline continuam funcionando sem
executar o modelo novamente. A câmera da captura vem do fluxo de [importação SHARP](sharp-import.md).

[Guia de uso pela janela e pela CLI](../guides/splat.md) · `./scripts/media help splat`

## Configuração local

Instale o [Apple ml-sharp](https://github.com/apple/ml-sharp) e o checkpoint no ambiente local.
O editor usa `sharp` no PATH ou o executável indicado por `TESSEL_SHARP`. Sempre fornece
um checkpoint local ao processo. Use `TESSEL_SHARP_CHECKPOINT` ou o cache padrão
`~/.cache/torch/hub/checkpoints/sharp_2572gikvuh.pt`. O padrão é MPS no Mac e CPU nos demais
sistemas. `TESSEL_SHARP_DEVICE` permite escolher outro dispositivo suportado pela instalação.
O modelo segue a [licença dos pesos](https://github.com/apple/ml-sharp/blob/main/LICENSE_MODEL).

O comando usa o mesmo FFmpeg e FFprobe das outras operações de mídia. O editor espera
uma imagem estática em um formato lido pelo SHARP, como PNG ou JPEG.

## CLI e biblioteca Rust

Abra um projeto com pasta de trabalho, ou crie um com `dir`. Passe `path` para uma
imagem em disco ou `image` para o ID de uma imagem da biblioteca. `folder` pode
escolher a pasta de destino. Se omitida para `image`, o splat fica na pasta da imagem.

```sh
./scripts/media --empty <<'JSONL'
{"op":"project.open","path":"/caminho/projeto.tessel.json"}
{"op":"media.splat.generate","path":"/caminho/ilustracao.png"}
{"op":"project.save","path":"/caminho/projeto.tessel.json"}
JSONL
```

O retorno traz o ID em `applied.asset`. Use-o em `clip.add` quantas vezes precisar ou
passe `place` para inserir durante a geração. `start` e `duration` são quadros.
A trilha já deve existir.

```json
{"op":"media.splat.generate","image":"ilustracao","place":{"id":"take","track":"v1","start":0,"duration":90}}
```

A API Rust usa `tessel_editor_core::headless::execute_ext` com `MediaHost` de
`tessel-editor-media`. O core continua responsável pelo registro de mídia, clipes e
transações. A inferência e a cópia passam pelo job de mídia. O comando espera e aplica
por padrão na CLI. Com `wait:false`, use `job.wait` e `job.commit`. `job.cancel`
interrompe o processo SHARP e aguarda a limpeza do diretório temporário.

## Biblioteca da janela

Selecione uma imagem com arquivo e clique em **Gerar splat**. A janela aplica o job
ao terminar e mostra o ativo na biblioteca. O comando semântico é
`ui invoke library.generate_splat generate`. `ui op` também aceita o JSON da CLI.

## Estado e falhas

Mídia e clipe entram em uma única operação de undo. A falha de inferência, captura
inválida, erro de inserção ou troca de projeto antes da aplicação preservam o documento.
O importador pode manter no disco um PLY gerenciado já validado para reutilização.
Um resultado com bytes e captura iguais aos de um splat já importado devolve o mesmo
ID. Com outra captura para os mesmos bytes, a operação devolve `capture_conflict`.

As mensagens incluem `no_workspace`, `not_image`, `sharp_unavailable`,
`sharp_checkpoint_missing`, `sharp_failed`, `invalid_splat` e `stale_result`.
As verificações da trilha e do clipe vêm das operações normais do editor.

## Evidências

`./scripts/cargo test --locked -p tessel-editor-media --test splat_generation` cobre
a CLI e a aplicação pelo host com um processo determinístico, incluindo falha do
modelo, reversão da inserção, deduplicação e Save/Open. A prova local de integração
usa três ilustrações em PNG e o SHARP em MPS. As três saídas de 1.179.648 gaussianas
foram colocadas na timeline pelo campo `place`, reutilizadas na montagem e exportadas
pelo Tessel Editor.

A janela nativa do Mac passou pela ação semântica do botão, executou o SHARP em MPS e
os dados de captura apareceram no novo item da biblioteca e o item permaneceu ao salvar.
