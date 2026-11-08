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

**Estado: implementado, sem execução de Cargo nesta entrega** (disco abaixo do piso). Ver "Evidências e status".

## Contrato do core / biblioteca

A regra mora em `tessel_editor_core::renderer_claims` (sem GPUI, janela, socket, relógio nem renderer): uma função pura
`decide(claimant, holders, reserved) -> Verdict`. O adaptador (`tessel-editor-media`, `MediaHost`) apenas lê o registro
de jobs, pergunta e executa o que a regra manda. **Nenhuma regra está na UI nem no JS.**

```text
                       pede o renderer
quem o tem             export / preflight           miniaturas (tick ou explícita)
---------------------  ---------------------------  -----------------------------------
ninguém                concedido                    concedido
export / preflight     recusa renderer_busy         recusa renderer_busy
um job de miniatura    recusa renderer_busy + o     a família decide (mesmo projeto:
                       job é mandado parar          reutiliza; outro projeto: espera);
                       (export tem prioridade)      nunca um segundo renderer
reservado p/ export    concedido                    recusa renderer_busy (reservado)
```

- **Quem tem o renderer** é lido do registro de jobs (única fonte de verdade): job `thumbnail` ou `video-export` que
  ainda não terminou (um job cancelado continua dono até o corpo retornar, porque um quadro em andamento não é
  interrompível), mais os jobs `export.preflight` que o host registra (o tipo `preflight` também cobre checagens sem
  renderer). Um job já terminado e ainda não liberado pelo `poll`/`job.commit`/`job.discard` **não** é dono.
- **Prioridade do export:** quando um export (ou seu preflight) só é recusado porque um job de miniaturas está no
  renderer, o job é **mandado parar** (`registry.cancel`, observado entre quadros: o quadro em voo termina) e o
  renderer fica **reservado** para o export por `EXPORT_PRIORITY_SECONDS` (30 s; `MediaHost::with_export_priority`).
  Sem a reserva, o tick (que roda a cada poucos ms) devolveria o renderer a um novo job de miniaturas antes da nova
  tentativa do chamador e o export nunca passaria. A reserva só atrasa trabalho que ninguém pediu; termina quando um
  export é concedido ou expira.
- **Nada é enfileirado.** Toda recusa é `renderer_busy` com `details {retryable: true, queued: false, job?, job_kind?,
  stopping?, reserved_for_export}`; o pedido não é lembrado e o chamador precisa pedir de novo. O export recusado nunca
  aparece sozinho. (`thumbnail_busy`, da família das miniaturas, segue como antes: filtro/retry/cancelamento em curso.)
- **Pausa do usuário:** um job de miniaturas cancelado pelo usuário (`job.cancel`) pausa o desenho automático daquele
  projeto/sessão (BL144.7). Nenhum caminho de export toca essa pausa, qualquer que seja o fim do export (concluído,
  cancelado, falho). Um job parado **pelo host** para o export (`thumb_yielded`) **não** é cancelamento do usuário: o
  desenho volta sozinho depois do export. Se o usuário cancela o job *depois* do pedido de parada do host, ou já o
  tinha cancelado antes, a decisão do usuário vence e a pausa vale. Só `media.thumbnails` explícito retoma.
- **Projeto/sessão:** o carimbo (projeto + sessão) das miniaturas não muda: um job de outro projeto continua cancelado
  por `poll` (e sem pausar o projeto novo), nunca reutilizado e nunca aplicado a outro. O export recusado atrás dele
  vale como qualquer outro. `release_thumbnail_job` (BL144.17) segue o único ponto que libera o slot no `poll`,
  `job.commit` e `job.discard`; ele também esquece o job parado para o export.
- **Preflight** (`export.preflight`) pede o renderer como um export (a sonda constrói um); `scene.export.preflight`
  não o constrói e fica fora.
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
`media.thumbnails`. O diálogo de export mostra o erro `renderer_busy` como qualquer erro tipado; **não há nova tentativa
automática** (BL144.19). Acesso: `./scripts/ui op '{"op":"export.start", ...}'` e `./scripts/ui op '{"op":"job.list"}'`
(o job parado aparece `cancelled` e a barra de status mostra "Miniaturas cancelada"; ele volta sozinho depois). Sem
`disabled_reason` nem `Workflow::has_dialog` novos.

## Evidências e status

Escritas, **não executadas** (o disco medido, ~8 GiB, está abaixo do piso de 12 GiB para Cargo; nenhum comando de
Cargo foi iniciado). Só `rustfmt` rodou.

- `crates/core/src/renderer_claims.rs` (testes unitários da regra, as duas ordens e todos os pedidos).
- `crates/media/tests/renderer_exclusion.rs`: renderers que **se contam** (`live`, `peak`, quadros em voo), quadro parado
  por uma trava para tornar cada ordem um fato: miniatura primeiro e export depois (recusa, parada no quadro, reserva,
  nova tentativa concedida, `peak == 1`, desenho retomado); export primeiro e miniatura depois (tick e explícita
  recusados, segundo export recusado, export nunca preemptado); preflight; recusa que não é fila (sem reserva, o export
  recusado nunca aparece); a pausa do usuário antes do export concluído/cancelado/falho (renderer devolvido, pausa
  intacta, retomada só por pedido explícito); cancelamento do usuário vencendo a parada do host nas duas ordens;
  troca de projeto; renderer que não inicia ou falha no quadro; job terminado não é dono; export dirigido nas duas ordens;
  e um teste que prova que o contador enxerga dois renderers (a asserção `peak == 1` não é vazia).
- Mutação negativa planejada (não executada): sem `admit_export`, o export iniciado atrás do quadro de miniatura sobe
  `peak` para 2 e os testes das duas ordens falham; sem `export_reserved`, o tick devolve o renderer antes da nova
  tentativa e o teste da reserva falha; sem `thumb_yielded`, o fim do job parado pausa o desenho e `a_refusal_is_not_a_queue…`
  falha; sem remover o job de `thumb_yielded` no `job.cancel`, o teste do cancelamento do usuário falha.
- Sem prova na janela nativa nem no runtime real (nenhuma execução; os testes usam dublês que se contam).

Limites declarados: (a) a sonda do preflight roda numa thread lateral; um preflight **cancelado** termina antes de a
sonda largar o renderer (BL144.18); (b) o host só coordena os renderers que ele mesmo constrói, não o monitor vivo;
(c) a parada de miniaturas termina entre quadros: no pior caso `FRAME_TIMEOUT` (60 s) mais a partida do renderer;
(d) o diálogo de export não repete sozinho (BL144.19); (e) o job parado para o export aparece como `cancelled` no
`poll`, como no cancelamento do usuário.
