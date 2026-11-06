# F025 — Jobs, cancelamento e staging (P03)

## Status

Lib **V**, `scripts/media`/`scripts/render` **V**, UI **V** (diálogo Tarefas, linha de progresso na barra de status,
cancelar; `flow.jobs`). `job.discard` (BL094/BL112): lib/host **V** (testes abaixo), CLI headless **V** (processo JSONL real
`tessel-media`, 20 checks), janela **V no escopo de jobs** (instância própria, só requisições semânticas, 19 checks: o tick
entrega a remoção a um job e a janela segue respondendo). Dois processos reais (BL111, S2): ver "Junção com claims persistentes". Sem pixels (jobs não desenham na viewport) e sem ponteiro. Ver "Evidência por superfície" abaixo.

## Contrato

`JobRegistry` (core, sem GPUI): ID, tipo (`import|hash|probe|proxy|package|restore|project_import|video_export|preflight`),
estado `queued → running → succeeded|failed|cancelled`, `discard` (descarte em segundo plano, abaixo), etapa com `done/total`, avisos e erro tipado
(`{code,message}`). No máximo 2 jobs rodam por vez por host. Cancelamento **cooperativo e confirmado**: `cancel`
vira o token e o estado só passa a `cancelled` quando o worker observou e terminou a limpeza.

| `op` | Campos | Notas |
|---|---|---|
| `job.list` | — | todos os jobs |
| `job.get` | `job` | estado, etapa, progresso, avisos, erro; `unknown_element` se não existe |
| `job.wait` | `job`, `timeout_ms?` (60000) | `timeout` se estourar (o job continua) |
| `job.cancel` | `job` | pede o cancelamento; o estado final chega depois |
| `job.commit` | `job` | aplica ao projeto o resultado de um job iniciado com `wait:false`; `job_running`, `job_finished` (nada a aplicar), `job_discarding`, `job_discarded` |
| `job.discard` | `job`, `wait?` | descarta o resultado **finalizado e não aplicado** de `job` num job `discard` em segundo plano; resposta e erros abaixo |

Operações que mudam o projeto (import, relink, project.import) preparam arquivos no job e aplicam o resultado
numa **única transação** na thread do chamador. `wait:true` (padrão) faz tudo numa chamada. No host
**interativo** (janela) a operação devolve **na hora** um job (`{job, state, kind}`) e `poll()` aplica os jobs concluídos no
tick da UI, **um por transação**. O mesmo vale para `media.prepare`, `project.verify`, `project.restore` e a sonda de
capacidades do `export.preflight` (que cria o renderer numa thread lateral e **observa o cancelamento**).

Evento de `poll()`: `{job, state, kind, result, applied}` quando o job terminou e foi aplicado;
`{job, state:"failed"|"cancelled", kind, error:{code,message}}` quando falhou — inclusive quando o trabalho deu certo
mas **aplicar** falhou. Resultados obsoletos **não são aplicados**: um `Decoded` confere o id do projeto e o hash do
asset; um `project.restore` confere `(id, revisão)` do projeto em que foi pedido. Se o usuário editou no meio,
o resultado vira `stale_result` e a edição fica (`async_host.rs`: edição concorrente preservada, projeto trocado, restore
disputado com edição, cancelamento sem arquivos).

**Decode automático:** ao abrir/importar, assets sem `decoded` entram num job «Prepare media» (`proxy`) agendado pelo
host (`schedule_prepare`); `auto_failed` evita laço de repetição com o mesmo hash. `render.frame`/`render.audio` e o
export rodam `ensure_prepared` antes. Não é preciso `media.prepare` manual (o teste `flow.rs` não o chama mais).

## Descartar um resultado (`job.discard`, BL094/BL112)

Um job que produz resultado para o projeto o **retém** (`ctx.hold_result()`): `job.get`/`job.list` trazem
`result_state` = `none | pending | discarding | discarded | applied`.

```text
none ─(corpo guarda resultado)─> pending ──apply (poll/commit/wait)──> applied
                                    │  └─ aplicar recusado (stale) ──> discarded
                                    └─ job.discard ──> discarding ──(job discard terminou)──> discarded
```

`job.discard {job, wait?}` reivindica o resultado **atomicamente** (`JobRegistry::begin_discard`): de dois chamadores
só um ganha, e um `job.commit`/`poll` concorrente não leva mais o resultado. O resultado **move** para um job
`discard` (tipo `discard`, **não cancelável**: `job.cancel` responde `job_not_cancellable`), que remove só os arquivos
que o resultado criou (claims, SHA-256 e lease de F040) e informa o que ficou (`kept`/`shared`). O dono (o resultado, com
seus claims) e o registro do job **permanecem até o fim do job discard**: `prune` nunca descarta um job
`pending`/`discarding`. Resposta: `{action:"job.discard", job:<job discard>, discards:<job original>, state, ...}`;
host sem janela espera e devolve também `discarded` e `cleanup:{removedFiles,removedFolders,kept,shared,busyRoots}`
(`wait:false` devolve `state:"discarding"` na hora); o host interativo responde sempre na hora e `poll()` entrega um
evento `{job, kind:"discard", state, result}` uma única vez. `discarded` quer dizer **o descarte terminou**, não que tudo foi
removido: o que ficou (`kept`, raiz ocupada, ledger indisponível) está em `result.cleanup` e a barra de status da janela o diz
(BL094.8). O que acontece com a **claim** do que ficou depende do motivo, e não é uma garantia única (N4):

1. **Ledger indisponível ao soltar a claim** (`finish_leave`/`leave` não conseguem gravar): o arquivo foi tratado, mas a entrada
   continua gravada e **retida por este processo vivo**; `recover` a resolve depois que o processo acaba (arquivo já removido:
   a entrada some; arquivo ainda lá e provadamente nosso: removido). `kept` pode nomear o mesmo caminho que `removedFiles`.
2. **Raiz ocupada** (a lease de publicação está com outra adoção): o descarte usa `abandon`, a entrada fica **órfã na hora**
   (sem dono vivo) e `recover` a recolhe quando quiser, sem esperar este processo acabar.
3. **Arquivo reescrito** (o SHA-256/metadata não confere, `kept` por hash): o arquivo **não é removido** e a claim é solta junto
   com o resto do descarte. A partir daí `recover` **não o enxerga mais**: ele fica no disco, de quem o reescreveu, sem aviso
   posterior. A única notícia é o `kept` do relatório do descarte.

Nenhum dos três casos promete que `recover` limpará o arquivo: só o 1 e o 2 deixam entrada para ele.

| Erro tipado | Quando (nada muda) |
|---|---|
| `unknown_element` | job inexistente |
| `job_running` | job ainda ativo (cancele e espere o fim) |
| `job_applied` | resultado já aplicado ao projeto |
| `job_discarding` / `job_discarded` | descarte em andamento / já concluído |
| `job_nothing_to_discard` | falhou, foi cancelado ou nunca produziu resultado (ex.: preflight) |
| `job_not_cancellable` | `job.cancel` num job `discard` |

Fora da UI: quando `poll()` (tick da janela) encontra um resultado de adopt/import que não serve mais, o host **não**
apaga nem calcula hash na thread da UI: o resultado vai para um job `discard` e o erro `stale_result` traz
`details.discard_job`. O mesmo vale ao descartar um `MediaHost` interativo com resultados pendentes (o `Drop`
entrega-os a jobs de descarte; um host sem janela continua removendo no próprio `Drop`, para o processo da CLI não
sair antes). O descarte de resultados sem arquivos próprios (decodificação, proxies, relink, restore, import de
mídia) só libera a memória: cópias que `media.import` gravou no workspace nunca tiveram dono registrado e
**permanecem** (limite herdado, sem registro novo).

**Evidência** (`crates/media/tests/job_discard.rs`, 14 + 1 de medição; `crates/core/src/jobs.rs`, 5 novos): resultado
pendente descartado em job observável; host sem janela espera/`wait:false`; commit após descarte → `job_discarded`; aplicado,
ativo, sem resultado, falho e inexistente recusados por código; arquivos, claims e registro retidos enquanto o job
`discard` está na fila (lotando os 2 slots do registro, sem `sleep`), `job_discarding` em commit/descarte, `job_not_cancellable`,
`prune(0)` preserva e depois libera; dois resultados compartilhando arquivos (descartar um não remove o do outro);
arquivo reescrito no lugar permanece; o tick de `poll()` não apaga (arquivos e claims continuam até o job terminar);
descarte sob demanda na janela; `Drop` interativo. `scene_import_cleanup.rs`: `try_apply` recusa sem remover e devolve os
arquivos ainda com dono; `job.discard` de um `project.import_scenes` pendente. Medição (`-- --ignored --nocapture`, 256 MiB,
este disco): `job.discard` retornou em **65 µs** (debug) / **30 µs** (release), o pior `poll` em 1,6 ms / 55 µs, e o job
terminou em **10,9 s** (debug) / **0,66 s** (release); o cálculo de hash saiu da thread do chamador.

### Evidência por superfície (integração S1 + S3 + S4, a validar até a revisão)

Head integrado: merge de `main` `e296b36` (S3 bytes imutáveis, S4 identidade) sobre `3c5383e`. O único conflito textual foi a
matriz (F040–F042); `jobs.rs` e `scene_resources/mod.rs` mesclaram sem sobreposição (S3 só acrescentou os códigos
`resource_changed`/`resource_missing` ao mapa de erros e o módulo `snapshot`; S4 não toca host/jobs).

| Superfície | Prova | Resultado |
|---|---|---|
| Lib (core, sem GPUI) | `jobs.rs` 219 testes do crate, 5 do descarte | verde |
| Host/lib | `job_discard` 14 (+1 de medição), `scene_adopt_host` 13, `scene_import_cleanup` 31, `scene_cleanup_ownership` 26 (3 novos de `settle`/`apply`, BL111), `scene_export` (S3) 12 com 1 novo: job de export recusa `job.discard` (`job_nothing_to_discard`) e nada é removido | verde |
| CLI headless (processo JSONL real `target/debug/tessel-media --empty`) | `scripts/job-discard-proof.py`: adopt pendente → `job.discard` espera e informa `removedFiles:3`; commit/descarte repetidos → `job_discarded`; `job.cancel` do job de descarte já final → `job_finished`; aplicado → `job_applied` com arquivos byte a byte iguais; falho → recusa; commit obsoleto remove **antes** da resposta; EOF com resultado pendente (64 MiB) só termina o processo depois de devolver os arquivos | 20/20 |
| Janela (binário `target/debug/tessel-editor`, HOME curto e socket próprios, só `op`/`get`/`invoke`) | adopt de 128 MiB torna-se obsoleto (cena removida durante o job); a linha do tempo observada é `none → pending → discarding → discarded`; enquanto descarta: arquivos e dono presentes, `job.discard`/`job.commit` → `job_discarding`, `job.cancel` do job de descarte → `job_not_cancellable`, diálogo Tarefas abre e fecha, sem mutação; depois: arquivos e pasta removidos, evento `discard` único no `recent`, evento do apply com `details.discard_job`; resultado já aplicado → `job_applied` sem mudar bytes | 19/19 |
| Responsividade da janela | uma segunda conexão pede `get jobs` a cada ~20 ms; os pedidos rodam na thread principal da janela (`window.update`). Tetos fixados antes da medição: ocioso p99 ≤ 100 ms; durante a remoção p99 ≤ 100 ms e máx ≤ 250 ms. Duas execuções: durante a remoção 173/177 respostas em ~4,3 s, p50 0,7–0,8 ms, p99 1,5–1,6 ms, máx 1,5–2,7 ms; ocioso máx 38–43 ms (1ª resposta após aquecimento) | verde |
| Controle negativo | (a) o mesmo pedido cronometrado enquanto suspendemos **o próprio processo** da janela por 1,5 s mede 1508–1548 ms (a sonda enxerga uma janela parada; nenhuma operação do editor bloqueia a thread de propósito: `wait:true` volta na hora na janela). (b) Mutação temporária no host (`SceneAdopt … if decode_inline \|\| true`, remoção inline no tick) reprova a prova nativa (a transição `discarding` some; a execução parou na 1ª reprovação após ~64 s com a thread presa); revertida | reprova com a mutação |
| Runtime/Chrome (`TESSEL_REQUIRE_RUNTIME=1`, sob lock de GPU) | workspace `cargo test` no head integrado com S2 (`main` `ac04dca`): 1152 passam, 0 falham, 9 ignorados, 71 suítes (inclui `job_discard_claims` 10 novos); `./scripts/runtime test` 93/93. Antes da junção (head `008af90` + `e296b36`): 1076/0/9, 65 suítes | verde |

Repetição da prova nativa no head integrado com S2 (`job-discard-proof.py`, 39/39: JSONL 20, janela 19): durante a remoção 161 respostas em ~4,5 s, p50 0,8 ms, p99 16,4 ms, máx 16,9 ms (teto 100/250 ms; a primeira execução tinha p99 1,5 ms: variação de carga, não mudança de teto); controle negativo mediu 1512,8 ms.

Medição 256 MiB reexecutada no head integrado: `job.discard` voltou em **68 µs** (debug) / **64 µs** (release), o pior `poll`
57 µs / 31 µs e o job terminou em **8,6 s** (debug) / **0,58 s** (release). Relatório gerado da prova nativa:
[`s1-job-discard-proof.json`](../migration/s1-job-discard-proof.json). Reproduzir: `./scripts/cargo build -p tessel-editor
-p tessel-editor-media --locked` e `python3 scripts/directed-scene-proof/with-gpu-lock.py python3 scripts/job-discard-proof.py`.

**Não provado:** pixels/estética (nada novo é desenhado), quadros por segundo e tempo de pintura (só latência de pedidos),
seletor nativo de arquivos, janela fechada com resultados pendentes (só o `Drop` por teste), build release da janela,
Windows/Linux e escritor hostil. Dois processos reais **são** provados desde a junção (BL094.3, `job_discard_claims.rs`; seção
abaixo): o que segue não provado é o encerramento da **janela** com um descarte em andamento (`cx.quit()` não espera, BL112.6). `job.discard` sob demanda **bem-sucedido** na janela não é
observável: o tick aplica um resultado atual em ≤ 250 ms, então o descarte sob demanda de um resultado pendente é provado
pela lib/host (host interativo sem tick) e pelo JSONL; na janela a prova é o descarte do obsoleto e a recusa do aplicado.

### Junção com claims persistentes e aceite fallível (BL094.3, merge de `main` `ac04dca`, S2)

A integração com a S2 (ledger `.tessel-claims/`, lease por processo, `Provisional::accept` fallível) é **semântica**:
o resultado recusado continua dono até o descarte terminar, e nenhuma aceitação é silenciosa.

* `PendingAdoption::settle` e `import::try_apply` aceitam com `Provisional::accept_nowait` (não lê nem calcula hash de recurso; o
  aceite lê e grava `claims.json` sob `flock` e, **no tick, não espera** esse lock: ver "Fechamento do descarte", BL112.3).
  `apply`/`job.commit` da CLI usam `Provisional::accept`, com espera limitada.
  No import o aceite é o **último passo** do closure de `state.transaction`: uma recusa desfaz projeto, undo e revisão.
  Não existe mais `Provisional::release` nem aceitação por `Drop` como prova (o `Drop` do `Created` avulso aceita só na melhor tentativa).
* Recusa por obsolescência **ou** do ledger devolve o resultado **ainda dono dos arquivos e das claims**: `StaleAdoption{why, refusal,
  pending}` / `RejectedImport{error, prepared}`. O host entrega-o ao job `discard` (que guarda o dono e as claims até terminar,
  inclusive em pânico por `DiscardGuard`). Uma recusa do ledger mantém o código `claims_unavailable` e `details.phase = "accept"`,
  com `details.discard_job`; obsolescência segue `stale_result`. Chamadores que podem bloquear (`apply`, `job.commit` da CLI)
  descartam no próprio fluxo e anexam `details.cleanup`.
* Para outro processo, o resultado em `queued`/`running` de descarte é um titular **vivo** (lease mantida): `claims` o lista sem
  `orphan`, `recover` não toca. Ao terminar, o job remove só o que nenhum outro resultado usa (um arquivo reaproveitado por
  outro processo permanece, listado em `shared`). `SIGKILL` com o descarte na fila deixa órfãos que só `recover` recolhe,
  depois da prova de conteúdo; um resultado **aceito** nunca é recolhido.

Testes: `crates/media/tests/job_discard_claims.rs` (10, dois deles com o host de uma janela em processo real re-executado),
`scene_claims_integrity.rs::a_settle_the_ledger_refuses_returns_the_adoption_still_owning_and_claiming_its_files`,
`scene_cleanup_ownership.rs` (`settle`/`apply`), `scene_import_cleanup.rs::try_apply_refuses_without_removing…`. Mutações que
reprovam: aceite ignorado no `settle` de adopt (2 testes), aceite ignorado no `try_apply` de import (2), código da recusa perdido
no host (3). Provas nativas repetidas no head integrado: `job-discard-proof.py` 39/39 e `scene-claims-proof.py` 23/23.
Limite: a recusa de aceite não é exercida pela janela nativa (só pelo host interativo, sem pixels).

### Fechamento do descarte (revisão independente da PR #6, achados 1–8)

Cada item corrige um achado da revisão [5800801032](https://github.com/journey-studios/tessel-editor/pull/6#pullrequestreview-5800801032).
**Executado** é o que o antecessor rodou, conforme o registro do coordenador (core, media 70, claims 12, EOF 2, desktop 5;
sem log por teste, e não reexecutado nesta entrega). **Escrito, não executado** é o que ainda não passou por Cargo: os testes de
F4 e tudo de BL112.3 abaixo, que aguardam os gates do coordenador.

| Item | Achado | Correção (onde) | Prova | Estado da prova |
|---|---|---|---|---|
| BL094.4 (F1) | `SIGKILL` no hash deixa arquivo sem claim | `Tracker::begin_leave` (`ledger.rs`) devolve `Last` com a entrada ainda gravada; `Created::remove`/`leave_dir` provam, removem e só depois `leave` (`drop_claim`) | `claims/tests.rs` (2: `begin_leave` mantém a claim e falha fechado com ledger sem escrita); `job_discard_claims.rs::killing_the_process_while_its_discard_hashes_a_large_file…` (peer real, 128 MiB, `SIGKILL` em `hashing`, `recover` recolhe) | executado (antecessor) |
| BL094.5 (F2) | `job.discard {wait:false}` + EOF encerra a CLI com o descarte em curso | `Drop` de host sem janela espera os jobs `discard` ativos (`wait_for_discards`); `tessel-media` solta o host **antes** de `process::exit` | `job_discard_exit.rs` (2, processo `tessel-media` real, 64 MiB: EOF e linha recusada com saída 1) | executado (antecessor) |
| BL094.6 (F3) | `job.discard` do job de uma cadeia de anexo antes do tick a deixa em `running` | `discarded_unreported` em `MediaHost`: `poll` emite uma vez um evento `failed` `job_discarded` (com `details.discard_job`) do job original | `scene_import_tests.rs::discarding_the_attach_job_before_the_tick_ends_the_chain…` (a cadeia termina `failed`, um novo anexo é aceito) | executado (antecessor) |
| BL094.7 (F4) | `result_state` derivado de `is_ok()` | `apply_pending` diz o que o projeto fez (`took`); um erro depois de aplicar leva `details.applied = true` | `async_host.rs` (preparo obsoleto → `discarded`, import anterior → `applied`); `job_commit_state.rs` (processo real: import aplicado, decode falha, `applied`, e `job.discard` → `job_applied`) | **escrito, não compilado nem executado** |
| BL094.8 (F5) | status e eventos escondem `kept`/recusa | `describe_discard` (`flows.rs`) e `Workflow::remember_refusal` (`workflow.rs`); a recusa do ledger não diz mais que a cena mudou (`host.rs`) | `flows.rs` e `workflow_tests.rs` (3) | executado (antecessor) |
| BL112.2 (F6) | `Drop` interativo podia remover na thread da UI numa corrida curta | `hand_over`/`defer_until_final` (`host.rs`): o resultado espera o job ficar final **numa thread própria** e é reivindicado ou solto lá | `host.rs` (2 testes de unidade: soltura fora da thread do chamador, um único job `discard`) | executado (antecessor) |
| BL112.4 (F7) | `finished_discards` clonava o registro inteiro a cada tick; `prune` sem chamador de produção | `JobRegistry::matching` clona só o que casa; o risco do `prune` fica registrado (abaixo) | `jobs.rs` (`matching_clones_only_the_jobs_it_keeps`; cancelado de verdade não guarda resultado) | executado (antecessor) |
| F8 | docs/contrato incorretos | esta página, F040, matriz e `backlog.md`; docstring de `adopt.rs` | leitura | n/a |

### Segunda revisão (R2, [5802069640](https://github.com/journey-studios/tessel-editor/pull/6#pullrequestreview-5802069640))

Revisão por leitura estática: "resolvido" abaixo é pela leitura e **nenhuma prova foi executada para este head**. Os testes novos
estão escritos e aguardam os gates (`fmt`, `check`, `clippy`, `test`, `build`, árvore do core, prova nativa, mutações).

| Item | Achado | Correção (onde) | Prova escrita (não executada) |
|---|---|---|---|
| N1 (BL112.3, BL112.5) | o tick olhava uma vez e recusava resultado válido por contenção do próprio processo, culpando "outro processo" | `Ledger::try_lock` espera `TICK_WAIT` = 20 ms (`ledger.rs`); mensagem neutra; seção "O lock do ledger no tick" acima | `claims/tests.rs` (3), `scene_claims_integrity.rs` (2), `job_discard_claims.rs` (`SIGSTOP`, teto 100 ms) |
| N2 (BL094.5) | o EOF esperava sem limite atrás de jobs que não são descarte | `wait_for_discards` chama `cancel_all` (pula `discard`) antes de esperar, só com descarte ativo (`host.rs`) | `job_discard.rs`: `end_of_input_cancels_what_is_not_a_discard_instead_of_waiting_behind_it`, `end_of_input_with_no_discard_active_cancels_nothing` (em processo: não há operação de produção que prenda um slot sem FFmpeg) |
| N3 (BL094.5) | asserção falha com descarte na fila travava a suíte | `impl Drop for Blockers` (cancela o token) em `job_discard.rs` e `job_discard_claims.rs` | `a_failed_assertion_while_the_slots_are_held_reports_instead_of_hanging_the_suite` (pânico controlado com `catch_unwind`, relógio de 30 s) e `blockers_that_are_dropped_instead_of_released_give_their_slots_back` |
| N4 | `jobs.md` dizia que a claim de tudo o que ficou segue retida até `recover` | os três casos descritos na seção do descarte, acima | leitura |
| N5 (BL094.8) | a barra dizia que o arquivo removido "permaneceu no disco" | `describe_discard` (`flows.rs`): "não foram concluídos", `arquivo(s) removido(s)` à parte, até 3 motivos | `flows.rs` (2) |
| N6 | BL111.2 ainda dizia "junção com a S1 não incorporada" | uma linha em `backlog.md`, com autorização do coordenador | leitura |
| N7 (BL094.10) | `job.commit` antes do tick deixava a cadeia em `running` (como F3) | `committed_unreported` em `MediaHost`: `poll` entrega uma vez o evento do job (`host.rs`) | `job_discard.rs` (2) e `scene_import_tests.rs::committing_the_attach_job_before_the_tick_still_moves_the_chain_on` |

Mutações a rodar quando houver slot de execução (nenhuma foi executada): `begin_leave` → `leave` em `remove`; `try_lock` com
`Duration::ZERO` (reprova o teste do holder de 2 ms) e com `LOCK_WAIT` (reprova os tetos de 100 ms); `took = is_ok()` em
`apply_pending`; remover o `drop(media)` do `tessel-media`; remover `cancel_all` de `wait_for_discards` (o teste de N2 passa a
estourar o relógio de 30 s); remover o `Drop` de `Blockers` (o de N3 passa a estourar o relógio); remover o `push` de
`committed_unreported` (os de N7 reprovam).

**Por que o teste de F4 usa um processo.** O decode que segue um `job.commit` descobre o FFmpeg por conta própria
(`Tools::discover()`, não as ferramentas do host) e o crate proíbe `unsafe` (então o teste não altera o ambiente do próprio
processo). `job_commit_state.rs` roda o `tessel-media` real com `TESSEL_FFMPEG` apontando a um wrapper que funciona até um arquivo
existir: o job de import roda de verdade e o commit encontra o FFmpeg indisponível. A versão em processo (com `with_tools`) não
forçava a falha e foi retirada.

**`prune` (BL112.4, risco latente).** Nada em produção chama `JobRegistry::prune`; os testes mostram que ele preserva
`pending`/`discarding`. Se alguém o ligar, um job `discard` já final pode ser removido **antes** de `poll` reportá-lo
(`reported_discards`/`finished_discards` precisam dele no registro). Registrar a decisão antes de ligar; hoje o registro cresce
na sessão, como já crescia.

**O lock do ledger no tick (BL112.3, R2/N1).** `accept` roda na thread da janela e escreve `claims.json` sob `ledger.lock`.
A primeira correção limitou a espera a 1,5 s (`LOCK_WAIT`), que **excede o teto de 100 ms** do tick. A segunda fez o tick
olhar uma vez (`Duration::ZERO`), mas `flock` conflita entre descritores **do mesmo processo** (cada `Tracker` abre o seu): um
job `discard` (duas gravações por arquivo), uma adoção ou um `recover` do outro slot segurando o ledger por microssegundos
recusava um resultado **válido**, o descarte removia a cópia e a mensagem culpava "outro processo". Agora o tick espera um
orçamento **fixo de 20 ms** (`TICK_WAIT`, olhando a cada `LOCK_POLL` de 500 µs; quatro vezes abaixo do teto): contenção de
microssegundos de qualquer origem é absorvida e só um holder que não solta (processo parado, travado) esgota o orçamento. Então
o ledger fica indisponível só para essa chamada (`fault`: "held right now by a holder that did not let go of it; gave up after
20ms", sem apontar quem) e o aceite é **recusado** como qualquer outra falha do ledger: projeto, undo e revisão intactos, o
resultado continua dono e vai para um job `discard` (`claims_unavailable`, `details.phase="accept"`, `details.why`). Quem pode
bloquear (job de descarte, `apply`/`job.commit` da CLI, `recover`, `claims`) segue com a espera limitada de 1,5 s. O que o
tick **ainda** faz na thread da UI: ler e reescrever `claims.json` (`O(entradas)`, sem `fsync`; a prova nativa mede só a latência
de pedidos, p99 1,5–16 ms) e, apenas numa corrida com o processo que esvazia o ledger, as repetições da abertura (pausas de
1–2 ms, no máximo 64 tentativas; com as entradas do próprio resultado o ledger não fica vazio, então não ocorre na prática, mas
**não foi medido**).

* **Disponibilidade, honestamente (BL112.5, pendente):** o orçamento de 20 ms cobre a contenção curta. Um holder que segura o
  lock além disso (outro editor parado, um ledger enorme) ainda recusa um resultado válido e **a cópia inteira é removida**:
  "nada se perde" não vale para o trabalho do usuário, só para a integridade (o descarte devolve apenas o que é nosso). A
  alternativa, manter o resultado `pending` e repetir o aceite no tick seguinte até um prazo, evita a recusa mas muda a máquina
  de estados do `poll`; fica registrada, sem implementação.
* **Medição pendente (BL112.3):** o custo de ler/reescrever `claims.json` no tick com um ledger de 1 mil e de 10 mil entradas
  **não foi medido**. Se passar de 100 ms é bloqueio genuíno e o aceite precisa sair da thread da UI (muda a arquitetura); hoje
  não há evidência disso.
* **Prova (escrita, não executada):** em `claims/tests.rs`: `the_tick_budget_is_fixed_and_well_below_the_ui_ceiling`
  (orçamento entre 10 e 25 ms e ≤ ¼ do teto), `a_tick_that_finds_the_lock_held_for_a_moment_by_this_process_gets_it` (holder de
  2 ms em outra thread, solto só depois que o tick começou a olhar: olhar uma vez recusa) e
  `a_tick_that_finds_the_lock_stuck_gives_up_inside_its_budget_with_a_neutral_reason` (≥ 20 ms e < 100 ms, sem culpar "outro
  processo"); em `scene_claims_integrity.rs`: `a_settle_rides_out_a_ledger_lock_another_job_of_this_process_holds_for_a_moment`
  e `a_settle_gives_up_inside_the_tick_budget_when_the_ledger_lock_stays_held`; e em `job_discard_claims.rs`:
  `a_peer_stopped_while_holding_the_ledger_lock_cannot_freeze_the_tick` (peer real em `SIGSTOP`; o tick inteiro < 100 ms,
  `why` "gave up after"). Os tetos de 1 s, que aceitavam dez vezes o limite, foram trocados por 100 ms; em máquina muito
  carregada eles podem falhar sem regressão (repetir antes de concluir). Mutações a rodar: `try_lock` com `Duration::ZERO`
  reprova o teste do holder de 2 ms; `try_lock` com `LOCK_WAIT` reprova os de teto de 100 ms. A espera limitada de quem pode
  bloquear segue coberta por `waiting_for_a_lock_nobody_lets_go_of_is_bounded` (executado pelo antecessor).

Todo trabalho escreve num `Staging` (pasta no mesmo volume do destino) e **publica por rename** só depois de
verificar; falha ou cancelamento removem o staging e preservam o destino anterior e os originais.

## Limites

Sem retomada: um export/pacote interrompido reinicia. `job.discard`: o ownership entre processos é o ledger de claims (BL111, S2).
Um descarte interrompido (inclusive `SIGKILL`) **no meio do hash/remoção** deixa a claim do último titular gravada (BL094.4: a
entrada só sai depois do arquivo), então `scene.resources.recover` recolhe o arquivo depois que a lease do processo morto some,
sem tocar o que foi aceito. Isso vale **só depois** da correção desta PR: a redação anterior ("deixa os arquivos como órfãos")
descrevia uma ordem em que a entrada saía antes do hash e era falsa para esse intervalo. O fim de entrada (EOF) de uma CLI
espera os descartes ativos (BL094.5) e, **antes de esperar**, cancela os jobs que não são descarte (N2: eles ocupam os 2 slots atrás
dos quais o descarte espera e o resultado deles seria jogado fora na saída; sem descarte ativo nada é cancelado); o encerramento da **janela** não espera (BL112.6, pendente): o job morre com o processo e a
claim fica para `recover`.
Um job `discard` espera um slot (máx. 2 jobs por host); ele fica `queued` e o resultado `discarding` enquanto isso. A prova de «a UI não trava» é por tempo de resposta do
`import` (job devolvido em < 400 ms com FFmpeg artificialmente lento, `async_host.rs`) e pelo teste do desktop que aplica o
import no tick; não há medição de quadros por segundo da janela durante o job. Ver o
[relatório](../media-project-export-report.md).

Integração S1/S5 após main6f7a944: resultados SceneAttach têm rótulo próprio e podem ser
descartados sem I/O sobre o bundle original, sem alterar cenas/undo/revisão. A guarda de
sessão do attach e a devolução de ownership dos resultados de import/adopt foram preservadas.
Regressão nova scene_attach_job passou; suítes de descarte/claims/attach/restore, clippy
workspace/build/fmt passaram. Provas nativas no código integrado: import59/59, descarte39/39.
