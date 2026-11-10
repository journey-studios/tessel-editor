# Transit probe (BL157.2)

Late pictures decomposed on one clock at a time (see the script's docstring). Diagnostic thresholds only, not approved limits. `valid` says the harness measured what it claims; a starved play or a UI block is a RED finding about the editor, not an invalid run.

- code `59ff7843aede` dirty files 2, binary sha256 `ac023ebadf863d1d`, runtime sha256 `fcde5a3846b2c222`, document sha256 `e93363aecc4c0591`
- asset I/O fixed at `async`, window frames [114, 210] (where the play starts and the frame it is stopped at; the stop is checked by polling, so pictures a little past the end are included and a gap ending there counts), 3 fresh editor(s)

| iter | valid | class | gaps >62.5 | worst gap ms | pictures | starved | UI blocks ≥50 (n/max ms) | late transit ≥30 | page loop stalls (n/max ms) | ipc wait p95/max ms | eval p95/max ms |
|---|---|---|---|---|---|---|---|---|---|---|---|
| 0 | yes | loaded | 3 | 98.727 | 109 | no | 0/None | 2 | 17/95 | 0.069/2.755 | 20.453/83.68 |
| 1 | yes | loaded | 0 | 62.169 | 109 | no | 0/None | 0 | 10/48.00000000000091 | 0.067/4.902 | 10.351/27.771 |
| 2 | yes | loaded | 1 | 73.938 | 97 | no | 0/None | 0 | 12/49.99999999999909 | 0.071/4.804 | 12.146/32.435 |

## Slow gaps: which part held the picture that ended each

| iter | frames | gap ms | total ms | pre-send | page queue | page draw | ipc wait | delivery (both ways) | eval ms | loop max ms | ahead | dominant |
|---|---|---|---|---|---|---|---|---|---|---|---|---|
| 0 | 186→187 | 98.727 | 83.497 | 0.007 | 0 | 1.0 | 0.037 | 82.453 | 83.68 | 9 | {'frames': 0, 'warms': 0, 'loads': 0} | delivery:in_dominant |
| 0 | 189→190 | 68.12 | 32.218 | 0.006 | 0 | 1 | 0.038 | 31.174 | 32.323 | 3 | {'frames': 0, 'warms': 0, 'loads': 0} | delivery:in_dominant |
| 0 | 195→196 | 69.399 | 28.041 | 0.006 | 0 | 28 | 0.022 | 0.013 | 28.149 | 5.9999999999990905 | {'frames': 0, 'warms': 0, 'loads': 0} | page_draw |
| 2 | 195→196 | 73.938 | 32.533 | 0.007 | 0 | 28 | 0.071 | 4.455 | 32.435 | 4.0000000000009095 | {'frames': 0, 'warms': 0, 'loads': 0} | page_draw |

## UI blocks: prefetch steps that held the window's thread

| iter | began ms | total ms | prefetch ms | plans ahead ms | warm ms | loads | per load: kind bytes fs/send ms |
|---|---|---|---|---|---|---|---|

## Counts by dominant part

- slow-gap pictures: {"delivery:in_dominant": 2, "page_draw": 2}
