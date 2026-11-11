# BL157.4 — resultados da prova nativa e da réplica do laboratório

Dados gerados da checagem do `releaseLock` do decodificador do Spark. Leitura, regras e limites: [relatório §16](../../viewport-real-latency-ab-report.md#16-bl1574-deadlock-do-webkit-no-decodificador-laboratório-de-webkit-real-e-prova-nativa-pr-41-após-4c16656)
e [F063](../../features/viewport-asset-io.md#bl1574--o-deadlock-do-webkit-no-decodificador-o-laboratório-e-a-prova-nativa). **Veredito de ambas as réplicas: NÃO REPRODUZIDO** (sem poder para julgar a correção).

| Arquivo | O que é |
|---|---|
| `analysis.md` | Resumo gerado por `scripts/runtime-bundle-ab.py analyze` das 72 sessões nativas (36 pares, mesmo binário, dois `runtime.js`): por braço, plays, sessões, veredito mecânico |
| `manifest.json` | Binário, documento, SHA-256 dos dois bundles, ordem dos braços, ambiente. O campo `git.sha` é o da correção **antes** de a mensagem do commit ser reescrita (mesma árvore; hoje `e4e5aa2`); `dirty_files: 9` eram os scripts e as docs desta entrega, ainda sem commit |
| `runs.sha256` | SHA-256 dos 72 registros brutos de sessão (143 MB, não versionados) |
| `deadlock-chain.txt` | A cadeia da thread do worker da sessão presa (a 24), só com nomes de símbolos; as duas amostras do vigia dão a mesma |
| `cadence-all.md`, `cadence-by-arm.md` | `scripts/playback-cadence.py` sobre as mesmas 72 sessões (BL157.6): todas juntas e por braço |
| `lab-replication.md` | Contagens dos lotes do laboratório (`runtime/scripts/perf`, caso `real-load`), a réplica de 300 por braço e os totais intercalados (4 de 493 contra 0 de 493); SHA-256 dos registros |
| `lab-replication-rule.txt` | A regra da réplica, escrita **antes** de rodá-la |

Nada aqui contém caminhos locais, nomes de arquivos de usuário ou as nuvens do projeto real.
