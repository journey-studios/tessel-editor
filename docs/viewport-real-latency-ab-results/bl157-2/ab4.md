# Transit probe A/B (BL157.2): is the stall after a draw the GPU still working?

- code `a01090b5d9a7` dirty files 1, binary sha256 `ac023ebadf863d1d`, runtime sha256 `1fed1ef93836d9f5`, document sha256 `e93363aecc4c0591`
- asset I/O fixed at `async`, window [114, 210], arm A `trace + page timing (the probe's own cost in both arms)` / arm B `arm A + the page waits for the GPU after each draw (TESSEL_VIEWPORT_PROBE_GPU_FINISH=1)`, 8 pairs, order alternates (pair 0 A first, pair 1 B first, ...)
- every run is a fresh editor: all are FIRST plays of a cold process; a repeated play in one process is not measured here

| pair | order | arm | valid | class | pictures | gap p50 | gap p95 | gap max | slow gaps (from→to ms) | UI blocks | page stalls ≥40 (n/max ms) | gpu finish max ms (frames ≥20 ms) |
|---|---|---|---|---|---|---|---|---|---|---|---|---|
| 0 | AB | A | yes | quiet | 107 | 43.227 | 48.374 | 50.05 | 0 (-) | 0 | 0/None | None ([]) |
| 0 | AB | B | yes | quiet | 109 | 43.19 | 47.792 | 64.652 | 1 (185→186 64.652) | 0 | 1/50.99999999999977 | 52 ([(3, 52), (112, 46), (75, 31.0), (76, 29), (2, 24), (4, 24)]) |
| 1 | BA | B | yes | quiet | 107 | 43.64 | 48.088 | 67.199 | 1 (185→186 67.199) | 0 | 1/40 | 42.0 ([(3, 42.0), (75, 31), (76, 29), (110, 29), (4, 26.0), (2, 22.0)]) |
| 1 | BA | A | yes | loaded | 108 | 43.102 | 49.042 | 62.474 | 0 (-) | 0 | 1/42.99999999999909 | None ([]) |
| 2 | AB | A | yes | loaded | 107 | 43.246 | 48.319 | 54.2 | 0 (-) | 0 | 1/40.99999999999909 | None ([]) |
| 2 | AB | B | yes | loaded | 108 | 43.789 | 50.892 | 84.738 | 3 (185→186 66.769; 195→196 64.482; 220→222 84.738) | 0 | 2/44 | 53.0 ([(111, 53.0), (3, 41), (76, 31), (75, 30), (4, 29), (2, 22)]) |
| 3 | BA | B | yes | loaded | 106 | 43.695 | 48.816 | 69.42 | 1 (185→186 69.42) | 0 | 2/50 | 45 ([(3, 45), (109, 44.0), (75, 32), (76, 32), (4, 30.0), (2, 23)]) |
| 3 | BA | A | yes | loaded | 97 | 43.995 | 50.052 | 101.847 | 2 (187→188 101.847; 195→196 81.874) | 0 | 2/102 | None ([]) |
| 4 | AB | A | yes | quiet | 108 | 44.018 | 49.749 | 88.014 | 2 (188→189 74.601; 220→221 88.014) | 0 | 1/77.00000000000091 | None ([]) |
| 4 | AB | B | yes | quiet | 109 | 44.08 | 48.683 | 66.707 | 1 (185→186 66.707) | 0 | 2/46 | 45 ([(112, 45), (3, 44), (76, 29), (75, 28.0), (4, 27), (2, 23)]) |
| 5 | BA | B | yes | quiet | 107 | 44.392 | 47.934 | 67.031 | 2 (185→186 67.031; 195→196 66.235) | 0 | 2/45 | 41 ([(3, 41), (75, 31), (4, 30.0), (76, 29.0), (110, 28), (2, 24)]) |
| 5 | BA | A | yes | quiet | 108 | 43.665 | 46.491 | 65.728 | 1 (195→196 65.728) | 0 | 0/None | None ([]) |
| 6 | AB | A | yes | quiet | 108 | 43.299 | 48.78 | 65.111 | 1 (195→196 65.111) | 0 | 1/43 | None ([]) |
| 6 | AB | B | yes | loaded | 106 | 43.479 | 48.824 | 131.691 | 3 (138→139 71.367; 186→187 131.691; 195→196 66.561) | 0 | 3/126 | 119 ([(76, 119), (109, 48), (3, 40.0), (75, 32), (4, 29), (2, 24)]) |
| 7 | BA | B | yes | quiet | 107 | 43.289 | 50.214 | 66.912 | 1 (185→186 66.912) | 0 | 2/47.99999999999977 | 44.0 ([(3, 44.0), (110, 40.0), (75, 31), (76, 30), (4, 25.0), (2, 20.0)]) |
| 7 | BA | A | yes | loaded | 109 | 43.465 | 47.271 | 83.082 | 4 (187→188 82.345; 195→196 83.082; 217→218 79.579; 220→221 82.516) | 0 | 2/87 | None ([]) |

## Per arm (runs listed, not averaged)

- arm A: 8 valid run(s); worst gap per run [50.05, 62.474, 54.2, 101.847, 88.014, 65.728, 65.111, 83.082]; page stalls ≥40 ms per run [0, 1, 1, 2, 1, 0, 1, 2] (max [None, 42.99999999999909, 40.99999999999909, 102, 77.00000000000091, None, 43, 87])
- arm B: 8 valid run(s); worst gap per run [64.652, 67.199, 84.738, 69.42, 66.707, 67.031, 131.691, 66.912]; page stalls ≥40 ms per run [1, 1, 2, 2, 2, 2, 3, 2] (max [50.99999999999977, 40, 44, 50, 46, 45, 126, 47.99999999999977])

Verdict: CONSISTENT with GPU work still pending after a draw: B's readback waited at least 40 ms in 8 of 8 runs (max per run [52, 42.0, 53.0, 45, 45, 41, 119, 44.0]) where A shows page stalls of 40 ms or more (per run [0, 1, 1, 2, 1, 0, 1, 2]); median worst gap A 65.4195 ms, B 67.11500000000001 ms: the delay was moved into the frame, not removed. Correlation under a causal arm; the arm adds its own wait, so it does not show this is the whole stall, and it is not a display or photon latency.
