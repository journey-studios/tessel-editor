# Transit probe A/B (BL157.2): is the stall after a draw the GPU still working?

- code `b0cca09e069b` dirty files 13, binary sha256 `6a087183057816a4`, runtime sha256 `1fed1ef93836d9f5`, document sha256 `e93363aecc4c0591`
- asset I/O fixed at `async`, window [114, 210], arm A `trace + page timing (the probe's own cost in both arms)` / arm B `arm A + the page waits for the GPU after each draw (TESSEL_VIEWPORT_PROBE_GPU_FINISH=1)`, 4 pairs, order alternates (pair 0 A first, pair 1 B first, ...)
- every run is a fresh editor: all are FIRST plays of a cold process; a repeated play in one process is not measured here

| pair | order | arm | valid | class | pictures | gap p50 | gap p95 | gap max | slow gaps (from→to ms) | UI blocks | page stalls ≥40 (n/max ms) | gpu finish max ms (frames ≥20 ms) |
|---|---|---|---|---|---|---|---|---|---|---|---|---|
| 0 | AB | A | NO: the page stopped answering: the play shows almost no pictures | loaded | 0 | None | None | None | 0 (-) | 0 | 1/82.00000000000023 | None ([]) |
| 0 | AB | B | yes | loaded | 96 | 43.088 | 66.671 | 85.156 | 5 (139→140 85.156; 185→186 68.927; 186→187 66.671; 192→193 69.563; 195→196 70.989) | 0 | 9/122 | 80.0 ([[4, 80.0], [3, 71], [74, 55], [99, 34], [73, 31], [2, 29]]) |
| 1 | BA | B | NO: the page stopped answering: the play shows almost no pictures | loaded | 0 | None | None | None | 0 (-) | 0 | 0/None | 1 ([]) |
| 1 | BA | A | yes | loaded | 100 | 42.327 | 58.113 | 176.757 | 3 (118→119 176.757; 139→140 84.673; 187→188 129.549) | 0 | 4/183 | None ([]) |
| 2 | AB | A | yes | loaded | 102 | 43.068 | 57.382 | 174.249 | 4 (115→117 174.249; 138→139 62.603; 187→188 107.344; 195→196 80.868) | 0 | 5/186.99999999999955 | None ([]) |
| 2 | AB | B | yes | loaded | 105 | 43.435 | 61.524 | 103.909 | 5 (139→140 65.395; 185→186 66.837; 187→188 92.2; 195→196 103.909; 221→222 82.421) | 0 | 10/123 | 148 ([[108, 148], [4, 88], [75, 80], [3, 42], [73, 29.0], [74, 26]]) |
| 3 | BA | B | yes | loaded | 104 | 43.633 | 58.124 | 98.975 | 5 (140→141 93.007; 185→186 97.677; 186→187 98.975; 192→193 62.527; 195→196 72.674) | 0 | 10/188.00000000000045 | 145 ([[4, 145], [73, 96.0], [3, 50], [72, 44], [107, 44], [2, 22]]) |
| 3 | BA | A | NO: the page stopped answering: the play shows almost no pictures | loaded | 0 | None | None | None | 0 (-) | 0 | 0/None | None ([]) |

## Per arm (runs listed, not averaged)

- arm A: 2 valid run(s); worst gap per run [176.757, 174.249]; page stalls ≥40 ms per run [4, 5] (max [183, 186.99999999999955])
- arm B: 3 valid run(s); worst gap per run [85.156, 103.909, 98.975]; page stalls ≥40 ms per run [9, 10, 10] (max [122, 123, 188.00000000000045])

Verdict: INCONCLUSIVE: fewer than 4 valid runs in an arm.
