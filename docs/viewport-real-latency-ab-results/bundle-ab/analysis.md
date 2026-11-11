# Runtime bundle A/B (BL157.4): the decoder's `releaseLock` kept and dropped, in the real editor

- code `b153f257b14a` dirty files 9; binary sha256 `4e3341359878bf93471fdeff48d6e77ea927720617cb9185c5dec36f08c6bef6`; document sha256 `0b456aec2fef6682ab3ab4fd62e092d7c10ec76b832bf6d406f3423dc337f132`
- bundles: unpatched sha256 `c7d27bd296dda9e699343e114fbeeb1dfea259b48da689c6475275cd2862e84c`, patched sha256 `78a0c8f4c799deac39edf3e9ce9c6f7f4fd12d8958a82364caddf6d83f0d17c7` (the only difference between the arms)
- the SAME binary for both arms; asset I/O `async`; scene warm on; first-use factors off; every session a fresh editor playing the project twice; 6 busy-loop process(es) alongside each session
- order ['unpatched', 'patched', 'patched', 'unpatched', 'unpatched', 'patched', 'patched', 'unpatched', 'unpatched', 'patched', 'patched', 'unpatched', 'unpatched', 'patched', 'patched', 'unpatched', 'unpatched', 'patched', 'patched', 'unpatched', 'unpatched', 'patched', 'patched', 'unpatched', 'unpatched', 'patched', 'patched', 'unpatched', 'unpatched', 'patched', 'patched', 'unpatched', 'unpatched', 'patched', 'patched', 'unpatched', 'unpatched', 'patched', 'patched', 'unpatched', 'unpatched', 'patched', 'patched', 'unpatched', 'unpatched', 'patched', 'patched', 'unpatched', 'unpatched', 'patched', 'patched', 'unpatched', 'unpatched', 'patched', 'patched', 'unpatched', 'unpatched', 'patched', 'patched', 'unpatched', 'unpatched', 'patched', 'patched', 'unpatched', 'unpatched', 'patched', 'patched', 'unpatched', 'unpatched', 'patched', 'patched', 'unpatched']
- `runtime.js` the host served (its own `asset_io` event): unpatched [4281059] bytes, patched [4281045] bytes; sessions without that event: 0; findings: none

## Per arm

| arm | sessions | hung | failed | invalid | deadlock samples |
|---|---|---|---|---|---|
| unpatched | 36 | 1 | 0 | 1 | 2 |
| patched | 36 | 0 | 0 | 0 | 0 |

## What the plays looked like (the state of the editor; not what this experiment decides)

| arm | plays | largest wait per play: p50 / p95 / max (ms) | slow gaps > 62.5 ms | > 100 ms | plays with a wait > 100 ms |
|---|---|---|---|---|---|
| unpatched | 72 | 68.74 / 116.791 / 334.774 | 356 | 22 | 9 |
| patched | 72 | 69.609 / 104.378 / 600.394 | 386 | 14 | 6 |
- unpatched: plays with a slow gap (> 62.5 ms) at each location: other 42, prefetch:268 29, prefetch:186 25, video:282 25, video:370 23, video:520 23, video:196 22, video:512 22, video:18 20, prefetch:410 19, video:326 19, cut:186 7, cut:410 4, cut:268 3
- patched: plays with a slow gap (> 62.5 ms) at each location: other 44, prefetch:186 32, video:512 29, prefetch:268 28, video:18 26, prefetch:410 25, video:196 25, video:282 23, video:520 23, video:326 21, video:370 18, cut:186 10, cut:410 3, cut:268 2

## Sessions

| # | pair | arm | runtime.js bytes | class | ext median/p95 cores | loadavg start / peak 1m | first play: pictures / max gap ms | repeat play: pictures / max gap ms | hung | valid | deadlock samples |
|---|---|---|---|---|---|---|---|---|---|---|---|
| 0 | 0 | unpatched | 4281059 | loaded | 7.4/9.42 | [4.34, 9.16, 12.15] / 9.17 | 600 / 64.715 | 595 / 68.561 | no | yes | - |
| 1 | 0 | patched | 4281045 | loaded | 7.34/8.41 | [9.17, 9.57, 12.08] / 10.23 | 600 / 60.024 | 595 / 67.981 | no | yes | - |
| 2 | 1 | patched | 4281045 | loaded | 7.02/8.66 | [8.95, 9.52, 11.86] / 10.06 | 600 / 64.273 | 595 / 59.197 | no | yes | - |
| 3 | 1 | unpatched | 4281059 | loaded | 7.0/8.07 | [9.03, 9.5, 11.68] / 9.88 | 600 / 61.597 | 596 / 62.124 | no | yes | - |
| 4 | 2 | unpatched | 4281059 | loaded | 7.0/8.23 | [9.25, 9.55, 11.54] / 10.2 | 600 / 63.252 | 596 / 59.876 | no | yes | - |
| 5 | 2 | patched | 4281045 | loaded | 6.98/7.63 | [9.24, 9.56, 11.4] / 12.13 | 600 / 66.623 | 595 / 59.517 | no | yes | - |
| 6 | 3 | patched | 4281045 | loaded | 7.83/8.61 | [10.07, 9.9, 11.4] / 12.29 | 600 / 66.686 | 596 / 65.359 | no | yes | - |
| 7 | 3 | unpatched | 4281059 | loaded | 7.96/8.82 | [10.96, 10.32, 11.46] / 11.8 | 600 / 61.804 | 596 / 62.022 | no | yes | - |
| 8 | 4 | unpatched | 4281059 | loaded | 7.8/8.39 | [11.2, 10.52, 11.45] / 12.04 | 600 / 66.301 | 596 / 68.569 | no | yes | - |
| 9 | 4 | patched | 4281045 | loaded | 8.81/9.61 | [11.46, 10.78, 11.48] / 14.07 | 600 / 72.341 | 595 / 95.448 | no | yes | - |
| 10 | 5 | patched | 4281045 | loaded | 7.97/9.01 | [13.03, 11.39, 11.65] / 13.51 | 600 / 67.47 | 594 / 63.379 | no | yes | - |
| 11 | 5 | unpatched | 4281059 | loaded | 7.87/9.04 | [10.92, 11.2, 11.57] / 12.65 | 600 / 64.084 | 596 / 75.75 | no | yes | - |
| 12 | 6 | unpatched | 4281059 | loaded | 8.03/9.61 | [11.71, 11.44, 11.63] / 12.38 | 600 / 63.581 | 596 / 65.923 | no | yes | - |
| 13 | 6 | patched | 4281045 | loaded | 9.06/10.07 | [13.55, 11.79, 11.73] / 18.34 | 600 / 68.471 | 593 / 75.352 | no | yes | - |
| 14 | 7 | patched | 4281045 | loaded | 9.07/9.94 | [16.99, 13.37, 12.34] / 17.26 | 600 / 69.609 | 594 / 68.646 | no | yes | - |
| 15 | 7 | unpatched | 4281059 | loaded | 9.18/9.75 | [14.93, 13.62, 12.52] / 18.34 | 600 / 65.781 | 593 / 83.616 | no | yes | - |
| 16 | 8 | unpatched | 4281059 | loaded | 8.93/9.55 | [18.34, 14.69, 12.98] / 17.51 | 600 / 72.789 | 591 / 73.209 | no | yes | - |
| 17 | 8 | patched | 4281045 | loaded | 8.75/9.73 | [15.55, 14.58, 13.07] / 14.62 | 600 / 77.998 | 594 / 70.393 | no | yes | - |
| 18 | 9 | patched | 4281045 | loaded | 8.1/9.79 | [13.65, 14.17, 13.03] / 16.39 | 600 / 82.663 | 596 / 67.135 | no | yes | - |
| 19 | 9 | unpatched | 4281059 | loaded | 8.71/10.15 | [15.18, 14.63, 13.29] / 43.38 | 600 / 79.646 | 585 / 112.258 | no | yes | - |
| 20 | 10 | unpatched | 4281059 | loaded | 8.77/9.89 | [46.47, 23.01, 16.48] / 56.17 | 600 / 115.366 | 594 / 79.487 | no | yes | - |
| 21 | 10 | patched | 4281045 | loaded | 8.56/9.51 | [38.18, 26.36, 18.32] / 38.18 | 600 / 66.673 | 594 / 71.244 | no | yes | - |
| 22 | 11 | patched | 4281045 | loaded | 8.27/9.23 | [23.89, 24.46, 18.2] / 30.73 | 600 / 83.23 | 588 / 85.949 | no | yes | - |
| 23 | 11 | unpatched | 4281059 | loaded | 8.74/10.0 | [31.07, 26.54, 19.44] / 31.23 | 600 / 85.668 | 594 / 64.486 | no | yes | - |
| 24 | 12 | unpatched | 4281059 | loaded | 8.76/9.5 | [23.53, 25.37, 19.53] / 22.04 | 1 / — HUNG | 1 / — HUNG | HUNG | NO: the scene warm never ran (it must be on in both arms) | DEADLOCK at 13.7 s, DEADLOCK at 24.5 s |
| 25 | 12 | patched | 4281045 | loaded | 9.09/9.91 | [15.75, 22.65, 18.93] / 16.84 | 600 / 64.41 | 594 / 82.201 | no | yes | - |
| 26 | 13 | patched | 4281045 | loaded | 8.06/8.94 | [15.63, 21.27, 18.69] / 18.24 | 600 / 59.031 | 596 / 62.985 | no | yes | - |
| 27 | 13 | unpatched | 4281059 | loaded | 9.3/10.03 | [14.3, 19.94, 18.38] / 17.56 | 600 / 103.767 | 594 / 65.457 | no | yes | - |
| 28 | 14 | unpatched | 4281059 | loaded | 9.32/10.03 | [16.88, 19.5, 18.33] / 22.43 | 600 / 61.479 | 594 / 73.502 | no | yes | - |
| 29 | 14 | patched | 4281045 | loaded | 8.86/10.24 | [22.63, 20.51, 18.78] / 34.26 | 600 / 99.046 | 586 / 74.975 | no | yes | - |
| 30 | 15 | patched | 4281045 | loaded | 9.26/9.81 | [34.88, 24.63, 20.46] / 37.05 | 600 / 100.248 | 590 / 84.004 | no | yes | - |
| 31 | 15 | unpatched | 4281059 | loaded | 8.76/9.89 | [36.08, 27.01, 21.67] / 43.86 | 600 / 79.195 | 590 / 79.197 | no | yes | - |
| 32 | 16 | unpatched | 4281059 | loaded | 7.56/9.41 | [41.89, 30.77, 23.51] / 94.34 | 599 / 109.671 | 564 / 334.774 | no | yes | - |
| 33 | 16 | patched | 4281045 | loaded | 7.39/8.28 | [98.07, 49.94, 31.35] / 112.91 | 597 / 116.761 | 574 / 201.491 | no | yes | - |
| 34 | 17 | patched | 4281045 | loaded | 7.82/8.98 | [107.52, 66.64, 39.72] / 109.94 | 598 / 99.83 | 563 / 600.394 | no | yes | - |
| 35 | 17 | unpatched | 4281059 | loaded | 8.13/9.11 | [105.02, 75.15, 45.24] / 104.7 | 596 / 133.422 | 588 / 97.73 | no | yes | - |
| 36 | 18 | unpatched | 4281059 | loaded | 8.3/9.32 | [87.47, 76.69, 48.21] / 84.47 | 600 / 116.791 | 587 / 82.63 | no | yes | - |
| 37 | 18 | patched | 4281045 | loaded | 8.37/9.43 | [71.35, 73.44, 49.05] / 72.17 | 600 / 85.918 | 590 / 79.625 | no | yes | - |
| 38 | 19 | patched | 4281045 | loaded | 8.34/9.26 | [66.95, 71.95, 50.43] / 66.87 | 599 / 102.526 | 585 / 104.378 | no | yes | - |
| 39 | 19 | unpatched | 4281059 | loaded | 8.38/9.35 | [63.21, 69.57, 51.1] / 63.43 | 599 / 87.853 | 586 / 105.522 | no | yes | - |
| 40 | 20 | unpatched | 4281059 | loaded | 8.69/9.43 | [50.81, 64.95, 50.84] / 50.81 | 600 / 76.09 | 593 / 68.74 | no | yes | - |
| 41 | 20 | patched | 4281045 | loaded | 8.56/9.65 | [37.69, 58.42, 49.42] / 36.59 | 600 / 71.717 | 593 / 71.363 | no | yes | - |
| 42 | 21 | patched | 4281045 | loaded | 8.72/9.82 | [29.82, 51.99, 47.67] / 30.64 | 600 / 82.832 | 593 / 70.668 | no | yes | - |
| 43 | 21 | unpatched | 4281059 | loaded | 8.69/9.93 | [28.81, 47.38, 46.25] / 28.42 | 600 / 64.312 | 593 / 75.57 | no | yes | - |
| 44 | 22 | unpatched | 4281059 | loaded | 8.62/10.01 | [23.9, 42.13, 44.36] / 25.48 | 600 / 65.411 | 593 / 68.391 | no | yes | - |
| 45 | 22 | patched | 4281045 | loaded | 8.46/10.1 | [24.69, 38.75, 42.92] / 26.07 | 600 / 65.2 | 594 / 67.048 | no | yes | - |
| 46 | 23 | patched | 4281045 | loaded | 8.61/9.58 | [24.62, 36.01, 41.58] / 25.14 | 600 / 66.491 | 594 / 74.292 | no | yes | - |
| 47 | 23 | unpatched | 4281059 | loaded | 8.71/9.54 | [23.04, 33.32, 40.16] / 22.13 | 600 / 77.603 | 594 / 66.947 | no | yes | - |
| 48 | 24 | unpatched | 4281059 | loaded | 8.45/9.24 | [22.13, 30.83, 38.7] / 22.2 | 600 / 68.594 | 594 / 66.606 | no | yes | - |
| 49 | 24 | patched | 4281045 | loaded | 8.86/9.78 | [20.48, 28.57, 37.23] / 22.94 | 600 / 75.443 | 587 / 78.907 | no | yes | - |
| 50 | 25 | patched | 4281045 | loaded | 9.14/10.15 | [22.06, 27.43, 36.16] / 23.69 | 600 / 88.337 | 595 / 64.687 | no | yes | - |
| 51 | 25 | unpatched | 4281059 | loaded | 9.04/10.11 | [18.39, 25.57, 34.84] / 19.27 | 600 / 71.705 | 592 / 159.242 | no | yes | - |
| 52 | 26 | unpatched | 4281059 | loaded | 8.86/9.51 | [19.27, 24.24, 33.64] / 18.52 | 600 / 71.214 | 593 / 67.127 | no | yes | - |
| 53 | 26 | patched | 4281045 | loaded | 8.56/9.71 | [18.8, 22.89, 32.37] / 20.12 | 600 / 77.533 | 593 / 81.311 | no | yes | - |
| 54 | 27 | patched | 4281045 | loaded | 9.12/10.06 | [18.48, 22.09, 31.38] / 20.09 | 600 / 68.871 | 593 / 73.96 | no | yes | - |
| 55 | 27 | unpatched | 4281059 | loaded | 9.18/10.16 | [18.67, 21.5, 30.48] / 22.36 | 600 / 83.303 | 593 / 74.527 | no | yes | - |
| 56 | 28 | unpatched | 4281059 | loaded | 9.04/10.01 | [21.29, 21.67, 29.88] / 22.18 | 600 / 78.245 | 594 / 66.294 | no | yes | - |
| 57 | 28 | patched | 4281045 | loaded | 9.55/10.48 | [19.02, 21.09, 29.06] / 17.81 | 600 / 65.996 | 595 / 64.674 | no | yes | - |
| 58 | 29 | patched | 4281045 | loaded | 9.77/10.26 | [14.17, 19.24, 27.73] / 14.79 | 600 / 62.506 | 594 / 65.839 | no | yes | - |
| 59 | 29 | unpatched | 4281059 | loaded | 9.54/10.48 | [14.01, 18.22, 26.73] / 15.81 | 600 / 70.693 | 594 / 68.445 | no | yes | - |
| 60 | 30 | unpatched | 4281059 | loaded | 9.8/10.66 | [15.03, 17.68, 25.89] / 18.66 | 600 / 75.308 | 595 / 72.323 | no | yes | - |
| 61 | 30 | patched | 4281045 | loaded | 9.7/10.33 | [17.15, 17.81, 25.34] / 18.22 | 600 / 80.547 | 596 / 61.696 | no | yes | - |
| 62 | 31 | patched | 4281045 | loaded | 9.57/10.0 | [16.42, 17.54, 24.68] / 17.1 | 600 / 68.13 | 594 / 68.413 | no | yes | - |
| 63 | 31 | unpatched | 4281059 | loaded | 9.66/10.42 | [15.04, 16.97, 23.94] / 15.67 | 600 / 61.003 | 594 / 66.049 | no | yes | - |
| 64 | 32 | unpatched | 4281059 | loaded | 9.43/10.55 | [15.38, 16.54, 23.26] / 15.2 | 600 / 67.457 | 595 / 68.042 | no | yes | - |
| 65 | 32 | patched | 4281045 | loaded | 9.59/10.31 | [14.14, 15.96, 22.5] / 14.36 | 600 / 62.464 | 592 / 63.901 | no | yes | - |
| 66 | 33 | patched | 4281045 | loaded | 8.66/9.4 | [13.29, 15.42, 21.82] / 13.64 | 600 / 65.944 | 596 / 67.791 | no | yes | - |
| 67 | 33 | unpatched | 4281059 | loaded | 8.69/8.98 | [11.74, 14.64, 21.06] / 12.52 | 600 / 64.962 | 596 / 94.4 | no | yes | - |
| 68 | 34 | unpatched | 4281059 | loaded | 8.73/9.53 | [11.49, 14.0, 20.35] / 12.15 | 600 / 63.254 | 595 / 64.054 | no | yes | - |
| 69 | 34 | patched | 4281045 | loaded | 9.18/9.82 | [11.06, 13.46, 19.68] / 15.56 | 600 / 64.27 | 593 / 67.609 | no | yes | - |
| 70 | 35 | patched | 4281045 | loaded | 9.06/9.53 | [15.56, 14.38, 19.57] / 15.96 | 600 / 69.984 | 593 / 76.439 | no | yes | - |
| 71 | 35 | unpatched | 4281059 | loaded | 9.17/9.73 | [15.56, 14.5, 19.22] / 15.12 | 600 / 65.729 | 594 / 71.626 | no | yes | - |

Exact one-sided chance of so few hung sessions in the patched arm if both arms were alike (diagnostic only): 0.5

Verdict (mechanical, pre-fixed rules): NOT REPRODUCED: unpatched 1 of 36 sessions hung, patched 0 of 36; the hang did not show enough in the unpatched arm for this run to say anything about the patch
