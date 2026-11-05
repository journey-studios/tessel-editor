# F025 — Jobs, cancelamento e staging (P03)

## Status

Lib **V**, `scripts/media`/`scripts/render` **V**, UI **V** (diálogo Tarefas, linha de progresso na barra de status,
cancelar; `flow.jobs`). `job.discard` (BL094/BL112): lib/host **V** (testes abaixo), CLI headless **V** por
`MediaHost` em JSONL (mesmos testes, sem janela), UI **parcial**: o tick da janela usa o descarte em job e o diálogo
mostra o estado, sem prova nativa nesta entrega (ver "Descartar um resultado").

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
evento `{job, kind:"discard", state, result}` uma única vez.

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

Todo trabalho escreve num `Staging` (pasta no mesmo volume do destino) e **publica por rename** só depois de
verificar; falha ou cancelamento removem o staging e preservam o destino anterior e os originais.

## Limites

Sem retomada: um export/pacote interrompido reinicia. `job.discard`: o ownership é do processo (claims em memória; dois
processos: BL111/S2) e um descarte em andamento no momento em que o processo termina deixa os arquivos (como uma queda).
Um job `discard` espera um slot (máx. 2 jobs por host); ele fica `queued` e o resultado `discarding` enquanto isso. A prova de «a UI não trava» é por tempo de resposta do
`import` (job devolvido em < 400 ms com FFmpeg artificialmente lento, `async_host.rs`) e pelo teste do desktop que aplica o
import no tick; não há medição de quadros por segundo da janela durante o job. Ver o
[relatório](../media-project-export-report.md).
