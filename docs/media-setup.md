# Setup de mídia, runtime e export

O que a máquina precisa para as fatias P03–P12. Nada disto é baixado em tempo de execução;
o runtime de render funciona offline (sem CDN).

| Componente | Para quê | Como é encontrado | Sem ele |
|---|---|---|---|
| FFmpeg + FFprobe | probe, decode, proxies, encode, verificação | `TESSEL_FFMPEG` / `TESSEL_FFPROBE`, `PATH`, `/opt/homebrew/bin`, `/usr/local/bin` | `backend_unavailable` / blocker `encoder_unavailable` no preflight |
| Node 20+ | renderer headless (export, `scripts/render`) | `TESSEL_NODE` ou `node` no `PATH` | `backend_unavailable` |
| Chrome/Chromium | WebGL do renderer headless | descoberto pelo runtime (ver `runtime/`) | idem |
| `runtime/` instalado | Three.js 0.187.0 e Spark 2.4.0 | `TESSEL_RUNTIME_DIR` ou o diretório do repositório; `pnpm install` dentro de `runtime/` | idem |
| Dispositivo de saída de áudio (CoreAudio no macOS, via `cpal`, crate `tessel-editor-audio`) | saída do playback e relógio de amostras | dispositivo padrão do sistema, aberto pela thread `tessel-audio`; nenhum binário externo | playback segue pelo relógio de parede, **mudo**, e `playback.status` informa `audio.error.code = audio_output_unavailable` (nunca falha em silêncio) |

`TESSEL_GL=swiftshader` (padrão, determinístico) ou `gpu`.

Os encoders realmente usados são os que `ffmpeg -encoders` lista: o preset pede `libx264` e `aac`.
`project.describe`/`export.preflight` devolvem a versão de FFmpeg e a lista de capacidades; não há
promessa de codec que a máquina não tenha. Distribuição e licença dos binários (FFmpeg com
`libx264` é GPL) **não** foram resolvidas: este repositório não empacota FFmpeg.

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
