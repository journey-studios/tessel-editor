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
