# Claims, leases e caminhos de `crates/media` no Windows (B3) — relatório de entrega

Origem: bloqueio **B3** de [computador-readiness-2026-11-12.md](../computador-readiness-2026-11-12.md) (suíte de `media` sobre
claims/leases/caminhos falhando no Windows) e o teste que travava a suíte
(`claims::tests::emptying_and_refilling_a_ledger_is_never_mistaken_for_damage`). Liga com BL078, BL111 e BL094.3 de
[backlog.md](../backlog.md); as linhas propostas estão no fim (o backlog não foi editado nesta entrega). Nenhum commit, push ou PR.

## Base e ambiente

- Windows 11 Pro (10.0.26200), NTFS, Rust 1.99.1, `./scripts/cargo` (wrapper). Base `ead9ca2` mais o trabalho não commitado do
  checkout principal (descoberta de ferramentas BL167.2, divisão de clipe F017, portabilidade dos testes de `media`), aplicado
  antes de qualquer mudança desta entrega.
- Escopo de arquivos: só `crates/media/` e este relatório. Sem dependência nova, sem mudança em `Cargo.lock`, sem `windows-sys`.
- Sem lock de GPU: nenhuma medição de desempenho foi feita nem declarada (os orçamentos de tempo dos testes são asserções
  funcionais já existentes).

## Fatos do Windows medidos (programa à parte, Windows 11 / NTFS, Rust 1.99.1)

| # | Fato | Consequência |
|---|---|---|
| W1 | `File::try_lock` é `LockFileEx` sobre o arquivo inteiro e **obrigatório**: outro handle que lê recebe `os error 33` | o PID escrito no arquivo de lock não pode ser lido por quem foi recusado |
| W2 | `fs::remove_file` de um arquivo aberto (e travado) por outro handle **funciona na hora** (semântica POSIX): o nome some e um arquivo novo pode ser criado com o mesmo nome enquanto o handle antigo ainda trava o antigo | "o caminho existe" não prova que o lock obtido é o do arquivo do caminho |
| W3 | o NTFS faz *tunneling* da data de criação: um arquivo criado com o nome de um apagado há pouco herda a data de criação do apagado | a data de criação, como está, não identifica o arquivo |
| W4 | `File::set_times` com `set_created` pelo handle muda a data de criação, inclusive de um arquivo já desligado do nome; `fs::metadata(caminho)` lê o valor novo | um carimbo aleatório posto por quem detém o lock identifica o arquivo |
| W5 | um handle aberto só com `append(true)` não pode ser truncado: `set_len` dá `Acesso negado (os error 5)` | o corte do registro rasgado do journal falhava |
| W6 | um *stream* NTFS (`lock:holder`) de um arquivo travado é legível por outro handle, não aparece em `read_dir` e some com o arquivo | lugar para o PID do detentor (W1) |
| W7 | `file_index`/`volume_serial_number` continuam instáveis (`windows_by_handle`) em Rust 1.99.1 | não há dev/ino do Windows sem dependência nova |

## Causas e correções, por grupo de falha

### G1. Dois detentores do mesmo lock (produto) — o teste que travava e o ciclo de dois processos

- **Falhas:** `claims::tests::emptying_and_refilling_a_ledger_is_never_mistaken_for_damage` (travava; com o harness corrigido,
  falha com `claims.json: os error 2` — outro "detentor" apagou a pasta do ledger no meio da escrita);
  `scene_claims_processes::two_processes_cycling_adoptions_on_one_workspace_leave_it_as_it_was` (sobravam `scenes` e
  `scenes\street` no workspace).
- **Causa:** o `same_file`/`is_still_the_lock_file` fora de Unix era `path.exists()`. Por W2, quem esperava o lock de um arquivo
  que o detentor anterior desligou ao esvaziar o ledger obtinha o lock do órfão, enquanto outro criava e travava o arquivo novo
  com o mesmo nome: dois detentores, leitura-modificação-escrita perdida, pasta do ledger removida sob o outro.
- **Correção:** novo `crates/media/src/lock_file.rs` (só Windows): `is_still_at` carimba a data de criação do arquivo recém-travado
  com um valor aleatório pelo próprio handle e compara com a do caminho (W3, W4). Só o detentor carimba, o carimbo é posto depois
  que o nome antigo já foi desligado (o protocolo desliga antes de soltar) e o tunneling só repassa valores do momento da
  remoção; logo o caminho mostra o carimbo do detentor se e somente se o arquivo é o dele. Usado por `ledger.rs::same_file` e
  `lease.rs::is_still_the_lock_file` em `cfg(windows)`; os ramos `cfg(unix)` (dev/ino) não mudaram. Volume que recusa mudar a data
  de criação cai no `path.exists()` anterior (limite declarado abaixo).

### G2. Inanição de quem espera o ledger dentro do mesmo processo (produto)

- **Falha:** com G1 corrigido, o mesmo teste falhava 3 em 15 rodadas com "being created and removed too quickly to be locked".
- **Causa (medida com contadores temporários):** 64 tentativas seguidas terminando em lock de órfão. Quem esvazia o ledger desliga
  `ledger.lock` ao soltar; todos que esperavam aquele arquivo ficam com órfãos, enquanto quem acabou de soltar volta a travar o
  arquivo novo antes do próximo *poll* (500 µs) de quem espera. A espera perde sempre. Antes de G1 isso não aparecia no Windows
  porque o órfão era aceito (o erro de G1).
- **Correção:** `ledger.rs`: `Inner.waiting` conta quem está dentro de `DiskLock::acquire` para aquele ledger neste processo
  (`acquire` = contador + `acquire_in`, o corpo anterior). O `Drop` do `Tracker` que esvazia o ledger continua removendo estado,
  journal e `live/`, mas **não** desliga `ledger.lock` nem a pasta enquanto houver quem espere; o último a soltar um ledger vazio
  remove tudo. Estado final idêntico ao anterior; a regra vale para todas as plataformas (é uma correção de justiça do protocolo,
  não do Windows). Quem espera em **outro** processo ainda pode cair num órfão e recomeça (limitado por `ATTEMPTS`).
- **Prova:** 20/20 e 50/50 rodadas do teste (seção Estabilidade).

### G3. `holder: null` no `root_busy` (produto)

- **Falhas:** `directed_resources::lease::tests::a_second_lease_on_the_same_root_is_refused_until_the_first_is_dropped`
  (`lease.rs:144`), `scene_resource_publication::a_second_operation_on_a_leased_root_is_refused_and_writes_nothing` (`:123`).
- **Causa:** W1 — quem é recusado lia o PID do próprio arquivo travado.
- **Correção:** `lease.rs`: no Windows o detentor também grava o PID no stream `.tessel-staging-lock:holder` (W6) e `busy` o lê
  dali (`lock_file::write_holder`/`read_holder`). Unix continua lendo do arquivo. O stream some com o arquivo de lock, então a raiz
  liberada continua "byte a byte como era" e uma listagem não o mostra. Volume sem streams (FAT/exFAT) só perde o nome do
  detentor (`holder: null`), nunca o lease.

### G4. Corte do registro rasgado do journal (produto)

- **Falha:** `claims::tests::a_record_that_never_finished_is_not_an_acceptance_and_the_next_writer_cuts_it_off`
  (`claims.journal: Acesso negado (os error 5)`).
- **Causa:** W5 — `append_accept` abria o journal com `append(true)` e depois fazia `set_len` para cortar a cauda rasgada (e para
  desfazer uma escrita que falhou).
- **Correção:** `ledger.rs::append_accept` abre com `write(true)` e posiciona em `end` antes de escrever. Todo appender segura
  `ledger.lock`, então escrever em `end` é escrever no fim; o comportamento em Unix é o mesmo.

### G5. Resto de rollback dado como sobra (produto)

- **Falha:** `scene_claims_failure::a_cancellation_that_races_the_failure_still_cancels_and_leaves_no_copy` (`:317`,
  `rollbackLeftovers` listava `backdrop.png` e `assets` que a própria adoção já tinha retirado).
- **Causa:** `adopt.rs::without_taken_back` filtrava as linhas do rollback por texto, procurando `/<relativo>`:` — com `/`
  fixo. No Windows a cópia escreve `\\?\C:\…\assets\backdrop.png`, nada casava e o erro dizia que ficaram arquivos que não existem.
- **Correção:** a linha é lida como `` `<caminho>`: <motivo> `` (`rollback_line_path`) e comparada por componentes
  (`Path::ends_with`), ignorando o relativo vazio (a própria raiz), como o texto antigo já fazia.

### G6. Premissas de teste presas a Unix (testes)

| Falha | Causa | Correção |
|---|---|---|
| `scene_resources::base::tests::the_managed_base_is_derived_from_the_workspace_and_the_scene_id` (`base.rs:223`) | `/p/ws` não é absoluto no Windows (sem unidade): o produto recusa corretamente | workspace absoluto por plataforma (`C:\p\ws` no Windows); teste novo `a_workspace_without_a_drive_is_refused_on_windows` (`/p/ws`, `\p\ws`, `C:p\ws` recusados) |
| `directed_resources_transport` ×3 (`:48/:182/:279`), `scene_resource_integration` (`:207`), `scene_resource_publication` (`:95`) | `tree()` montava o relativo com `display()` (`assets\figure.png`) | `tree()` junta componentes com `/`, o separador de caminho de recurso |
| `scene_import_cleanup` (`:186`) | mesma coisa no `names` local | idem |
| `scene_claims_integrity` ×4 (`:283/:324/:497/:569`) | `contains("/lot/")`, `ends_with("lot/assets/…")`, `join("scenes/{id}/…")` comparados como texto com caminhos `\` | `in_folder` por componentes; `files()` por `join` de componentes; `Path::ends_with`. Corrige também dois `!contains("/lot/")` que no Windows passavam sem provar nada |
| `job_discard_claims` ×6 (`:464/:554/:614/:686/:945/:1012`) | `claims_of` procurava `/scenes/<cena>/` como texto | `below_scene` por componentes |
| `job_discard_claims::a_peer_stopped_while_holding_the_ledger_lock_cannot_freeze_the_tick` (`:764`) | `kill -STOP` não existe no Windows | ver G7 |
| `claims::tests::emptying_and_refilling…` (harness) | só os escritores davam `stop`; um escritor que falhava deixava o leitor em laço infinito | `StopOnExit` (drop guard) em cada escritor; `thread::scope` repassa o pânico |

### G7. O que ficou `cfg(unix)` e por quê

- `job_discard_claims::a_peer_stopped_while_holding_the_ledger_lock_cannot_freeze_the_tick`: **continua só Unix**, inalterado no
  que faz. `SIGSTOP`/`SIGCONT` não existem no Windows e suspender um processo lá exige `NtSuspendProcess` (ntdll, não documentado),
  que o crate não liga. O corpo virou `a_peer_that_keeps_the_ledger_lock_cannot_freeze_the_tick(name, Frozen)` e o Windows roda a
  variante `a_peer_idle_while_holding_the_ledger_lock_cannot_freeze_the_tick`: o par trava o ledger e fica ocioso esperando um
  comando que não vem — do lado do lock é o mesmo que um processo parado. O tick voltou dentro do teto de 100 ms (≈20,5 ms, ver
  Estabilidade) com o motivo "gave up after". O processo **parado** em si segue sendo prova só de Unix (lacuna BL168.6.4).
- `scene_claims_budget.rs` (`#![cfg(unix)]`, preexistente): mede o tick (desempenho, exige o lock de GPU pela regra do usuário) e
  usa `std::os::unix::fs::MetadataExt`. Não tocado.
- `scene_claims_journal.rs` **deixou de ser só Unix**: o gate vinha desde a criação, sem motivo declarado; as falhas no Windows eram
  as mesmas comparações de texto de G6 (`has_run` por componentes, registro lido como JSON). 13/13 no Windows.

## Contagens antes/depois (Windows, `./scripts/cargo test -p tessel-editor-media --locked --no-fail-fast`)

"Antes" = base aplicada + só o harness do teste que travava (sem ele a rodada não termina).

| Alvo | Antes (ok/falhas/ignorados) | Depois |
|---|---|---|
| lib (`src/lib.rs`) | 105 / 4 / 2 | 113 / 0 / 2 (+3 `lock_file`, +1 `base` só Windows) |
| `directed_resources_transport` | 13 / 3 / 0 | 16 / 0 / 0 |
| `job_discard_claims` | 6 / 7 / 0 | 13 / 0 / 0 (no Windows a variante ociosa substitui a de `SIGSTOP`) |
| `scene_claims_failure` | 5 / 1 / 0 | 6 / 0 / 0 |
| `scene_claims_integrity` | 7 / 4 / 0 | 11 / 0 / 0 |
| `scene_claims_processes` | 13 / 1 / 0 | 14 / 0 / 0 |
| `scene_import_cleanup` | 31 / 1 / 0 | 32 / 0 / 0 |
| `scene_resource_integration` | 18 / 1 / 0 | 19 / 0 / 0 |
| `scene_resource_publication` | 2 / 2 / 0 | 4 / 0 / 0 |
| `scene_claims_journal` | 0 (só Unix) | 13 / 0 / 0 |
| `scene_claims_budget` | 0 (só Unix) | 0 (só Unix, desempenho) |
| demais 29 alvos | todos ok | todos ok (sem mudança) |
| **Total de falhas** | **24** (4 lib + 20 integração) | **0** |

## Estabilidade (repetições no Windows, binários de teste já compilados)

| Execução | Resultado |
|---|---|
| `emptying_and_refilling_a_ledger_is_never_mistaken_for_damage` (só G1, antes de G2) | 12/15 (3× "created and removed too quickly") |
| idem, com G1 + G2 (`emptying_and_refilling`, `a_second_lease`, `concurrent_acceptances`, `lock_file`) | 20/20 |
| `emptying_and_refilling_a_ledger_is_never_mistaken_for_damage` | 50/50 |
| lib inteira | 5/5 |
| `scene_claims_processes`, `scene_claims_journal`, `job_discard_claims`, `scene_resource_publication`, `scene_cleanup_ownership` | 5/5 cada |

O `BUDGET` da variante ociosa (`tick-idle-holder-peer`, três rodadas fora da suíte) deu 20,6 / 20,7 / 20,4 ms: o orçamento fixo
de 20 ms do tick e nada mais, contra o teto de 100 ms. É saída funcional do teste, não medição de desempenho aceita (sem lock).

## Gates

- `cargo fmt --all -- --check`: limpo.
- `./scripts/cargo clippy -p tessel-editor-media --all-targets --locked -- -D warnings`: limpo (Windows).
- `./scripts/cargo check -p tessel-editor-media --lib --locked --target wasm32-unknown-unknown`: limpo — confere que os ramos
  `cfg(not(any(unix, windows)))` compilam. Os testes não compilam para wasm32 por `tests/common/wrapper.rs` (da base, só
  Unix/Windows), fora do escopo.
- **Não verificado:** compilação e execução em macOS/Linux (nenhum alvo Unix nesta máquina). Os ramos `cfg(unix)` de produção não
  foram editados; as mudanças comuns (G2, G4, G5) foram revisadas para dar o mesmo resultado em Unix.

## Lacunas que continuam

1. **Identidade de recurso no Windows (BL078):** `Stamp`/`Observed` (`cleanup.rs`) e `ensure_same_file` (`source.rs`) seguem sem
   dev/ino fora de Unix (tamanho + SHA-256; a troca entre `canonicalize` e `open` não é detectada). O carimbo de G1 serve só para
   arquivos de lock (exige ser o detentor e escrever metadados); não substitui identidade de arquivo de recurso.
2. **Volume que não aceita mudar a data de criação** (rede, sistemas exóticos): `is_still_at` volta ao `path.exists()` anterior,
   com a corrida de G1. Volume sem streams NTFS: `holder: null`.
3. **Órfão entre processos:** G2 cobre quem espera no mesmo processo; um processo que espera o ledger que outro processo esvazia
   ainda pode perder a corrida até `ATTEMPTS` (64) e receber `claims_unavailable` (falha fechada). Não reproduzido nos testes de
   dois processos.
4. **`SIGSTOP`** só em Unix (G7); **`scene_claims_budget`** só em Unix (desempenho).
5. Unix não executado nesta entrega (ver Gates).

## Linhas propostas para o backlog (não editei `docs/backlog.md`)

| ID | Status proposto | Proposta | Aceite / evidência |
|---|---|---|---|
| BL078 (reuso) | Pendente (parcial) | Acrescentar: identidade de **arquivo de lock** no Windows resolvida por carimbo de criação (`lock_file.rs`, BL168.6.1); identidade de recurso (`Stamp`, `ensure_same_file`) continua sem equivalente a dev/ino | Aceite original mantido para recursos |
| BL111 (reuso) | A validar | Acrescentar: suíte de claims/leases roda no Windows (24 falhas → 0; `scene_claims_journal` passa a rodar lá); "sem prova de Windows" vira "Windows 11/NTFS provado; NFS não" | Este relatório; logs da rodada |
| BL094.3 (reuso) | A validar | Acrescentar: `job_discard_claims` 13/13 no Windows (variante ociosa no lugar do `SIGSTOP`) | Idem |
| BL168.6.1 (novo) | A validar | Lock de ledger/lease no Windows: carimbo de criação identifica lock de arquivo desligado (dois detentores eliminados) e PID do detentor em `:holder` | Testes `lock_file::*`, `emptying_and_refilling…` 50/50, `lease` e `scene_resource_publication` verdes; revisão em macOS de que `cfg(unix)` não mudou |
| BL168.6.2 (novo) | A validar | Ledger esvaziado não é removido enquanto outro chamador do mesmo processo espera o lock (fim da inanição) | `emptying_and_refilling…` 50/50; suíte inteira verde em macOS |
| BL168.6.3 (novo) | A validar | Testes de `media` comparam caminhos por componentes (não por texto com `/`); `scene_claims_journal` sem `cfg(unix)`; journal sem `append` (corte do registro rasgado) e filtro de sobras de rollback por componentes | Contagens deste relatório; macOS verde |
| BL168.6.4 (novo) | Pendente (lacuna) | Provar no Windows o tick contra um processo **suspenso** (hoje só a variante ociosa) e decidir sobre `scene_claims_budget` no Windows (com lock de GPU) | Teste com suspensão real ou decisão registrada |
| BL168.6.5 (novo) | Pendente | Órfão entre processos (lacuna 3): medir/decidir se a espera por órfão deve ser limitada pelo prazo do chamador em vez de `ATTEMPTS` | Teste de dois processos esvaziando/enchendo o mesmo ledger |
