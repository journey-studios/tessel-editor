# Transit probe A/B (BL157.2): is the stall after a draw the GPU still working?

- code `b0cca09e069b` dirty files 13, binary sha256 `6a087183057816a4`, runtime sha256 `2f4c427996f7d256`, document sha256 `e93363aecc4c0591`
- asset I/O fixed at `async`, window [114, 210] (start and target stop; polling can overshoot, and pictures past the end are included), arm A `trace + page timing (the probe's own cost in both arms)` / arm B `arm A + the page waits for the GPU after each draw (TESSEL_VIEWPORT_PROBE_GPU_FINISH=1)`, 4 pairs, order alternates (pair 0 A first, pair 1 B first, ...)
- every run is a fresh editor: all are FIRST plays of a cold process; a repeated play in one process is not measured here

| pair | order | arm | valid | class | pictures | gap p50 | gap p95 | gap max | slow gaps (from→to ms) | UI blocks | page stalls ≥40 (n/max ms) | gpu finish max ms (frames ≥20 ms) |
|---|---|---|---|---|---|---|---|---|---|---|---|---|
| 0 | AB | A | yes | loaded | 107 | 43.041 | 48.881 | 49.909 | 0 (-) | 0 | 0/None | None ([]) |
| 0 | AB | B | yes | loaded | 107 | 42.898 | 49.798 | 73.976 | 1 (138→139 73.976) | 0 | 3/61 | 0 ([]) |
| 1 | BA | B | yes | loaded | 109 | 42.679 | 55.143 | 80.787 | 3 (139→140 68.188; 187→188 80.787; 195→196 64.395) | 0 | 4/144 | 0 ([]) |
| 1 | BA | A | yes | loaded | 108 | 43.145 | 51.92 | 95.133 | 4 (115→116 84.521; 138→139 70.821; 187→188 95.133; 221→222 69.55) | 0 | 2/85.00000000000091 | None ([]) |
| 2 | AB | A | yes | loaded | 108 | 42.745 | 51.982 | 111.452 | 3 (138→139 64.011; 188→189 111.452; 195→196 74.993) | 0 | 2/119 | None ([]) |
| 2 | AB | B | NO: the page stopped answering: the play shows almost no pictures | loaded | 0 | None | None | None | 0 (-) | 0 | 0/None | 0 ([]) |
| 3 | BA | B | yes | loaded | 107 | 42.737 | 58.638 | 100.379 | 4 (115→116 90.194; 139→140 67.345; 187→188 100.379; 195→196 77.674) | 0 | 7/119 | 0 ([]) |
| 3 | BA | A | yes | loaded | 97 | 43.142 | 55.787 | 89.776 | 1 (187→188 89.776) | 0 | 2/102.99999999999909 | None ([]) |

## Per arm (runs listed, not averaged)

- arm A: 4 valid run(s); worst gap per run [49.909, 95.133, 111.452, 89.776]; page stalls ≥40 ms per run [0, 2, 2, 2] (max [None, 85.00000000000091, 119, 102.99999999999909])
- arm B: 3 valid run(s); worst gap per run [73.976, 80.787, 100.379]; page stalls ≥40 ms per run [3, 4, 7] (max [61, 144, 119])

Verdict: INCONCLUSIVE: fewer than 4 valid runs in an arm.
