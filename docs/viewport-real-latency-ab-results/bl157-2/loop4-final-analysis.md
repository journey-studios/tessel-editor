# Transit probe (BL157.2)

Late pictures decomposed on one clock at a time (see the script's docstring). Diagnostic thresholds only, not approved limits.

- code `a01090b5d9a7` dirty files 1, binary sha256 `ac023ebadf863d1d`, runtime sha256 `1fed1ef93836d9f5`, document sha256 `e93363aecc4c0591`
- asset I/O fixed at `async`, window frames [114, 210], 4 fresh editor(s)

| iter | valid | class | gaps >62.5 | worst gap ms | pictures | starved | UI blocks ≥50 (n/max ms) | late transit ≥30 | page loop stalls (n/max ms) | ipc wait p95/max ms | eval p95/max ms |
|---|---|---|---|---|---|---|---|---|---|---|---|
| 0 | yes | loaded | 4 | 185.175 | 105 | no | 0/None | 2 | 16/174 | 0.139/4.068 | 27.906/143.524 |
| 1 | yes | loaded | 3 | 126.627 | 108 | no | 0/None | 1 | 10/124 | 0.098/5.714 | 17.993/84.879 |
| 2 | yes | loaded | 3 | 125.698 | 106 | no | 0/None | 2 | 16/128 | 0.197/0.863 | 26.406/113.399 |
| 3 | yes | loaded | 4 | 163.617 | 106 | no | 0/None | 3 | 19/170.99999999999955 | 0.39/1.341 | 30.283/161.171 |

## Slow gaps: which part held the picture that ended each

| iter | frames | gap ms | total ms | pre-send | page queue | page draw | ipc wait | delivery (both ways) | eval ms | loop max ms | ahead | dominant |
|---|---|---|---|---|---|---|---|---|---|---|---|---|
| 0 | 115→116 | 108.51 | 99.802 | 0.009 | 0 | 4 | 0.048 | 95.745 | 99.722 | 27 | {'frames': 0, 'warms': 0, 'loads': 0} | delivery:in_dominant |
| 0 | 140→141 | 81.781 | 28.311 | 0.012 | 0 | 1 | 0.044 | 27.255 | 28.237 | 5 | {'frames': 0, 'warms': 0, 'loads': 1} | delivery:in_dominant |
| 0 | 187→188 | 185.175 | 143.115 | 0.006 | 0 | 6.0 | 0.063 | 137.046 | 143.524 | 5 | {'frames': 0, 'warms': 0, 'loads': 0} | delivery:in_dominant |
| 0 | 195→196 | 63.225 | 28.471 | 0.005 | 0 | 26 | 0.025 | 2.441 | 28.525 | 14 | {'frames': 0, 'warms': 0, 'loads': 0} | page_draw |
| 1 | 138→139 | 69.413 | 12.857 | 0.007 | 0 | 2 | 0.028 | 10.822 | 12.907 | 5.0000000000009095 | {'frames': 0, 'warms': 0, 'loads': 1} | none |
| 1 | 187→188 | 126.627 | 84.738 | 0.006 | 0 | 2.0 | 0.031 | 82.701 | 84.879 | 0 | {'frames': 0, 'warms': 0, 'loads': 0} | delivery:in_dominant |
| 1 | 195→196 | 70.005 | 27.882 | 0.006 | 0 | 25 | 0.044 | 2.832 | 27.814 | 4.9999999999990905 | {'frames': 0, 'warms': 0, 'loads': 0} | page_draw |
| 2 | 139→140 | 67.965 | 41.955 | 0.007 | 0 | 15 | 0.044 | 26.904 | 50.378 | 8 | {'frames': 0, 'warms': 0, 'loads': 1} | delivery:in_dominant |
| 2 | 187→188 | 125.698 | 113.235 | 0.005 | 0 | 3 | 0.04 | 110.19 | 113.399 | 14 | {'frames': 0, 'warms': 0, 'loads': 0} | delivery:in_dominant |
| 2 | 195→196 | 78.412 | 37.578 | 0.006 | 0 | 28 | 0.119 | 9.453 | 37.423 | 5 | {'frames': 0, 'warms': 0, 'loads': 0} | page_draw |
| 3 | 116→117 | 163.617 | 160.9 | 0.007 | 0 | 2.0 | 0.476 | 158.417 | 161.171 | 0 | {'frames': 0, 'warms': 0, 'loads': 0} | delivery:in_dominant |
| 3 | 139→140 | 77.241 | 30.025 | 0.007 | 0 | 16 | 0.065 | 13.953 | 30.283 | 19 | {'frames': 0, 'warms': 0, 'loads': 1} | none |
| 3 | 188→189 | 102.184 | 64.661 | 0.006 | 0 | 2.0 | 0.023 | 62.632 | 64.824 | 0 | {'frames': 0, 'warms': 0, 'loads': 0} | delivery:in_dominant |
| 3 | 195→196 | 65.418 | 25.083 | 0.007 | 0 | 24 | 0.026 | 1.05 | 25.128 | 4 | {'frames': 0, 'warms': 0, 'loads': 0} | none |

## UI blocks: prefetch steps that held the window's thread

| iter | began ms | total ms | prefetch ms | plans ahead ms | warm ms | loads | per load: kind bytes fs/send ms |
|---|---|---|---|---|---|---|---|

## Counts by dominant part

- slow-gap pictures: {"delivery:in_dominant": 8, "none": 3, "page_draw": 3}
