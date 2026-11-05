# R8 — Ciclo de vida do monitor dirigido no WKWebView (BL092)

Contrato e tabela de eventos: [features/scene-renderer.md](../features/scene-renderer.md#ciclo-de-vida-no-wkwebview-bl092).
Estado de execução fica só no backlog; este relatório entrega o resultado e os patches para o coordenador.

## Resultado

**Provado no WKWebView nativo, com um bug real encontrado e corrigido.**

Bug: após uma perda de contexto (e após reload/timeout/erro de cena com a página viva) o host fazia `slots.reset()`
sem enviar `asset.release`. O runtime guardava as imagens antigas e o próximo pedido, com nomes de slot escolhidos
pelo LRU, carregava outras. Medido pela prova nativa: livro com 3 slots e runtime com 6 (uma perda), 9, 21
(seis perdas). Com o limite do runtime em 64 recursos, o monitor passaria a recusar todas as cargas.

Correção em `scene_monitor_host.rs` (três caminhos explícitos):

| Situação | Função | Efeito |
|---|---|---|
| Contexto perdido (página viva, bitmaps no runtime) | `suspend_directed` | cancela o frame em voo (`context_lost`), **mantém o livro**, suspende até `context.restored` |
| Timeout / erro de cena (página viva) | `reset_directed_runtime_for` | responde o pedido, **libera** cada slot (`asset.release`) |
| Reload (página sumiu) | `forget_directed_runtime` | responde `cancelled`, `slots.reset()` sem mensagens |

Outras correções de pedido explícito pendurado ou frame errado:

- Pedido explícito com o monitor oculto desenha e responde (só os frames do próprio monitor ficam parados); ocultar/reabrir com
  seek parado redesenha mesmo com chave igual.
- Pedido explícito com runtime subindo/indisponível é recusado na hora (`backend_unavailable`) e durante a perda de contexto
  (`context_lost`), em vez de estacionar.
- Reload: toda mensagem da página velha é descartada até `page.ready`.
- `cancelled` de load substituído, ou `resource_stale` de outra geração/recurso, não falha o frame novo; falha real continua falhando.
- `frame.done`/`frame.failed` de id que não é o ativo é recusado e contado.
- Banner "Falha: context_lost" só para cena dirigida (o monitor comum tem o seu).

## Arquivos

| Arquivo | Mudança |
|---|---|
| `crates/desktop/src/workstation/scene_monitor_host.rs` | lógica acima; `DirectedStats`; `directed_report()`; `loads`/`shown`; `Active.edit_frame` |
| `crates/desktop/src/workstation/viewport_host.rs` | `begin_reload` usa `forget_directed_runtime`; `handle_ipc` descarta tudo menos `page.ready` em `Booting`; timeout libera; relatório ganha `directed` |
| `crates/desktop/src/workstation/scene_lifecycle_tests.rs` (novo) | 13 testes de host com página roteirizada |
| `scripts/scene-lifecycle-proof.py` (novo) | prova nativa (S0–S10) |
| `scripts/scene-lifecycle-proof.spec.py` (novo) | 7 testes puros do harness |
| `docs/features/scene-renderer.md` | linha de status, seção do ciclo de vida, limites |

Nada de `workflow.rs`, `flows.rs`, `scene_panel`, `model`, `gesture`, `core/state`, `runtime/` (incl. `index.js`), backlog,
matriz, índices ou `Cargo.*` foi editado.

## APIs / seams

- `directed` em `viewport.report` com `TESSEL_VIEWPORT_INSPECT=1` (aditivo; `directed_scene` intocado):
  `scene`, `suspended`, `active{id,sent,waiting,explicit}`, `queued{explicit}`, `slots{occupied,generation,tracked_loads}`,
  `refused{stale_frames,stale_asset_states}`, `context_losses`, `shown{request_id,edit_frame}`.
- Respostas tipadas de `scene.render`: `cancelled`, `context_lost`, `backend_unavailable`, `timeout`.
- Página de teste da prova: `TESSEL_RUNTIME_DIR` aponta para uma cópia de `runtime/dist` cujo `index.html` embrulha
  `TesselRuntime.receive` e `fetch`; `runtime.js` não é alterado. Os gatilhos (frame de edição que provoca a perda) rearmam a cada reload.

## Testes

Executados:

| Comando | Resultado |
|---|---|
| `cargo-serial.py test -p tessel-editor --locked scene_lifecycle` | 13/13 |
| mesmo comando com a correção do vazamento revertida (mutação) | 3 falham (vazamento por perdas repetidas, release vs forget no timeout, redesenho após perda) |
| `... test -p tessel-editor --locked viewport` | 57/57 |
| `... test -p tessel-editor --locked scene_` | 77/77 |
| `python3 scripts/scene-lifecycle-proof.spec.py` | 7/7 |
| `with-gpu-lock.py python3 scripts/scene-lifecycle-proof.py --binary /tmp/te-r8-editor --out ...` | 218 checks, 0 falhas, em 2 execuções seguidas (Apple GPU) |

A primeira execução nativa (antes da correção) falhou nos checks de orçamento (S1, S3, S5a, S10), o que originou a correção.

Cenários nativos (todos conferem frame atual contra o modelo independente e o hash do render explícito original, resposta
tipada e livro = runtime = orçamento ≤ 32):

S0 base · S1 perda no meio do frame do monitor · S2 monitor oculto + explícito + perda · S3 ocultar/reabrir ·
S3b explícito pendente ao ocultar · S4 `monitor.reload` com explícito esperando assets lentos · S5a troca de recurso sem
frame · S5b troca com frame em voo · S6 troca de projeto com loads substituídos (`stale_asset_states` ≥ 1) · S7 perda e
troca de projeto antes do restore · S8 perda nunca restaurada · S9 perda e reload · S10 seis perdas seguidas (final: 11 perdas, livro 3, runtime 3).

Binário: `/tmp/te-r8-editor`, sha256 `b14603de3728cb0321b666a7cf739af82c9d90ac5c9d88440a1a4037aa3db32e`, base 914c830
com árvore suja (checkout compartilhado, 76 arquivos de várias frentes): a evidência vale para esta árvore, não para um commit.

Não executados:

- Gate global (`pnpm`/workspace inteiro de Cargo, clippy, fmt) — do coordenador.
- Reset real de driver/GPU (a perda é forçada por `WEBGL_lose_context`).
- Orçamento nativo com mais recursos que o corpus de 3 (o limite de 32 slots/64 recursos só é exercitado nos testes de biblioteca).
- Linux/Windows.

## Limites

- Janela estreita: `asset.state ready` atrasado de um load substituído **para o mesmo slot** não traz a identidade da cena; o
  host não o distingue de um `ready` novo. Hoje o runtime responde `cancelled` a loads substituídos (ignorado), então não
  foi observado. Fechar por completo exige o runtime ecoar `{assetId, generation}` em `asset.state` (patch abaixo).
- A prova depende de uma página de teste; a página de produção não tem hook para provocar perda de contexto.

## Patches ao coordenador

### Runtime (R7, aditivo e opcional; não editado aqui)

Em `runtime/src/index.js`, no envio de `asset.state` de recursos de cena, acrescentar `assetId` e `generation` ao payload
(campos novos, nada muda de sentido; cliente antigo segue valendo). O host ainda não lê esses campos: a comparação em
`directed_asset_state` (recusar `ready` de outra identidade, quando presente) é uma mudança pequena a fazer junto, depois
que o runtime os enviar.

### Backlog / matriz (IDs sugeridos)

- **BL092**: evidência = 13 testes de host (`scene_lifecycle`) + `scripts/scene-lifecycle-proof.py` (218 checks, 0 falhas, 2 execuções, WKWebView nativo); bug do vazamento de slots após perda de contexto corrigido. Pendente: reset real de driver, orçamento nativo com muitos recursos.
- **F041**: acrescentar a linha "Ciclo de vida do monitor dirigido no WKWebView — V no corpus" (igual à de `scene-renderer.md`).
- **Novo (sugerido, BL093)**: runtime ecoa identidade da cena em `asset.state` para fechar a janela do `ready` atrasado (R7/coordenador).
- **Novo (sugerido, BL094)**: prova nativa de orçamento com > 32 recursos distintos e LRU sob perda de contexto.
