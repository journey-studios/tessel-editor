# Playback clock cadence (BL157.6)

What the clock's polls and the monitor's pictures did in the recorded plays. A cadence, not a pause: nothing here is an acceptance.

| group | sessions | plays | frame changes | polls per frame change | poll ms mean / p95 / p99 | delivered ms p50 / p95 / max | delivered < 30 ms | > 62.5 ms | skipped frames |
|---|---|---|---|---|---|---|---|---|---|
| patched | 36 | 72 | 43040 | 1 poll: 13.1%, 2 polls: 86.9% | 22.304 / 23.072 / 28.787 | 43.991 / 52.704 / 600.394 | 0.13 | 0.009 | 16 |
| unpatched | 36 | 72 | 43058 | 1 poll: 13.0%, 2 polls: 87.0%, 3 polls: 0.0% | 22.284 / 23.068 / 28.243 | 43.997 / 52.138 / 334.774 | 0.1295 | 0.0085 | 12 |

- patched: a 41.667 ms frame over a 22.304 ms poll is 1.868 polls per frame, so the model predicts 1 poll: 13.2%, 2 polls: 86.8%
- unpatched: a 41.667 ms frame over a 22.284 ms poll is 1.87 polls per frame, so the model predicts 1 poll: 13.0%, 2 polls: 87.0%
