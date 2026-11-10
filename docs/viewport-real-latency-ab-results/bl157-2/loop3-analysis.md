# Transit probe (BL157.2)

Late pictures decomposed on one clock at a time (see the script's docstring). Diagnostic thresholds only, not approved limits. `valid` says the harness measured what it claims; a starved play or a UI block is a RED finding about the editor, not an invalid run.

- code `b0cca09e069b` dirty files 13, binary sha256 `6f1e6b52656ae4c1`, runtime sha256 `a4514782ce1347bb`, document sha256 `e93363aecc4c0591`
- asset I/O fixed at `async`, window frames [114, 210] (where the play starts and the frame it is stopped at; the stop is checked by polling, so pictures a little past the end are included and a gap ending there counts), 6 fresh editor(s)

| iter | valid | class | gaps >62.5 | worst gap ms | pictures | starved | UI blocks ≥50 (n/max ms) | late transit ≥30 | page loop stalls (n/max ms) | ipc wait p95/max ms | eval p95/max ms |
|---|---|---|---|---|---|---|---|---|---|---|---|
| 0 | yes | loaded | 2 | 78.425 | 107 | no | 0/None | 1 | 10/76 | 0.056/5.476 | 19.3/37.024 |
| 1 | yes | loaded | 0 | 52.987 | 108 | no | 0/None | 0 | 6/38.99999999999909 | 0.057/4.77 | 11.214/24.676 |
| 2 | yes | loaded | 1 | 109.619 | 109 | no | 0/None | 1 | 6/111 | 0.07/2.553 | 13.42/67.706 |
| 3 | yes | loaded | 1 | 63.189 | 107 | no | 0/None | 0 | 7/30.99999999999909 | 0.068/4.564 | 12.742/27.967 |
| 4 | yes | quiet | 1 | 82.174 | 109 | no | 0/None | 1 | 9/87 | 0.057/4.842 | 6.848/40.634 |
| 5 | yes | loaded | 0 | 58.264 | 107 | no | 0/None | 0 | 4/47 | 0.059/2.6 | 10.623/29.324 |

## Slow gaps: which part held the picture that ended each

| iter | frames | gap ms | total ms | pre-send | page queue | page draw | ipc wait | delivery (both ways) | eval ms | loop max ms | ahead | dominant |
|---|---|---|---|---|---|---|---|---|---|---|---|---|
| 0 | 187→188 | 78.425 | 36.98 | 0.006 | 0 | 2 | 0.027 | 34.947 | 37.024 | 4 | {'frames': 0, 'warms': 0, 'loads': 0} | delivery:in_dominant |
| 0 | 195→196 | 66.935 | 26.944 | 0.006 | 0 | 24 | 0.092 | 2.846 | 26.809 | 10.99999999999909 | {'frames': 0, 'warms': 0, 'loads': 0} | none |
| 2 | 188→189 | 109.619 | 67.48 | 0.007 | 0 | 1.0 | 0.048 | 66.425 | 67.706 | 0 | {'frames': 0, 'warms': 0, 'loads': 0} | delivery:in_dominant |
| 3 | 123→124 | 63.189 | 21.414 | 0.007 | 0 | 3 | 0.052 | 18.355 | 21.508 | 13 | {'frames': 0, 'warms': 0, 'loads': 0} | none |
| 4 | 188→189 | 82.174 | 40.49 | 0.007 | 1 | 1 | 0.048 | 38.435 | 40.634 | 0 | {'frames': 0, 'warms': 0, 'loads': 0} | delivery:in_dominant |

## UI blocks: prefetch steps that held the window's thread

| iter | began ms | total ms | prefetch ms | plans ahead ms | warm ms | loads | per load: kind bytes fs/send ms |
|---|---|---|---|---|---|---|---|

## Counts by dominant part

- slow-gap pictures: {"delivery:in_dominant": 3, "none": 2}
