# BL157.4: laboratory batches of the `releaseLock` check

Counts read from the saved `webkit-real.jsonl` of each batch (a run is stuck when its asset load never answered within `--load-timeout`; no run is dropped). `prev` is the bundle without the fix, `current` the one with it. The raw records are not published; their SHA-256 is below.

| batch | interleaved | runs per arm | stuck without the fix | stuck with the fix | load1 median (p10-p90) | note |
|---|---|---|---|---|---|---|
| loadmany | yes | 28 | 2 | 0 | 10.1 (6.7-10.7) |  |
| loadmany2 | yes | 45 | 1 | 0 | 5.4 (5.0-7.4) |  |
| loadfix | yes | 120 | 1 | 0 | 7.5 (6.8-12.7) |  |
| loadrep | yes | 300 | 0 | 0 | 9.7 (7.1-12.8) | pre-registered replication (rule in lab-replication-rule.txt) |
| load120 | no | 120 | 8 | - | 35.4 (18.2-50.6) | one variant (the build without the fix), no control arm |

Pooled interleaved batches: **4 of 493 stuck without the fix, 0 of 493 with it**; exact one-sided hypergeometric p = 0.0621 (diagnostic only).

Replication verdict by the pre-fixed rule (`loadrep`): the build without the fix had fewer than 3 stuck runs -> NOT REPRODUCED.

| batch | SHA-256 of `webkit-real.jsonl` |
|---|---|
| loadmany | `d6c308f80004ad5e089fc3ad7c517e0c7ff7cdbeb672c32e3ab4d37917354c11` |
| loadmany2 | `5e2e57079f762f4f30108bd311b6e1532291b969ccb7dee6d8bf32327d924e4d` |
| loadfix | `c4a9b8715a6f98dbf45a81a3303cacf7d53c4d7f91c3c566c6c557ffbde8b34e` |
| loadrep | `d79851b2489cc204f9e6cfb4cbc8928179a9065aa4eee5a25b3ec5d847a65437` |
| load120 | `43e88378def5195c85319702e0cadd70c5e600c54adba365a2bb38a0a9bff71d` |
