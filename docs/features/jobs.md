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
Windows/Linux, dois processos (BL111) e escritor hostil. `job.discard` sob demanda **bem-sucedido** na janela não é
observável: o tick aplica um resultado atual em ≤ 250 ms, então o descarte sob demanda de um resultado pendente é provado
pela lib/host (host interativo sem tick) e pelo JSONL; na janela a prova é o descarte do obsoleto e a recusa do aplicado.

### Junção com claims persistentes e aceite fallível (BL094.3, merge de `main` `ac04dca`, S2)

A integração com a S2 (ledger `.tessel-claims/`, lease por processo, `Provisional::accept` fallível) é **semântica**:
o resultado recusado continua dono até o descarte terminar, e nenhuma aceitação é silenciosa.

* `PendingAdoption::settle` e `import::try_apply` aceitam com `Provisional::accept` (sem ler arquivo; o aceite só grava o ledger).
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

Todo trabalho escreve num `Staging` (pasta no mesmo volume do destino) e **publica por rename** só depois de
verificar; falha ou cancelamento removem o staging e preservam o destino anterior e os originais.

## Limites

Sem retomada: um export/pacote interrompido reinicia. `job.discard`: o ownership entre processos é o ledger de claims (BL111, S2); um descarte em andamento no momento em que o processo termina (inclusive `SIGKILL`) deixa os arquivos como órfãos que `scene.resources.recover` recolhe, sem tocar o que foi aceito.
Um job `discard` espera um slot (máx. 2 jobs por host); ele fica `queued` e o resultado `discarding` enquanto isso. A prova de «a UI não trava» é por tempo de resposta do
`import` (job devolvido em < 400 ms com FFmpeg artificialmente lento, `async_host.rs`) e pelo teste do desktop que aplica o
import no tick; não há medição de quadros por segundo da janela durante o job. Ver o
[relatório](../media-project-export-report.md).

Integração S1/S5 após main6f7a944: resultados SceneAttach têm rótulo próprio e podem ser
descartados sem I/O sobre o bundle original, sem alterar cenas/undo/revisão. A guarda de
sessão do attach e a devolução de ownership dos resultados de import/adopt foram preservadas.
Regressão nova scene_attach_job passou; suítes de descarte/claims/attach/restore, clippy
workspace/build/fmt passaram. Provas nativas no código integrado: import59/59, descarte39/39.
