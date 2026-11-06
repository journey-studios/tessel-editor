# S2 / BL111.2 — mutações: cada regra antiga reintroduzida faz um teste falhar

Reprodução: aplicar a troca descrita no arquivo indicado (a regra antiga), rodar o comando e
restaurar. Executado no head integrado (merge com `main` `e296b36`), debug, via
`cargo-serial.py test -p tessel-editor-media --locked ...`. Sem a mutação todos passam.

| # | Regra antiga reintroduzida | Onde | Comando | Teste que falha |
|---|---|---|---|---|
| M1 | aceitar ignora a falha de gravação (`Created::accept` devolve `Ok`) | `scene_resources/cleanup.rs` | `--test scene_claims_integrity` | `an_import_whose_acceptance_cannot_be_written_attaches_nothing_and_adds_no_undo_step`, `an_adoption_whose_acceptance_cannot_be_written_is_refused_and_keeps_its_files`, `a_ledger_without_its_lock_refuses_every_operation_and_removes_no_file` (3 de 7) |
| M2 | fundir move as listas e ignora o `transfer` (`let _ = ...transfer(...)`) | `scene_resources/cleanup.rs` | `--test scene_claims_integrity` | `a_merge_the_ledger_cannot_write_fails_and_nobody_loses_ownership` (1 de 7) |
| M3 | a lease é solta antes de gravar o ledger | `scene_resources/claims/ledger.rs` (`flush`) | `--test scene_claims_processes a_process_refused` | `a_process_refused_its_acceptance_stays_alive_for_the_ledger_and_is_not_recovered_from` (o `recover` do outro processo reapava o vivo e apagava os 2 arquivos) |
| M4 | estado sem lock lido como ledger vazio (`look_for_damage` sempre `Empty`) | `scene_resources/claims/ledger.rs` | `--test scene_claims_processes lock_was_deleted` | `a_ledger_whose_lock_was_deleted_refuses_and_removes_nothing_whoever_is_alive` |
| M5 | o hash do `recover` ignora o token (`CancelToken::default()`) | `scene_resources/recover.rs` | `--lib cancelling_during` | `cancelling_during_the_hash_of_a_large_file_stops_it_and_keeps_the_file_claimed` (sem a regra nova hasheia os 2 GiB inteiros: 67 s em debug; com ela o cancelamento sai em milissegundos) |

Primeira rodada de M2 e M4 passou (falso negativo do teste): M2 rodava com duas cenas e a falha
aparecia antes, na recusa da 2ª cena; M4 usava um filtro que não casava com nenhum teste. Os
testes foram corrigidos (import de uma cena só, para a fusão ser o último passo; filtro
`lock_was_deleted`) e as duas mutações então falharam como na tabela.
