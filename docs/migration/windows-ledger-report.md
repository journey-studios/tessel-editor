# BL167.6 — ledger de claims no Windows

Relatório da frente W2. O [backlog comum](../backlog.md) centraliza status e seguimentos (esta entrega não o
edita; o patch proposto está na resposta ao coordenador). Contrato da feature:
[scene-resources](../features/scene-resources.md).

## Escopo

`emptying_and_refilling_a_ledger_is_never_mistaken_for_damage` ficava mais de 30 min com a CPU ocupada no
Windows. Reproduzir isolado, diagnosticar e corrigir sem pular cobertura, sem alongar o prazo como única
correção, sem aceitar ledger corrompido e sem perder claims, arquivos ou locks. Fora de escopo: BL167.3/.7
(W1), descoberta de ferramentas, `Stamp` dev/inode (BL078).

## Base e ambiente

| Item | Valor |
|---|---|
| Checkout | `E:\www\tessel-roadmap-w2-20261114`, branch `fix/bl167-6-ledger-windows` |
| Base | `2b54ac13b4b9f595b14614ae541cd0beb5921daf` (`origin/main`, PR57) |
| Ambiente | Windows 10.0.26200, Rust 1.99.1, Git Bash + mbx, target padrão, sem `CARGO_TARGET_DIR`; 313 GiB livres; RAM 31,9 GiB livres de 63,7 |
| Binário `target\debug\tessel-editor.exe` | SHA-256 `EDD86551ED94938B3C124284F206232D0E76A6959540688A6CF838039B9DC6F7`, 69 385 216 bytes (código final, árvore com as mudanças desta entrega) |
| Logs duráveis | `C:\Users\wally\tessel-readiness-logs\2026-11-14-bl167-6\` (`run-test.ps1` = executor com prazo, `gates.ps1`, `_gates-summary.log`, logs numerados) |

## Reprodução

O teste rodado como processo filho gerenciado, prazo de 120 s: estourava o prazo com ~118 s de CPU. Parte
do que parecia "travar" era o **teste**: o leitor da corrida girava (`while` sobre um contador) depois que os
escritores entraram em pânico, então uma falha real virava uma espera infinita. O teste agora usa um guard
(`Writer` com `Drop`) e termina com a mensagem da falha.

## Causas (com evidência)

1. **Exclusão mútua violada no Windows.** `same_file(path, &file)`, a checagem de que o `ledger.lock` aberto
   ainda é o arquivo do caminho, era `path.exists()` fora do Unix. Quem esperava um lock desvinculado o
   obtinha e achava que valia. Dois `Tracker` seguravam o ledger ao mesmo tempo. Correção: identidade
   volume + índice do arquivo via `same-file` (`Handle::from_file/from_path`, sem `unsafe`; já estava no
   lockfile por `walkdir`). Prova determinística: `a_lock_file_replaced_at_its_path_is_not_the_one_that_was_opened`;
   estresse: `trackers_never_hold_an_emptied_and_refilled_ledger_at_the_same_time`. Mutação (voltar a
   `path.exists()`) faz os dois falharem.
2. **Inanição por contagem fixa de tentativas.** O laço de `DiskLock::acquire` tinha `ATTEMPTS` fixo; sob
   disputa por um arquivo que é criado/removido o tempo todo, o contador acabava antes do prazo
   ("created and removed too quickly"). Agora a contagem é um piso e o orçamento (`LOCK_WAIT`/`TICK_WAIT`) é o
   teto. Mutação confirma.
3. **`append_accept` não cortava a cauda rompida do diário no Windows.** O handle de append não tem
   `FILE_WRITE_DATA`, então `set_len` é negado. Agora abre com escrita, corta no fim do prefixo completo e
   posiciona (`seek`) explicitamente. **Afeta também Unix** (ver Mac).
4. **Falha residual por inanição (medida, não era do FS).** Com a instrumentação temporária (removida) em 40
   execuções do teste de exclusividade: retenção máxima por tracker mediana 2,1 ms (pico ~10 ms), espera
   máxima mediana 629 ms (pico 1,29 s). Então o lock não é segurado por muito tempo: o polling de 500 µs não
   tem fila e um dos quatro workers espera perto do 1,5 s dos jobs. Isso é política de espera, não violação.
   O teste de exclusividade usa `lock_creating_for/lock_for(60 s)` (hooks `#[cfg(test)]`), e o limite de 1,5 s
   segue coberto pelos seus testes. Produção não foi alterada por isto.
5. **"Acesso negado" ao abrir `ledger.lock` com remoção pendente.** Visto em 2 de 40 execuções da corrida
   sem tratamento. O open agora é repetido até 50 vezes (2 ms) quando o Windows responde `PermissionDenied`,
   depois a recusa é reportada como é. É uma mitigação de sintoma observado, **não provada por mutação**
   isolada (a taxa é baixa).

### Hipóteses descartadas

- `unlock` explícito no `Drop`: parecia ajudar (3/30 → 0/30), mas a mutação sem ele deu 0/40; revertido.
- Erros espúrios de `same_file`: 0 observados.
- Retry de remoção pendente retirado cedo demais (0/80), reposto após 2/40 no código final (causa 5).

## Resultados da implementação original (Windows real; anteriores à revisão)

| Prova | Resultado |
|---|---|
| `emptying_and_refilling_a_ledger_is_never_mistaken_for_damage`, 40 execuções isoladas | 0/40 falhas |
| `trackers_never_hold_an_emptied_and_refilled_ledger_at_the_same_time`, 40 execuções isoladas | 0/40 falhas (antes: 1/20 com o orçamento de 1,5 s) |
| Módulo `scene_resources::claims` | 48/48 |
| Dois processos: `scene_claims_processes` 14, `scene_claims_journal` 13 (×3), `job_discard_claims` 13, `scene_claims_integrity` 11, `scene_claims_jsonl` 6 | passam |
| `cargo fmt --all -- --check` | exit 0 |
| `check --workspace --locked` / `clippy --workspace --all-targets --locked -- -D warnings` / `build -p tessel-editor --locked` | exit 0 / 0 / 0 |
| `test --workspace --locked --no-fail-fast` | 12 falhas, **todas fora do ledger** (abaixo); sem falha em `claims` |

`test --workspace --locked` sem `--no-fail-fast` para no primeiro binário que falha; o gate completo fica
**A validar** até as 12 falhas abaixo serem tratadas pelos donos.

### Falhas restantes e dono

| Falhas | Causa | Dono |
|---|---|---|
| `scene_gizmo_rotation` 1, `scene_gizmo_scale` 1 (BL167.3); `directed_resources_transport` 3, `scene_resource_integration` 1, `scene_resource_publication` 1 (helper `tree()` com separador `\`, BL167.7) | frente W1 | W1 |
| `directed_resources::lease` 1, `scene_resource_publication` 1 (`a_second_operation_on_a_leased_root…`) | `lease.rs` não consegue ler o PID do dono: o lock obrigatório do Windows (`LockFileEx`) impede outro handle de ler o arquivo (`Null` em vez do PID) | novo ID proposto |
| `scene_import_cleanup` 1 | `adopt.rs` compara caminhos com `/` (`scenes\street` ≠ `scenes/street`) | novo ID proposto |
| `scene_claims_failure` 1 (`a_cancellation_that_races…`) | `without_taken_back` em `adopt.rs` (~l.241–262) usa `/`, então uma cópia já devolvida é tratada como sobra (os error 3). Patch com prova em `adopt-rs-proposed.diff` nos logs; **não aplicado** (fora do ownership) | novo ID proposto |
| `scene_resources::base` 1 | `/p/ws` não é absoluto no Windows (teste) | novo ID proposto |

Nenhuma dessas falhas foi atribuída ao ledger e nenhuma foi mascarada.

## Ajustes de teste (portabilidade, sem afrouxar oráculo)

- `scene_claims_integrity.rs`: comparações com `/` passam por `slashed()`/`path_of()`. Duas asserções
  `!contains("/lot/")` eram vazias no Windows e agora verificam de fato.
- `job_discard_claims.rs`: `claims_of` normaliza `\`; `signal()` é real no Unix e no-op documentado fora
  dele (sem `SIGSTOP`).
- `scene_claims_journal.rs`: removido `#![cfg(unix)]`; os 13 testes rodam no Windows.
- `claims/tests.rs`: guard de pânico na corrida, testes novos (identidade do lock, exclusividade, `lock_for(ZERO)`
  2000 vezes, leitura repetida de ledger esvaziado/recheado).

## Limites

- `scene_claims_budget` continua só Unix (mede tempo do tick; exige a rodada de desempenho com o lock de GPU).
- Sem `SIGSTOP` no Windows: o peer segura o lock, mas não é suspenso.
- `Stamp` sem dev/inode (BL078). Identidade volume+índice exercitada em NTFS; ReFS e FS de rede não foram provados.
- Testes `read_only` são vazios no Windows (permissão de pasta não impede o dono).
- Sem fila no lock: inanição sob disputa contínua é possível dentro do orçamento (causa 4); fairness em produção
  não foi implementada (decisão proposta).

## Mac: revalidação pendente

Não executado no Mac. O que pode mudar lá: `append_accept` (abertura com escrita + `seek`, afeta Unix); `same_file`
no Unix não mudou, só passou a `pub(super)`; `scene_claims_journal` era `cfg(unix)` e roda nos dois; o retry de
`PermissionDenied` é `cfg!(windows)`. Rodar: `scene_resources::claims`, `scene_claims_*`, `job_discard_claims`
(inclusive `scene_claims_budget` e o peer com `SIGSTOP`) antes de integrar.

## Arquivos fora do ownership nominal

`crates/media/Cargo.toml` (dependência `same-file = "1"` em `cfg(windows)`) e uma linha em `Cargo.lock`
(`"same-file"` em `tessel-editor-media`; a crate já estava no lockfile). Necessários porque `unsafe` é proibido e
`file_index` do std é instável.

## Revisão independente de `0cea8d6` e correção do coordenador

A revisão **não aprovou** este head. Confirmou identidade do lock e exclusividade, com mutação
negativa de `path.exists()`, e repetiu claims 48/48 e os testes de processos listados acima.
A corrida falhou por inanição em **1/140** execuções; exclusividade teve 0/40 falhas. Os 0/40
do autor continuam como resultado histórico, sem contradizer a falha posterior.

**P1 / BL167.6.1:** PermissionDenied repetia 50 pausas sem olhar o deadline. Com diretório ou
arquivo somente-leitura em `ledger.lock`, ZERO levou 123–126 ms, tick 121 ms e accept 122 ms,
acima do orçamento de 20 ms e do teto de 100 ms. A correção exige prazo restante antes de
retentar e limita a pausa a esse restante. LOCK_WAIT (1,5 s), TICK_WAIT (20 ms) e a mitigação
de remoção pendente para jobs são preservados. Os dois testes novos em
`claims/denied_budget_tests.rs` são Windows reais, verificam as três recusas e preservação
das claims; até a rechecagem Windows, são testes escritos, **não executados**.

**BL167.6.2:** a corrida usa `lock_creating_for/lock_for(60 s)` em leitores e escritores,
assim como o teste de exclusividade. É prazo limitado de teste para verificar integridade
sob churn, não mudança do limite produtivo nem alegação de fairness. O guard de pânico do
escritor permanece. O BOM UTF-8 introduzido em `claims/tests.rs` foi removido.
**Isto não bastou:** a revisão R2 em `8cad5c2` ainda viu a corrida falhar (seção abaixo). A
corrida **não** era só um problema de orçamento de teste.

**Proveniência:** `_gates-summary.log` do autor identifica a base `2b54ac1` com árvore ainda
sem commit. O hash do binário identifica aquele artefato; os logs não validam por SHA o head
publicado `0cea8d6`. Nenhum log demonstra TESSEL_REQUIRE_RUNTIME=1; esta entrega não afirma
esse gate. As execuções independentes acima pertencem ao head revisado, não ao novo patch.
CodeRabbit NEUTRAL não é aprovação e não existe CI de testes nesta PR.

**Validação da correção:** `cargo fmt --all` executado no Mac. O teste de claims pelo wrapper
`./scripts/cargo` falhou antes de compilar: ao criar o target gerenciado, retornou
`Not a directory (os error 20)`. Não houve desvio de target, limpeza ou execução de Rust.
Windows e Unix continuam A validar no novo patch; resultados serão atribuídos ao seu head.

Seguimentos registrados no backlog: RootLease ainda usa `path.exists()` fora de Unix e tem
diagnóstico PID ilegível sob lock obrigatório (BL167.6.3/BL079); rollback de append parcial e
disco cheio sem prova (BL167.6.4); possível giro quente se same_file falha sempre, ainda só
por leitura (BL167.6.5). Nenhum foi tratado como corrigido por este patch.

## R2 em `8cad5c2`: acesso negado ao criar a pasta do ledger (BL167.6.2)

**Revisão R2 (head `8cad5c2`).** O P1 de BL167.6.1 está fechado: o open do lock recusado responde em
ZERO ~0,2 ms e tick/accept em ~20 ms (guarda de prazo + pausa mínima). O **P2 continua aberto**: a
corrida `emptying_and_refilling_a_ledger_is_never_mistaken_for_damage` falhou **5/320** com
`PermissionDenied` (os error 5) em `fs::create_dir(.tessel-claims)`, **antes** de qualquer open do
lock, então a retentativa do open não a cobria. A remoção pendente é a hipótese do mecanismo, **não
uma causa provada**; só a recusa observada e o ponto onde ela ocorre estão comprovados.

**Não reproduzido localmente.** `base-8cad.exe` (SHA-256
`B3AC779AC5D5C781200B79C3928B5A984F9AEC07D9DC87D41DD901CEAD9B225C`, build de `8cad5c2`),
320 execuções sequenciais, runner próprio, uma por vez, prazo de processo: **0/320** falhas
(`baseline-8cad.log`). Sondas de instrumentação temporária (binários descartáveis, fora da árvore) com a
corrida ociosa e sob carga: 0/160 e 0/160. Então o 0/320 abaixo **não prova a eficácia** da correção:
a base também dá 0/320 nesta máquina. O que ele prova é a ausência de regressão e de timeout.

### Correção (`2522290`, filho de `8cad5c2`; ainda não publicado até esta seção)

`DiskLock::acquire` passa a usar uma política única, `denial_is_transient`, para o `create_dir` da pasta
e para o open do lock: só `PermissionDenied` no Windows; no máximo `DENIED_RETRIES` = 50 tentativas
**somadas** nas duas chamadas (contador `denied` compartilhado, sem zerar); só enquanto o prazo do
chamador não passou (ZERO nunca espera; a pausa de 2 ms é limitada ao restante; o prazo não é
renovado). `LOCK_WAIT` (1,5 s), `TICK_WAIT` (20 ms), a exclusão mútua fail-closed e os erros
definitivos (reportados como são) não mudaram. Sem laço ilimitado, sem aceitar erro definitivo e sem
detecção por PID. Visibilidade: só `DENIED_RETRIES` virou `pub(super)` (os testes contam as tentativas); o
gancho `CREATE_FOLDER` é `#[cfg(test)]`, thread-local, e não existe no binário de produção.

### Testes: o que cada um prova (e o que não)

| Teste (`claims/denied_budget_tests.rs`) | Tipo | Prova |
|---|---|---|
| `a_denied_create_that_goes_away_is_retried_to_a_lock` | **injeção** (os error 5 no `create_dir`, 5 vezes) | o controle de retentativas: 6 chamadas, lock obtido, claim gravada e ledger removido ao sair |
| `a_denied_create_is_not_retried_by_a_caller_that_does_not_wait` | **injeção** | ZERO: 1 chamada, mensagem com `os error 5`, nada criado |
| `a_denied_create_that_stays_is_given_up_at_the_retry_cap` | **injeção** | o teto: 51 chamadas e a recusa é reportada; prazo de 5 s não alarga |
| `a_real_denied_create_respects_the_zero_and_tick_budgets` | **ACL real persistente** (`icacls /deny *S-1-1-0:(AD)` na pasta do teste; levantada no `Drop`) | uma recusa real do `create_dir` dentro do orçamento (ZERO e tick, abaixo do teto de 100 ms), mensagem da recusa, nada criado, ledger funciona após levantar a regra |

A injeção **não** prova o mecanismo do NTFS na corrida; a ACL real **não** reproduz a recusa transitória
(ela fica na pasta, não some em milissegundos): prova apenas a recusa real e o orçamento. Cada
falha do mecanismo real continua só observada pelo revisor.

**Teste anterior, substituído.** O commit local não publicado `2d4f58b` trazia um teste ACL
**transitório** (a regra era levantada por uma thread enquanto o `create_dir` repetia). Ele foi
frágil: `gate-claims-lib.log` registra `a_denied_create_that_goes_away_is_retried_to_a_lock … FAILED`
(51 passaram, 1 falhou, exit 101). Não foi ignorado nem relaxado: foi trocado pelo seam
`CREATE_FOLDER` (injeção determinística) mais o teste de ACL real persistente. `2d4f58b` nunca foi
publicado e foi substituído por `2522290` com o mesmo pai (`8cad5c2`), via plumbing local.

### Evidência por head (Windows 10.0.26200, Git Bash, `./scripts/cargo`, target padrão)

Todos os logs em `C:\Users\wally\tessel-bl167-6-2-logs\`. Fonte final: `ledger.rs` SHA-256
`9A57B83B485DD9BD24CB9F11F56E1E4158D31EDE33B98BF312C8817089434774` (cópia `ledger.rs.HEAD2`).

| Prova em `2522290` | Resultado | Log |
|---|---|---|
| `cargo fmt --all -- --check` | exit 0 | `g3-fmt.log` |
| `./scripts/cargo check --workspace --locked` | exit 0 | `g3-check.log` |
| `./scripts/cargo clippy --workspace --all-targets --locked -- -D warnings` | exit 0 | `g3-clippy.log` |
| `test -p tessel-editor-media --lib claims` | **54/54** (os 6 `denied_budget_tests` incluídos) | `g3-claims-lib.log` |
| `scene_claims_processes` / `scene_claims_journal` / `scene_claims_integrity` / `scene_claims_jsonl` / `job_discard_claims` | 14 / 13 / 11 / 6 / 13 passam | `g3-integ-*.log` |
| Corrida ×320 sequencial (`final-head2.exe`, SHA-256 `553BC7B37A2BCD87FFB76C681F1D6F9BAFEB1EDBB359BDF3E6BBE6FDFD315F21`, binário compilado de `2522290` limpo; runner `run-seq.ps1`: um processo por vez, 60 s de prazo, um runner por log) | **320/320**, 0 falhas, 0 timeouts, sem sobreposição (tag `final-head2-2522290`) | `final2-race-320.log` |
| Exclusividade ×40 (mesmo binário, mesmo runner) | **40/40** | `final2-excl-40.log` |
| Mutantes sobre a fonte final (binários `h2-mut*`, fonte restaurada, árvore limpa) | todos mortos, abaixo | `h2-mutants.log` |

Mutantes (`denied_budget_tests`, `--test-threads=8`): sem retentativa do create → 2 falham
(`…goes_away…`, `…given_up_at_the_retry_cap`); sem guarda de prazo → 4 falham (ZERO, ACL real, e os dois
da recusa do lock); sem teto → 1 falha (`…given_up_at_the_retry_cap`; passa por 5 s de prazo).

**Não herdados.** `final-race-320.log` (tag `2d4f58b`, 320/320), `final-excl-40.log` e os `gate-*.log`
de 16:43–16:44 pertencem ao `2d4f58b`: o `ledger.rs` daquele head tem outro hash
(`05205B12A3320874948F324B370970811E96D7437959583EAE62C35E95B0BC7F`) e os testes diferem. Não são
prova de `2522290` e foram repetidos (`g3-*`, `final2-*`).

### Limites e o que continua em aberto

- Workspace inteiro **não** foi declarado verde: falhas conhecidas de W1/`lease`/`adopt`/`base`
  (tabela acima) estão fora do ledger e não foram reexecutadas.
- Sem prova de performance/GPU, janela, Unix ou Mac. `append_accept` (abrir com escrita + `seek`) **ainda
  precisa de Mac**; o retry novo é `cfg!(windows)`.
- A correção do P2 depende da hipótese do mecanismo; se a recusa vier de outra janela (por exemplo
  um antivírus mantendo a pasta por mais de ~100 ms), o teto de 50 retentativas ainda a reportaria.
- BL167.6.2 permanece **A validar** até a revisão independente do head publicado. Fechar BL167.6.1 no
  Windows não equivale a PR aprovada.

## Revisão independente R3 e integração com main

R3 aprovou o código Windows no head f0544b09d1b0f4d344f329c0bac83ec855edb584,
sem achado P0/P1/P2 de código. O ledger é2522290 (SHA9A57B83B…434774).
O revisor executou:claims54/54, denied6/6, processos14, journal13, integrity11,
jsonl6, discard13, fmt/check/clippyall-targets e treecore semGPUI/wry.
Corrida320/320 e exclusividade40/40 independentes/sequenciais, semtimeout.
Quatro variantes de mutantes foram detectadas (retrycreate, deadline, deadline
mais minpause e teto), com fonte restaurada por SHA. A base0/320 do autor continua
impedindo alegar eficácia causal via stress; o seam e a ACL provam o controle.

O conflito com main e3c20cb estava apenas no backlog: históricos W2 e S0/S1a
anexados no mesmo ponto. A resolução preserva os dois; não altera ledger nem
testes/media revisados. Cargo.lock combina as arestas same-file Windows da W2
e proc-macro2 somente dev do core/S1a. Dois bytes de controle foram removidos
de scene-resources.md e a matriz/backlog receberam o estado R3. Estas mudanças
e a integração ainda precisam dos gates do seu próprio head.

Unix/Mac append_accept(write+seek), builddesktop e workspace completo não foram
executados na R3. O target legado do checkout Mac aponta para a raiz antiga do
cache; doctor vê a raiz atual no SSD, mas adopt não repara links. A prova Unix
será feita em checkout limpo pelo wrapper e target padrão, sem override/clean/gc.
Sem prova de ReFS/rede, mecanismoNTFS, GPU, janela ou performance.

Dívidas baixas da R3 foram registradas no backlog: BL167.6.7 (segundo panic se
Drop da ACL falhar durante unwind) e BL167.6.8 (pastas vazias de teste de descarte
filho). Nenhuma muda o contrato do produto desta correção.

## Fechamento no head integrado89a8401

A nova revisão independente aceitou a integração Windows no head exato
89a840119a0ce3f0d728d76a63129fdaa12656dc. Parents f0544b0 e e3c20cb; diff
media versusf054 e core/desktop versuse3c20cb vazios. Só o backlog tinha
conflito; nenhum código de merge inventado. Verificou ausência dos dois bytes
de controle nos259Markdown com controle negativo.

Windows: fmt/check/clippy, workspace no-run/build e treecore passam. Claims54,
denied6, processos14, journal13, integrity11, jsonl6, discard13, corefx25,
transições61 eGPUI80 passaram. Os320/40 e mutantes continuam sendo da R3f054,
fontes do ledger/deniedtest idênticas; não houve repetição por ritual.

Coordenador no Mac nativo aarch64 executou sobre89a8401 limpo, usando checkout
novo e targetpadrão gerenciado pelo mbx no SSD (sem target-dir/lanes/clean/gc):

| Verificação | Resultado |
|---|---|
| fmt, checkworkspace, clippyworkspacealltargets-Dwarnings | exit0 em todos |
| build-p tessel-editor | exit0 |
| claimsunit | 51/51 |
| discard/failure/integrity/journal/jsonl/processos | 13/6/11/13/6/16, todos exit0 |
| Core completo | 1528ok,0falhas,7ignorados;62resultados,exit0 |
| Treecore | semgpui/wry |

Isso cobre o write+seek Unix do journal, não apenas o Windows. Logs e manifesto
com hashes ficaram fora do Git; o binário desktop tem hash registrado no
manifesto local, mas não houve janela real. O link legado do outro checkout
permanece quebrado; doctor/adopt não o corrigiram e não foi alterado à mão.

P3 confirmado pela revisão Windows em89: create_dir sobre pai inexistente
retenta64/478/32358vezes paraZERO/20ms/1.5s. Orçamento respeitado e failclosed,
mas CPU quente e mensagem imprecisa. Registrado no mesmo BL167.6.5; correção
de pausa limitada ao prazo fica pendente. same_file persistente continua
hipótese por leitura. Nenhum P0/P1/P2 de código nesta revisão.

Workspace inteiro, performance/claims_budget, Linux, ReFS/rede, mecanismo
NTFS causal, janela/GPU e áudio continuam sem prova. O fechamento posterior
altera só documentação; não atribuir os gates de89 a mudanças de código.

## Correção posterior a89: pausa nos retries rápidos (BL167.6.5)

Para fechar o comentário automático antes do merge, o coordenador implementou
a correção pequena: helper de pausa500us paga do deadline original, sem sleep
quando restantezero; todos os retries rápidos de create/open NotFound,
InvalidInput e identidade divergente o usam. No último, drop do handle precede
a pausa. Mensagem final: retry budget exhausted, sem presumir a causa.

O helper fica em módulo próprio, com substituição do sleep somente cfg(test)
e por thread. Quatro regressões novas de controle (create/open transitório,
ZERO nosdois e workspace real ausente) não dependem de tempo de agendamento.
Nada de benchmark de CPU/FPS ou redução medida é alegado.

Esta mudança altera o código depois de89: as tabelas anteriores continuam
históricas. Execução nativa, mutantes e rechecagem Windows do novo head são
necessários antes de integrar. Retentativas PermissionDenied50 e prazos
LOCK_WAIT/TICK_WAIT permanecem inalterados.
