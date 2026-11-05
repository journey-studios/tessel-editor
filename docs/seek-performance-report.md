# Seek / scrub: o monitor atualiza enquanto o playhead é arrastado

Relatório da correção "ao arrastar o seek o monitor só atualiza tarde". Contrato:
[F007](features/seek.md) (tempo/seek) e [F021](features/viewport.md#agendador-de-preview-scrub) (agendador).
Medidas de uma máquina e de uma execução (Apple M3 Pro, WKWebView "Apple GPU", build **debug**): são
evidência da causa e do efeito, não orçamento.

> **Correção posterior (leia antes das tabelas).** O "~20 quadros/s" e o "intervalo entre quadros de ~50 ms"
> abaixo **não são o ritmo do monitor**: são em boa parte o período de amostragem do próprio medidor. Ver
> [§ O "~20 quadros/s" era o medidor](#o-20-quadross-era-o-medidor). As tabelas ficam como foram medidas, com esse aviso.

## Sintoma e causa

Reprodução original (60 `set timeline.playhead` em 2 s): 0 quadros aceitos, 91 pedidos, 90 cancelados,
17 respostas velhas descartadas, `render_ms` ≈ 109. Nesta máquina, com o binário anterior à correção
(`scripts/seek-scrub-proof.py`, 3 corridas): **0 quadros em 2 s**, 60 pedidos / 1 aceito / 59 cancelados /
59 velhas, e o único quadro aparece 4,6 s depois de soltar.

1. `ViewportHost::sync` pedia um quadro a cada mudança e `ViewportSession::request_frame` **cancelava o
   quadro em curso** e subia a revisão; toda resposta que chegava já era velha. Com um render de ~110 ms e
   movimentos a cada ~33 ms, nenhum quadro terminava "atual": inanição por cancelamento.
2. O runtime também abortava (`latestRender !== id`) o quadro anterior assim que chegava um `frame.render`
   mais novo, então enviar pedidos sem esperar a resposta nunca ajudaria.
3. Custo do quadro: `renderFrame` sempre fazia `readPixels` + `describePixels` (hash FNV + média) mesmo no
   preview, sem PNG nem probes. **Medido** (não presumido):

| Quadro 960×540, imagem, 40 amostras | com leitura (`inspect`) | só desenho (`inspect:false`) |
|---|---|---|
| Chrome + GPU (ANGLE/Metal) `render_ms` | 38,1 (readback 3,6 + hash/mean 30,3) | 3,7 |
| Chrome + SwiftShader `render_ms` | 172 (readback 137,4 + hash/mean 30,3) | 52,5 |
| WKWebView da janela (`last_frame.render_ms`) | 107–114 | 8 |

   (`TESSEL_GL=gpu node runtime/scripts/measure-frame-cost.mjs`.) O hash já usa duas lanes de 32 bits,
   não BigInt; ainda assim hash+média são ~30 ms por 2 MB no Chrome e a maior parte dos ~110 ms na WebKit.
   O hash/readback continua disponível (pedido explícito, probes, PNG, `TESSEL_VIEWPORT_INSPECT=1`).

## O que mudou

* `crates/core/src/session.rs` — dois caminhos: `request_frame` (explícito, semântica antiga) e
  `request_preview` (interativo). Com um quadro interativo **já enviado** e só o tempo mudado (replanejar o
  quadro em curso no estado atual ainda dá exatamente o mesmo plano; mesmo assunto e tamanho), a intenção
  vira o único `pending` (mais recente vence) em vez de cancelar; ao aceitar o quadro em curso,
  `dispatch_pending` pede a posição pendente. Edição de projeto, troca de assunto/tamanho, pedido explícito,
  reset/reload, perda de contexto, falha, timeout e quadro esperando assets continuam cancelando e
  descartando a resposta velha (`Stale`). Por que replanejar e não comparar `revision`: undo pode devolver a
  revisão a um valor antigo, e o replan responde à pergunta real (o projeto de agora ainda produz o plano
  que está renderizando?).
* `crates/desktop/src/workstation/viewport_host.rs` — `sync` usa `request_preview`; `drain` chama
  `dispatch_pending` depois dos eventos; a retenção de assets passou para o início de cada pedido na sessão
  (nunca libera textura sob um quadro que está terminando).
* Bridge v1 (aditivo, compatível nos dois sentidos): `frame.render.inspect:false` e, em `frame.done`,
  `inspected`, `timings.{settle_ms|readback_ms, describe_ms}`; `hash`/`mean` `null` quando não medido.
  Host antigo + runtime novo e o inverso funcionam (o campo ausente significa "ler", como sempre).
* `runtime/src/{index,compositor}.js` — `inspect:false` desenha e entrega os comandos (`gl.finish` na época; hoje `gl.flush`, porque `finish` não espera a GPU), sem readback
  nem hash. Probes ou PNG nunca são pulados.
* `session.presented` (`request`, `frame`), `stats.coalesced`, `pending`, `inspect_previews` no estado
  `monitor.viewport`.
* Não houve preview adaptativo nem queda de qualidade/export: o preview continua 960×540.

## Regressão antes da correção

Os testes do agendador foram escritos contra o comportamento que a correção muda: com a fusão desligada
(`can_coalesce` forçado a `false`, equivalente ao comportamento anterior) 7 testes da sessão e 5 do host
falham, por exemplo `move 1: the frame in flight is not cancelled nor replaced: ["frame.cancel",
"frame.render"]`. Com a correção, passam:

* sessão (backend controlado): scrub de 20 movimentos com 4 quadros terminando → 4 apresentados, 0
  cancelados, 0 velhos, e o último pedido é a posição final; slot único mantém só a mais nova; voltar ao
  quadro em curso descarta o pendente; edição cancela e a resposta velha é `Stale`; assunto/tamanho
  supersedem; pedido explícito supersede e limpa o pendente; explícito em curso nunca recebe fusão;
  reset/perda de contexto/falha limpam o pendente; explícito, probes e PNG mantêm `inspect`; retenção de
  assets não libera sob o quadro em curso.
* host (página scriptada que responde devagar, 90 movimentos): 15 quadros apresentados durante o arrasto,
  nenhum `frame.cancel` chegou ao runtime, cada `frame.render` novo é a posição mais nova, `presented.frame`
  ao soltar = 95; edição/troca de projeto/reload/falha/`render.frame` explícito/timeout seguros.

## Janela real, antes × depois

Cada configuração numa instância própria (HOME isolado, projeto demo; o projeto do usuário não é tocado),
`python3 scripts/seek-scrub-proof.py` — 60 `set timeline.playhead` a 30 Hz em 2 s sobre 120 frames, o mesmo
`Command::SetPlayhead` do `on_drag_move`. PASS = ≥ 5 quadros em 2 s e convergência.

| 3 corridas | antes (binário anterior) | depois | depois, `TESSEL_VIEWPORT_INSPECT=1` |
|---|---|---|---|
| Quadros apresentados em 2 s | 0 / 0 / 0 | 41 / 43 / 45 | 14 / 13 / 14 |
| Primeiro quadro novo após o 1º seek | 6,6 s (só depois de soltar) | 59–64 ms | 121–170 ms |
| ~~Intervalo entre quadros~~ `sampled_present_gap_ms` (mediana / p95), **período do amostrador, não do monitor** | — | 50 / 55–101 ms | 148–152 / 152–160 ms |
| Atraso da imagem vs playhead (frames, mediana / máx) | — | 4 / 6 | 12 / 18 |
| Soltar → imagem da posição final | 4,6–4,8 s | 93–96 ms | 163–223 ms |
| Pedidos / aceitos / cancelados / velhos | 60 / 1 / 59 / 59 | 42–47 / 42–47 / 0 / 0 (+53–55 fundidos) | 15–16 / 15–16 / 0 / 0 (+59 fundidos) |
| `last_frame.render_ms` (custo GPU+readback) | 108–112 | 8 | 107–114 |

Leitura: a mudança de agendamento tira o scrub da inanição (0 → 13–14 quadros mesmo com leitura completa) e
a remoção do readback/hash do caminho interativo divide o custo do quadro por ~14 (≈110 → 8 ms). (A leitura original
dizia "chegando a ~20 quadros/s": isso era a contagem de quadros vistos por um observador que consulta a thread de UI, não
um teto do monitor; ver a correção abaixo.) Uma primeira execução do binário antigo convergiu em 781 ms em vez de 4,6 s: o tempo de
convergência dele depende de quantos quadros já entraram na fila do runtime; o que não varia é 0 quadros
durante o arrasto.

### Imagem durante o gesto (não só contadores)

Com `TESSEL_VIEWPORT_INSPECT=1` e `--pixel-proof`: nos 3 arrastos, **15 / 14 / 15 quadros apresentados, todos
com hash diferente** (as imagens mudam, não só o contador), e ao soltar o hash da imagem na tela
(`3ead194324887083`) é **idêntico** ao de um `render.frame` explícito da posição final no mesmo tamanho.

## O "~20 quadros/s" era o medidor

Duas leituras do relatório original estavam erradas, e o parecer em
[viewport-performance-opus-review.md](viewport-performance-opus-review.md) (§1) e a validação
([viewport-performance-opus-validation.md](viewport-performance-opus-validation.md), § Resultados 1) as refutaram
com medidas:

1. **`present_gap_ms` mediano mede o amostrador.** O script perguntava `get monitor.viewport` a cada `--poll-ms` e
   chamava de "intervalo entre quadros" a diferença entre as consultas que **viram** uma imagem nova. Quadros que terminam
   entre duas consultas viram um só. O número acompanha o intervalo de consulta, não o monitor: ~50 ms com poll de 10 ms
   (a consulta custa ~13 ms de thread de UI em debug) e ~256 ms com poll de 250 ms, enquanto a **contagem** de quadros no
   mesmo experimento quase não muda (26–30 quadros/s com entrada de 30 Hz). Um gap de 256 ms com 30 quadros/s é impossível
   como intervalo real; só é possível como período de amostragem.
2. **A entrada de 30 Hz já é o teto.** 60 `set` em 2 s limitam a 30 quadros/s; "~20/s" no build debug era esse teto menos
   o custo de o observador disputar a thread de UI (que no debug passa ~80% do tempo no draw do GPUI: 36 ms por render contra
   ~4 ms em release, ver a validação, § 1).

**O que o harness faz agora** (`scripts/seek-scrub-proof.py`, `crates/core/src/trace.rs`):

* Com o editor iniciado com `TESSEL_VIEWPORT_TRACE=1`, cada quadro aceito carrega `submitted_ms`/`done_ms`/`shown_frame` no
  **relógio do host** (monótono desde a criação do trace). O script **não consulta nada durante o arrasto**; lê a janela
  de 128 quadros uma vez depois e calcula os intervalos sobre os carimbos do editor (`clock: "host"`,
  `present_gap_ms`). O ponto zero do relógio é alinhado ao do script pelo ponto médio de uma consulta
  (`clock_uncertainty_ms` ≈ meia ida e volta).
* Sem o trace o script só consegue amostrar. Então devolve `sampled_present_gap_ms` com `clock: "observer"`: **limite
  inferior do ritmo, nunca evidência de intervalo**, e esse campo nunca entra em PASS.
* `frame.presented` (rAF depois de `frame.done`, opt-in por `hello.telemetry.presented`) é um **proxy de apresentação**:
  houve uma oportunidade de composição depois do quadro. Não é prova de que o pixel chegou à tela, não dispara com a vista
  oculta, e uma contagem de quadros aceitos acima da cadência do rAF (~60 Hz no display de teste) **não é** fps
  apresentado. Seek-to-photon (câmera) continua **não medido**.

O custo do quadro e a retenção (o que realmente reduziu o tempo de CPU do runtime) estão em
[viewport-performance-implementation-report.md](viewport-performance-implementation-report.md).

## Corrigido no harness

O `release_to_ready` de `/tmp/tessel-seek-repro.py` considerava `phase == ready and not needs_redraw` como
"terminou", mas a fase fica `ready` enquanto o pedido da posição final ainda renderiza.
`scripts/seek-scrub-proof.py` define convergência como `last_frame.revision == session.revision`, status
`ready`, `pending` falso e (quando o build informa) `presented.frame == última posição`. Também mede o atraso
real da imagem, restaura o playhead do usuário no fim e nunca inicia nem fecha o editor.

## Reproduzir

```sh
# 1. janela aberta (o editor não é iniciado nem fechado pelo script)
python3 scripts/seek-scrub-proof.py --socket ~/.cache/tessel-editor/ui.sock           # PASS/FAIL + JSON
# 2. prova de imagem: editor iniciado com a inspeção ligada
TESSEL_VIEWPORT_INSPECT=1 ./target/debug/tessel-editor
python3 scripts/seek-scrub-proof.py --pixel-proof
# 3. custo do quadro (runtime puro, sem janela)
TESSEL_GL=gpu node runtime/scripts/measure-frame-cost.mjs
# 4. testes
./scripts/cargo test -p tessel-editor-core --locked      # agendador da sessão
./scripts/cargo test -p tessel-editor --locked viewport_host
./scripts/runtime test
```

`scripts/live-proof.py` continua 56/56 na janela nova e na antiga (a "rajada de 14 seeks" passou de 14
pedidos / 1 aceito / 13 cancelados para 2 pedidos / 2 aceitos / 0 cancelados).

## Limites

* A prova envia `set` pelo socket (≈13 ms de trabalho na thread de UI cada, build debug) e lê o estado em
  paralelo; um arrasto com ponteiro real faz menos trabalho por tick mas não foi medido (sem cliques por
  coordenada). Em build release os números tendem a melhorar.
* ~~O ritmo de ~50 ms entre quadros~~ era o período do amostrador (ver a correção abaixo), não "ida e volta da WebView".
  `gl.finish()` devolveu `settle_ms ≈ 0` no ANGLE/Metal e **no WebKit também não espera a GPU**: não prova que a GPU
  terminou (o runtime hoje chama `gl.flush()` e diz isso); a prova de imagem é o hash do quadro na tela (acima).
* Hash/média custam ~30 ms por 2 MB (Chrome); não foram otimizados porque o caminho interativo não os usa
  mais e o valor do hash é contrato das provas. Pedido explícito e `inspect` continuam com o custo.
* Edições arrastadas (slider de propriedade, mover clipe) mudam o plano a cada passo: seguem superseding como
  antes (a regra é "projeto mudou ⇒ obsoleto"). Se isso também engasgar, o próximo passo seria uma política
  separada para edição contínua; não foi tratado aqui.
* Um quadro que espera assets não é fundido (cada intenção o substitui); ao ficarem prontos, o pedido mais
  novo é enviado.
* Falha de um quadro mostra o erro e descarta a posição pendente; a próxima mudança tenta de novo.
* Sem preview adaptativo (resolução menor durante o arrasto): não foi necessário.
* O "antes" usou o binário antigo com o `runtime/dist` novo (o antigo foi sobrescrito ao reconstruir); como o
  host antigo nunca envia `inspect`, o runtime novo se comporta como o antigo.

## Entrega do build ao usuário (sem reiniciar o editor dele)

* O editor em uso **não foi reiniciado nem teve o projeto tocado**; as provas rodaram em instâncias próprias
  com `HOME` isolado (`/tmp/tessel-seek`, encerradas ao fim).
* **Incidente com o `target/`:** durante os builds, o GC automático do mbx (teto de 8 GiB, ver
  [cargo-worktrees](cargo-worktrees.md)) apagou o diretório gerenciado que o symlink `target` apontava, e com ele
  o `target/debug/tessel-editor` de 18:07 (o processo do usuário segue vivo; só o arquivo no disco sumiu).
  O symlink pendente foi removido e `./scripts/cargo build -p tessel-editor -p tessel-editor-render --locked`
  recriou `target/debug/` (18:43), agora **com a correção do scrub** e com o que o outro agente tinha na árvore
  naquele instante. Evite repetir: rode com `MBX_GC_MAX_TOTAL_SIZE=64GiB` enquanto houver duas tarefas na
  mesma máquina.
* `runtime/dist` foi reconstruído (compatível com o host antigo: o campo novo é opcional); um
  `monitor.reload` no editor atual carrega o runtime novo sem trocar o host.
* Para o usuário passar a ter o scrub corrigido: **salvar** o projeto, fechar o editor e abri-lo de novo
  (`./scripts/cargo run -p tessel-editor --locked`, que usa `target/debug`). Quem for reiniciar deve esperar a
  task 823e349a terminar os testes dela na janela dela. Não existe um protocolo escrito de coordenação no
  repositório além deste aviso; nada foi reiniciado por esta tarefa.
