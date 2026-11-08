# F056 — Posse única do renderer: export × miniaturas (BL144.10)

## Objetivo e limites

Um renderer é um navegador mais a GPU. Dois ao mesmo tempo brigam pelos dois, então o host deixa **um job por vez**
com a posse dele. A política é única e vale para todos os caminhos que constroem um renderer:

| Quem pede | Caminho |
|---|---|
| Export normal | `export.start` |
| Export dirigido | `scene.export` |
| Preflight de export (sonda os limites do renderer) | `export.preflight` (só quando o host tem renderer) |
| Miniaturas automáticas | o tick da janela (`MediaHost::poll`) |
| Miniaturas explícitas | `media.thumbnails` (janela ou CLI) |

Fora da política (não constroem renderer do host): `scene.export.preflight`, `media.thumbnail` (singular, FFmpeg),
`package`, `import`, o monitor vivo da janela (WebView própria; outra instância) e o carregamento de recursos de cena.

**Estado: implementado e validado por testes que contam renderers** (lib e `MediaHost`), pelos gates do workspace e
por provas reais headless (`tessel-render`) e na janela nativa (`scripts/ui`), todos em SwiftShader, sem GPU real nem
ponteiro físico. Ver "Evidências e status".

## Contrato do core / biblioteca

A regra mora em `tessel_editor_core::renderer_claims` (sem GPUI, janela, socket, relógio nem renderer): uma função pura
`decide(claimant, holders, reserved) -> Verdict`. O adaptador (`tessel-editor-media`, `MediaHost`) apenas lê o registro
de jobs, pergunta e executa o que a regra manda. **Nenhuma regra está na UI nem no JS.**

```text
                       pede o renderer
quem o tem             export / preflight           miniaturas (tick ou explícita)
---------------------  ---------------------------  -----------------------------------
ninguém                concedido                    concedido
um export              recusa renderer_busy         recusa renderer_busy
uma sonda (preflight)  recusa renderer_busy + a     recusa renderer_busy
                       sonda é mandada parar
um job de miniatura    recusa renderer_busy + o     a família decide (mesmo projeto:
                       job é mandado parar          reutiliza; outro projeto: espera);
                       (export tem prioridade)      nunca um segundo renderer
reservado p/ export    concedido                    recusa renderer_busy (reservado)
```

- **Quem tem o renderer** é lido do registro de jobs (única fonte de verdade): job `thumbnail` ou `video-export` que
  ainda não terminou (um job cancelado continua dono até o corpo retornar, porque um quadro em andamento não é
  interrompível), mais os jobs `export.preflight` que o host registra (o tipo `preflight` também cobre checagens sem
  renderer). Um job já terminado e ainda não liberado pelo `poll`/`job.commit`/`job.discard` **não** é dono.
- **A sonda do preflight é dona até largar o renderer (BL144.18).** A sonda pergunta os limites ao renderer numa thread
  lateral, para que cancelar não espere um renderer lento. O job vira `cancelled` na hora, mas o renderer só some quando a
  thread o larga. O host guarda, por preflight, uma testemunha (`Weak`) de um token que o corpo do job e a thread da
  sonda seguram; a thread o solta **depois** de largar o renderer. Enquanto o token vive, a sonda conta como dona, mesmo
  com o job final. Não há espera bloqueante (o host não trava) e não há garantia falsa: uma sonda que nunca termina
  mantém `renderer_busy` (retryable) até terminar. Provado com um renderer que bloqueia na consulta de capacidades e o
  contador `peak == 1` (sem o token o pedido seguinte constrói um segundo renderer: `peak == 2`).
- **Prioridade do export:** quando um export (ou seu preflight) só é recusado porque um job de miniaturas **ou uma
  sonda de preflight** está no renderer, esses jobs são **mandados parar** (`registry.cancel`, observado entre quadros: o
  quadro em voo termina) e o renderer fica **reservado** para o export por `EXPORT_PRIORITY_SECONDS` (30 s;
  `MediaHost::with_export_priority`). Sem a reserva, o tick (que roda a cada poucos ms) devolveria o renderer a um novo
  job de miniaturas antes da nova tentativa do chamador e o export nunca passaria. Um export em andamento **nunca** é
  parado. Uma sonda é só uma verificação e fica velha quando chega um export ou outra sonda, por isso cede.
- **Reserva (avaliada):** (a) *expira* (teste com 1,5 s: dentro dela o tick não desenha, depois dela o desenho volta
  sozinho e o export recusado não aparece); (b) é *do projeto/sessão* em que foi feita (`Stamp` de projeto + sessão): outro
  projeto, ou o mesmo documento reaberto, não a herda e desenha sem esperar (teste com limite de 10 s dentro dos 30 s);
  (c) *termina na primeira concessão* a um pedido de classe export (a nova tentativa chegou, nada mais espera); (d) uma
  recusa **inválida** (preset desconhecido, destino ausente, intervalo malformado, ativo inexistente) acontece antes da
  admissão e **não para nada nem reserva nada** (teste: o job de miniaturas fica com `cancel_requests == 0`).
- **Nada é enfileirado.** Toda recusa é `renderer_busy` com `details {retryable: true, queued: false, job?, job_kind?,
  stopping?, reserved_for_export}`; o pedido não é lembrado e o chamador precisa pedir de novo. O export recusado nunca
  aparece sozinho. (`thumbnail_busy`, da família das miniaturas, segue como antes: filtro/retry/cancelamento em curso.)
- **Pausa do usuário:** um job de miniaturas cancelado pelo usuário pausa o desenho automático daquele projeto/sessão
  (BL144.7). Nenhum caminho de export (nem preflight) toca essa pausa, qualquer que seja o fim do export ou do
  preflight (concluído, cancelado, falho, renderer que não inicia). Um job parado **pelo host** para o export
  (`thumb_yielded`) **não** é cancelamento do usuário: o desenho volta sozinho depois do export. **Como o host distingue:**
  `JobInfo.cancel_requests` (novo, no `core::jobs`, fora do JSON) conta cada `cancel` que chegou ao job; o host anota a
  contagem logo depois de parar o job e nunca pede a parada duas vezes. No fim do job, uma contagem acima da anotada é
  um pedido de outra origem, por **qualquer porta** (`job.cancel`, o botão do diálogo de tarefas, `registry().cancel`
  direto, outro thread): a decisão do usuário vence e a pausa vale. Um job que o usuário já tinha cancelado antes
  nunca é marcado como parado pelo host. Antes dessa correção a regra dependia de `job.cancel` passar pelo host, e o
  diálogo de tarefas (que cancela pelo registro) perdia a pausa. Só `media.thumbnails` explícito retoma.
- **Projeto/sessão:** o carimbo (projeto + sessão) das miniaturas não muda: um job de outro projeto continua cancelado
  por `poll` (e sem pausar o projeto novo), nunca reutilizado e nunca aplicado a outro. O export recusado atrás dele
  vale como qualquer outro. `release_thumbnail_job` (BL144.17) segue o único ponto que libera o slot no `poll`,
  `job.commit` e `job.discard`; ele também esquece o job parado para o export.
- **Preflight** (`export.preflight`) pede o renderer como um export (a sonda constrói um); `scene.export.preflight`
  não o constrói e fica fora. Um export que chega **depois** da sonda do próprio diálogo (clicar em *Exportar* com
  "Verificando…" na tela) para a sonda e responde `renderer_busy` com `stopping: true`; o mesmo vale para uma nova
  sonda logo depois de cancelar a anterior (o que *Verificar* faz). O diálogo trata isso (seção abaixo).
- A admissão acontece **depois** de toda recusa que não precisa de renderer (preset, intervalo, recursos congelados): um
  pedido que falharia de qualquer modo nunca para um job de miniaturas.

Fluxo em Rust (sem janela): `MediaHost::new().with_renderer(f).interactive()`; um tick (`poll`) com um quadro de
miniatura em voo, `headless::execute_ext(&mut state, None, Some(&mut host), &json!({"op":"export.start", ...}))` →
`Err(renderer_busy, retryable)`; depois que o job termina, o mesmo pedido concede um job de export.

## CLI headless

`./scripts/render` (processo `tessel-render`, runtime real, **sem janela**) usa o mesmo `MediaHost`; no modo síncrono
(`wait` padrão) cada operação termina antes da próxima, então a exclusão só aparece com `"wait":false`:

```sh
printf '%s\n' '{"op":"project.open","path":"/tmp/aurora/aurora.tessel.json"}' \
 '{"op":"media.thumbnails","wait":false}' \
 '{"op":"export.start","preset":"fullhd","destination":"/tmp/out.mp4","wait":false}' | ./scripts/render
```

A segunda linha cria o job de miniaturas; a terceira responde `renderer_busy` (`stopping: true`) enquanto o job não
terminou. `./scripts/headless` não tem `MediaHost` (sem mídia nem renderer): **N/A**, sem export.

## UI e CLI da janela

Nenhum controle novo, nenhuma operação nova: a janela envia os mesmos `export.start`/`scene.export`/`export.preflight`/
`media.thumbnails`. Acesso: `./scripts/ui op '{"op":"export.start", ...}'` e `./scripts/ui op '{"op":"job.list"}'` (o
job parado aparece `cancelled` e a barra de status mostra "Miniaturas cancelada"; ele volta sozinho depois). Sem
`disabled_reason` nem `Workflow::has_dialog` novos.

### Diálogo de export: recusa clara e nova tentativa (BL144.19)

Decisão: **o host não enfileira; quem pede de novo é o chamador**, e o diálogo é um chamador.

| Ação no diálogo | Se o host responde `renderer_busy` (retryable) | Por quê |
|---|---|---|
| *Verificar* / abrir o diálogo (relatório, só leitura e idempotente) | o diálogo **pede de novo, explicitamente, a cada 250 ms, até 40 vezes** (cerca de 10 s, o tempo de um quadro ou sonda parados); a tela diz "Aguardando o renderer ficar livre…". Esgotado, para e diz a causa; o botão *Verificar* tenta outra vez | cada nova tentativa é um pedido novo e visível; nada é lembrado no host |
| *Exportar* (muda algo) | **não repete sozinho**: o diálogo continua aberto com o erro; o clique seguinte é o novo pedido | um export nunca começa depois do clique que o pediu |

- A mensagem é do `flows::describe_refusal`: "O renderer está ocupado com <exportação | uma verificação anterior | o
  desenho das miniaturas>[, que foi mandado parar]. **Nada foi enfileirado**: tente de novo …". Qualquer outro erro
  mantém a mensagem do host.
- O diálogo **nunca fica preso em "Verificando…"**: o pedido recusado não é um job (`report_job = None`) e o
  relatório vira "Aguardando…" ou a mensagem. Antes de *Exportar* o diálogo cancela a própria verificação, para que ela não
  segure o renderer contra o pedido nem escreva um relatório velho por cima da resposta.
- O nó `dialog` da árvore expõe `state.retrying` (além de `verifying`, `report`, `error`); `dialog.verify` e
  `dialog.submit` seguem os mesmos manipuladores do clique.
- Testes (`crates/desktop/src/workstation/workflow.rs::dialog_retry_tests`, `flows.rs`): espera e retomada no prazo,
  uma tentativa por vencimento, o orçamento inteiro e depois a mensagem, outro erro nunca repetido, nenhum job pendente.
  A tela do diálogo precisa do `Root` do Kit (a janela de teste não tem), por isso o comportamento visível é o da prova nativa
  (`scripts/header-thumbnails-proof.py`, preflight re-pedido logo após cancelar) e `scripts/live-flow-proof.py`.

## Evidências e status

Executado em 2026-11-08, macOS (Apple Silicon), Rust 1.99.1, **SwiftShader** (nenhuma execução com GPU real; nenhum
ponteiro físico), sob o lock de GPU (`/tmp/tessel-scene-gpu.lock` + `/tmp/tessel-video-bl140-bl144-review-gpu.lock`),
tudo serializado, disco ≥ 50 GiB livres (piso 12 GiB). Código de produção no commit `9325927` (os commits seguintes
só tocam docs e o script de prova); binários e runtime reconstruídos nesta branch: `tessel-editor` sha256
`e6672048d5305bd92c9539b5c26d07edb7b8498d12a0f2019976a9c3fb4299e1`, `tessel-render`
`01559d45d8f7da367f6966c18a7a22bcaed6fbf0060b98b2145ba898e8b2f45e`, `runtime/dist/runtime.js`
`7b6b43381b55f4d9768164931783f943944608bada70240016d366eb803a2a98` (`./scripts/runtime setup` nesta branch: o `dist`
antigo já causou falha).

| Superfície | O que rodou | Resultado |
|---|---|---|
| Lib (`tessel-editor-core`, sem GPUI) | unit de `renderer_claims` (13 casos, sondas incluídas) e `jobs` (`cancel_requests`) | verde |
| Lib + `MediaHost` | `crates/media/tests/renderer_exclusion.rs`, 21 testes, renderers **que se contam** (`live`, `peak`, quadros em voo): as duas ordens (miniatura→export e export→miniatura), tick e explícita, preflight, export dirigido, troca de projeto, renderer que falha, `job.commit`/`job.discard`, pausa do usuário antes/durante/depois de export e de preflight (concluído, cancelado, falho), cancelamento do usuário contra a parada forçada (nas duas ordens, por `job.cancel`, pelo registro e numa corrida de 12 rodadas em duas threads), reserva (expira, outro projeto, pedido inválido), sonda cancelada que segura o renderer, preflight re-pedido logo após cancelar, export atrás da sonda | 21/21 |
| Mutações (`renderer_exclusion`) | sem o token da sonda; sem `admit_export` no export; sem a contagem de cancelamentos (a parada do host vale sempre); reserva sem o carimbo do projeto | as 4 mortas (1, 11, 2 e 1 teste falham); a 4ª só morreu depois de eu limitar o teste a 10 s (antes passava por esperar os 30 s da reserva) |
| Workspace | `TESSEL_REQUIRE_RUNTIME=1 ./scripts/cargo test --workspace --locked --no-fail-fast` | 1822 passaram, 0 falharam, 12 ignorados (os mesmos 12 da `main`) |
| Gates | `cargo fmt --all -- --check`; `./scripts/cargo check --workspace --locked`; `./scripts/cargo clippy --workspace --all-targets --locked -- -D warnings`; build de `-p tessel-editor` e de `-p tessel-editor-render --bins`; `cargo tree -p tessel-editor-core` sem `gpui`/`wry` | todos rc=0 |
| Runtime JS | `./scripts/runtime test` (Chrome real) | 202/202 |
| CLI headless real (`tessel-render`, sem janela) | `scripts/header-thumbnails-proof.py --only headless`: sequência **miniatura em voo → export recusado → miniatura parada → nova tentativa concedida → filme escrito → miniaturas de volta**, preflight cancelado e re-pedido | 143/143 |
| Janela nativa (`scripts/ui`, sem coordenadas) | a mesma sequência e, só nela (há o tick), **pausa de 5 s do usuário com um export no meio**: nenhum job de miniatura nasce depois do export, as imagens seguem pendentes e `media.thumbnails` explícito retoma; depois de um export que parou o desenho a janela desenha de novo sozinha | 157/157 |
| Specs Python | `header-thumbnails-proof.spec.py` (27, 9 novos) e `proof-surface.spec.py` (29), normal e com `-O` | verde nos dois modos |

**Diálogo (BL144.19):** a política e os testes estão acima; a tela do diálogo exige o `Root` do Kit, então o
comportamento visível é comprovado pela prova nativa (preflight re-pedido logo após cancelar: recusa `renderer_busy`
com `stopping`, depois concedido), não por cliques.

**Defeitos reais achados e corrigidos nesta PR:** (1) BL144.18, a sonda cancelada deixava o renderer sem dono (provado
com `peak == 2` sem o token); (2) o cancelamento pelo diálogo de tarefas ou pelo registro não vencia a parada do host
(o desenho voltava, a pausa se perdia); (3) a reserva era global e atrasava o desenho de outro projeto por até 30 s;
(4) o export (ou novo preflight) logo depois do preflight do próprio diálogo era recusado e o diálogo ficava sem saída
(achado do review da PR).

Limites que seguem (sem fingir garantia): (a) uma sonda que nunca larga o renderer mantém `renderer_busy` (retryable)
para sempre; o host não mata a thread; (b) o host só coordena os renderers que ele constrói, não o monitor vivo da
janela (outra WebView); (c) a parada de miniaturas termina entre quadros: no pior caso `FRAME_TIMEOUT` (60 s) mais a
partida do renderer; (d) o job parado para o export aparece como `cancelled` no `poll`, como no cancelamento do
usuário; (e) GPU real e ponteiro físico não foram exercitados.
