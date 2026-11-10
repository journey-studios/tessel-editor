# Transit probe (BL157.2)

Late pictures decomposed on one clock at a time (see the script's docstring). Diagnostic thresholds only, not approved limits. `valid` says the harness measured what it claims; a starved play or a UI block is a RED finding about the editor, not an invalid run.

- code `7850adea60ef` dirty files 0, binary sha256 `4add680ead4d37f9`, runtime sha256 `fcde5a3846b2c222`, document sha256 `e93363aecc4c0591`
- asset I/O fixed at `async`, window frames [114, 210] (where the play starts and the frame it is stopped at; the stop is checked by polling, so pictures a little past the end are included and a gap ending there counts), 2 fresh editor(s)

| iter | valid | class | gaps >62.5 | worst gap ms | pictures | starved | UI blocks ≥50 (n/max ms) | late transit ≥30 | page loop stalls (n/max ms) | ipc wait p95/max ms | eval p95/max ms |
|---|---|---|---|---|---|---|---|---|---|---|---|
| 0 | yes | loaded | 3 | 140.598 | 96 | no | 0/None | 2 | 17/137 | 0.076/0.201 | 28.433/102.816 |
| 1 | yes | loaded | 2 | 101.713 | 108 | no | 0/None | 0 | 18/39.99999999999909 | 0.098/0.339 | 17.423/27.966 |

## Slow gaps: which part held the picture that ended each

| iter | frames | gap ms | total ms | pre-send | page queue | page draw | ipc wait | delivery (both ways) | eval ms | loop max ms | ahead | dominant |
|---|---|---|---|---|---|---|---|---|---|---|---|---|
| 0 | 188→189 | 140.598 | 102.647 | 0.007 | 0 | 2 | 0.031 | 100.609 | 102.816 | 3 | {'frames': 0, 'warms': 0, 'loads': 0} | delivery:in_dominant |
| 0 | 192→193 | 74.692 | 32.162 | 0.009 | 0 | 1.0 | 0.108 | 31.045 | 32.022 | 4 | {'frames': 0, 'warms': 0, 'loads': 0} | delivery:in_dominant |
| 0 | 195→196 | 67.275 | 28.381 | 0.008 | 0 | 28 | 0.027 | 0.346 | 28.433 | 19 | {'frames': 0, 'warms': 0, 'loads': 0} | page_draw |
| 1 | 195→196 | 63.463 | 21.44 | 0.007 | 0 | 16 | 0.188 | 5.245 | 21.203 | 23.00000000000091 | {'frames': 0, 'warms': 0, 'loads': 0} | none |
| 1 | 220→222 | 101.713 | 27.253 | 0.009 | 0 | 12.0 | 0.05 | 15.194 | 27.397 | 5 | {'frames': 0, 'warms': 0, 'loads': 1} | none |

## UI blocks: prefetch steps that held the window's thread

| iter | began ms | total ms | prefetch ms | plans ahead ms | warm ms | loads | per load: kind bytes fs/send ms |
|---|---|---|---|---|---|---|---|

## Counts by dominant part

- slow-gap pictures: {"delivery:in_dominant": 2, "none": 2, "page_draw": 1}
