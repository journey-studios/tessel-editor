# Transit probe (BL157.2)

Late pictures decomposed on one clock at a time (see the script's docstring). Diagnostic thresholds only, not approved limits. `valid` says the harness measured what it claims; a starved play or a UI block is a RED finding about the editor, not an invalid run.

- code `b0cca09e069b` dirty files 13, binary sha256 `6f1e6b52656ae4c1`, runtime sha256 `d71ab4a4ffb404c4`, document sha256 `e93363aecc4c0591`
- asset I/O fixed at `async`, window frames [114, 210] (where the play starts and the frame it is stopped at; the stop is checked by polling, so pictures a little past the end are included and a gap ending there counts), 6 fresh editor(s)

| iter | valid | class | gaps >62.5 | worst gap ms | pictures | starved | UI blocks ≥50 (n/max ms) | late transit ≥30 | page loop stalls (n/max ms) | ipc wait p95/max ms | eval p95/max ms |
|---|---|---|---|---|---|---|---|---|---|---|---|
| 0 | yes | loaded | 3 | 80.855 | 109 | no | 0/None | 2 | 13/69 | 0.076/0.316 | 26.09/43.908 |
| 1 | yes | loaded | 0 | 56.706 | 98 | no | 0/None | 0 | 5/26 | 0.042/5.33 | 10.456/35.029 |
| 2 | yes | loaded | 2 | 121.319 | 97 | no | 0/None | 1 | 11/119.99999999999909 | 0.102/5.465 | 19.151/80.329 |
| 3 | yes | loaded | 2 | 76.783 | 107 | no | 0/None | 1 | 14/82 | 0.07/0.652 | 21.993/36.269 |
| 4 | yes | loaded | 3 | 79.483 | 109 | no | 0/None | 1 | 9/83.00000000000091 | 0.065/5.12 | 19.607/39.808 |
| 5 | yes | loaded | 0 | 50.863 | 106 | no | 0/None | 1 | 7/42 | 0.055/2.602 | 13.845/44.423 |

## Slow gaps: which part held the picture that ended each

| iter | frames | gap ms | total ms | pre-send | page queue | page draw | ipc wait | delivery (both ways) | eval ms | loop max ms | ahead | dominant |
|---|---|---|---|---|---|---|---|---|---|---|---|---|
| 0 | 188→189 | 66.481 | 31.943 | 0.007 | 0 | 1.0 | 0.022 | 30.914 | 31.986 | 4 | {'frames': 0, 'warms': 0, 'loads': 0} | delivery:in_dominant |
| 0 | 195→196 | 64.552 | 31.628 | 0.006 | 0 | 19 | 0.036 | 12.586 | 31.567 | 24 | {'frames': 0, 'warms': 0, 'loads': 0} | spread |
| 0 | 220→221 | 80.855 | 39.317 | 0.007 | 0 | 9.0 | 0.076 | 30.234 | 39.193 | 13.00000000000091 | {'frames': 0, 'warms': 0, 'loads': 1} | delivery:in_dominant |
| 2 | 188→189 | 121.319 | 80.057 | 0.007 | 0 | 2 | 0.024 | 78.026 | 80.329 | 0 | {'frames': 0, 'warms': 0, 'loads': 0} | delivery:in_dominant |
| 2 | 195→196 | 79.123 | 35.008 | 0.006 | 0 | 29 | 0.064 | 5.938 | 34.907 | 6 | {'frames': 0, 'warms': 0, 'loads': 0} | page_draw |
| 3 | 188→189 | 76.346 | 36.038 | 0.007 | 0 | 3.0 | 0.022 | 33.009 | 36.269 | 4 | {'frames': 0, 'warms': 0, 'loads': 0} | delivery:in_dominant |
| 3 | 195→196 | 76.783 | 33.233 | 0.006 | 0 | 31 | 0.025 | 2.202 | 33.284 | 16 | {'frames': 0, 'warms': 0, 'loads': 0} | page_draw |
| 4 | 138→139 | 71.911 | 14.333 | 0.007 | 0 | 2 | 0.039 | 12.287 | 14.447 | 6 | {'frames': 0, 'warms': 0, 'loads': 1} | none |
| 4 | 188→189 | 79.483 | 38.054 | 0.008 | 0 | 1 | 0.031 | 37.015 | 38.105 | 0 | {'frames': 0, 'warms': 0, 'loads': 0} | delivery:in_dominant |
| 4 | 195→196 | 73.334 | 31.527 | 0.007 | 0 | 30 | 0.025 | 1.495 | 31.635 | 6 | {'frames': 0, 'warms': 0, 'loads': 0} | page_draw |

## UI blocks: prefetch steps that held the window's thread

| iter | began ms | total ms | prefetch ms | plans ahead ms | warm ms | loads | per load: kind bytes fs/send ms |
|---|---|---|---|---|---|---|---|

## Counts by dominant part

- slow-gap pictures: {"delivery:in_dominant": 5, "none": 1, "page_draw": 3, "spread": 1}
