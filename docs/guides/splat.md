# Gerar um splat a partir de uma imagem

## Pela janela

1. Salve o projeto e importe uma imagem estática, como PNG ou JPEG, na **Biblioteca**.
2. Selecione a imagem, clique em **Gerar splat** e acompanhe a tarefa. O splat aparece na Biblioteca quando a geração termina.
3. Arraste o splat para uma trilha de vídeo e salve o projeto. Você pode usar o mesmo item em várias tomadas. O editor guarda o PLY gerado com o projeto.

A geração usa o Apple SHARP local. Instale o comando `sharp` e seu checkpoint. Para apontar para uma instalação específica, defina `TESSEL_SHARP` e `TESSEL_SHARP_CHECKPOINT` antes de abrir o editor ou rodar a CLI. No Mac, o padrão é MPS. Consulte a configuração completa e a licença dos pesos no [contrato de geração](../features/splat-generation.md).

## Pela CLI

Troque os caminhos pelos seus e `v1` pelo ID de uma trilha de vídeo existente. `place` é opcional e recebe posição e duração em quadros. Sem ele, o resultado entra direto na Biblioteca. O ID do splat vem em `applied.asset`.

```sh
./scripts/tessel media --empty <<'JSONL'
{"op":"project.open","path":"/caminho/aventura.tessel.json"}
{"op":"media.splat.generate","path":"/caminho/aventura.png","place":{"id":"take-final","track":"v1","start":0,"duration":90}}
{"op":"project.save","path":"/caminho/aventura.tessel.json"}
JSONL
```

Com a janela aberta, `./scripts/tessel ui tree` mostra os IDs das trilhas. O equivalente ao botão da Biblioteca é `./scripts/tessel ui invoke library.generate_splat generate`.
