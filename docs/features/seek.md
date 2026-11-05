# F007 — Tempo, seek e passos por frame

## Objetivo e limites

Frame inteiro com clamp à sequência demo. O relógio de playback é [F018](playback.md).

**Scrub (arrastar o playhead) mantém o monitor atualizando.** Cada movimento é um `SetPlayhead`, e o
monitor não cancela mais o quadro que está sendo desenhado a cada um deles: o quadro em curso termina
e é apresentado, e as posições novas se fundem numa única posição pendente (a mais recente), que é
desenhada em seguida. Ao soltar, o monitor converge para a última posição. Detalhes, medições e
limites em [docs/seek-performance-report.md](../seek-performance-report.md); regras do agendador em
[F021](viewport.md#agendador-de-preview-scrub). O custo do quadro (retenção de GPU) e os spans do host estão em
[F021](viewport.md#retenção-de-gpu-no-runtime) e em
[viewport-performance-implementation-report](../viewport-performance-implementation-report.md).

## Contrato do core / biblioteca

Entrada pública: variante(s) `Command::SetPlayhead` em `tessel_editor_core`, executada(s)
por `EditorState::execute`. Tipos auxiliares ficam em `tessel_editor_core::state`.
Consulte [o fluxo Rust](../core-architecture.md#usar-como-biblioteca) e as definições em
`crates/core/src/state/`. O estado e as validações são os mesmos para os adapters.
Resultados são JSON; falhas são `CommandError` com `code`, `message` e detalhes opcionais.
As pré-condições e limites são os implementados pelo modelo demo; não há transação de lote.

## CLI headless

```sh
./scripts/headless <<'JSONL'
{"op":"set","target":"timeline.playhead","field":"frame","value":120}
{"op":"status"}
JSONL
```

A resposta da operação vem em `result`. `status` é resumo, não snapshot completo do
inspetor; leia `EditorState::inspector` pela lib para consultas detalhadas de propriedades.

## UI e CLI da janela

O controle correspondente na biblioteca/inspetor/timeline traduz o gesto para o mesmo comando.
Com a janela aberta:

```sh
./scripts/ui set timeline.playhead frame 120
```

## Prova do scrub (janela real)

```sh
# janela do editor aberta; só `set timeline.playhead`, sem coordenadas, restaura o playhead no fim
python3 scripts/seek-scrub-proof.py                   # >= 5 quadros em 2 s e convergência ao soltar
TESSEL_VIEWPORT_INSPECT=1 ./target/debug/tessel-editor &   # monitor que expõe hash dos quadros
python3 scripts/seek-scrub-proof.py --pixel-proof     # imagens diferentes no arrasto e igual ao render.frame final
TESSEL_VIEWPORT_TRACE=1 ./target/debug/tessel-editor &     # ritmo pelos carimbos do próprio editor (clock: host)
python3 scripts/seek-scrub-proof.py                   # sem sondar durante o arrasto; present_gap_ms vem do host
```

Com proxy (`preview.policy` `auto`) o script reporta `release_to_picture_ms` (primeira imagem da última posição, mesmo de
proxy) e `release_to_original_polled_ms` (a original, sondada, limitada pelo `--poll-ms`); `converged` só vale depois do
refino (`session.refine_pending` falso e nenhuma camada de proxy no último quadro). `scripts/proxy-live-proof.py` compara
`original` × `auto` na mesma janela, clipe e arrasto; `scripts/proxy-soak.py` repete arrastos por minutos medindo memória.

Sem `TESSEL_VIEWPORT_TRACE=1` o script só pode sondar `get monitor.viewport` e então devolve
`sampled_present_gap_ms` (`clock: observer`): o intervalo entre **sondagens que viram** uma imagem nova, limitado pelo
`--poll-ms`, que junta quadros terminados entre duas sondagens e que carrega a thread de UI que mede. É um limite
inferior do ritmo, nunca evidência de intervalo, e não entra em nenhum PASS.

## Evidências e status

Regras cobertas por testes em `crates/core/src/state/timeline.rs`. Parser compartilhado: `crates/core/src/commands.rs`.
Agendamento do scrub: `crates/core/src/session/tests.rs` (sessão, backend controlado), `viewport_host_tests`
(página lenta com fluxo contínuo de seeks) e `scripts/seek-scrub-proof.py` (janela real).
Ver [matriz](README.md) para distinguir o caminho implementado da validação específica.
Os testes GPUI do desktop não substituem a inspeção de cada interação na janela nativa.
