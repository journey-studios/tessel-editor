# Prontidão do Tessel Editor no "Computador" (Windows 11) — 2026-11-12

Verificação local, sem implementação de feature, PR, push, merge nem limpeza. Executada na
thread T3 do environment "Computador" (checkout `W:\www\tessel-editor`, `journey-studios/tessel-editor`;
não é Tessel RPG). Tudo abaixo foi **executado** nesta máquina; o que não foi executado está em
[Não verificado](#não-verificado).

## Veredito por camada

| Camada | Estado | Evidência |
|---|---|---|
| Core / headless (lib + CLI sem janela) | **Funciona**, com 2 falhas de teste de último bit de ponto flutuante | `tessel-editor-core`: 1407 ok / 2 falhas / 7 ignorados; `./scripts/headless` responde (erro tipado e continua) |
| Runtime Three.js + Chrome | **Funciona** com `TESSEL_CHROME` explícito | `./scripts/runtime test`: 384/384 (Chrome + FFmpeg) |
| FFmpeg / export | **Funciona** com `TESSEL_FFMPEG/FFPROBE/NODE` explícitos | `export` 11/11, `flow` 2/2 (import→salvar→pacote→restaurar→merge→export), `scene_export*` 12 + 29 |
| Build GPUI desktop Windows | **Funciona** | `./scripts/cargo build -p tessel-editor --locked` exit 0; `check --workspace` exit 0; `fmt` exit 0 |
| Janela nativa | **Funciona** | abriu a janela própria; `ui status/get/set/invoke/op` por socket, sem cliques; fechada por `window.close` |
| GPU | **Funciona** | WebView2 reporta `ANGLE (AMD, AMD Radeon RX 6750 XT … Direct3D11)`; `render.frame` 1920×1080 em 147 ms (debug) |
| Áudio | **Funciona** | `playback` em `clock: audio_device`, WASAPI "Alto-falantes", 0 underruns |
| `clippy --workspace --all-targets -D warnings` | **Falha** (compilação de testes de `media`) | ver [Bloqueios](#bloqueios-do-windows-reproduzíveis) B1 |
| `cargo test --workspace` | **Falha ao compilar** (testes Unix-only de `media`) | B1; contornado por pacote/alvo |

Não declaro o Windows "pronto" para qualquer tarefa: ele é **utilizável para build, edição, export e
validação de UI**, mas a suíte de `media` tem lacunas reais no Windows (B1–B4).

## Repositório

| Item | Valor |
|---|---|
| Remote | `origin https://github.com/journey-studios/tessel-editor.git` (fetch/push) |
| Branch | `main` rastreando `origin/main` |
| SHA ao chegar | `3e48861e9914ed2fd6c71e13c62c5673f068768f` (checkout limpo, sem stash, um só worktree) |
| `origin/main` após `git fetch origin` | `ead9ca2bde510a5d1cc15571443a93b6162dc499` (115 commits à frente; 23 branches novas no remoto) |
| Ação | `git merge --ff-only origin/main` com árvore limpa → **fast-forward** sem reset/force |
| SHA final testado | `ead9ca2bde510a5d1cc15571443a93b6162dc499` ("docs(backlog): Give the orphan rows of the backlog a table (BL166) (#56)") |
| Árvore ao final | limpa, exceto este relatório (não commitado) |

`3e48861` já continha o trabalho de Windows (`feat(desktop): Run the ui automation channel on Windows…`,
`chore: Store text with LF…`, `test(core): Pin planner digests for Windows…`), ver BL054.

## Máquina

| Item | Valor |
|---|---|
| Host | `Computador` |
| SO | Windows 11 Pro 10.0.26200 (build 26200) |
| CPU | Intel Core i9-12900KS, 16 núcleos / 24 lógicos, carga 26–28 % (T3 Code, dois `node`, Blender, Radeon Software, Discord ficam abertos) |
| RAM | 63,7 GiB; 24–31 GiB livres durante a verificação |
| GPU | AMD Radeon RX 6750 XT, driver 32.0.21045.16010 |
| Disco | `W:` 1 745 GiB livres (checkout); `C:` 583 GiB livres (cache do mbx, Cargo, WinGet); `D:` 7,3 GiB |
| Rust | `rustc 1.99.1 (58cf8f9c3 2026-10-13)`, `cargo 1.99.1`, `stable-x86_64-pc-windows-msvc`, fixado por `rust-toolchain.toml` |
| Linker | VS 2022 Build Tools (`C:\Program Files (x86)\Microsoft Visual Studio\2022\BuildTools`); `link`/`cl` não estão no `PATH`, o Cargo os encontra sozinho |
| Node / pnpm | Node v26.12.1 (requisito: 20+), pnpm 10.34.9 |
| Chrome | `C:\Program Files\Google\Chrome\Application\chrome.exe`; WebView2 do sistema |
| FFmpeg | **instalado nesta tarefa** (ver abaixo): FFmpeg 9.0.2 (gyan.dev, `libx264`, `aac`) |
| Python | **ausente** (só o alias da Microsoft Store): os `scripts/*-proof.py` não rodam aqui |
| Shell dos scripts | Git for Windows `bash.exe` (`C:\Program Files\Git\bin\bash.exe`). O `bash` do `PATH` é o do WSL **sem distribuição**: não usar |
| `mbx` | **instalado nesta tarefa**: 1.29.2 |

### Instalações feitas (locais, reversíveis, sem segredos)

1. `mbx` 1.29.2: asset oficial `mbx-x86_64-pc-windows-msvc.zip` do release `v1.29.2` (jdx/mr-boxington).
   SHA-256 conferido contra o `SHA256SUMS` do release: `5ccfe5942c224844ffc50eb689de9c02033e401b210d3c80fffe4b94c929b0a2`.
   Copiado para `%USERPROFILE%\.cargo\bin\mbx.exe`. **Sem** `mbx setup` (nenhum shim global, nenhum `PATH` alterado).
   Reverter: apagar esse `mbx.exe`.
2. FFmpeg: `winget install --id Gyan.FFmpeg -e --scope user` (hash do instalador verificado pelo winget).
   Caminho: `%USERPROFILE%\AppData\Local\Microsoft\WinGet\Packages\Gyan.FFmpeg_Microsoft.Winget.Source_8wekyb3d8bbwe\ffmpeg-9.0.2-full_build\bin\`.
   Reverter: `winget uninstall Gyan.FFmpeg`. Nota de licença: este build "full" é GPL (ver `docs/media-setup.md`).
3. `./scripts/runtime setup` (`pnpm install --frozen-lockfile && pnpm run build`): reconstruiu `runtime/dist`
   (estava defasado em relação às 115 commits). Diretórios ignorados pelo Git.

Nada foi limpo: `target\` (11,8 GiB de builds anteriores, agora 16,4 GiB) e o cache do mbx (4,6 GiB) foram preservados.

## Cache de build (mbx) no Windows

* `./scripts/cargo` é um script `sh`; funciona via Git Bash (`bash.exe ./scripts/cargo …`). Resolveu `mbx` pelo `PATH`.
* `mbx doctor` (`01-mbx-doctor.log`): **0 falhas, 3 avisos**: sem shim global (esperado), `ar`/`llvm-ar` ausentes
  (determinismo de arquivos não testado) e **o cache em `C:` não consegue clonar nem fazer hard link para o checkout em `W:`**
  (cada byte restaurado seria copiado).
* `mbx adopt --dry-run`: "left `W:\www\tessel-editor\target` alone: it is not on the same filesystem as the managed target root".
  O `target\` existente **permaneceu um diretório real e intacto**; o mbx o usa sem gerenciá-lo (não há symlink; `whoami /priv` não mostrou `SeCreateSymbolicLinkPrivilege` ativo, o que também impediria o link de `target`).
* Em todas as execuções o mbx reportou `0 hits, 0 misses, N not looked up` e `mbx stats` fecha em `cache hits 0` após
  50 builds. **O mbx roda, mas o cache de ações compartilhado não está efetivo neste Windows.** Não foi diagnosticado
  além disso (ver Plano, P5). Nenhum `CARGO_TARGET_DIR`/`--target-dir`/`build.target-dir` foi usado, e nenhum build
  grande foi feito em tmp.
* `docs/cargo-worktrees.md` descreve só macOS (`~/Library/Caches/mbx`, APFS clone); nada lá cobre Windows.

## Comandos e exit codes

Prefixo usado para scripts: `& 'C:\Program Files\Git\bin\bash.exe' -c '<comando>'` a partir de `W:\www\tessel-editor`.
Logs duráveis em `%USERPROFILE%\tessel-readiness-logs\2026-11-12\` (índice em `00-INDEX.md`; fora do repositório).

| # | Comando | Exit | Resultado |
|---|---|---|---|
| 1 | `git fetch origin`; `git merge --ff-only origin/main` | 0 | `3e48861` → `ead9ca2` |
| 2 | `./scripts/mbx doctor` | 0 | 0 falhas, 3 avisos |
| 3 | `cargo fmt --all -- --check` | 0 | limpo |
| 4 | `./scripts/cargo tree -p tessel-editor-core --locked -e normal` | 0 | só `flate2`, `serde`, `serde_json`; **0 ocorrências de gpui/wry** |
| 5 | `./scripts/cargo test -p tessel-editor-core --locked` | 101 | para no 1º alvo falho |
| 6 | `… -p tessel-editor-core --locked --no-fail-fast` | 101 | **1407 ok, 2 falhas, 7 ignorados** (B5) |
| 7 | `./scripts/headless < sessão.jsonl` | 1 | 4 linhas ok; a 4ª (`property:nope`) falha **de propósito** com `unknown_element`; o processo continua e sai com 1 (contrato documentado) |
| 8 | `./scripts/cargo check --workspace --locked` | 0 | 34 s |
| 9 | `./scripts/cargo clippy --workspace --all-targets --locked -- -D warnings` | 101 | B1 |
| 10 | `./scripts/cargo build -p tessel-editor --locked` | 0 | 2 min 4 s, `target\debug\tessel-editor.exe` 69 MB |
| 11 | `./scripts/runtime setup` | 0 | `dist\runtime.js` 4,2 MB |
| 12 | `./scripts/runtime test` (sem variáveis) | 0 | 187 ok, **197 ignorados** (Chrome não encontrado: B2) |
| 13 | `TESSEL_CHROME=… ./scripts/runtime test` | 0 | 377 ok, 7 ignorados (sem FFmpeg) |
| 14 | idem com FFmpeg instalado | 0 | **384/384**, 0 ignorados (155 s) |
| 15 | `./scripts/cargo test --workspace --locked --no-fail-fast` (com env do runtime) | 101 | **não compila** (B1); nenhum teste executou |
| 16 | por pacote, `--no-fail-fast`, `TESSEL_FFMPEG/FFPROBE/NODE/CHROME` + `TESSEL_REQUIRE_RUNTIME=1` | ver abaixo | `pkgtests2\_summary.log` |
| 17 | janela: `tessel-editor.exe` (debug) + `tessel-editor.exe ui …` | 0 | ver "Janela nativa" |

Rodada 16 (testes por pacote, depois das variáveis explícitas):

| Pacote | ok | falhas | ignorados | Observação |
|---|---|---|---|---|
| `tessel-editor-core` | 1407 | 2 | 7 | B5 |
| `tessel-editor-audio` | 10 | 0 | 0 | |
| `tessel-editor` (desktop) | **502** | 0 | 0 | inclui os testes GPUI |
| `tessel-editor-render` | 125 | 2 | 3 | B4 (`shasum`) |
| `tessel-editor-media` (38 alvos, 2 não compilam) | 416 | 27 | 4 | B1, B3 |

Uma primeira rodada **sem** `TESSEL_FFMPEG/FFPROBE/NODE` falhou em dezenas de testes (desktop 2, render 76, media 133) e foi
descartada como causa de ambiente (B2, em `pkgtests/`); não é evidência de regressão.

## Janela nativa e comandos semânticos

Binário de debug iniciado por mim (PID 16096, `Start-Process`, stdout/stderr em log; stderr vazio). Havia um `ui.sock` velho de
2026-11-10; o editor o substituiu sem problema. Nenhum clique por coordenada. Tudo por `tessel-editor.exe ui …` (o que
`./scripts/ui` chama):

| Comando | Resultado |
|---|---|
| `ui status` | `ok`, janela 1440×900, tema `dark`, projeto demo |
| `ui invoke window.activate` | `dispatched: true` |
| `ui invoke clip:atlantic-coast select` | ok |
| `ui set property:zoom value 150` / `ui get property:zoom` | `previous 100 → 150`, `modified: true` |
| `ui set property:nope value 1` | exit 1, `unknown_element` estruturado |
| `ui get monitor.viewport` | `phase: ready`, `on_screen: true`, "Three.js · 960×540 · 23 ms" |
| `ui op render.frame {frame:120}` | frame 1920×1080, `hash 9926fcf83f485b82`, `render_ms 146,9` (debug) |
| `ui op playback.start/status/stop` | `clock: audio_device`, WASAPI "Alto-falantes", 48 kHz, 0 underruns; no debug `skipped_frames 49` |
| `ui op history.status` | `can_undo: true`, `undo_label: "Set property"` |
| `ui invoke window.close` | processo encerrou; novo `ui status` → "Cannot connect" (10061) |

Captura nativa da janela (`PrintWindow`, 1456×908): `32-window.png` (biblioteca, monitor com a imagem, inspetor com Zoom 150 %,
timeline e cabeçalho com controles de janela desenhados). A WebView2 usou a **GPU real** (ANGLE/D3D11 na RX 6750 XT), não SwiftShader;
isso vale só para o monitor da janela. Os testes do runtime/export rodam em Chrome headless com o padrão `TESSEL_GL=swiftshader`.

## Bloqueios do Windows (reproduzíveis)

**B1. Testes de `crates/media` não compilam no Windows (bloqueia `clippy --all-targets` e `cargo test --workspace`).**
`crates/media/tests/job_commit_state.rs:20` e `async_host.rs:9` usam `std::os::unix::fs::PermissionsExt`/`Permissions::from_mode`
sem `cfg(unix)`; `directed_resources.rs:21` importa `SymlinkPolicy` sem uso fora de Unix (vira erro com `-D warnings`).
Os 36 outros alvos compilam. Os demais `grep` de `std::os::unix`/`flock`/`libc::` no código de produção compilam.

**B2. O Windows não descobre Node, FFmpeg nem Chrome sozinho.**
`crates/media/src/tools.rs` (`find`) e `crates/render/src/runtime.rs` (`find_in_path`) fazem `dir.join("ffmpeg")`/`join("node")`
sem `.exe`, e os candidatos extras são só macOS/Unix (`/opt/homebrew/bin`). `runtime/test` procura Chrome em caminhos de macOS e
`C:\Program Files\Git\ucrt64\bin\google-chrome` (sem `Program Files\Google\Chrome`). Contorno sem mexer em código:
`TESSEL_FFMPEG`, `TESSEL_FFPROBE`, `TESSEL_NODE`, `TESSEL_CHROME` (usados nesta rodada). Sem eles: 197 testes do runtime ignorados e
`tool_unavailable`/"Node.js was not found" em dezenas de testes.

**B3. Suíte de `media` sobre claims/leases/caminhos falha no Windows (27 falhas, em 11 alvos).** Causas observadas nos logs:
`lease` devolve `holder: null` em vez do PID (`directed_resources/lease.rs:144`, `scene_resource_publication.rs:123`);
caminhos `\\?\C:\…` e `claims.json`/`claims.journal` "não encontrado"/"Acesso negado (os error 5)" em corridas do ledger
(`claims/tests.rs:783,998,1165`); um teste usa o workspace `/p/ws` que não é absoluto no Windows (`base.rs:223`);
`thumbnails` (2) e as `directed_resources_transport`/`scene_import_cleanup`/`scene_resource_integration` comparam listas de arquivos.
Coincide com os limites já registrados em BL078, BL111 e BL094.3 ("sem Windows/NFS"). Não aprofundei cada causa.

**B4. Teste do `render` usa `shasum`** (`crates/render/tests/scene_export.rs:600,830`; ferramenta de macOS/Perl) → "program not found" em 2 testes. Só afeta teste.

**B5. Duas falhas de último bit em ponto flutuante no core**, determinísticas na reexecução:
`scene_gizmo_rotation::the_rotate_overlay_has_three_closed_rings_and_the_metrics_to_draw_them` (`v` 0.6491406061697664 vs …665) e
`scene_gizmo_scale::four_parents_of_anisotropic_scale_with_a_rotation_between_them_refuse_the_world_shear`
(-4.44e-8 vs -2.22e-8). Provável diferença de libm/trigonometria entre Windows e macOS; precisa de tolerância no teste ou fixture por plataforma.

## Não verificado

* **Build `release`** do desktop nesta máquina (BL054 registra medições em release de 2026-11-09, anteriores a esta rodada).
* Todos os `scripts/*-proof.py` (`live-flow-proof.py`, `scene-*-proof.py`, `seek-scrub-proof.py`…): **Python ausente**.
* Qualquer comparação visual/pixel em Windows contra macOS; export de vídeo conferido só pelos testes (`export` 11/11), não por inspeção do arquivo.
* Áudio *ouvido* (o dispositivo foi aberto e a posição avançou; ninguém escutou).
* Efeito em segundo plano de rodar o cache do mbx de `C:` com checkout em `W:` (cache sem hits, ver acima).
* NFS/multiprocesso em Windows (BL078/BL111): não coberto.

## O que esta máquina pode assumir agora

Capacidade confortável: 24 threads, 64 GiB, 1,7 TiB livres no volume do checkout, GPU AMD real e um dispositivo de áudio.
Recomendado já:

* Builds e checks de `core`, `desktop`, `render`, `audio`; testes do core/desktop/audio/render; runtime/Chrome; export FFmpeg.
* Provas de UI por socket com a janela nativa (`tessel-editor ui …`), com **uma só janela por vez** (o `ui.sock` é único por usuário).
* Rodadas de GPU real do monitor (WebView2/D3D11). Não rodar duas ao mesmo tempo (carga ~26 % já existe por T3/Blender).

Não assumir ainda: `clippy --workspace --all-targets` e `cargo test --workspace` como gates (B1); suíte de claims/leases de `media` (B3);
qualquer prova em Python; benefício do cache compartilhado do mbx.

## Plano concreto (para o coordenador decidir; nada implementado aqui)

1. **P1 — testes `media` portáveis (B1):** `#[cfg(unix)]` nos usos de `PermissionsExt`/`from_mode` e no import de `SymlinkPolicy`
   (ou gate do teste inteiro). Aceite: `clippy --workspace --all-targets -D warnings` e `cargo test --workspace --no-fail-fast` compilam no Windows.
2. **P2 — descoberta de ferramentas no Windows (B2):** `.exe`/`PATHEXT` em `tools.rs` e `runtime.rs`, candidatos `Program Files\Google\Chrome`,
   `Program Files (x86)\Microsoft\Edge`, e (se desejado) WinGet/choco. Documentar em `docs/media-setup.md` e em `docs/cargo-worktrees.md` o setup Windows.
3. **P3 — claims/leases no Windows (B3):** triagem por teste: PID no lease, normalização `\\?\`, semântica de `flock`/`LockFileEx` e rename com handle aberto.
   Liga com BL078, BL111, BL094.3. Antes disso, marcar esses alvos `cfg(unix)` com motivo, para não mascarar a lacuna.
4. **P4 — `shasum` e floats (B4, B5):** hash em Rust no teste; tolerância ou valores por plataforma nos dois testes de gizmo.
5. **P5 — mbx no Windows:** mover o cache para `W:` (setting `cache_dir`, ver `mbx settings ls`) para permitir hard link e investigar por que as ações ficam "not looked up"
   (`mbx explain`); instalar `llvm-ar`. Decidir se `scripts/cargo` ganha equivalente `.ps1`/`.cmd` (hoje exige Git Bash).
6. **P6 — Python 3** (winget) para destravar os `scripts/*-proof.py`, e uma rodada `release` + prova viva de fluxo.

### Linhas propostas para o backlog (não editei `docs/backlog.md`)

| ID | Proposta | Aceite |
|---|---|---|
| BL054 (reuso) | Adicionar: Windows 11 em `ead9ca2` — build/janela/GPU/áudio/runtime/export validados; lacunas B1–B5 | Evidência: este relatório e os logs referidos |
| novo | Testes `media` não compilam fora de Unix (B1) | P1 |
| novo | Descoberta de Node/FFmpeg/Chrome no Windows (B2) | P2 |
| BL078/BL111 (reuso) | Claims/leases/paths no Windows: 27 falhas em `media` (B3) | P3 |
| novo | `shasum` e 2 testes de float no Windows (B4, B5) | P4 |
| novo | Cache mbx sem hits no Windows e `scripts/cargo` exige Git Bash (P5) | P5 |

## Retomar

```powershell
cd W:\www\tessel-editor
$ff = "$env:USERPROFILE\AppData\Local\Microsoft\WinGet\Packages\Gyan.FFmpeg_Microsoft.Winget.Source_8wekyb3d8bbwe\ffmpeg-9.0.2-full_build\bin"
$env:TESSEL_FFMPEG = "$ff\ffmpeg.exe"; $env:TESSEL_FFPROBE = "$ff\ffprobe.exe"
$env:TESSEL_NODE   = 'C:\Program Files\nodejs\node.exe'
$env:TESSEL_CHROME = 'C:\Program Files\Google\Chrome\Application\chrome.exe'
$env:TESSEL_REQUIRE_RUNTIME = '1'
$sh = 'C:\Program Files\Git\bin\bash.exe'
git fetch origin; git status -sb                       # só fast-forward com árvore limpa
& $sh -c './scripts/cargo build -p tessel-editor --locked'
& $sh -c './scripts/cargo test -p tessel-editor --locked --no-fail-fast'     # 502 ok
& $sh -c './scripts/cargo test -p tessel-editor-core --locked --no-fail-fast'
& $sh -c './scripts/runtime test'                                            # 384/384
Start-Process .\target\debug\tessel-editor.exe                                # janela própria
.\target\debug\tessel-editor.exe ui status                                    # depois: ui get/set/invoke/op
.\target\debug\tessel-editor.exe ui invoke window.close
```

Não use `cargo test --workspace` nem `clippy --all-targets` como gate até P1. Para JSON em `ui op`, grave um arquivo e use `ui op @arquivo.json`
(o PowerShell remove as aspas de argumentos nativos).
