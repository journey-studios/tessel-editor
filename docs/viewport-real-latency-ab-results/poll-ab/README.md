# BL157.6 — A/B nativo do agendamento da consulta do relógio de reprodução

Compara, no editor real, `TESSEL_PLAYBACK_POLL=fixed` (o relógio é consultado meio quadro depois de cada resposta) com `deadline` (a consulta é armada para a próxima fronteira de quadro, no mínimo duas vezes por
quadro). Fala da **regularidade das imagens** entregues e de **quão cedo uma troca de quadro é pedida**; **não** fala das pausas de 60–160 ms de BL157 e **não é aceite do usuário**.
Contrato e leitura: [F018](../../features/playback.md), [F060 §BL157.6](../../features/viewport-asset-io.md#bl1576--a-cadência-do-relógio-de-reprodução-e-o-agendamento-da-consulta),
[relatório §16.7](../../viewport-real-latency-ab-report.md#167-bl1576-a-cadência-do-relógio-de-reprodução-e-o-agendamento-da-consulta).

| Arquivo | O que é |
|---|---|
| `rule.txt` | A regra do veredito, escrita e commitada **antes** de qualquer sessão. `scripts/playback-poll-ab.py run` se recusa a começar (BLOCKED, saída 2) se ela não está commitada e inalterada, e grava o hash e o commit dela no `manifest.json` da rodada |

**Estado: nenhuma sessão nativa foi rodada.** Quando uma rodada existir, esta pasta recebe só o que `--report-dir` copia: `analysis.md`, `analysis.json` (que já traz o manifesto da rodada: commit do código, SHA-256 do binário
e do documento, ordem dos braços, hash e commit da regra) e `runs.sha256`; os registros brutos de sessão (centenas de MB) **não** são versionados (`runs.sha256` os lista). Nada aqui contém caminhos locais, nomes de arquivos
de usuário ou as nuvens do projeto real.

Como rodar (precisa de macOS com janela e áudio reais, ≥ 16 GiB livres e o projeto real, que é um caminho local seu):

```sh
./scripts/runtime setup && ./scripts/cargo build -p tessel-editor --locked --release
python3 scripts/playback-poll-ab.py run --binary target/release/tessel-editor --project <projeto.tessel.json> \
  --out <dir> --pairs 12 --report-dir docs/viewport-real-latency-ab-results/poll-ab      # ≈ 35 min; trava os locks de GPU
python3 scripts/playback-poll-ab.py analyze --out <dir> --project <projeto.tessel.json> [--report-dir <pasta>]   # refaz a análise dos registros salvos
python3 scripts/playback-cadence.py <dir> --group auto --markdown cadence-by-arm.md                              # o leitor sobre os mesmos registros
```

Só um veredito **ADOTAR** permite trocar o padrão (`PollMode::default()` em `crates/desktop/src/workstation/playback_poll.rs` e os testes que o fixam); qualquer outro resultado, inclusive CONTROLE NÃO REPRODUZIDO e
INCONCLUSIVO, deixa o padrão como está.
