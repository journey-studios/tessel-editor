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
- **Windows**: somente binários nativos `.com`/`.exe`, na ordem de `PATHEXT` (`.exe` sempre entra),
  na busca automática Rust e JS. Wrappers `.bat`/`.cmd` são ignorados para que o processo filho
  seja a ferramenta real, que o cancelamento consegue encerrar (BL167.2.7). Pastas com espaços e entradas entre aspas são aceitas. Depois do `PATH`:
  - Node: `%ProgramFiles%\nodejs`, `%ProgramFiles(x86)%\nodejs`, `%LOCALAPPDATA%\Programs\nodejs`, WinGet `Links`.
  - FFmpeg/FFprobe: WinGet `Links` (`%LOCALAPPDATA%\Microsoft\WinGet\Links` e `%ProgramFiles%\WinGet\Links`) e o layout do pacote
    `Gyan.FFmpeg*` do WinGet, `Packages\Gyan.FFmpeg*\ffmpeg-*\bin`, **dois níveis e nada mais fundo**, o build de versão
    numérica maior primeiro. É o que cobre um `PATH` herdado antes do `winget install`.
  - Chrome: `%ProgramFiles%`, `%ProgramFiles(x86)%` e `%LOCALAPPDATA%` + `Google\Chrome\Application\chrome.exe`, depois
    `Chromium\Application\chrome.exe`, depois o Canary (`Google\Chrome SxS`).
- Sem varredura recursiva do disco, sem shell para procurar, sem versão nem usuário no código, sem mexer em `PATH` nem no
  registro. Edge não é procurado (não é Chrome/Chromium); `TESSEL_CHROME` aceita o `msedge.exe` se o usuário quiser.
- Limites: overrides explícitos preservam a escolha anterior, inclusive wrappers configurados pelo usuário;
  sua árvore de processos não é gerenciada por esta descoberta. A busca automática só procura `.com`/`.exe`.
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

## Provas Python no Windows (BL168.8)

As provas `scripts/*-proof.py` precisam de Python 3 (só biblioteca padrão); nesta máquina, Python 3.13 instalado com
`winget install --id Python.Python.3.13 -e --scope user`. Rode com `py scripts/<prova>.py …` (ou o `python.exe` instalado),
não com o `python` da Microsoft Store. Os scripts `sh` do repositório (`scripts/fixtures`, `scripts/cargo`) rodam pelo `sh`
do Git (no PowerShell: `& "$env:ProgramFiles\Git\bin\sh.exe" ./scripts/fixtures <pasta>`).

O que `scripts/portable.py` e `scripts/proof_surface.py` resolvem para as provas, sem mudar nada no macOS/Linux:

- **Texto em UTF-8.** O Python do Windows lê texto em cp1252 fora do modo UTF-8; o script se reexecuta com `-X utf8`.
- **Socket da janela.** O Python do Windows não tem `socket.AF_UNIX`; `portable.connect_ui` abre o `AF_UNIX` nativo pelo
  Winsock (`ctypes`). A janela de uma prova tem socket próprio via `TESSEL_UI_SOCKET` (caminho absoluto, até 107 bytes), e
  nunca se fala com uma janela que você tenha aberta.
- **FFmpeg/ffprobe** achados como o editor acha (`TESSEL_*`, `PATH`, pacote WinGet), não pelo nome no `PATH`.
- **Processos e pastas.** A janela é encerrada com a árvore (`taskkill /T`, inclusive o WebView2); pastas de trabalho em
  caminho longo passam de 260 caracteres dentro do projeto, então use a pasta padrão da prova (em `%TEMP%`) ou uma curta.
- **Captura nativa** da janela por `PrintWindow`: observação, sem clique.

- **Unix sem equivalente:** `/tmp` vira `portable.TEMP` (no Unix continua `/tmp`), `ps` vira `portable.processes()`/`rss_kib()`
  (no Windows os ajudantes do WebView são `msedgewebview2.exe`), `fcntl.flock` vira `portable.lock_file()` (`msvcrt`),
  `os.killpg` vira `portable.kill_group()`, symlink vira junção (`portable.link`, sem privilégio), `os.statvfs` vira
  `shutil.disk_usage`, `os.getloadavg` vira `portable.loadavg()` (`None` no Windows) e o uso da GPU do `ioreg` fica
  desconhecido.
- **Provas que esperam um editor aberto** (`seek-scrub-proof.py`, `live-proof.py`): abra o editor com `TESSEL_UI_SOCKET`
  e passe o mesmo caminho (`--socket` ou a variável) à prova.

Executadas no Windows 11 (janela real, build debug, GPU AMD): `live-flow-proof.py` 116/116, `accented-titles-proof.py`
14/14 + 28/28, `grading-proof.py` 4/4 + 20/20, `video-live-proof.py` 25/25, `viewport-lifecycle-proof.py` 151/151,
`scene-lifecycle-proof.py` 221/221, `seek-scrub-proof.py`, `point-light-ui-proof.py` 28/28, `scene-edit-proof.py`,
`proxy-live-proof.py` 32/32, `live-proof.py` 57/57, `directed-scene-proof/proof.py` (todas as fases executáveis) e
`directed-scene-export-proof/proof.py` (22 de 23 fases; a da janela espera a recusa antiga de `audio=yes`, ver
[scene-export-audio](features/scene-export-audio.md)). As de
desempenho (`perf-meter`, `scene-warm`, `preview-quality`, `measure-render`) rodam, mas sem o lock de GPU nenhum número
delas é aceito; `gizmo-perf/native.py` e `viewport-validation/run-live.py` foram portadas e não executadas (ver BL168.8).
