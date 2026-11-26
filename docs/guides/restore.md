# Restaurar um projeto editável

Escolha um pacote e uma pasta de destino nova.

```sh
./scripts/tessel restore aventura.tesselpack aventura-restaurada
```

O comando aceita arquivos `.tesselpack` e pastas de pacote. Ele valida caminhos e hashes antes de gravar o projeto restaurado. Um destino existente é recusado.

Abra `aventura-restaurada/project.tessel.json` no editor para continuar a edição. O campo `document` do resultado JSON traz o caminho. As mídias passam a vir da pasta restaurada. [Criar um pacote](package.md)
