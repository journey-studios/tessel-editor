# Transit probe (BL157.2)

Late pictures decomposed on one clock at a time (see the script's docstring). Diagnostic thresholds only, not approved limits.

- code `b0cca09e069b` dirty files 11, binary sha256 `779d657b1a787e71`, runtime sha256 `d71ab4a4ffb404c4`, document sha256 `e93363aecc4c0591`
- asset I/O fixed at `async`, window frames [114, 210], 3 fresh editor(s)

| iter | valid | class | gaps >62.5 | worst gap ms | pictures | starved | UI blocks ≥50 (n/max ms) | late transit ≥30 | page loop stalls (n/max ms) | ipc wait p95/max ms | eval p95/max ms |
|---|---|---|---|---|---|---|---|---|---|---|---|
| 0 | yes | loaded | 1 | 317.898 | 103 | no | 0/None | 0 | 8/40 | 0.067/0.243 | 12.878/46.562 |
| 1 | yes | loaded | 0 | None | 0 | YES | 2/3152.4 | 0 | 0/None | 0.031/0.031 | 33.241/33.241 |
| 2 | yes | loaded | 3 | 404.623 | 97 | no | 0/None | 1 | 10/115 | 0.07/0.121 | 18.452/74.366 |

## Slow gaps: which part held the picture that ended each

| iter | frames | gap ms | total ms | pre-send | page queue | page draw | ipc wait | delivery (both ways) | eval ms | loop max ms | ahead | dominant |
|---|---|---|---|---|---|---|---|---|---|---|---|---|
| 0 | 216→216 | 317.898 | 14.618 | 0.004 | 0 | 14.0 | 0.026 | 0.588 | 1.068 | 6 | {'frames': 0, 'warms': 0, 'loads': 0} | none |
| 2 | 188→189 | 116.897 | 74.219 | 0.008 | 0 | 1.0 | 0.06 | 73.151 | 74.366 | 0 | {'frames': 0, 'warms': 0, 'loads': 0} | delivery:in_dominant |
| 2 | 195→196 | 65.006 | 23.403 | 0.006 | 0 | 23 | 0.021 | 0.376 | 23.441 | 6 | {'frames': 0, 'warms': 0, 'loads': 0} | none |
| 2 | 210→210 | 404.623 | 36.792 | 0.006 | 1 | 35 | 0.031 | 0.755 | 2.047 | 6.0000000000009095 | {'frames': 0, 'warms': 0, 'loads': 0} | page_draw |

## UI blocks: prefetch steps that held the window's thread

| iter | began ms | total ms | prefetch ms | plans ahead ms | warm ms | loads | per load: asset bytes fs/send ms |
|---|---|---|---|---|---|---|---|
| 1 | 5122.0 | 3152.4 | 3152.3922500000003 | 0.007541 | 0.005625000000000001 | 1 | none recorded |
| 1 | 8297.5 | 2809.6 | 2809.6206669999997 | 0.005791999999999999 | 0.014333 | 2 | none recorded |

## Counts by dominant part

- slow-gap pictures: {"delivery:in_dominant": 1, "none": 2, "page_draw": 1}
