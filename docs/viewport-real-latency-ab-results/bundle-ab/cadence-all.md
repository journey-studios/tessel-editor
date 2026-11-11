# Playback clock cadence (BL157.6)

What the clock's polls and the monitor's pictures did in the recorded plays. A cadence, not a pause: nothing here is an acceptance.

| group | sessions | plays | frame changes | polls per frame change | poll ms mean / p95 / p99 | delivered ms p50 / p95 / max | delivered < 30 ms | > 62.5 ms | skipped frames |
|---|---|---|---|---|---|---|---|---|---|
| all | 72 | 144 | 86098 | 1 poll: 13.1%, 2 polls: 86.9%, 3 polls: 0.0% | 22.294 / 23.07 / 28.512 | 43.995 / 52.405 / 600.394 | 0.1298 | 0.0088 | 28 |

- all: a 41.667 ms frame over a 22.294 ms poll is 1.869 polls per frame, so the model predicts 1 poll: 13.1%, 2 polls: 86.9%
