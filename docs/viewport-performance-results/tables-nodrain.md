
### chrome-gpu / bench

| case | variant | render ms mean (p50/p95/p99) | wall ms | wall+drain ms | cold 1st | spread | err | raf stalls |
|---|---|---|---|---|---|---|---|---|
| image | current | 0.18 (p50 0.2, p95 0.3, p99 0.3) | 0.23 (p50 0.2, p95 0.3, p99 0.4) | - | 10.5 | 0.02 | 0 | 0 |
| image | legacy | 2.45 (p50 2.4, p95 2.7, p99 3.4) | 2.51 (p50 2.5, p95 2.8, p99 3.5) | - | 9.8 | 0.33 | 0 | 0 |
| image | baseline | 2.64 (p50 2.6, p95 2.9, p99 3.6) | 2.69 (p50 2.6, p95 3, p99 3.6) | - | 9.3 | 0.21 | 0 | 0 |
| image4k | current | 0.54 (p50 0.6, p95 0.9, p99 1.1) | 0.66 (p50 0.8, p95 1.1, p99 1.2) | - | 15.2 | 0.35 | 0 | 0 |
| image4k | legacy | 7.1 (p50 7.4, p95 7.9, p99 8.3) | 7.16 (p50 7.4, p95 8, p99 8.4) | - | 15.5 | 0.66 | 0 | 0 |
| image4k | baseline | 7.07 (p50 7, p95 7.9, p99 8.2) | 7.12 (p50 7.1, p95 8, p99 8.3) | - | 14.4 | 1.36 | 0 | 0 |
| image-2layers | current | 0.48 (p50 0.4, p95 1, p99 1.2) | 0.58 (p50 0.5, p95 1.1, p99 1.3) | - | 18.1 | 0.46 | 0 | 0 |
| image-2layers | legacy | 7.99 (p50 8, p95 8.7, p99 9.2) | 8.04 (p50 8, p95 8.7, p99 9.4) | - | 18.3 | 2.29 | 0 | 0 |
| image-2layers | baseline | 7.99 (p50 8, p95 8.7, p99 9.5) | 8.04 (p50 8.1, p95 8.8, p99 9.5) | - | 18.5 | 2.23 | 0 | 0 |
| text | current | 0.21 (p50 0.2, p95 0.3, p99 0.4) | 0.28 (p50 0.3, p95 0.4, p99 0.4) | - | 9.7 | 0.02 | 0 | 0 |
| text | legacy | 2.25 (p50 2.2, p95 2.5, p99 3.3) | 2.31 (p50 2.3, p95 2.6, p99 3.3) | - | 9.6 | 0.13 | 0 | 0 |
| text | baseline | 2.25 (p50 2.2, p95 2.6, p99 3.3) | 2.3 (p50 2.2, p95 2.7, p99 3.4) | - | 9.5 | 0.16 | 0 | 0 |
| text-changing | current | 0.35 (p50 0.3, p95 0.5, p99 0.6) | 0.4 (p50 0.4, p95 0.5, p99 0.7) | - | 9.7 | 0.03 | 0 | 0 |
| text-changing | legacy | 2.2 (p50 2.2, p95 2.6, p99 3.4) | 2.26 (p50 2.2, p95 2.6, p99 3.4) | - | 9.6 | 0.14 | 0 | 0 |
| text-changing | baseline | 2.26 (p50 2.2, p95 2.5, p99 2.9) | 2.32 (p50 2.3, p95 2.6, p99 3.1) | - | 9.4 | 0.16 | 0 | 0 |
| image+text | current | 0.22 (p50 0.2, p95 0.3, p99 0.4) | 0.28 (p50 0.3, p95 0.4, p99 0.4) | - | 10.5 | 0.02 | 0 | 0 |
| image+text | legacy | 3.17 (p50 3.1, p95 3.9, p99 4.3) | 3.24 (p50 3.2, p95 4, p99 4.3) | - | 11 | 0.38 | 0 | 0 |
| image+text | baseline | 3.14 (p50 3.1, p95 3.8, p99 4.5) | 3.2 (p50 3.1, p95 3.8, p99 4.6) | - | 10.8 | 0.29 | 0 | 0 |
| video-hit | current | 0.21 (p50 0.2, p95 0.3, p99 0.4) | 0.27 (p50 0.3, p95 0.4, p99 0.4) | - | 10.1 | 0.02 | 0 | 0 |
| video-hit | legacy | 1.86 (p50 1.9, p95 2.1, p99 2.9) | 1.93 (p50 1.9, p95 2.2, p99 3) | - | 8.8 | 0.1 | 0 | 0 |
| video-hit | baseline | 1.82 (p50 1.8, p95 2, p99 2.3) | 1.87 (p50 1.9, p95 2.1, p99 2.4) | - | 10 | 0.15 | 0 | 0 |
| video-miss | current | 8.52 (p50 8.5, p95 9.5, p99 10.3) | 8.58 (p50 8.5, p95 9.6, p99 10.4) | - | 9.9 | 0.84 | 0 | 0 |
| video-miss | legacy | 10.29 (p50 10, p95 11.8, p99 15.9) | 10.36 (p50 10.1, p95 12, p99 15.9) | - | 9.6 | 0.61 | 0 | 0 |
| video-miss | baseline | 10.12 (p50 9.9, p95 11.8, p99 13.8) | 10.17 (p50 10, p95 11.8, p99 13.9) | - | 10.6 | 0.33 | 0 | 0 |
| scene | current | 0.29 (p50 0.3, p95 0.4, p99 0.5) | 0.34 (p50 0.3, p95 0.4, p99 0.6) | - | 17.7 | 0.06 | 0 | 0 |
| scene | legacy | 5.38 (p50 5.3, p95 5.8, p99 6.2) | 5.46 (p50 5.4, p95 5.9, p99 6.3) | - | 17.7 | 0.33 | 0 | 0 |
| scene | baseline | 5.44 (p50 5.4, p95 5.8, p99 6.8) | 5.5 (p50 5.5, p95 5.9, p99 6.8) | - | 16.3 | 0.7 | 0 | 0 |
| spark-100k | current | 7.63 (p50 7.6, p95 7.9, p99 8.9) | 7.68 (p50 7.6, p95 8, p99 9) | - | 39.8 | 0.37 | 0 | 0 |
| spark-100k | legacy | 9.48 (p50 8.9, p95 13.5, p99 13.9) | 9.54 (p50 9, p95 13.5, p99 14) | - | 38.4 | 1.69 | 0 | 0 |
| spark-100k | baseline | 9.58 (p50 9.3, p95 13.7, p99 14) | 9.63 (p50 9.4, p95 13.7, p99 14.1) | - | 38.2 | 0.77 | 0 | 0 |
| spark-100k-orbit | current | 7.32 (p50 7.3, p95 7.7, p99 7.8) | 7.39 (p50 7.3, p95 7.8, p99 7.8) | - | 37 | 0.42 | 0 | 0 |
| spark-100k-orbit | legacy | 9.59 (p50 9.2, p95 13.6, p99 13.8) | 9.64 (p50 9.3, p95 13.6, p99 13.9) | - | 40.5 | 0.61 | 0 | 0 |
| spark-100k-orbit | baseline | 9.46 (p50 8.9, p95 13.5, p99 14.7) | 9.52 (p50 9, p95 13.5, p99 14.7) | - | 38.2 | 2.79 | 0 | 0 |
| spark-1m | current | 28.58 (p50 30.9, p95 35.8, p99 48.8) | 28.65 (p50 30.9, p95 35.8, p99 48.8) | - | 53.2 | 3.55 | 0 | 0 |
| spark-1m | legacy | 47.87 (p50 37.3, p95 85.8, p99 102.3) | 47.92 (p50 37.3, p95 85.9, p99 102.3) | - | 52.6 | 20.97 | 0 | 0 |
| spark-1m | baseline | 32.38 (p50 32.6, p95 41.6, p99 50.3) | 32.43 (p50 32.7, p95 41.6, p99 50.4) | - | 56.9 | 3.86 | 0 | 0 |
| spark-1m-orbit | current | 29.38 (p50 30.7, p95 40.5, p99 45.3) | 29.43 (p50 30.8, p95 40.6, p99 45.4) | - | 52.4 | 1.46 | 0 | 0 |
| spark-1m-orbit | legacy | 31.94 (p50 32.8, p95 41.4, p99 45) | 31.99 (p50 32.9, p95 41.6, p99 45) | - | 55.2 | 7.22 | 0 | 0 |
| spark-1m-orbit | baseline | 39.05 (p50 38, p95 52.9, p99 55.6) | 39.1 (p50 38, p95 52.9, p99 55.6) | - | 58.7 | 23.12 | 0 | 0 |

### chrome-gpu / hash

| case | runs | variants | frames cmp | identical | failed |
|---|---|---|---|---|---|
| image | 9 | current,legacy,baseline | 7 | true | 0 |
| image+text | 9 | current,legacy,baseline | 8 | true | 0 |
| text-changing | 9 | current,legacy,baseline | 9 | true | 0 |
| video-miss | 9 | current,legacy,baseline | 8 | true | 0 |
| scene | 9 | current,legacy,baseline | 7 | true | 0 |
| spark-100k-orbit | 9 | current,legacy,baseline | 8 | true | 0 |

### chrome-gpu / life

| case | variant | pass | swap changed | restore exact | context exact | errors |
|---|---|---|---|---|---|---|
| image | current | 1 | true | true | true | 0 |
| image | legacy | 1 | true | true | true | 0 |
| image | baseline | 1 | true | true | true | 0 |
| image-2layers | current | 1 | true | true | true | 0 |
| image-2layers | legacy | 1 | true | true | true | 0 |
| image-2layers | baseline | 1 | true | true | true | 0 |
| image+text | current | 1 | true | true | true | 0 |
| image+text | legacy | 1 | true | true | true | 0 |
| image+text | baseline | 1 | true | true | true | 0 |
| video-hit | current | 1 | true | true | true | 0 |
| video-hit | legacy | 1 | true | true | true | 0 |
| video-hit | baseline | 1 | true | true | true | 0 |
| scene | current | 1 | true | true | true | 0 |
| scene | legacy | 1 | true | true | true | 0 |
| scene | baseline | 1 | true | true | true | 0 |
| image | baseline | 2 | true | true | true | 0 |
| image | legacy | 2 | true | true | true | 0 |
| image | current | 2 | true | true | true | 0 |
| image-2layers | baseline | 2 | true | true | true | 0 |
| image-2layers | legacy | 2 | true | true | true | 0 |
| image-2layers | current | 2 | true | true | true | 0 |
| image+text | baseline | 2 | true | true | true | 0 |
| image+text | legacy | 2 | true | true | true | 0 |
| image+text | current | 2 | true | true | true | 0 |
| video-hit | baseline | 2 | true | true | true | 0 |
| video-hit | legacy | 2 | true | true | true | 0 |
| video-hit | current | 2 | true | true | true | 0 |
| scene | baseline | 2 | true | true | true | 0 |
| scene | legacy | 2 | true | true | true | 0 |
| scene | current | 2 | true | true | true | 0 |

### chrome-swiftshader / hash

| case | runs | variants | frames cmp | identical | failed |
|---|---|---|---|---|---|
| image | 9 | current,legacy,baseline | 7 | true | 0 |
| image+text | 9 | current,legacy,baseline | 8 | true | 0 |
| text-changing | 9 | current,legacy,baseline | 9 | true | 0 |
| video-miss | 9 | current,legacy,baseline | 8 | true | 0 |
| scene | 9 | current,legacy,baseline | 7 | true | 0 |
| spark-100k-orbit | 9 | current,legacy,baseline | 8 | true | 0 |

### chrome-swiftshader / life

| case | variant | pass | swap changed | restore exact | context exact | errors |
|---|---|---|---|---|---|---|
| image | current | 1 | true | true | true | 0 |
| image | legacy | 1 | true | true | true | 0 |
| image | baseline | 1 | true | true | true | 0 |
| image-2layers | current | 1 | true | true | true | 0 |
| image-2layers | legacy | 1 | true | true | true | 0 |
| image-2layers | baseline | 1 | true | true | true | 0 |
| image+text | current | 1 | true | true | true | 0 |
| image+text | legacy | 1 | true | true | true | 0 |
| image+text | baseline | 1 | true | true | true | 0 |
| video-hit | current | 1 | true | true | true | 0 |
| video-hit | legacy | 1 | true | true | true | 0 |
| video-hit | baseline | 1 | true | true | true | 0 |
| scene | current | 1 | true | true | true | 0 |
| scene | legacy | 1 | true | true | true | 0 |
| scene | baseline | 1 | true | true | true | 0 |
| image | baseline | 2 | true | true | true | 0 |
| image | legacy | 2 | true | true | true | 0 |
| image | current | 2 | true | true | true | 0 |
| image-2layers | baseline | 2 | true | true | true | 0 |
| image-2layers | legacy | 2 | true | true | true | 0 |
| image-2layers | current | 2 | true | true | true | 0 |
| image+text | baseline | 2 | true | true | true | 0 |
| image+text | legacy | 2 | true | true | true | 0 |
| image+text | current | 2 | true | true | true | 0 |
| video-hit | baseline | 2 | true | true | true | 0 |
| video-hit | legacy | 2 | true | true | true | 0 |
| video-hit | current | 2 | true | true | true | 0 |
| scene | baseline | 2 | true | true | true | 0 |
| scene | legacy | 2 | true | true | true | 0 |
| scene | current | 2 | true | true | true | 0 |

### webkit / bench

| case | variant | render ms mean (p50/p95/p99) | wall ms | wall+drain ms | cold 1st | spread | err | raf stalls |
|---|---|---|---|---|---|---|---|---|
| image | current | 0.38 (p50 0, p95 1, p99 1) | 0.43 (p50 0, p95 1, p99 1) | - | 19 | 0.06 | 0 | 0 |
| image | legacy | 8.1 (p50 8, p95 9, p99 10) | 8.26 (p50 8, p95 9, p99 10) | - | 19 | 0.69 | 0 | 0 |
| image | baseline | 8.09 (p50 8, p95 9, p99 10) | 8.13 (p50 8, p95 9, p99 10) | - | 17 | 0.67 | 0 | 0 |
| image4k | current | 0.36 (p50 0, p95 1, p99 1) | 0.42 (p50 0, p95 1, p99 1) | - | 60 | 0.13 | 0 | 0 |
| image4k | legacy | 30.48 (p50 30, p95 33, p99 37) | 30.57 (p50 30, p95 33, p99 37) | - | 60 | 1.13 | 0 | 0 |
| image4k | baseline | 30.65 (p50 30, p95 34, p99 36) | 30.71 (p50 30, p95 34, p99 36) | - | 58 | 1.82 | 0 | 0 |
| image-2layers | current | 0.76 (p50 1, p95 2, p99 2) | 0.92 (p50 1, p95 2, p99 3) | - | 62 | 0.06 | 0 | 0 |
| image-2layers | legacy | 33.76 (p50 34, p95 36, p99 36) | 33.84 (p50 34, p95 36, p99 36) | - | 63 | 1.11 | 0 | 0 |
| image-2layers | baseline | 33.68 (p50 33, p95 36, p99 37) | 33.73 (p50 34, p95 36, p99 37) | - | 62 | 1.02 | 0 | 0 |
| text | current | 0.32 (p50 0, p95 1, p99 1) | 0.43 (p50 0, p95 1, p99 1) | - | 16 | 0.08 | 0 | 0 |
| text | legacy | 3.42 (p50 3, p95 4, p99 6) | 3.49 (p50 3, p95 4, p99 6) | - | 9 | 0.16 | 0 | 0 |
| text | baseline | 3.46 (p50 3, p95 4, p99 5) | 3.55 (p50 3, p95 4, p99 5) | - | 9 | 0.89 | 0 | 0 |
| text-changing | current | 1.52 (p50 1, p95 2, p99 3) | 1.6 (p50 2, p95 2, p99 3) | - | 9 | 0.77 | 0 | 0 |
| text-changing | legacy | 3.39 (p50 3, p95 5, p99 7) | 3.54 (p50 3, p95 5, p99 7) | - | 9 | 0.53 | 0 | 0 |
| text-changing | baseline | 3.49 (p50 3, p95 5, p99 5) | 3.54 (p50 3, p95 5, p99 5) | - | 10 | 1.14 | 0 | 0 |
| image+text | current | 0.39 (p50 0, p95 1, p99 1) | 0.46 (p50 0, p95 1, p99 1) | - | 20 | 0.14 | 0 | 0 |
| image+text | legacy | 8.65 (p50 9, p95 10, p99 11) | 8.69 (p50 9, p95 10, p99 11) | - | 19 | 0.69 | 0 | 0 |
| image+text | baseline | 8.65 (p50 9, p95 10, p99 11) | 8.67 (p50 9, p95 10, p99 11) | - | 19 | 0.98 | 0 | 0 |
| video-hit | current | 0.26 (p50 0, p95 1, p99 1) | 0.44 (p50 0, p95 1, p99 1) | - | 11 | 0.17 | 0 | 0 |
| video-hit | legacy | 2.08 (p50 2, p95 3, p99 3) | 2.12 (p50 2, p95 3, p99 3) | - | 11 | 0.9 | 0 | 0 |
| video-hit | baseline | 2.32 (p50 2, p95 3, p99 4) | 2.38 (p50 2, p95 4, p99 4) | - | 12 | 0.51 | 0 | 0 |
| video-miss | current | 11.16 (p50 11, p95 12, p99 13) | 11.22 (p50 11, p95 12, p99 13) | - | 14 | 0.08 | 0 | 0 |
| video-miss | legacy | 12.66 (p50 13, p95 14, p99 14) | 12.76 (p50 13, p95 14, p99 14) | - | 11 | 0.28 | 0 | 0 |
| video-miss | baseline | 12.59 (p50 12, p95 14, p99 14) | 12.62 (p50 13, p95 14, p99 14) | - | 11 | 0.23 | 0 | 0 |
| scene | current | 0.84 (p50 1, p95 2, p99 2) | 1.01 (p50 1, p95 2, p99 3) | - | 13 | 0.13 | 0 | 0 |
| scene | legacy | 5.52 (p50 5, p95 7, p99 7) | 5.58 (p50 6, p95 7, p99 7) | - | 14 | 0.14 | 0 | 0 |
| scene | baseline | 5.55 (p50 5, p95 7, p99 8) | 5.59 (p50 6, p95 7, p99 8) | - | 13 | 0.2 | 0 | 0 |
| spark-100k | current | 7.48 (p50 7, p95 8, p99 9) | 7.55 (p50 7, p95 8, p99 9) | - | 41 | 0.09 | 0 | 0 |
| spark-100k | legacy | 11.19 (p50 10, p95 17, p99 19) | 11.3 (p50 10, p95 17, p99 19) | - | 41 | 1.61 | 0 | 0 |
| spark-100k | baseline | 15.5 (p50 16, p95 19, p99 21) | 15.53 (p50 16, p95 20, p99 21) | - | 41 | 2.54 | 0 | 0 |
| spark-100k-orbit | current | 7.53 (p50 7, p95 8, p99 9) | 7.62 (p50 8, p95 8, p99 9) | - | 39 | 0.07 | 0 | 0 |
| spark-100k-orbit | legacy | 9.89 (p50 10, p95 12, p99 15) | 10.06 (p50 10, p95 12, p99 15) | - | 40 | 0.57 | 0 | 0 |
| spark-100k-orbit | baseline | 15.2 (p50 16, p95 19, p99 20) | 15.25 (p50 16, p95 19, p99 20) | - | 39 | 0.88 | 0 | 0 |
| spark-1m | current | 34.83 (p50 33, p95 46, p99 54) | 34.97 (p50 34, p95 46, p99 54) | - | 58 | 12.59 | 0 | 0 |
| spark-1m | legacy | 33.77 (p50 33, p95 41, p99 42) | 33.88 (p50 33, p95 41, p99 42) | - | 57 | 15.71 | 0 | 0 |
| spark-1m | baseline | 40.37 (p50 39, p95 50, p99 51) | 40.4 (p50 39, p95 50, p99 51) | - | 54 | 16.32 | 0 | 0 |
| spark-1m-orbit | current | 37.81 (p50 38, p95 48, p99 54) | 37.91 (p50 38, p95 48, p99 54) | - | 54 | 8.44 | 0 | 0 |
| spark-1m-orbit | legacy | 43.92 (p50 45, p95 52, p99 61) | 44.01 (p50 45, p95 52, p99 61) | - | 53 | 13.08 | 0 | 0 |
| spark-1m-orbit | baseline | 37.64 (p50 37, p95 44, p99 54) | 37.71 (p50 37, p95 44, p99 54) | - | 57 | 13.62 | 0 | 0 |

### webkit / bridge

| case | pass | round trip mean | p95 | p99 |
|---|---|---|---|---|
| image | 1 | 0.21 | 0.42 | 0.74 |
| scene | 1 | 0.35 | 0.87 | 1.38 |
| image | 2 | 0.25 | 0.57 | 1.17 |
| scene | 2 | 0.26 | 0.5 | 1.1 |
| image | 3 | 0.18 | 0.36 | 0.5 |
| scene | 3 | 0.35 | 0.88 | 1.31 |

### webkit / hash

| case | runs | variants | frames cmp | identical | failed |
|---|---|---|---|---|---|
| image | 9 | current,legacy,baseline | 7 | true | 0 |
| image+text | 9 | current,legacy,baseline | 8 | true | 0 |
| text-changing | 9 | current,legacy,baseline | 9 | true | 0 |
| video-miss | 9 | current,legacy,baseline | 8 | true | 0 |
| scene | 9 | current,legacy,baseline | 7 | true | 0 |
| spark-100k-orbit | 9 | current,legacy,baseline | 8 | true | 0 |

### webkit / life

| case | variant | pass | swap changed | restore exact | context exact | errors |
|---|---|---|---|---|---|---|
| image | current | 1 | true | true | true | 0 |
| image | legacy | 1 | true | true | true | 0 |
| image | baseline | 1 | true | true | true | 0 |
| image-2layers | current | 1 | true | true | true | 0 |
| image-2layers | legacy | 1 | true | true | true | 0 |
| image-2layers | baseline | 1 | true | true | true | 0 |
| image+text | current | 1 | true | true | true | 0 |
| image+text | legacy | 1 | true | true | true | 0 |
| image+text | baseline | 1 | true | true | true | 0 |
| video-hit | current | 1 | true | true | true | 0 |
| video-hit | legacy | 1 | true | true | true | 0 |
| video-hit | baseline | 1 | true | true | true | 0 |
| scene | current | 1 | true | true | true | 0 |
| scene | legacy | 1 | true | true | true | 0 |
| scene | baseline | 1 | true | true | true | 0 |
| image | baseline | 2 | true | true | true | 0 |
| image | legacy | 2 | true | true | true | 0 |
| image | current | 2 | true | true | true | 0 |
| image-2layers | baseline | 2 | true | true | true | 0 |
| image-2layers | legacy | 2 | true | true | true | 0 |
| image-2layers | current | 2 | true | true | true | 0 |
| image+text | baseline | 2 | true | true | true | 0 |
| image+text | legacy | 2 | true | true | true | 0 |
| image+text | current | 2 | true | true | true | 0 |
| video-hit | baseline | 2 | true | true | true | 0 |
| video-hit | legacy | 2 | true | true | true | 0 |
| video-hit | current | 2 | true | true | true | 0 |
| scene | baseline | 2 | true | true | true | 0 |
| scene | legacy | 2 | true | true | true | 0 |
| scene | current | 2 | true | true | true | 0 |
