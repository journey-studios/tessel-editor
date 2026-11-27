# Preparar o ambiente de edição

O comando roda com Python 3 e usa as ferramentas de build do repositório. Tenha Node 20+, pnpm, FFmpeg, FFprobe e Chrome ou Chromium instalados.

```sh
./scripts/tessel setup
./scripts/tessel setup --check
```

`setup` instala as dependências do runtime do checkout com o lockfile e compila a página do editor. Em seguida, confere FFmpeg e FFprobe e abre uma sessão curta do Chrome para verificar o render. O relatório JSON mostra caminhos, codecs detectados e o renderizador WebGL.

`--check` executa só a verificação da instalação atual. O modo padrão usa GPU, como `tessel export`. Passe `--gl swiftshader` para verificar o render por software. Para ferramentas instaladas fora dos locais usuais, configure `TESSEL_FFMPEG`, `TESSEL_FFPROBE` ou `TESSEL_CHROME`. [Configuração e descoberta de ferramentas](../media-setup.md)
