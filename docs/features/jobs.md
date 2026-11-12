# F025 — Jobs, cancelamento e staging (P03)

## Status

Lib **V**, `scripts/media`/`scripts/render` **V**, UI **V** (diálogo Tarefas, linha de progresso na barra de status,
cancelar; `flow.jobs`). `job.discard` (BL094/BL112): lib/host **V** (testes abaixo), CLI headless **V** (processo JSONL real
`tessel-media`, 20 checks), janela **V no escopo de jobs** (instância própria, só requisições semânticas, 19 checks na
medição original e 21 no gate final, 41 no total: o tick entrega a remoção a um job e a janela segue respondendo). Dois processos reais (BL111, S2): ver "Junção com claims persistentes". Sem pixels (jobs não desenham na viewport) e sem ponteiro. Ver "Evidência por superfície" abaixo.

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

`job.wait` só espera o término e não aplica o resultado; `wait:true` (o padrão) aplica na chamada original somente no host não interativo.

Um job que produz resultado para o projeto o **retém** (`ctx.hold_result()`): `job.get`/`job.list` trazem
`result_state` = `none | pending | discarding | discarded | applied`.

```text
none ─(corpo guarda resultado)─> pending ──apply (poll/commit/wait:true)──> applied
                                    │  └─ aplicar recusado (stale) ──> discarded
                                    └─ job.discard ──> discarding ──(job discard terminou)──> discarded
```

`job.discard {job, wait?}` reivindica o resultado **atomicamente** (`JobRegistry::begin_discard`): de dois chamadores
só um ganha, e um `job.commit`/`poll` concorrente não leva mais o resultado. O resultado **move** para um job
`discard` (tipo `discard`, **não cancelável**: `job.cancel` responde `job_not_cancellable`), que pode remover arquivos
criados pelo resultado ou reutilizados de outro resultado pendente, se for o último titular (claims, SHA-256 e lease de F040) e informa o que ficou (`kept`/`shared`). O dono (o resultado, com
seus claims) e o registro do job **permanecem até o fim do job discard**: `prune` nunca descarta um job
`pending`/`discarding`. Resposta: `{action:"job.discard", job:<job discard>, discards:<job original>, state, ...}`;
host sem janela espera e devolve também `discarded` e `cleanup:{removedFiles,removedFolders,kept,shared,busyRoots}`
(`cleanup` é `null` para resultados sem arquivos, como `SceneAttach`; `wait:false` devolve `state:"discarding"` na hora); o host interativo responde sempre na hora e `poll()` entrega um
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
  aceite é `Ledger::accept`: sob `flock`, **no tick só espera 20 ms** (BL112.3) e, num ledger grande, **acrescenta um registro a um diário em vez de reescrever `claims.json`**, então não depende do tamanho do ledger (BL112.7); ver "Fechamento do descarte").
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
sem log por teste, e não reexecutado nesta entrega). **Escrito, não executado** era o que ainda não passou por Cargo; os gates finais da PR 6 (head `9bb4d18`) executaram tudo, inclusive F4 e BL112.3: ver o resumo em `docs/backlog.md` ("PR6 — gates finais e medições").

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

Mutações **executadas nos gates finais da PR 6** (head `9bb4d18`, cada uma revertida, logs `mut-*.log` em `/tmp/tessel-pr6-final-gates`): `begin_leave` → `leave` em `remove` (morta por `job_discard_claims`); `try_lock` com `Duration::ZERO` (morta pelos testes do holder de 2 ms, em `claims/tests.rs` e `scene_claims_integrity.rs`) e com `LOCK_WAIT` (morta pelos tetos de 100 ms, incluindo o `SIGSTOP`); `took = is_ok()` (morta por `job_commit_state` e `async_host`); remover o `drop(media)` do `tessel-media` (morta por `job_discard_exit`); remover `cancel_all` de `wait_for_discards` (morta pelo teste de N2, por tempo); remover o `Drop` de `Blockers` (morta nas duas suítes, por tempo); remover o `push` de `committed_unreported` (morta pelos dois testes de `commit_before_the_tick` em `job_discard.rs`). Não foi executada contra esta mutação a prova de cadeia da janela (`scene_import_tests.rs::committing_the_attach_job_before_the_tick_still_moves_the_chain_on`).

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
job `discard` (duas gravações por arquivo), uma adoção ou um `recover` do outro slot segurando o ledger durante uma atualização
recusava um resultado **válido**, o descarte removia a cópia e a mensagem culpava "outro processo". Agora o tick espera um
orçamento **fixo de 20 ms** (`TICK_WAIT`, olhando a cada `LOCK_POLL` de 500 µs; bem abaixo do teto): contenção
curta pode ser absorvida, mas um holder longo saudável também pode esgotar o orçamento: `recover` mantém um único lock enquanto cada `forget` reescreve o ledger inteiro. Isso pode ocorrer com muitos órfãos, além de processos parados ou travados; a duração foi medida nos gates finais (100 órfãos: 102,6–105,6 ms no total; ver abaixo). Então
o ledger fica indisponível só para essa chamada (`fault`: "held right now by a holder that did not let go of it; gave up after
20ms", sem apontar quem) e o aceite é **recusado** como qualquer outra falha do ledger: projeto, undo e revisão intactos, o
resultado continua dono e vai para um job `discard` (`claims_unavailable`, `details.phase="accept"`, `details.why`). Quem pode
bloquear (job de descarte, `apply`/`job.commit` da CLI, `recover`, `claims`) segue com a espera limitada de 1,5 s. O que o
tick faz na thread da UI: tomar o lock (20 ms no máximo) e, num ledger pequeno, reescrever `claims.json` (poucos ms) ou, num ledger grande, acrescentar um registro ao diário (`O(chaves do resultado)`: 0,27–0,33 ms com 10 mil entradas em debug, BL112.7; antes 304–310 ms) e, apenas numa corrida com o processo que esvazia o ledger, as repetições da abertura (pausas de
1–2 ms, no máximo 64 tentativas; com as entradas do próprio resultado o ledger não fica vazio, então não ocorre na prática, mas
**não foi medido em corrida real**).

* **Disponibilidade, honestamente (BL112.5, pendente):** o orçamento de 20 ms cobre a contenção curta. Um holder que segura o
  lock além disso (outro editor parado, um ledger enorme) ainda recusa um resultado válido e **a cópia inteira é removida**:
  "nada se perde" não vale para o trabalho do usuário, só para a integridade (o descarte devolve apenas o que é nosso). A
  alternativa, manter o resultado `pending` e repetir o aceite no tick seguinte até um prazo, evita a recusa mas muda a máquina
  de estados do `poll`; fica registrada, sem implementação.
* **Aceite do tick por diário (BL112.7, resolvido; a validar até a revisão).** O custo do aceite deixou de depender do tamanho do ledger. `Ledger::accept` (um só caminho, tick e bloqueantes) toma `ledger.lock` (20 ms no tick, 1,5 s nos demais) e, se `claims.json` + `claims.journal` passam de 64 KiB (`INLINE_LIMIT`), **acrescenta uma linha** ao diário sem ler nem parsear o estado; todo leitor aplica o diário ao abrir e o compacta (estado reescrito sem as chaves, diário removido) **antes de responder** a quem o abriu, fora da UI. Projeto, undo e revisão voltam ao que eram quando o acréscimo falha (último passo da transação), e um registro cujo escritor morreu no meio (sem `\n` final) nunca foi um aceite. Contrato, alternativas descartadas (aceite assíncrono/duas fases, reserva reversível), invariantes e limites: [F040, "Diário de aceites (BL112.7)"](scene-resource-integration.md#diário-de-aceites-bl1127). **Medido** (build debug, `MediaHost::poll` interativo real, ledger semeado uma vez e verificado por um 3º processo, 3 execuções × 3 réplicas, head `dd36cc8`, `/tmp/tessel-bl1127-gates`): controle vazio 0,37–0,46 ms; **1 mil entradas 0,16–0,23 ms; 10 mil entradas (4,37 MB) 0,27–0,33 ms, contra o teto de 100 ms e contra os 304–310 ms de antes (~900× menos)**; cada tick deixa um registro de 490–595 bytes e `claims.json` byte a byte igual. **Detectores independentes** em `scene_claims_budget.rs` (agora executado, não `#[ignore]`): o tempo e a conferência estrutural (estado intacto + um registro); a mutação `INLINE_LIMIT = u64::MAX` reprova os dois. Provas: `claims/tests.rs` (10 novos: acréscimo sem reescrita, compactação idempotente, caminho reivindicado de novo não é aceito pelo registro antigo, registro incompleto, diário truncado sozinho, linha completa que não parseia é dano, falha de acréscimo é `Err` e as entradas continuam, o tick não espera um holder preso num ledger grande, ledger pequeno continua inline e esvaziado desaparece, threads concorrentes) e `scene_claims_journal.rs` (10: host real, adoção e import com um registro e o projeto/undo como numa reescrita, resultado obsoleto e descartado sem registro, diário que não grava recusa com `claims_unavailable`/`phase="accept"` e nada muda, `job.commit` bloqueante da CLI com o mesmo resultado e o mesmo código, e **dois processos reais**: um leitor martelando o ledger durante o tick e `SIGKILL` do processo da janela com o diário ainda não compactado: `recover` de outro processo remove 0 arquivos e o diário é compactado). Mutações executadas e revertidas: `INLINE_LIMIT = u64::MAX` (o aceite pesado volta ao tick: morta por `scene_claims_budget` pelos dois detectores, 2 falhas, por `scene_claims_journal`, 4, e pelo teste de unidade, que agora falha em vez de preencher o disco), diário não lido (`replay_journal` fora: `scene_claims_journal` 7 falhas, `claims/tests.rs` 7), diário não removido na compactação (2 e 4), cauda truncada não aparada (1), linha danificada ignorada (1), acréscimo omitido (`journal` 5, unidade 4), tick com `LOCK_WAIT` em `Ledger::accept` (unidade, `scene_claims_integrity` e o peer em `SIGSTOP`: 1 cada), e a conferência de `fault` do `recover` removida (BL112.8, 1).
* **Fechamento da revisão independente do diário (BL112.7, F1–F5, a validar).** Só testes e provas; o protocolo foi revisado e não mudou. Novos: ver "Provas do diário no fechamento" em [F040](scene-resource-integration.md#diário-de-aceites-bl1127). **Mutações novas** (cada uma revertida, `/tmp/tessel-pr6-f15-gates`, head de código `90e27ad`), para mostrar que os testes novos discriminam: o replay que só entrega entrada de um titular (morta por 2 testes novos), o replay desligado (16 testes de unidade, 3 do import), a compactação que não remove o diário (6, incluindo o teste do `chflags`), o primeiro acréscimo da execução que falha (o teste **antigo** de `SIGKILL` do head `44c3de4` **sobrevive**, o novo morre: era a fraqueza F2) e o rollback da transação do projeto omitido (morta pelo teste de import). **Gates no head final** `80281a1` (`main` `c55467f`, PR15 e PR16 integradas): fmt, check, clippy sem avisos; workspace `--no-fail-fast --locked` com `TESSEL_REQUIRE_RUNTIME=1`: **1391 passam, 0 falham, 12 ignorados com motivo, 87 binários** (a 1ª execução terminou com SIGTERM em 79 de 87 binários, 1337/0, sem timeout nem disco baixo do driver e com origem não identificada: não conta, foi repetida inteira). Provas nativas na árvore reconstruída e limpa: `job-discard-proof.py` 39 checks (janela ociosa p99 38,5 ms; durante o descarte de 128 MiB 130 respostas, p50 0,8 / p99 13,0 / máx 15,9 ms) e `scene-claims-proof.py` 23 checks, com `tessel-editor` `8e9d12bd4dff5e6f1bfd50a53f14e6b1729d78dfd9ea4f8c95810ed41f782015` e `tessel-media` `410961bddc45e62f86bdb9d79c4bc0fb472f692f676b556c0427c434d85280bb` (iguais antes e depois, iguais ao build limpo). **F1:** o `tessel-media` que as provas anteriores usaram podia ser o de uma mutação (`LOCK_WAIT`, mtime 22:06:22). Agora as mutações rodam **antes** de tudo, a árvore é conferida limpa, e as provas só rodam depois de um `cargo build` limpo; o driver registra o hash do binário que estava no disco antes desse build (um binário de `cargo test --workspace` não é o de `cargo build`: `8400619…` contra `3c576c2…` no head `90e27ad`, `4aff6e8…` contra `410961b…` no final) e recusa a prova se o binário mudar durante ela ou não for o do build limpo. Não repetidas: as 8 mutações de 15 execuções e a medição do tick (0,27–0,33 ms com 10 mil entradas) da rodada anterior.
* **Medido antes do diário (histórico, preservado; BL112.3, gates finais da PR 6, head `9bb4d18`, build debug, 3 réplicas, `/tmp/tessel-pr6-final-gates`):** o tick lê e reescreve `claims.json` inteiro, `O(entradas)`. Ledger vazio (controle) 0,53–0,55 ms; **1 mil entradas 28,9–31,0 ms; 10 mil entradas 304–310 ms, acima do teto de 100 ms** (`scene_claims_budget.rs`, `--ignored`: ledger semeado uma vez e verificado por um segundo processo, host real; leitura+parse 112 ms, JSON 70 ms, serializar 113 ms). É o bloqueio genuíno previsto: **BL112.7**, decisão de arquitetura (o aceite é o último passo da transação do projeto), **não corrigido naquele head** e sem teto relaxado; **resolvido depois pelo diário de aceites** (próximo item). `recover` saudável com 100 órfãos segura o lock 102,6–105,6 ms; durante o hold o tick fica ≤ 20,4 ms, mas 12 de 19 ticks são recusados (BL112.5); com 10 órfãos o hold é de 3,0–3,3 ms e nada é recusado.
* **Prova (executada nos gates finais):** em `claims/tests.rs`: `the_tick_budget_is_fixed_and_well_below_the_ui_ceiling`
  (orçamento entre 10 e 25 ms e ≤ ¼ do teto), `a_tick_that_finds_the_lock_held_for_a_moment_by_this_process_gets_it` (holder de
  2 ms em outra thread, solto só depois que o tick começou a olhar: olhar uma vez recusa) e
  `a_tick_that_finds_the_lock_stuck_gives_up_inside_its_budget_with_a_neutral_reason` (≥ 20 ms e < 100 ms, sem culpar "outro
  processo"); em `scene_claims_integrity.rs`: `a_settle_rides_out_a_ledger_lock_another_job_of_this_process_holds_for_a_moment`
  e `a_settle_gives_up_inside_the_tick_budget_when_the_ledger_lock_stays_held`; e em `job_discard_claims.rs`:
  `a_peer_stopped_while_holding_the_ledger_lock_cannot_freeze_the_tick` (peer real em `SIGSTOP`; o tick inteiro < 100 ms,
  `why` "gave up after"). Os tetos de 1 s, que aceitavam dez vezes o limite, foram trocados por 100 ms; em máquina muito
  carregada eles podem falhar sem regressão (repetir antes de concluir). Mutações executadas (ver acima): `try_lock` com `Duration::ZERO`
  reprova o teste do holder de 2 ms; `try_lock` com `LOCK_WAIT` reprova os de teto de 100 ms. A espera limitada de quem pode
  bloquear segue coberta por `waiting_for_a_lock_nobody_lets_go_of_is_bounded` (executado pelo antecessor).

### Fechamento do gate final da PR #6 (BL094.12, BL112.10–BL112.12; seis threads abertas, a validar)

Seis threads ficaram abertas depois das revisões anteriores. Todas foram verificadas contra o código do head `1221ddd` antes
de qualquer mudança: cinco achados eram válidos e foram corrigidos (cada um com regressão que a mutação mata) e um era
documentação parcialmente já fechada. Nada do contrato do diário (BL112.7), da posse do job até o descarte, do rollback
transacional, do orçamento de 20 ms do tick nem do tick de 10 mil entradas (< 100 ms) mudou; não houve mudança de escopo
(motor completo/S10).

| Thread | Veredito | Fechamento | Prova |
|---|---|---|---|
| CodeRabbit `4497296574` (`cleanup.rs:597`) | válido | falha de lease por **qualquer** motivo mantém arquivo e claim e diz o motivo (BL112.10); só raiz inexistente é pulada | `a_root_whose_lease_fails_for_another_reason_keeps_its_files_claims_and_says_why` (`scene_import_cleanup`) e `recover::…a_root_whose_lease_cannot_be_taken_for_another_reason…`; mutação `Err(_) => {}` mata os dois |
| CodeRabbit `4497296585` (docs 304–310) | parcialmente já fechado em `44c3de4`; **sobra** corrigida | `jobs.md`, BL112.7 e a matriz já separavam o histórico do atual. Sobravam: o estado de BL112.3 ("bloqueio medido") e a seção "Problema" de F040 em tempo presente; agora dizem que 304–310 ms é o código **anterior ao diário** (head `9bb4d18`) e o atual é 0,27–0,33 ms (head `dd36cc8`) | leitura; o teto de 100 ms não é declarado atingido por texto: a medição é a de `scene_claims_budget.rs` (executada, não ignorada) |
| OpenCodeReview `4498843850` (`scene_adopt_host.rs:361`) | válido (teste) | `World::unread` + `take_event`: o lote de um `poll` nunca perde o evento que ninguém pediu ainda (BL094.12) | `an_event_in_the_same_batch_is_kept_for_the_next_ask_not_dropped` (lote nas duas ordens, determinístico); mutação (voltar ao `find`) mata |
| OpenCodeReview `4498843854` (`recover.rs:169`) | válido | a guarda de BL112.8 só com `handled` não vazio; cancelamento mantém `cancelled` e `undecidedFiles` com a falha do ledger ao lado em `details.claimsFault` e em `cleanup.kept` (BL112.11) | quatro testes dirigidos em `recover.rs` (antes do 1º, depois de parcial, raiz ocupada, `forget`/lock preso existentes); duas mutações mortas |
| OpenCodeReview `4498843858` (`job-discard-proof.py:424`) | válido (média) | latência classificada pelo **instante de envio** dentro do intervalo `[último envio pré-descarte, 1º recebimento de `discarded`]`; controle `SIGSTOP` pelo mesmo classificador (BL112.12) | `scripts/job-discard-proof.spec.py` (13); o filtro antigo **aprova** a mesma amostra lenta (falso PASS demonstrado) e a nova a reprova |
| OpenCodeReview `4498843863` (`job-discard-proof.py:156`) | válido | cada sessão JSONL fecha seu próprio filho (EOF, `kill` só dele) antes de a fase retornar; `Probe` parado no `finally` | `check` forçado: 0 filhos vivos, `eof` recebido, workspace intacto; mutações sem `ExitStack` e sem `kill` mortas |

**Contrato de `recover` quando os dois erros coexistem (BL112.11).** O cancelamento do usuário continua `cancelled`, com
`undecidedFiles` e o relatório parcial; a falha do ledger ao registrar os órfãos já tratados não substitui o código, vai em
`details.claimsFault` e numa linha de `cleanup.kept` (`could not record that N orphan file(s) were handled`). Sem cancelamento a
falha continua `claims_unavailable` com o relatório parcial, como em BL112.8. Escolha registrada: quem lê só `error.code` vê
`cancelled` e precisa ler `claimsFault`/`kept` para saber que o ledger também falhou.

**Prova nativa refeita (head de código `683a511`, build limpo depois das mutações, sob o lock de GPU).** `job-discard-proof.py`
**41/41** checks (os 39 anteriores mais 2 novos: o intervalo do descarte é traçável e o controle `SIGSTOP` reprova o
classificador; JSONL 20 + janela 21; **não** foi forçado a 39). Ocioso p99 34,6 ms (60 respostas); controle por pedido único
1532,6 ms; controle pelo classificador: n = 2, p99 = máx = 1500,1 ms, reprova p99 e máx como deve; **durante o descarte:
523 respostas em 14,12 s, p50 1,1 / p99 14,8 / máx 17,0 ms** (tetos p99 ≤ 100 ms e máx ≤ 250 ms, inalterados) e, pelo
filtro antigo, 522 (a diferença é uma resposta de fronteira); `during_discard_answered_after_the_end` 0. O JSON
(`docs/migration/s1-job-discard-proof-final-gate.json`) leva as duas medições, o intervalo e a base. `scene-claims-proof.py` 23/23.
O intervalo de 14 s (4,4 s nas medições anteriores) reflete a máquina compartilhada muito carregada (load 24–130), não uma
mudança de teto; as latências ficaram iguais. Os binários, o `runtime/dist` e os scripts tiveram o mesmo SHA-256 antes e depois
de cada prova (sidecars em `/tmp/pr6fix/native`, não versionados; logs de workspace, clippy e build em `/tmp/pr6fix/gates`: `test.log`, `summary` e `head`, também não versionados).

**Limites mantidos.** A prova nativa continua com ledger pequeno (o aceite grande é provado no host real e em dois processos);
a falha de lease não foi provada com um adotante concorrente real (é persistente e dirigida); a corrida do descarte terminar no
mesmo tick da recusa depende de tempo e foi coberta pela estrutura do lote, não provocada; BL112.5 (recusa por holder longo),
BL112.6 e BL112.1 seguem como estavam; sem release, pixels, Windows ou NFS.

**Saída das mutações Rust, honestamente.** As mutações Rust desse gate (lease, guarda de `recover`, falha do ledger, lote do `poll`)
foram relatadas pelo autor e os scripts que as aplicam (`/tmp/pr6fix/mut.py`, `mut2.py`) só imprimem: **a saída não foi retida**. Não
há log para reler; o que existe é o relato acima, o código dos testes que as matam e a releitura da revisão independente. As mutações
**novas** deste fechamento (abaixo) são testes do próprio spec e deixam log (`/tmp/pr6n-logs/native/job-discard-spec*.log`). Os
logs de workspace/clippy/build do gate ficaram em `/tmp/pr6fix/gates` (`test.log`, `summary`, `head`), não versionados.

### Preparação automática depois de `job.commit` antes do tick (BL094.11; a validar)

**Defeito (preexistente, também em `main`).** O host marca a preparação automática em andamento (`auto_prepare` e `auto_proxy`,
com os ativos que ela cobre em `auto_assets` e `proxy_assets`), e `schedule_prepare`, `schedule_proxies` e `schedule_thumbnails`
esperam a marca sumir antes de agendar mais trabalho. Quem a limpava era o `poll`, ao ver o job terminar. Mas `job.commit` tira
o job de `pending` e aplica o resultado antes de o `poll` olhar, então o `poll` nunca via o fim: a marca ficava para sempre e a
janela seguia em `media_preparing`, sem preparar mídia nova nem miniaturas, até reiniciar. Só acontece por automação
(`ui op job.commit` ou JSONL interativo) na janela de até 250 ms entre dois ticks, como BL094.6 e BL094.10. `job.discard` já
limpava as marcas; o `poll` também.

**Correção (só `crates/media/src/host.rs`).** Um ponto só, `MediaHost::release_automation_job`, esquece o job como o automático
e acerta o que ele preparava; `poll`, `job_commit` e `job_discard` chamam o mesmo. Um resultado que **não aplicou** deixa os
ativos em `auto_failed`/`proxy_failed`, como o `poll` já fazia (não são tentados de novo até o arquivo mudar, para um vídeo
corrompido não reiniciar um job a cada tick); um resultado aplicado ou descartado não marca falha (descartar é abrir mão, e o
próximo tick pode preparar a mesma mídia). Um `job.commit` recusado antes de tirar o job de `pending` (`job_running`,
`job_discarding`, `job_discarded`, job que falhou) não muda nada: o `poll` ainda verá o job.

**Prova (2026-11-12, base `d5d641d` mais este delta, FFmpeg real, um gate pesado por vez).** Três testes novos, todos verdes:

* `host.rs` (unidade, 2): `forgetting_an_automatic_job_clears_its_marker_and_leaves_the_other_alone` e
  `only_a_result_that_did_not_apply_keeps_its_assets_from_being_retried`.
* `async_host.rs::committing_the_automatic_prepare_job_before_the_tick_does_not_stall_the_next_one` (host interativo): importa um
  vídeo, espera o preparo automático terminar sem tick (`job.get`, porque o host interativo não espera), faz `job.commit`, confere
  que o `poll` entrega o fim do job uma vez e que um **segundo** vídeo, de bytes diferentes, é preparado sozinho; a biblioteca
  fica com 2 itens, ambos decodificados.
* `workflow_tests.rs::committing_the_automatic_prepare_job_before_the_tick_leaves_the_window_free_to_prepare_more` (GPUI): o
  mesmo cenário pelo tick da janela.

**Mutações** (cada uma revertida; `host.rs` voltou byte a byte, mesmo sha256): tirar a chamada de `job_commit` reprova o teste do
host e o da janela com `the second import started its own automatic prepare job`; fazer `release_automation_job` ignorar a falha
reprova o teste de unidade (`auto_failed` vazio em vez do ativo).

**Prova mais larga (mesma base, porque `host.rs` é usado pela maioria das suítes).** `cargo fmt --check` e `clippy -D warnings`
(mídia e desktop, todos os alvos) limpos; crate de mídia inteiro: 499 passam, 6 ignorados e **1 falha de ambiente que já existe
em `main`** (`export.rs::a_full_disk_ends_the_export_with_disk_full_and_leaves_the_volume_clean`: o `hdiutil attach` é negado
nesta máquina, BL082.47); crate desktop inteiro: 509/509. **Não** foram repetidos o workspace inteiro nem a prova nativa da
janela real.

### Requisição que falha durante o descarte (BL112.12, CodeRabbit `4499357511`, revisão Sonnet `5809214591` N1/N2; a validar)

**Defeito.** `Probe._run` capturava só `OSError` e descartava a requisição: nada era registrado e uma resposta que não é JSON
(`ValueError`) **matava a thread da sonda em silêncio**. Com 20 respostas rápidas e uma requisição enviada durante o descarte que
falhou, o veredito passava.

**Contrato novo (só `scripts/job-discard-proof.py`; Rust, runtime e tetos 100/250 ms e 20 amostras inalterados).**

* `Probe` registra **toda** requisição, na ordem em que terminou: `Sample(sent, received, latência, estado)` para respostas e
  `Failure(sent, failed, latência, causa, erro)` para o resto. Causas: `timeout`, `network` (outros `OSError`), `protocol` (resposta que
  não é JSON), `unexpected` (qualquer outra exceção), `rejected` (`ok:false`, também é amostra cronometrada) e `unfinished`
  (requisição ainda no ar quando a sonda parou; `failed` nulo). A thread **continua** perguntando depois de uma falha.
* Uma falha é julgada pelo **instante de envio**, como uma resposta tardia, nunca pelo estado de uma resposta (não há). Se qualquer
  requisição **enviada dentro da janela** do descarte falhou, `judge_discard` reprova (`no request sent while it removed failed`),
  qualquer que seja o número de respostas rápidas e os tetos. Falhas fora da janela são contadas e relatadas (`failed_outside_window`,
  `probe.failures`), **não** julgadas. Sem resposta `discarded` o fim do descarte é desconhecido: a janela vai até o último instante
  registrado, resposta ou falha.
* Uma thread ainda viva depois de `PROBE_JOIN_S` **não é escondida**: `thread_alive_after_stop`, falha `unfinished` e o check
  `the probe thread stopped` reprova. Resultado tardio depois do veredito não altera o registro.
* O relatório (`window_results.probe`, `.discard_verdict`) leva contagens, causas, e para cada falha os instantes de envio e falha
  (segundos desde o início da janela) e o texto do erro; só números finitos (`null` no lugar de NaN/infinito).
  O mesmo `judge_discard` é chamado pela fase da janela e pelo spec após os checks anteriores. Se um check anterior reprovar,
  o veredito não é calculado; somente o relatório bruto da sonda é preservado pelo `finally`.

**Prova.** `scripts/job-discard-proof.spec.py` agora tem **27** testes (13 anteriores preservados + 14). Dirigem a `Probe` **real** com um
`lab` falso que levanta `OSError`, `JSONDecodeError`, `socket.timeout`, devolve `ok:false` ou nunca responde, e julgam com o
`judge_discard` **real**: 20+ respostas rápidas e **uma** falha na janela reprovam **no check das falhas** (não por outro motivo);
a mesma amostra sem falha passa; falha antes do início ou depois do fim da janela passa e fica registrada; falha depois da última
resposta sem `discarded` reprova; JSON do relatório sem NaN/infinito. **Mutantes no código-fonte** (cada um deve deixar de reprovar):
registro removido, guarda removida, `except OSError` no lugar de `Exception` (thread morre), janela que ignora falhas, thread viva ignorada.
Os tetos são zerados só dentro desses cenários (ruído de escalonamento não decide uma prova sobre falhas); um teste confere que os
valores fixados (100/100/250/20) estão intactos.

**N2 (spec do filho que ignora EOF).** O teste dormia 0,2 s esperando o filho falso gravar o PID; sob carga o filho morria antes de
iniciar (`IndexError`). Agora espera, com teto de 60 s, o arquivo de PID existir antes de sair da sessão. Prova sob carga limitada,
sem soak: 24 execuções **em paralelo** do teste (load ~17) 24/24; a suíte inteira 8 em paralelo 8/8 (27 testes cada).

**Prova nativa refeita** (head `ffc71e9` + este delta, binários **novos** construídos neste worktree: nenhum binário anterior existia
(`target` do worktree-fonte era um link quebrado) e o código Rust é idêntico ao de `683a511` — `git diff 683a511 ffc71e9 -- crates
Cargo.toml Cargo.lock runtime` vazio —; sob o lock de GPU, entrada 34,1 GiB, piso 12): `job-discard-proof.py` **43/43** (os 41 de antes
mais `the probe thread stopped` e `no request sent while it removed failed`), 0 falhas, 759 requisições, 0 falhas de requisição.
Ocioso p99 37,1 ms; controle por pedido 1509,9 ms; controle pelo classificador n = 3, p99 = máx = 1489,1 ms (reprova); **durante o
descarte 322 respostas em 8,7 s, p50 1,2 / p99 16,0 / máx 17,9 ms** (tetos inalterados). Esta máquina estava com load médio ~17; a
janela é mais curta que a da rodada anterior (14 s) e uma resposta cruzou o fim (`during_discard_answered_after_the_end` 1), o que
exercita o classificador por instante de envio de fato. `scene-claims-proof.py` **não** foi reexecutado: o script, o Rust e o
runtime que ele usa não mudaram; seus 23/23 de `683a511` continuam sendo a evidência. JSON: [`s1-job-discard-proof-probe-failures.json`](../migration/s1-job-discard-proof-probe-failures.json)
(o de `683a511`, 41/41, fica preservado). Sidecars e logs em `/tmp/pr6n-logs/native` (cmd, head, árvore, `dirty`, UTC, espaço livre, SHA-256 de binários,
`runtime/dist` e scripts, iguais antes e depois de cada passo), não versionados.

Todo trabalho escreve num `Staging` (pasta no mesmo volume do destino) e **publica por rename** só depois de
verificar; falha ou cancelamento removem o staging e preservam o destino anterior e os originais.

### O teste do cancelamento da sonda do preflight (BL139; a validar)

`async_host::a_slow_renderer_probe_in_preflight_is_a_cancellable_job` falhou uma vez numa execução completa do workspace
(`export.preflight` devolveu o job, `job.cancel` foi aceito e o job terminou `succeeded`, não `cancelled`) e passou isolado.
Não era defeito do produto: era uma corrida do teste, e o host não mudou.

**Causa.** O teste dizia "renderizador lento" com `Settings { delay: 900 ms }`, mas o atraso do `Mock` só vale em
`render_frame`, e a sonda do preflight só pergunta `capabilities()`: ela nunca esperou. O job durava alguns milissegundos e o
`job.cancel` só o alcançava a tempo com a máquina livre; sob carga o job já tinha terminado. A asserção de tempo (< 400 ms)
também não provava o que dizia, porque a sonda nunca demorava.

**Correção (só `crates/media/tests/async_host.rs`).** A fábrica do renderizador fica parada até o teste soltá-la (`go`) e
avisa que chegou lá (`starting`). O teste pede o preflight (tem de voltar com um job: o teto passou a 30 s, e uma sonda
síncrona ficaria presa na fábrica até os 60 s do limite), espera a sonda estar **dentro** da fábrica, cancela, confere
`cancelled` **e** que a fábrica ainda não saiu (`returned`: cancelar não espera por ela), solta o sinal e pede o preflight
de novo. Esse segundo pedido repete só enquanto a resposta for `renderer_busy` (a sonda órfã ainda segura o renderizador,
BL144.18) e então confere `succeeded` com `ok: true`. Nenhuma espera por tempo decide o resultado; os limites de 60 s só
impedem que uma regressão pendure a suíte.

**Prova (2026-11-12, base `d5d641d`, FFmpeg real, máquina compartilhada com load médio de 20 a 40).**

* *O defeito, sem depender de sorte:* uma cópia temporária do arquivo de testes, com 400 ms de pausa antes do `job.cancel` (a
  máquina ocupada, de propósito), reprova com o teste antigo (`job_finished`: `Job job-3 already succeeded`) e passa com o novo.
* *Estabilidade:* 20 execuções seguidas do teste novo, todas verdes; o `async_host` inteiro, 7 de 7 na época e verde depois.
* *O cancelamento que não espera a sonda (achado do CodeRabbit na PR #53):* o estado `cancelled` sozinho não distingue um
  cancelamento que esperou a sonda, porque o job só termina quando a fábrica solta (aos 60 s) e o `poll_until` espera até
  120 s. Por isso o teste confere também que a fábrica ainda não retornou quando o evento chega. Prova, com uma mutação de
  produção em `probe_capabilities` (a checagem de cancelamento só depois de a sonda retornar): a versão anterior do teste
  **passa**, em 61 s, e a nova **reprova** (`the job ended only after the factory returned`); sem a mutação, 10 execuções
  seguidas verdes e o `async_host` inteiro verde (8 de 8, 4,95 s); `host.rs` restaurado byte a byte.
* *O produto não mudou:* a mutação acima só prova a asserção. O que torna o teste independente da velocidade é a fábrica
  que espera o sinal do teste, não o atraso do `Mock`.

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

Borda conservadora da janela: se a primeira resposta já indicar descarte, o início inclui também os envios falhos anteriores, pois nenhuma resposta anterior estabeleceu uma fronteira ociosa. O spec dirige a Probe real e demonstra que a borda antiga deixava passar a mesma falha.
