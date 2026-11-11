# First-use A/B (BL157.3): `preupload` on and off, narrow

- code `b5ad3b824944` dirty files 6; binary sha256 `4e3341359878bf93471fdeff48d6e77ea927720617cb9185c5dec36f08c6bef6`; runtime sha256 `f721cd6551017f6bb67c6a9898d9036ad4092883bd3f42164d6bbfc8c38a3aa5`; document sha256 `e93363aecc4c05915fa5b74892f2f7b26b8d1cc9442c1b1a49866960eec309d7`
- the SAME binary and runtime for both arms; asset I/O `async`; scene warm on; gpu_finish off; arms: control = `TESSEL_VIEWPORT_PREUPLOAD=0`, on = `TESSEL_VIEWPORT_PREUPLOAD=1`; the other factor `TESSEL_VIEWPORT_WARM_DRAW=0` in every arm
- window [114, 210]; order ['control', 'on']; every session a fresh editor

Pre-fixed rules: see the script. Entry judged: 196 (the picture clip's entry). References 41.7 / 60 / 62.5 ms are diagnostic only.

## Runs

| # | pair | arm | valid | class | ext median/p95 cores | loadavg start / peak 1m | pictures | gaps >62.5 / >100 | max gap ms | hang | pre-cut excess ms | play→first picture ms | memory median/peak MiB (editor + helpers, RSS sum) |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| 0 | 0 | control (first) | yes | loaded | 4.38/6.61 | [24.84, 23.44, 15.84] / 24.84 | 95 | 2 / 0 | 92.111 | no | 110.2 | 216.1 | 689.8 / 864.7 |
| 1 | 0 | on (first) | yes | loaded | 5.24/6.98 | [22.01, 22.88, 15.73] / 23.49 | 95 | 0 / 0 | 56.458 | no | 92.2 | 191.9 | 863.5 / 1051.9 |

## Entries (the longest gap ending on the entry's first frames, ms, with the frames it spans; request→accept in parentheses; page draw after the slash)

| run | play | png 196 | cut 186 |
|---|---|---|---|
| 0 control | first | 69.5 195→196 (26.981) / 25 | 92.1 187→188 (56.211) / 1 |
| 1 on | first | 49.8 196→197 (8.312) / 1.0 | 56.5 188→189 (14.83) / 2 |

## Every gap above 62.5 ms (where, what the host and page were doing)

| run | play | frames | gap ms | where | transit ms | runtime ms | UI callback ms | page work ms | reads |
|---|---|---|---|---|---|---|---|---|---|
| 0 control | first | 187→188 | 92.111 | cut:186 | 55.211 | 1.0 | 0 | 0 | 0 |
| 0 control | first | 195→196 | 69.53 | video:196 | 1.981 | 25.0 | 0 | 0 | 0 |

## Prefetch, locations and the factor's own work

| run | play | prefetch total ms p50/p95/max | plans ahead p95/max | by location (n, p95, slow) | factor spans (outcomes, total ms, max ms) | slow GL calls |
|---|---|---|---|---|---|---|
| 0 control | first | 0.009/0.015/0.064 | 0.009/0.01 | cut:186: 7, 92.111, 1; other: 36, 48.097, 0; prefetch:186: 48, 48.993, 0; video:196: 3, 69.53, 1 | {}, 0, None | {"getBufferSubData": {"max_ms": 8, "n": 10}, "texSubImage2D": {"max_ms": 23, "n": 3}, "texSubImage3D": {"max_ms": 4, "n": 1}} |
| 1 on | first | 0.009/0.015/0.081 | 0.009/0.01 | cut:186: 7, 56.458, 0; other: 36, 48.098, 0; prefetch:186: 48, 47.874, 0; video:196: 3, 49.806, 0 | {'uploaded': 1}, 14, 14 | {"texSubImage2D": {"max_ms": 14, "n": 3}} |

## Per arm and play kind (valid runs only; per-run values are listed, never averaged away)

| arm/play | runs (quiet; hangs) | gaps >62.5 per run | >100 per run | max gap per run ms | p50 / p95 of the runs' gaps (median) | PNG entry gaps n p50/p95/max | cut entry gaps n p50/p95/max | pre-cut excess per run ms | memory: median of medians / peaks per run MiB |
|---|---|---|---|---|---|---|---|---|---|
| control/first | 1 (0; 0) | [2] | [0] | [92.111] | 43.0 / 49.0 | 1 69.5/69.5/69.5 | 1 92.1/92.1/92.1 | [110.2] | 689.8 / [864.7] |
| on/first | 1 (0; 0) | [0] | [0] | [56.458] | 43.3 / 48.3 | 1 49.8/49.8/49.8 | 1 56.5/56.5/56.5 | [92.2] | 863.5 / [1051.9] |

## Pairs (control vs on; the same play of the same pair)

| pair | play | comparable | entry gap ms control→on | PNG entries median ms | cut entries median ms | gaps >62.5 | >100 | max gap ms | pre-cut excess ms | peak MiB |
|---|---|---|---|---|---|---|---|---|---|---|
| 0 | first | no (loaded or invalid) | 69.5→49.8 | 69.5→49.8 | 92.1→56.5 | 2→0 | 0→0 | 92.111→56.458 | 110.2→92.2 | 864.7→1051.9 |

Verdict (mechanical, pre-fixed rules): narrow: read the entry rows; one pair is a diagnostic, not a decision
