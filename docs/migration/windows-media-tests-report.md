# BL167.1 — testes de `media` compilam no Windows

Relatório da entrega Windows e da revisão no Mac. O [backlog comum](../backlog.md)
centraliza status, responsáveis e seguimentos. A entrega do implementador foi sem commit;
a seção Mac registra a integração posterior pelo coordenador.

## Escopo

Fazer a compilação dos testes de `crates/media` (e o que o acompanha em clippy) passar no Windows sem
esconder suítes portáveis e sem mudar produto. Fora de escopo, explícitos e inalterados: BL167.2
(descoberta de ferramentas), BL167.3 (`shasum`/floats), BL167.4 (cache), B2/B3/B4/B5.

## Base e identidade

| Item | Valor |
|---|---|
| Worktree | `C:\Users\wally\.t3\worktrees\tessel-editor\fix-windows-media-tests-bl167-1` (branch `fix/windows-media-tests-bl167-1`) |
| Base | `ead9ca2bde510a5d1cc15571443a93b6162dc499` = `origin/main` após `git fetch` (árvore limpa no início) |
| Diff (não commitado) | `C:\Users\wally\tessel-readiness-logs\2026-11-13-bl167-1\bl167-1.diff` — SHA-256 `FC02E703C33B0E250FE00EF1A129A6F485081AB6A34795F20CAAB9F3263776BC`, 11 462 bytes, `git diff --binary HEAD` com `wrapper.rs` em intent-to-add (índice temporário; o índice real não foi tocado) |
| Este relatório | `docs/migration/windows-media-tests-report.md` (novo, fora do hash acima) |
| Logs duráveis | `C:\Users\wally\tessel-readiness-logs\2026-11-13-bl167-1\{before,after}\` (`_summary.log` lista comando, exit e duração; `run.ps1` é o executor) |
| Ferramentas | mbx 1.29.2; Git Bash; `TESSEL_FFMPEG/FFPROBE/NODE/CHROME` explícitos; Rust 1.99.1; target padrão, sem `CARGO_TARGET_DIR` |

## Reprodução (antes)

Em `ead9ca2`, no Windows (`before/`):

| Comando | Exit |
|---|---|
| `cargo fmt --all -- --check` | 0 |
| `./scripts/cargo check --workspace --locked` | 0 |
| `./scripts/cargo clippy --workspace --all-targets --locked -- -D warnings` | **101** |
| `./scripts/cargo test --workspace --locked --no-run` | **101** |

Causas: `job_commit_state.rs` e `async_host.rs` importavam `std::os::unix::fs::PermissionsExt` e criavam
wrappers `#!/bin/sh` com `Permissions::from_mode` (E0433/E0599); `directed_resources.rs` importava
`SymlinkPolicy`, usado só num módulo `cfg(unix)` (unused import sob `-D warnings`).

Com `clippy --keep-going` apareceram mais **três** bloqueios só-Windows da mesma classe (item usado só em
código `cfg(unix)`), tratados com a expansão aprovada pelo coordenador (itens 4–6 abaixo).

## O que mudou

1. **Novo `crates/media/tests/common/wrapper.rs`** (+ `pub mod wrapper;` em `common/mod.rs`):
   `forwarding_tool(dir, name, real, flag, delay)` escreve um stand-in de `ffmpeg`/`ffprobe` que é a
   ferramenta real até o `flag` existir, depois imprime `forced failure` e sai com 1; opcionalmente espera
   `delay` s antes de cada execução. Unix: script `#!/bin/sh` com modo `0o755` (texto idêntico ao anterior;
   a linha `sleep` só é emitida se `delay > 0`). Windows: arquivo `.cmd` (CRLF) com `%*` e
   `exit /b %ERRORLEVEL%`; a espera usa `powershell Start-Sleep -Milliseconds` (`timeout` recusa stdin
   redirecionado e `ping` tem grão de 1 s). O helper recusa, com mensagem, caminhos com `%`, `"`, CR ou LF
   (o `cmd.exe` os reinterpretaria).
2. `job_commit_state.rs` e `async_host.rs`: usam o helper; sem `PermissionsExt`/`from_mode`. **Nenhum teste
   foi escondido**: continuam todos compilando e rodando nas duas famílias de SO.
3. `directed_resources.rs`: `SymlinkPolicy` movido para dentro do módulo `symlinks` (`cfg(unix)`).
4. `package_atomic.rs`: `MediaHost` movido para o módulo `permissions` (`cfg(unix)`), o único que o usa.
5. `directed_resources_transport.rs`: `SymlinkPolicy` movido para o módulo `containment` (`cfg(unix)`).
6. `crates/desktop/src/workstation/viewport_host.rs`: `allow_file` (`#[cfg(test)]`) passa a
   `#[cfg(all(test, unix))]`. Confirmado por busca: o **único** chamador é `asset_io_tests.rs:804`, dentro do
   módulo `held_read` (`#[cfg(unix)]`, fifo). Sem outro chamador; nenhuma mudança de produto.

`cfg` restritos que permanecem, cada um com motivo: os módulos de symlink (`symlinks`, `containment`):
criar symlink no Windows exige privilégio (Developer Mode/token elevado) que uma execução comum não tem; o
módulo `permissions` de `package_atomic` e o `held_read` (fifo) dependem de bits de modo e de pipes
nomeados Unix. **Limite de cobertura no Windows:** política de symlink de `directed_resources`, permissões
somente-leitura de `package_atomic` e a leitura retida por fifo da viewport não são exercitadas no Windows;
continuam rodando no Mac.

## Depois — aceite de compilação (Windows, diff final)

Todos após a última edição de código (`after/`):

| Comando | Exit | Log |
|---|---|---|
| `cargo fmt --all -- --check` | 0 | `20-fmt.log` |
| `./scripts/cargo check --workspace --locked` | 0 | `21-check.log` |
| `./scripts/cargo clippy --workspace --all-targets --locked -- -D warnings` | **0** | `22-clippy.log` |
| `./scripts/cargo test --workspace --locked --no-run` | **0** | `23-test-norun.log` |

(`03-clippy`, `03b-clippy-keep-going`, `04-test-norun` e `10–12` em `after/` são execuções intermediárias,
com só os três arquivos originais corrigidos; ficam como evidência do diagnóstico.)

## Depois — execução dos testes afetados (Windows, diff final)

| Alvo | Resultado |
|---|---|
| `media --test job_commit_state` | 1 passou, 0 falhas (`40-final-job_commit_state.log`) |
| `media --test async_host` | 8 passaram, 0 falhas (`41-final-async_host.log`) |
| `media --test directed_resources` | 14 passaram, 0 falhas (`42-final-directed_resources.log`) |
| `media --test package_atomic` | 7 passaram, 0 falhas (`43-final-package_atomic.log`) |
| `media --test directed_resources_transport` | 13 passaram, **3 falharam** — bloqueio de execução já conhecido, não causado por este diff (ver abaixo) |

Os wrappers `.cmd` foram exercitados de verdade: `job_commit_state` (o `tessel-media` filho descobre
`TESSEL_FFMPEG` apontando para o `.cmd`, o import roda e o commit vê o FFmpeg "sumir") e `async_host`
(import lento, cancelamento, falha de decode sem retry), com FFmpeg 9.0.2 real.

## Workspace `--no-fail-fast` (separar execução de compilação)

`./scripts/cargo test --workspace --locked --no-fail-fast` (`30-test-workspace-nofailfast.log`, exit
101, 2024 s). **Compilar não é workspace verde.** O que a execução mostrou:

- **Travamento (B3):** o teste de lib
  `scene_resources::claims::tests::emptying_and_refilling_a_ledger_is_never_mistaken_for_damage` ficou
  >30 min com CPU saturada; encerrei só esse processo de teste filho meu para o resto rodar. A lib de
  `media` re-executada pulando-o (`32-…`): 81 ok, **3 falhas**
  (`directed_resources::lease::…second_lease…`, `scene_resources::base::…managed_base…`,
  `claims::…record_that_never_finished…`), 1 filtrado.
- **Integração `media` com falha (execução, não compilação):** `directed_resources_transport` 3 (separador
  `\` vs `/` no helper `tree()` de `directed_resources_common`; é de teste, não do produto),
  `job_discard_claims` 7, `scene_claims_integrity` 4, `thumbnails` 2, `scene_resource_publication` 2,
  `scene_claims_failure` 1, `scene_claims_processes` 1, `scene_import_cleanup` 1,
  `scene_resource_integration` 1 (22 no total). Mesmo conjunto de alvos que a rodada por pacote de
  2026-11-12 (`pkgtests2`); não afirmo equivalência exata com as "27 falhas" de B3 (contagem e o teste que
  trava diferem), apenas que o conjunto de alvos coincide.
- **B5 confirmado:** `core --test scene_gizmo_rotation` (1) e `scene_gizmo_scale` (1) — 2 falhas de floats.
- **B4 (`shasum`) não observável aqui:** esta worktree não tem o runtime Three.js
  (`./scripts/runtime setup` não foi executado nela). Com `TESSEL_REQUIRE_RUNTIME=1`, 16 alvos de `render`
  falham por **ausência do runtime** (asserção do próprio teste), não por regressão; sem a variável, `render`
  passa porque os testes se auto-ignoram (`31-…`, exit 0). **Isso não prova nada sobre render/B4.**
- `desktop` (502 ok), `audio` (10 ok), `core` lib (574 ok) e demais alvos de `core` passam.

Nada acima foi tocado por este diff; nenhum teste foi excluído para fazer algo passar.

## Pendente / não validado

- **Mac:** a entrega original não incluía prova Mac; a validação posterior do coordenador
  está registrada abaixo.
- Execução real, claims B2/B3/B4/B5, descoberta de ferramentas (BL167.2), `shasum`/floats (BL167.3), cache
  (BL167.4) seguem abertos. Esses limites têm IDs: separador de caminho em
  `directed_resources_common::tree()` (BL167.7) e travamento de claims no Windows (BL167.6).
- Nenhuma claim de aceite físico (janela/UI) foi feita; esta mudança é só de testes.

## Registro no backlog

BL167.1 registra o aceite desta entrega; BL167.6 registra o travamento de claims e
BL167.7, a representação de caminhos no helper de testes. BL167.2–.5 permanecem
fora deste patch. Não há fila de execução independente neste relatório.

Ownership usado: `crates/media/tests/{job_commit_state,async_host,directed_resources}.rs`,
`crates/media/tests/common/{mod,wrapper}.rs`, e (ampliado pelo coordenador)
`crates/media/tests/{package_atomic,directed_resources_transport}.rs` e
`crates/desktop/src/workstation/viewport_host.rs`; relatório novo em `docs/migration/`.


## Recebimento e revisão no Mac

Os bytes originais foram recuperados dos outputs JSON do transcript da
[thread de implementação](t3-thread://v2/33595e42-547d-42b7-92a7-cb4acb430dd7/mcp:launch:bfdb8ea7c5701b4cac09ace241a44b8d1a24dc9d86f05f45996409a2972dca4c),
com base64, tamanho e SHA-256 verificados antes de aplicar. Patch: 11462 bytes,
`fc02e703c33b0e250fe00ef1a129a6f485081ab6a34795f20caab9f3263776bc`.
Relatório original: 10169 bytes,
`012dbc3e68c7aa885ba1a46f9665ff0e71bfe09377fffe34356901b40760437e`.
Os originais ficam preservados no diretório de evidências, em `received/`.
Este Markdown foi decodificado como UTF-8 e teve CRLF normalizado para LF antes
destas atualizações: seu hash atual não é o hash do arquivo original remoto.

Aplicado sem conflito no worktree T3 `fix/bl167-1-macos-validation`, base
`ead9ca2bde510a5d1cc15571443a93b6162dc499`, macOS arm64. A raiz antiga e seus cinco
planos locais foram preservados; somente as linhas novas de coordenação BL167
foram integradas ao backlog desta base, sem copiar o backlog antigo inteiro.

Revisão do código: os dois testes passam a usar o mesmo helper; os corpos dos
testes e suas asserções foram preservados. O shell Unix continua usando `exec`
e encaminhando `"$@"`, com os mesmos sinalizadores/delays; `delay == 0` deixa de
executar `sleep 0`. O ramo Windows usa extensão `.cmd`, espera explícita quando
necessária e encaminhamento ao executável real. Os testes Windows registrados
exercitam esse caminho com FFmpeg real. Os imports foram movidos para seus únicos
consumidores Unix e o único chamador de `allow_file` está no módulo Unix `held_read`.
Nenhuma regra de domínio, comando público ou comportamento de UI foi alterado;
por isso não há nova feature ou promoção da matriz de lib/CLI/UI.

Evidências Mac: `/Volumes/Extreme Pro/tessel-bl1671-validation/` (`received/`,
`mac/results.jsonl`, logs por comando e `mac/code-manifest.json`, com SHA-256 dos
8 arquivos de código). O índice JSONL registra comando, exit, duração e hash de
cada log. Essas durações são execução dos checks, sem aceite de desempenho.


### Resultados Mac da árvore aplicada

Todos os comandos abaixo usaram `./scripts/cargo`/mbx, target padrão, sem
`CARGO_TARGET_DIR`, lane própria, limpeza de cache ou lock de desempenho.
A mudança de código é exatamente o patch recebido: a conferência reversa
`git apply --reverse --check` passou e o manifesto identifica os 8 arquivos.

| Comando | Exit | Log em `mac/` |
|---|---|---|
| `./scripts/cargo fmt --all -- --check` | 0 | `01-fmt.log` |
| `./scripts/cargo check --workspace --locked` | 0 | `02-check.log` |
| `./scripts/cargo clippy --workspace --all-targets --locked -- -D warnings` | 0 | `03-clippy.log` |
| `./scripts/cargo test --workspace --locked --no-run` | 0 | `04-test-norun.log` |
| `./scripts/cargo test -p tessel-editor-media --locked --test job_commit_state --test async_host --test directed_resources --test directed_resources_transport --test package_atomic --no-fail-fast` | 0 | `05-media-targets.log` |
| `./scripts/cargo test -p tessel-editor --locked held_read -- --nocapture` | 0 | `06-held-read.log` |
| `./scripts/cargo build -p tessel-editor --locked` | 0 | `07-build.log` |

Os cinco alvos de mídia passaram sem ignorados: `job_commit_state` 1/1,
`async_host` 8/8, `directed_resources` 19/19, `directed_resources_transport` 20/20
(módulo Unix de containment incluído), `package_atomic` 10/10 (permissões incluídas).
Total mídia: 58/58. O filtro `held_read` executou 7/7 no desktop, sem ignorados,
incluindo o único chamador de `allow_file` (substituição/reload do projeto).
A spec de tabelas do backlog passou 19/19 após a integração documental.

**Aceite local BL167.1 cumprido:** Clippy e compilação de todos os testes no Windows
e no Mac, execução dos testes portáveis afetados no Windows e da cobertura Unix
no Mac. Não equivale a workspace Windows verde: suas falhas de execução, o hang
de claims, runtime ausente nesse worktree, B2–B5 e BL167.2–.7 continuam explícitos.
O workspace Mac inteiro não foi executado, nem render/runtime completo, porque
não há mudança no renderer ou regras de domínio; foram executados os 65 testes
Mac que cobrem os arquivos alterados e seus mecanismos de plataforma.
Não foi feita nova prova física, medição de desempenho, export GPU ou áudio ouvido.
Sem push, PR ou merge nesta entrega. O backlog registra o commit local do código.
