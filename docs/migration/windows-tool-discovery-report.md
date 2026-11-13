# Descoberta automática de FFmpeg/FFprobe, Node e Chrome no Windows (BL167.2)

Entrega B2 da rodada BL167 (Windows). Contrato das regras: [media-setup](../media-setup.md#descoberta-dos-executáveis).
Status: **implementado e validado no Windows** (lib, CLI sem janela e runtime Chrome); **sem prova no macOS/Linux**
(só testes unitários da plataforma `Other`); janela GPUI não repetida.

## Problema

`tools::find` (FFmpeg/FFprobe) e `runtime::find_in_path` (Node) faziam `dir.join("ffmpeg")` sem `.exe`; `chrome.mjs` só tinha
os candidatos do macOS e nomes Unix no `PATH`. No Windows, sem `TESSEL_FFMPEG`/`TESSEL_FFPROBE`/`TESSEL_NODE`/`TESSEL_CHROME`
nada era encontrado, mesmo com tudo instalado. Além disso, um `PATH` herdado antes do `winget install` não tem o FFmpeg.

## Base e dependência

- Base: `ead9ca2bde510a5d1cc15571443a93b6162dc499` (`origin/main`, `fetch` sem mudança), worktree T3
  `fix-windows-tool-discovery-bl167-2`.
- B1 aplicado como dependência já validada: `bl167-1.diff`, 11462 bytes, SHA-256
  `FC02E703C33B0E250FE00EF1A129A6F485081AB6A34795F20CAAB9F3263776BC` (conferido; `git apply --check` e `git apply`
  sem erro). Os 8 paths de B1 estão **fora** do diff B2. Manifesto SHA-256 dos 8 arquivos antes e depois de todo o trabalho
  de B2: **idêntico** (`b1-manifest-before-b2.txt` = `b1-manifest-after-b2.txt`, hash do manifesto
  `7E9DDCF4D36E77A7AED51C31136FCFA41EA6DF6C17BDFC92B8045F586661B239`; inclui o arquivo novo `common/wrapper.rs`).
- Evidência = HEAD + B1 + B2. Este relatório é um path de B2 e não pode conter o próprio hash nem o do diff que o inclui: a
  entrega emite o diff dos demais 10 paths de B2 (`diff_sha256`) e este arquivo à parte (`report_sha256`); os dois, aplicados
  sobre HEAD + B1, são o B2 completo. Os hashes estão na saída da entrega.

## O que mudou

| Path | Mudança |
|---|---|
| `crates/media/src/tools/locate.rs` (novo) | Resolvedor pequeno do adapter: `Host` (plataforma, variáveis, `is_file`, `entries`) injetável, `locate(host, Tool)` com origem. Sem crate nem dependência nova, sem regra de domínio. |
| `crates/media/src/tools.rs` | `find` delega ao resolvedor; mensagem de erro igual à anterior. `Tools` **não** ganhou campo (`tests/async_host.rs`, congelado de B1, o constrói por literal). |
| `crates/render/src/runtime.rs` | `find_node` usa o mesmo resolvedor; 5 testes. `SetupError::NodeMissing` e sua mensagem intactos. |
| `runtime/worker/chrome.mjs` | `locateChrome(env, {platform, fileExists})` devolve `{path, origin}`; `findChrome` segue devolvendo o caminho. Windows: `PATH` + `PATHEXT`, instalações usuais; macOS/Linux: lista e ordem anteriores. |
| `runtime/test/chrome-discovery.test.mjs` (novo) | 13 testes. |
| `docs/media-setup.md`, `docs/features/{media-io,viewport,video-export}.md`, linhas F019/F021/F029 de `docs/features/README.md` | Só descoberta e limites reais. |

Comportamento de override **preservado**: `TESSEL_FFMPEG`/`TESSEL_FFPROBE` que não é arquivo falha (inclui valor vazio e
diretório); `TESSEL_NODE`/`TESSEL_CHROME` que não é arquivo é ignorado e a busca continua. Única mudança em macOS/Linux/Windows
para o Chrome: `TESSEL_CHROME` apontando para um **diretório** era aceito (`existsSync`) e agora é rejeitado (`isFile`), como pedido.
Ordem no Windows: override → `PATH` (ordem, `PATHEXT`) → instalações. macOS/Linux: candidatos e ordem como antes (FFmpeg: `PATH`,
`/opt/homebrew/bin`, `/usr/local/bin`; Node: só `PATH`; Chrome: apps do macOS e depois `PATH`).

Sem varredura recursiva (o único olhar em subpastas é `Packages\Gyan.FFmpeg*\ffmpeg-*\bin`, dois níveis), sem shell, sem versão ou
usuário no código, sem alterar `PATH` nem registro. Edge não é procurado.

## Ambiente da prova

Windows 11 (10.0.26200), Git Bash `C:\Program Files\Git\bin\bash.exe`, mbx 1.29.2, rustc/cargo 1.99.1, pnpm 10.34.9, Node v26.12.1,
Chrome 157.0.8103.78, FFmpeg 9.0.2 (Gyan, WinGet). Python ausente. Nenhuma variável `TESSEL_FFMPEG`, `TESSEL_FFPROBE`,
`TESSEL_NODE`, `TESSEL_CHROME` definida em **nenhum** comando de prova (conferido por `Get-ChildItem Env:`);
só `TESSEL_REQUIRE_RUNTIME=1` onde indicado. Cargo sempre por `scripts/cargo`/mbx, alvo padrão. Sem `CARGO_TARGET_DIR`,
`--target-dir`, `TESSEL_CARGO_LANE`, sem lock de GPU (prova funcional), sem commit/push/PR. Logs em
`C:\Users\wally\tessel-readiness-logs\2026-11-13-bl167-2\`.

O processo que executou os comandos tinha um `PATH` **herdado antes do `winget install` do FFmpeg**: o diretório do FFmpeg está só
no `PATH` de usuário do registro, não no do processo (`Get-Command ffmpeg` falha no processo). Isso é o caso real que a descoberta
precisa cobrir, e foi mantido para todas as provas Rust e de CLI (o app acha o FFmpeg sozinho).

## Reprodução antes (código de `ead9ca2` + B1, sem overrides)

| Superfície | Comando | Resultado |
|---|---|---|
| Node (Rust) | `tessel-render` antigo (cópia em `tessel-render-before.exe`), `export.preflight` | `Render backend unavailable: Node.js was not found.` — `node.exe` está em `C:\Program Files\nodejs` no `PATH` |
| FFmpeg (Rust) | idem | `tool_unavailable`: `` `ffmpeg` was not found. Install FFmpeg or set TESSEL_FFMPEG `` |
| Chrome (JS) | `findChrome()` antigo (`before-chrome-js.log`) | `ChromeNotFound`, lista só caminhos do macOS e nomes sem extensão |

`before-render-preflight.log`, `before-chrome-js.log`.

## Controles que reprovam o código antigo

- **Rust:** os testes de `locate.rs` foram executados contra a lógica antiga embutida em `locate` (cópia do `find`/`find_in_path`,
  só nesse experimento; arquivo restaurado, hash conferido). **13 de 24 falham** (PATH com `.exe` e espaços, ordem de `PATH` e
  de `PATHEXT`, diretório com nome de ferramenta, WinGet `Links`, layout Gyan e versão numérica, Program Files, o override do Node
  como arquivo, extensão minúscula, Homebrew com origem) e 11 passam (os que fixam o que **não** mudou: overrides do FFmpeg e do
  Node vindos de arquivo, vazio, Node sem Homebrew, Windows sem Homebrew, etc.). `control-legacy-media-locate.log`.
- **JS:** `chrome-old.mjs` × `chrome-new.mjs` sobre arquivos reais em diretório temporário (`control-legacy-chrome.log`):
  `chrome.exe` no `PATH` com espaço — antigo **lança**, novo acha; instalação em `ProgramFiles` com `PATH` vazio — antigo **lança**,
  novo acha; `TESSEL_CHROME` = diretório — antigo **devolve o diretório**, novo o ignora e usa o `chrome.exe` do `PATH`.

## Testes unitários (sem mutar o ambiente do processo)

Todos usam `Host`/`platform`/`fileExists` injetados.

| Suíte | Resultado | Cobre |
|---|---|---|
| `tessel-editor-media --lib -- tools::` | **24 passam, 0 falham, 1 ignorado** (23 novos + o `listings_are_parsed…` existente; o ignorado é o relatório da máquina real) | precedência override > `PATH` > instalação; FFmpeg inválido falha, Node inválido cai para o `PATH`; override vazio/diretório; `.exe`, `PATHEXT` (ordem, caixa, script hosts ignorados), pastas com espaços, entradas vazias; diretório rejeitado; Program Files/x86/`LOCALAPPDATA`; WinGet `Links`; Gyan (build mais novo por número, não texto; dois níveis; só o pacote `Gyan.FFmpeg*`); sem variáveis; macOS/Linux (nome sem extensão, Homebrew só para FFmpeg, nada do Windows, Node sem Homebrew) |
| `tessel-editor-render --lib -- runtime::` | **5 passam** | `node.exe` no `PATH` com espaços, override válido/inválido, nada achado, Program Files com `PATH` velho, Unix |
| `node --test test/chrome-discovery.test.mjs` | **13 passam, 0 falham, 0 pulados** | `PATH`, aspas, vazias, nome sem extensão, `PATHEXT`, instalações (ordem), Chromium/Canary, `PATH` > instalação, override, diretório real no disco, mensagem de erro, macOS/Linux |

Logs: `unit-rust-locate.log`, `unit-rust-render-runtime.log`, `unit-js-chrome-discovery.log`.

## Gates Rust (árvore final)

| Comando | Saída |
|---|---|
| `cargo fmt --all -- --check` | 0 |
| `./scripts/cargo check --workspace --locked` | 0 |
| `./scripts/cargo clippy --workspace --all-targets --locked -- -D warnings` | 0 (um aviso `cloned_ref_to_slice_refs` do teste novo corrigido antes) |
| `./scripts/cargo test --workspace --locked --no-run` | 0 |

`cargo test --workspace` **completo não foi repetido** (travou 30 min antes; B3 fora do escopo). Os testes dirigidos abaixo cobrem a mudança.
O `mbx` aplicou sua política de GC própria (`MBX_GC_MAX_TOTAL_SIZE=8GiB` do `scripts/mbx`) e evictou objetos do cache
compartilhado durante a primeira rodada; nenhuma limpeza foi pedida por esta tarefa.

## Prova real no Windows, sem os quatro overrides

### Executáveis resolvidos (origem e caminho)

| Ferramenta | Caminho | Origem | Como foi obtido |
|---|---|---|---|
| FFmpeg | `C:\Users\wally\AppData\Local\Microsoft\WinGet\Packages\Gyan.FFmpeg_Microsoft.Winget.Source_8wekyb3d8bbwe\ffmpeg-9.0.2-full_build\bin\ffmpeg.exe` | **lugar de instalação**: pacote WinGet `Gyan.FFmpeg` (o `PATH` do processo não o tem; `WinGet\Links` existe e está vazio) | `real-machine-resolution-rust.log` (`cargo test … --ignored --nocapture real_machine`) |
| FFprobe | `…\ffmpeg-9.0.2-full_build\bin\ffprobe.exe` (mesma pasta) | idem | idem |
| Node | `C:\Program Files\nodejs\node.exe` | **`PATH`** | idem |
| Chrome | `C:\Program Files\Google\Chrome\Application\chrome.exe` | **lugar de instalação** (Program Files) | `real-machine-resolution-chrome.log` (`locateChrome()`) |

Esses caminhos são diagnóstico; nenhum está no código nem foi passado como override.

### Lib Rust, `TESSEL_REQUIRE_RUNTIME=1` (runtime real: Node + Chrome + FFmpeg)

| Comando | Resultado |
|---|---|
| `-p tessel-editor-render --test frames --test export --test video` | `export` 3/3, `frames` 7/7, `video` 5/5; 0 ignorados |
| `-p tessel-editor-media --test export --test import --test video_audio` | `export` 11/11, `import` 4/4, `video_audio` 7/7 |

`real-render-frames-export-video.log`, `real-media-export-import-video-audio.log`.

### CLI sem janela (`tessel-render --demo`, build novo, sem overrides)

`export.preflight` → `ok:true`, `ffmpeg: "ffmpeg version 9.0.2-full_build-www.gyan.dev …"`, encoder `libx264`, renderer ANGLE/SwiftShader
(Chrome real); `render.frame` (frame 10) → `ok:true`, PNG de 96 286 bytes (SHA-256 `CF100CD9…EBDE6`); `export.start` (frames 0–12, 640×360,
`wait:true`) → `ok:true`, MP4 de 9 835 bytes (SHA-256 `E72BF656…7A42`), `verification` com container `mp4`, `h264`, 640×360, `yuv420p`, bt709 todos `ok`
(a verificação usa o FFprobe descoberto). `after-requests.jsonl`, `after-render-cli.out.jsonl`, `after-export.mp4`, `after-frame.png`.
O frame headless e o export provam Node e Chrome: sem eles não haveria `render.frame`.

### Runtime JS completo (Chrome real, sessão filha com `PATH` atualizado)

`./scripts/runtime setup` rodou antes por Git Bash (`pnpm install --frozen-lockfile` + `pnpm run build`, saída 0, `runtime-setup.log`;
o `node_modules` do worktree estava ausente e foi gerado, não ignorado). Depois `pnpm test` com `TESSEL_REQUIRE_RUNTIME=1`:
**397 testes, 397 passam, 0 falham, 0 pulados, 0 cancelados** (baseline 384/384; +13 deste arquivo novo). `runtime-js-full-out.log`.
O Chrome desses testes vem do `findChrome()` novo, sem `TESSEL_CHROME`.

**Separação obrigatória:** `runtime/test/video.test.mjs` (e `scripts/perf/assets.mjs`) chamam `spawnSync('ffmpeg')` pelo nome, então precisam
do `PATH` do processo. Como o `PATH` herdado é anterior ao `winget install`, **só** essa execução JS rodou em um processo filho cujo `PATH` foi
reconstruído explicitamente com os `PATH` de máquina + usuário do registro (`child-refreshed-path.ps1`; leitura, nenhum registro foi alterado).
Isso não é fallback do app: o app (Rust, `Tools::discover`) achou o FFmpeg sem esse `PATH` atualizado, como mostra a tabela acima.

## Limites e achados (não escondidos)

1. ~~Duas falhas de `crates/media/tests/thumbnails.rs`~~ (22/24 na primeira rodada): **corrigidas em BL167.2.1**, ver "Seguimento" abaixo.
2. ~~`workflow_tests.rs` chamava `Command::new("ffmpeg")` por nome~~: **corrigido em BL167.2.2**, ver "Seguimento" abaixo. Resta só
   `runtime/scripts/perf/assets.mjs` (script de desempenho, sob o lock de GPU; não é teste funcional).
3. macOS/Linux: preservados por construção e por testes da plataforma `Other`; **nenhuma execução** no macOS/Linux. A coordenadora testa no Mac.
4. Edge não é procurado. WinGet `Links` e `ProgramFiles\WinGet` entram como candidatos, mas só o layout Gyan foi exercitado numa máquina real.
5. `.bat`/`.cmd` de `PATHEXT` entram na busca (o Rust os inicia por `cmd.exe`); versões antigas de FFmpeg em pastas `Gyan.FFmpeg*` são escolhidas
   só quando há mais de um build (o de versão numérica maior vence).
6. Janela GPUI/`./scripts/ui` não foi exercitada; não há prova nem alegação de UI para esta mudança (a descoberta é de processos filhos).
7. Nenhuma alegação de desempenho, física, áudio ou GPU. B3/B4/B5 e BL167.3–.7 fora do escopo.
8. Um teste novo está `#[ignore]` de propósito (`real_machine_resolution_is_reported`): imprime a resolução da máquina atual, não verifica nada.

## Seguimento: BL167.2.1, BL167.2.2 e BL167.2.3

### BL167.2.1 — `thumbnails.rs` no Windows

Causa: duas asserções liam o caminho do PNG como texto com `/` (`.tessel/cache/`, `/film/`), e o Windows usa `\`. O helper `slashed()` do arquivo
de teste normaliza o separador antes de comparar; a asserção continua a mesma (a mesma pasta, o mesmo trecho), nada foi afrouxado.
Prova: `cargo test --locked -p tessel-editor-media --test thumbnails` → **24 passed, 0 failed** (antes 22/24). Log `fix21-thumbnails.out.log`.

### BL167.2.2 — `ffmpeg` por nome fora do app

| Lugar | Antes | Agora |
|---|---|---|
| `crates/desktop/src/workstation/workflow_tests.rs` (2 chamadas) | `Command::new("ffmpeg")`, e sem `PATH` a prova era **pulada em silêncio** | `ffmpeg()` usa `tessel_editor_media::tools::locate` (o mesmo resolvedor do app); sem nada achado volta ao nome e pula como antes |
| `runtime/test/video.test.mjs` | `spawnSync('ffmpeg')` | `ffToolPath('ffmpeg')` de `runtime/test/ffmpeg-locate.mjs` |
| `runtime/scripts/perf/assets.mjs` | `spawnSync('ffmpeg')` | **não tocado**: script de desempenho (lock de GPU), fora da prova funcional |

`runtime/test/ffmpeg-locate.mjs` repete em JS a ordem do resolvedor Rust (override estrito → `PATH`/`PATHEXT` → Homebrew ou WinGet `Links` e
`Packages\Gyan.FFmpeg*\ffmpeg-*\bin`, dois níveis, sem varredura). É duplicação deliberada: o teste JS não pode chamar o Rust. Ambiente, plataforma e
sistema de arquivos são injetados; `runtime/test/ffmpeg-locate.test.mjs` tem 6 testes.

Provas (Windows, sem os quatro `TESSEL_*`, **sem** `PATH` atualizado: `Get-Command ffmpeg` do processo não acha nada):

- `node --test test/video.test.mjs` com `TESSEL_REQUIRE_RUNTIME=1` e Chrome real: **7/7, 0 pulados** (`fix22-js-video.out.log`).
- Controle: o `video.test.mjs` antigo (`git show HEAD:...`) no mesmo ambiente: **0/7**, `spawnSync ffmpeg ENOENT` (`fix22-control-old-js-video.out.log`).
- `cargo test --locked -p tessel-editor --bin tessel-editor workflow_tests`: **12/12, nenhuma linha `skipping`**, ou seja, as duas provas de FFmpeg da
  janela rodaram de verdade com o FFmpeg descoberto (`fix22-desktop.out.log`).
- `runtime/test/ffmpeg-locate.test.mjs` + `chrome-discovery.test.mjs`: 19/19.
- Gates na árvore final: `cargo fmt --all -- --check`, `check --workspace`, `clippy --workspace --all-targets -D warnings` e `test --workspace --no-run`, todos exit 0 (`fix2-*.log`).

### BL167.2.3 — macOS/Linux (não executado aqui)

Esta máquina é Windows, sem WSL (nenhuma distribuição) nem Docker: **não há prova de macOS/Linux neste seguimento**, e o item continua A validar.
O que existe: os testes da plataforma `Other` rodam aqui com host/ambiente injetados, mas isso prova a regra, não o sistema operacional.
Roteiro para a coordenação no Mac (nenhum precisa de override):

```sh
./scripts/cargo test --locked -p tessel-editor-media --lib -- tools::
./scripts/cargo test --locked -p tessel-editor-render --lib -- runtime::
./scripts/cargo test --locked -p tessel-editor-media --test thumbnails
TESSEL_REQUIRE_RUNTIME=1 ./scripts/cargo test --locked -p tessel-editor-render --test export --test video --test frames
TESSEL_REQUIRE_RUNTIME=1 ./scripts/cargo test --locked -p tessel-editor --bin tessel-editor workflow_tests
(cd runtime && node --test test/chrome-discovery.test.mjs test/ffmpeg-locate.test.mjs)
(cd runtime && TESSEL_REQUIRE_RUNTIME=1 node --test test/video.test.mjs)
printf '%s\n' '{"op":"export.preflight","preset":{"id":"proof640","name":"Proof","width":640,"height":360},"destination":"/tmp/bl167-probe.mp4"}' \
  '{"op":"render.frame","frame":10,"out":"/tmp/bl167-frame.png"}' \
  | env -i HOME="$HOME" PATH=/usr/bin:/bin ./target/debug/tessel-render --demo   # PATH mínimo, sem Homebrew
```

O último comando verifica o caso que a descoberta existe para resolver: processo iniciado sem Homebrew no `PATH` (mesmas requisições da prova do Windows).
Aceite: tudo verde, 0 pulados, e a origem de cada executável (`PATH` ou `install location`) registrada.

## Proposta de linhas para `docs/backlog.md` (texto para a coordenadora; backlog não foi editado)

- **BL167.2** — *Descoberta automática de FFmpeg/FFprobe, Node e Chrome no Windows* — **Implementado; validado no Windows (lib, CLI, runtime Chrome real); macOS/Linux a validar pela coordenação** — dep.: BL167.1 — responsável: este agente (B2) — aceite: sem `TESSEL_FFMPEG/FFPROBE/NODE/CHROME`, `export.preflight`, `render.frame` e `export.start` reais passam no Windows; overrides e macOS/Linux preservados; testes injetáveis com controles que reprovam o código antigo — detalhes: [relatório](migration/windows-tool-discovery-report.md), [media-setup](media-setup.md#descoberta-dos-executáveis), [F019](features/media-io.md)/[F021](features/viewport.md)/[F029](features/video-export.md); commit/PR: pendente (sem commit nesta entrega).
- **BL167.2.1** — *`thumbnails.rs`: duas asserções com `/` em caminhos no Windows* — **Implementado; validado no Windows** (`thumbnails` 24/24, antes 22/24; a asserção continua a mesma, só o separador é normalizado) — dep.: BL167.2 — detalhes: [seguimento](migration/windows-tool-discovery-report.md#seguimento-bl16721-bl16722-e-bl16723); commit/PR: pendente.
- **BL167.2.2** — *Chamadas de `ffmpeg` por nome fora do app* — **Implementado; validado no Windows** para `workflow_tests.rs` (12/12, sem skip) e `runtime/test/video.test.mjs` (7/7 com Chrome real e sem `PATH` atualizado; o teste antigo dá 0/7); **resta** `runtime/scripts/perf/assets.mjs` (script de desempenho, lock de GPU), que continua exigindo `ffmpeg` no `PATH` — dep.: BL167.2 — commit/PR: pendente.
- **BL167.2.3** — *Validar a descoberta no macOS/Linux* — **A validar** (nenhuma execução fora do Windows: sem WSL/Docker aqui); roteiro exato no [seguimento](migration/windows-tool-discovery-report.md#seguimento-bl16721-bl16722-e-bl16723) — dono: coordenação (Mac).
