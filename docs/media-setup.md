# Setup de mídia, runtime e export

O que a máquina precisa para as fatias P03–P12. Nada disto é baixado em tempo de execução;
o runtime de render funciona offline (sem CDN).

| Componente | Para quê | Como é encontrado | Sem ele |
|---|---|---|---|
| FFmpeg + FFprobe | probe, decode, proxies, encode, verificação | `TESSEL_FFMPEG` / `TESSEL_FFPROBE`, `PATH`, depois os lugares usuais da plataforma (macOS/Linux: `/opt/homebrew/bin`, `/usr/local/bin`; Windows: WinGet, ver [Descoberta](#descoberta-dos-executáveis)) | `backend_unavailable` / blocker `encoder_unavailable` no preflight |
| Node 20+ | renderer headless (export, `scripts/render`) | `TESSEL_NODE`, `node` no `PATH` (`node.exe` no Windows), depois Program Files / `LOCALAPPDATA` / WinGet no Windows | `backend_unavailable` |
| Chrome/Chromium | WebGL do renderer headless | `TESSEL_CHROME`, depois (macOS/Linux) os apps do macOS e os nomes no `PATH`; (Windows) `chrome.exe` no `PATH` e as instalações usuais do Chrome/Chromium; `runtime/worker/chrome.mjs` | idem |
| `runtime/` instalado | Three.js 0.187.0 e Spark 2.4.0 | `TESSEL_RUNTIME_DIR` ou o diretório do repositório; `pnpm install` dentro de `runtime/` | idem |
| Dispositivo de saída de áudio (CoreAudio no macOS, via `cpal`, crate `tessel-editor-audio`) | saída do playback e relógio de amostras | dispositivo padrão do sistema, aberto pela thread `tessel-audio`; nenhum binário externo | playback segue pelo relógio de parede, **mudo**, e `playback.status` informa `audio.error.code = audio_output_unavailable` (nunca falha em silêncio) |

`TESSEL_GL=swiftshader` (padrão, determinístico) ou `gpu`.

Os encoders realmente usados são os que `ffmpeg -encoders` lista: o preset pede `libx264` e `aac`.
`project.describe`/`export.preflight` devolvem a versão de FFmpeg e a lista de capacidades; não há
promessa de codec que a máquina não tenha. Distribuição e licença dos binários (FFmpeg com
`libx264` é GPL) **não** foram resolvidas: este repositório não empacota FFmpeg.

## Descoberta dos executáveis

Um resolvedor só para FFmpeg, FFprobe e Node (`crates/media/src/tools/locate.rs`, usado por `Tools::discover` e por
`RuntimeConfig::discover`) e um para o Chrome (`runtime/worker/chrome.mjs`). Ordem, em todos: **override** (variável
`TESSEL_*`), cada pasta do **`PATH`** na ordem, **lugares de instalação** da plataforma. Só um **arquivo** conta: um
diretório chamado `ffmpeg.exe` é rejeitado.

| Variável inválida (não é arquivo) | Efeito |
|---|---|
| `TESSEL_FFMPEG`, `TESSEL_FFPROBE` | **falha** (`tool_unavailable`): pediu-se aquele binário, não há fallback |
| `TESSEL_NODE`, `TESSEL_CHROME` | **ignorada**, segue a busca (comportamento anterior) |

- **macOS/Linux** (nada mudou): o nome como está em cada pasta do `PATH`; para FFmpeg/FFprobe, depois `/opt/homebrew/bin` e
  `/usr/local/bin`; Chrome: os três apps do macOS e depois os nomes Unix no `PATH`. Node não tem lugar extra.
- **Windows**: `.exe` e as demais extensões de `PATHEXT` que o sistema consegue iniciar (`.com .exe .bat .cmd`, na ordem de
  `PATHEXT`; `.exe` sempre entra) para os processos Rust. O launcher JS do Chrome aceita apenas `.com`/`.exe`
  na busca automática, pois não usa shell. Pastas com espaços e entradas entre aspas são aceitas. Depois do `PATH`:
  - Node: `%ProgramFiles%\nodejs`, `%ProgramFiles(x86)%\nodejs`, `%LOCALAPPDATA%\Programs\nodejs`, WinGet `Links`.
  - FFmpeg/FFprobe: WinGet `Links` (`%LOCALAPPDATA%\Microsoft\WinGet\Links` e `%ProgramFiles%\WinGet\Links`) e o layout do pacote
    `Gyan.FFmpeg*` do WinGet, `Packages\Gyan.FFmpeg*\ffmpeg-*\bin`, **dois níveis e nada mais fundo**, o build de versão
    numérica maior primeiro. É o que cobre um `PATH` herdado antes do `winget install`.
  - Chrome: `%ProgramFiles%`, `%ProgramFiles(x86)%` e `%LOCALAPPDATA%` + `Google\Chrome\Application\chrome.exe`, depois
    `Chromium\Application\chrome.exe`, depois o Canary (`Google\Chrome SxS`).
- Sem varredura recursiva do disco, sem shell para procurar, sem versão nem usuário no código, sem mexer em `PATH` nem no
  registro. Edge não é procurado (não é Chrome/Chromium); `TESSEL_CHROME` aceita o `msedge.exe` se o usuário quiser.
- Limites: Rust pode iniciar `.bat`/`.cmd` por `cmd.exe`; os launchers JS sem shell só procuram `.com`/`.exe`.
  Os testes funcionais de vídeo usam o helper de descoberta `runtime/test/ffmpeg-locate.mjs`, com override estrito,
  PATH e candidatos Homebrew/WinGet. O script de desempenho `runtime/scripts/perf/assets.mjs` ainda exige FFmpeg
  no PATH do processo (BL167.2.2). A descoberta não altera o PATH global nem o registro.
- Prova: testes unitários com ambiente/plataforma/arquivos injetados (nenhum muda `env` global) e a prova real do
  [relatório](migration/windows-tool-discovery-report.md). `cargo test -p tessel-editor-media --lib -- --ignored --nocapture
  real_machine` imprime o executável e a origem de cada ferramenta na máquina atual.

## Comandos

```sh
./scripts/runtime test          # JS: formatos, estágio Spark, frames, contexto perdido (Chrome real)
./scripts/fixtures /tmp/dir     # imagem, .splat/.ply/.spz, tom WAV, cena e frames determinísticos
./scripts/media [--empty]       # JSON Lines com as operações de mídia/projeto/export, sem janela
./scripts/render                # JSON Lines com renderer + mídia (export.start precisa deste)
./scripts/headless              # JSON Lines só com o core
```

`TESSEL_REQUIRE_RUNTIME=1 ./scripts/cargo test --workspace --locked` transforma "runtime ausente" em
falha em vez de pular os testes que dependem de Node/Chrome/FFmpeg.
