### raw/webkit-heavy-current.jsonl
```
case | cold1 render | render mean | wall p50/p95/p99 | complete p50/p95/p99 | proxy/orig frames | err
image4k/current | 53.0 | 1.3 | 2.0/3.0/3.0 | 4.0/5.0/5.0 | 0/60 |
spark-3m/current | 89.0 | 1.0 | 1.0/2.0/3.0 | 3.0/4.0/4.0 | 0/60 |
spark-3m-orbit/current | 88.0 | 21.4 | 21.0/22.0/65.0 | 68.0/75.0/114.0 | 0/60 |
```
### raw/webkit-heavy-msaa0.jsonl
```
case | cold1 render | render mean | wall p50/p95/p99 | complete p50/p95/p99 | proxy/orig frames | err
image4k/msaa0 | 51.0 | 1.6 | 2.0/2.0/3.0 | 4.0/4.0/5.0 | 0/60 |
spark-3m/msaa0 | 88.0 | 1.3 | 2.0/3.0/3.0 | 3.0/4.0/5.0 | 0/60 |
spark-3m-orbit/msaa0 | 87.0 | 20.8 | 21.0/22.0/42.0 | 44.0/46.0/66.0 | 0/60 |
```
### raw/webkit-heavy-nocache.jsonl
```
case | cold1 render | render mean | wall p50/p95/p99 | complete p50/p95/p99 | proxy/orig frames | err
image4k/nocache | 52.0 | 1.4 | 2.0/3.0/3.0 | 4.0/5.0/5.0 | 0/60 |
spark-3m/nocache | 85.0 | 21.3 | 21.0/23.0/63.0 | 68.0/75.0/109.0 | 0/60 |
spark-3m-orbit/nocache | 89.0 | 21.3 | 21.0/23.0/64.0 | 69.0/83.0/112.0 | 0/60 |
```
### raw/webkit-msaa.jsonl
```
case | cold1 render | render mean | wall p50/p95/p99 | complete p50/p95/p99 | proxy/orig frames | err
spark-100k-orbit/current | 39.0 | 7.6 | 8.0/8.0/9.0 | 16.0/18.0/19.0 | 0/120 |
spark-100k-orbit/msaa2 | 39.0 | 7.5 | 8.0/8.0/9.0 | 13.0/19.0/21.0 | 0/120 |
spark-100k-orbit/msaa0 | 39.0 | 7.5 | 8.0/8.0/9.0 | 12.0/15.0/15.0 | 0/120 |
spark-1m-orbit/current | 54.0 | 12.2 | 12.0/13.0/34.0 | 35.0/38.0/63.0 | 0/90 |
spark-1m-orbit/msaa2 | 55.0 | 12.1 | 12.0/13.0/21.0 | 27.0/28.0/38.0 | 0/90 |
spark-1m-orbit/msaa0 | 56.0 | 11.9 | 12.0/13.0/18.0 | 21.0/23.0/28.0 | 0/90 |
```
### raw/webkit-prefetch-removed-experiment.jsonl
```
case | cold1 render | render mean | wall p50/p95/p99 | complete p50/p95/p99 | proxy/orig frames | err
video1080-fwd/current | 20.0 | 36.0 | 36.0/38.0/38.0 | 39.0/41.0/41.0 | 0/120 |
video1080-fwd/prefetch | 19.0 | 34.0 | 34.0/36.0/37.0 | 37.0/39.0/40.0 | 0/120 |
video1080-fwd-jpg/current | 15.0 | 8.1 | 8.0/10.0/11.0 | 9.0/12.0/13.0 | 120/0 |
video1080-fwd-jpg/prefetch | 16.0 | 5.3 | 5.0/8.0/10.0 | 7.0/9.0/12.0 | 120/0 |
video1080-rand/current | 46.0 | 36.1 | 36.0/38.0/38.0 | 39.0/40.0/41.0 | 0/120 |
video1080-rand/prefetch | 45.0 | 36.1 | 36.0/37.0/38.0 | 39.0/40.0/41.0 | 0/120 |
```
### raw/webkit-proxy.jsonl
```
case | cold1 render | render mean | wall p50/p95/p99 | complete p50/p95/p99 | proxy/orig frames | err
video1080-fwd/current | 19.0 | 35.8 | 36.0/37.0/38.0 | 39.0/40.0/41.0 | 0/120 |
video1080-fwd-jpg/current | 15.0 | 7.9 | 8.0/10.0/10.0 | 9.0/11.0/11.0 | 120/0 |
video1080-fwd-png/current | 18.0 | 10.6 | 11.0/12.0/12.0 | 12.0/13.0/13.0 | 120/0 |
video1080-rand/current | 44.0 | 36.0 | 36.0/38.0/39.0 | 39.0/41.0/42.0 | 0/120 |
video1080-rand-jpg/current | 15.0 | 7.9 | 8.0/10.0/10.0 | 9.0/11.0/12.0 | 120/0 |
video1080-rand-png/current | 19.0 | 10.7 | 11.0/12.0/12.0 | 12.0/13.0/13.0 | 120/0 |
```
### raw/webkit-spark.jsonl
```
case | cold1 render | render mean | wall p50/p95/p99 | complete p50/p95/p99 | proxy/orig frames | err
spark-100k/current | 36.0 | 1.0 | 1.0/2.0/2.0 | 3.0/4.0/4.0 | 0/120 |
spark-100k/nocache | 36.0 | 9.7 | 9.0/15.0/17.0 | 18.0/23.0/26.0 | 0/120 |
spark-1m/current | 49.0 | 1.0 | 1.0/2.0/2.0 | 2.0/4.0/4.0 | 0/90 |
spark-1m/nocache | 49.0 | 12.5 | 12.0/15.0/32.0 | 35.0/41.0/60.0 | 0/90 |
```
