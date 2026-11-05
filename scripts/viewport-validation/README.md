# viewport-validation

Harnesses da validação de `docs/viewport-performance-opus-validation.md`. Nada aqui altera produção:
tudo opera em uma **cópia** das fontes (`snapshot.sh`) e em variantes de runtime construídas dessa cópia.

| Arquivo | Papel |
|---|---|
| `snapshot.sh` | Copia as fontes para `$TEV/snap/<fingerprint>` com manifesto de hashes (o repositório não tem commits) |
| `patch-host.py` | Instrumenta uma cópia do host (spans em Rust, comandos `vv_dump`/`vv_drag`) |
| `build-runtimes.mjs` | Gera variantes: `baseline` (puro), `baseline-trace`, `retained`, `retained-trace`, `retained-rtc`, `retained-singlert` |
| `make-assets.py` | Assets sintéticos: splats, cenas, vídeo com código de barras por quadro |
| `page/` + `vv-wk.swift` + `vv-chrome.mjs` | Bancada sem GPUI: WKWebView real e Chrome (Metal/SwiftShader) |
| `run-live.py` | Item 1/7: janela real, debug e release, spans + injeção de input GPUI |
| `capture-latency.swift` | Item 7: ScreenCaptureKit (saída do compositor, **não é fóton**) |
| `run-export.py` | Item 6: export completo por `tessel-render` |
| `runs.py`, `bench.sh`, `summarize-{live,bench,sample}.py`, `cross-stack.py` | Listas de execução, execução da bancada, resumos em `results/` e PSNR entre stacks |

`TEV` (padrão `/tmp/tev`) guarda artefatos pesados fora do git; `results/` guarda só resumos com proveniência.
