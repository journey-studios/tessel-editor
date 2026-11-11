# First-use A/B (BL157.3): `warm_draw` on and off, narrow

- code `b5ad3b824944` dirty files 6; binary sha256 `4e3341359878bf93471fdeff48d6e77ea927720617cb9185c5dec36f08c6bef6`; runtime sha256 `f721cd6551017f6bb67c6a9898d9036ad4092883bd3f42164d6bbfc8c38a3aa5`; document sha256 `e93363aecc4c05915fa5b74892f2f7b26b8d1cc9442c1b1a49866960eec309d7`
- the SAME binary and runtime for both arms; asset I/O `async`; scene warm on; gpu_finish off; arms: control = `TESSEL_VIEWPORT_WARM_DRAW=0`, on = `TESSEL_VIEWPORT_WARM_DRAW=1`; the other factor `TESSEL_VIEWPORT_PREUPLOAD=0` in every arm
- window [114, 210]; order ['control', 'on']; every session a fresh editor

Pre-fixed rules: see the script. Entry judged: 186 (the cut's entry). References 41.7 / 60 / 62.5 ms are diagnostic only.

## Runs

| # | pair | arm | valid | class | ext median/p95 cores | loadavg start / peak 1m | pictures | gaps >62.5 / >100 | max gap ms | hang | pre-cut excess ms | play→first picture ms | memory median/peak MiB (editor + helpers, RSS sum) |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| 0 | 0 | control (first) | yes | quiet | 2.89/3.0 | [17.31, 21.71, 15.51] / 18.12 | 95 | 1 / 0 | 68.042 | no | 112.9 | 167.2 | 723.2 / 843.0 |
| 1 | 0 | on (first) | yes | loaded | 4.36/4.91 | [15.5, 21.18, 15.4] / 16.33 | 95 | 2 / 1 | 100.654 | no | 114.5 | 150.3 | 621.8 / 958.5 |

## Entries (the longest gap ending on the entry's first frames, ms, with the frames it spans; request→accept in parentheses; page draw after the slash)

| run | play | png 196 | cut 186 |
|---|---|---|---|
| 0 control | first | 56.5 195→196 (14.36) / 14 | 48.9 188→189 (6.474) / 1.0 |
| 1 on | first | 65.2 195→196 (23.043) / 23 | 100.7 186→187 (59.617) / 2 |

## Every gap above 62.5 ms (where, what the host and page were doing)

| run | play | frames | gap ms | where | transit ms | runtime ms | UI callback ms | page work ms | reads |
|---|---|---|---|---|---|---|---|---|---|
| 0 control | first | 138→139 | 68.042 | prefetch:186 | 14.356 | 1.0 | 0.015 | 69.0 | 2 |
| 1 on | first | 186→187 | 100.654 | cut:186 | 57.617 | 2.0 | 0 | 0 | 0 |
| 1 on | first | 195→196 | 65.159 | video:196 | 0.043 | 23.0 | 0 | 0 | 0 |

## Prefetch, locations and the factor's own work

| run | play | prefetch total ms p50/p95/max | plans ahead p95/max | by location (n, p95, slow) | factor spans (outcomes, total ms, max ms) | slow GL calls |
|---|---|---|---|---|---|---|
| 0 control | first | 0.008/0.015/0.089 | 0.01/0.01 | cut:186: 7, 48.923, 0; other: 36, 48.573, 0; prefetch:186: 48, 48.508, 1; video:196: 3, 56.477, 0 | {}, 0, None | {"texSubImage2D": {"max_ms": 17, "n": 3}} |
| 1 on | first | 0.009/0.014/0.065 | 0.009/0.01 | cut:186: 7, 100.654, 1; other: 36, 48.91, 0; prefetch:186: 48, 49.176, 0; video:196: 3, 65.159, 1 | {'drawn': 1}, 1, 1 | {"texSubImage2D": {"max_ms": 20, "n": 3}} |

## Per arm and play kind (valid runs only; per-run values are listed, never averaged away)

| arm/play | runs (quiet; hangs) | gaps >62.5 per run | >100 per run | max gap per run ms | p50 / p95 of the runs' gaps (median) | PNG entry gaps n p50/p95/max | cut entry gaps n p50/p95/max | pre-cut excess per run ms | memory: median of medians / peaks per run MiB |
|---|---|---|---|---|---|---|---|---|---|
| control/first | 1 (1; 0) | [1] | [0] | [68.042] | 43.1 / 48.7 | 1 56.5/56.5/56.5 | 1 48.9/48.9/48.9 | [112.9] | 723.2 / [843.0] |
| on/first | 1 (0; 0) | [2] | [1] | [100.654] | 43.2 / 49.5 | 1 65.2/65.2/65.2 | 1 100.7/100.7/100.7 | [114.5] | 621.8 / [958.5] |

## Pairs (control vs on; the same play of the same pair)

| pair | play | comparable | entry gap ms control→on | PNG entries median ms | cut entries median ms | gaps >62.5 | >100 | max gap ms | pre-cut excess ms | peak MiB |
|---|---|---|---|---|---|---|---|---|---|---|
| 0 | first | no (loaded or invalid) | 48.9→100.7 | 56.5→65.2 | 48.9→100.7 | 1→2 | 0→1 | 68.042→100.654 | 112.9→114.5 | 843.0→958.5 |

Verdict (mechanical, pre-fixed rules): narrow: read the entry rows; one pair is a diagnostic, not a decision
