# Transit probe A/B (BL157.2): is the stall after a draw the GPU still working?

- code `b0cca09e069b` dirty files 13, binary sha256 `6a087183057816a4`, runtime sha256 `1fed1ef93836d9f5`, document sha256 `e93363aecc4c0591`
- asset I/O fixed at `async`, window [114, 210] (start and target stop; polling can overshoot, and pictures past the end are included), arm A `trace + page timing (the probe's own cost in both arms)` / arm B `arm A + the page waits for the GPU after each draw (TESSEL_VIEWPORT_PROBE_GPU_FINISH=1)`, 6 pairs, order alternates (pair 0 A first, pair 1 B first, ...)
- every run is a fresh editor: all are FIRST plays of a cold process; a repeated play in one process is not measured here

| pair | order | arm | valid | class | pictures | gap p50 | gap p95 | gap max | slow gaps (from→to ms) | UI blocks | page stalls ≥40 (n/max ms) | gpu finish max ms (frames ≥20 ms) |
|---|---|---|---|---|---|---|---|---|---|---|---|---|
| 0 | AB | A | yes | loaded | 108 | 43.102 | 51.212 | 73.772 | 1 (187→188 73.772) | 0 | 1/71 | None ([]) |
| 0 | AB | B | yes | loaded | 108 | 43.481 | 46.851 | 65.218 | 1 (185→186 65.218) | 0 | 2/55 | 58 ([(3, 58), (111, 46.0), (4, 31), (75, 30), (76, 30), (2, 25)]) |
| 1 | BA | B | yes | quiet | 109 | 42.673 | 47.941 | 90.52 | 3 (185→186 65.793; 186→187 90.52; 195→196 69.649) | 0 | 3/82.99999999999909 | 79 ([(76, 79), (112, 41), (3, 38), (75, 29), (2, 25), (4, 21)]) |
| 1 | BA | A | yes | loaded | 108 | 43.185 | 49.272 | 56.937 | 0 (-) | 0 | 0/None | None ([]) |
| 2 | AB | A | yes | loaded | 108 | 43.119 | 48.348 | 52.238 | 0 (-) | 0 | 0/None | None ([]) |
| 2 | AB | B | yes | loaded | 108 | 43.175 | 48.984 | 76.913 | 3 (185→186 68.084; 186→187 76.913; 195→196 70.619) | 0 | 5/66 | 63 ([(76, 63), (3, 50), (111, 39), (75, 31), (4, 27), (2, 26)]) |
| 3 | BA | B | yes | loaded | 97 | 43.732 | 48.362 | 65.813 | 2 (185→186 65.646; 195→196 65.813) | 0 | 5/48 | 42.0 ([(100, 42.0), (76, 41.0), (3, 38), (4, 30.0), (75, 29), (2, 26)]) |
| 3 | BA | A | yes | loaded | 108 | 43.232 | 48.686 | 55.683 | 0 (-) | 0 | 0/None | None ([]) |
| 4 | AB | A | yes | loaded | 97 | 43.187 | 50.073 | 86.78 | 1 (165→166 86.78) | 0 | 1/74.00000000000091 | None ([]) |
| 4 | AB | B | yes | loaded | 108 | 43.104 | 48.287 | 66.984 | 1 (185→186 66.984) | 0 | 2/45 | 40 ([(3, 40), (111, 38), (4, 29), (76, 29), (75, 27), (2, 24.0)]) |
| 5 | BA | B | NO: the page stopped answering: the play shows almost no pictures | loaded | 0 | None | None | None | 0 (-) | 0 | 0/None | 0 ([]) |
| 5 | BA | A | yes | loaded | 97 | 43.023 | 48.837 | 92.716 | 2 (187→188 92.716; 195→196 65.355) | 0 | 1/95 | None ([]) |

## Per arm (runs listed, not averaged)

- arm A: 6 valid run(s); worst gap per run [73.772, 56.937, 52.238, 55.683, 86.78, 92.716]; page stalls ≥40 ms per run [1, 0, 0, 0, 1, 1] (max [71, None, None, None, 74.00000000000091, 95])
- arm B: 5 valid run(s); worst gap per run [65.218, 90.52, 76.913, 65.813, 66.984]; page stalls ≥40 ms per run [2, 3, 5, 5, 2] (max [55, 82.99999999999909, 66, 48, 45])

Verdict: CONSISTENT with GPU work still pending after a draw: B's readback waited at least 40 ms in 5 of 5 runs (max per run [58, 79, 63, 42.0, 40]) where A shows page stalls of 40 ms or more (per run [1, 0, 0, 0, 1, 1]); median worst gap A 65.3545 ms, B 66.984 ms: the delay was moved into the frame, not removed. Correlation under a causal arm; the arm adds its own wait, so it does not show this is the whole stall, and it is not a display or photon latency.
