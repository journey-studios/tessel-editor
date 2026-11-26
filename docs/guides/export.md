# Exportar vídeo com progresso

O projeto precisa estar salvo. Rode `./scripts/tessel setup --check` para conferir as ferramentas de render. Os caminhos relativos dos comandos abaixo partem da pasta em que você chamou o CLI.

```sh
./scripts/tessel export aventura.tessel.json final.mp4
./scripts/tessel export aventura.tessel.json trecho.mp4 --range 120 150
./scripts/tessel export aventura.tessel.json previa.mp4 --clip take-final
```

- `--range START END` usa quadros da sequência, com início inclusivo e fim exclusivo. O exemplo exporta 30 quadros.
- `--clip ID` exporta o trecho da aproximação de câmera salva nesse clipe. A prévia padrão tem até 640 px no lado maior e sai sem áudio.
- `--preset ID` escolhe um preset. Vídeos inteiros e intervalos usam `fullhd` por padrão.
- `--sequence ID` escolhe a sequência. `--replace` autoriza trocar a saída existente após a verificação.
- `--gl gpu` é o padrão. Use `--gl swiftshader` para render por software. `--renderer CAMINHO` usa um binário já compilado.

O progresso aparece em stderr e o relatório JSON final sai em stdout. Pressione Ctrl+C para pedir o cancelamento ao job e aguardar a limpeza. [Opções de exportação](../features/video-export.md)
