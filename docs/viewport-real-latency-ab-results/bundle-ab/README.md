# BL157.4 — resultados da prova nativa e da réplica do laboratório

Dados gerados da checagem do `releaseLock` do decodificador do Spark. Leitura, regras e limites: [relatório §16](../../viewport-real-latency-ab-report.md#16-bl1574-deadlock-do-webkit-no-decodificador-laboratório-de-webkit-real-e-prova-nativa-pr-41-após-4c16656)
e [F063](../../features/viewport-asset-io.md#bl1574--o-deadlock-do-webkit-no-decodificador-o-laboratório-e-a-prova-nativa). **Veredito de ambas as réplicas: NÃO REPRODUZIDO** (sem poder para julgar a correção).
Proveniência (brutos fora do Git, o que não é versionado e por quê): [README da pasta](../README.md).

| Arquivo | O que é |
|---|---|
| `analysis.md` | Resumo gerado por `scripts/runtime-bundle-ab.py analyze` das 72 sessões nativas (36 pares, mesmo binário, dois `runtime.js`): por braço, plays, sessões, veredito mecânico. O cabeçalho cita `code b153f257b14a` (hoje `e4e5aa2`, mesma árvore; 9 arquivos ainda sem commit na época): rótulo de proveniência, não link |
| `deadlock-chain.txt` | A cadeia da thread do worker da sessão presa (a 24), só com nomes de símbolos; as duas amostras do vigia dão a mesma |
| `lab-replication.md` | Contagens dos lotes do laboratório (`runtime/scripts/perf`, caso `real-load`), a réplica de 300 por braço e os totais intercalados (4 de 493 contra 0 de 493); SHA-256 dos registros |
| `lab-replication-rule.txt` | A regra da réplica, escrita **antes** de rodá-la |

Os 72 registros brutos de sessão (143 MB) e os registros de cada lote do laboratório **não são versionados**; a lista de SHA-256 dos brutos e o manifesto da rodada não foram mantidos no Git (eram dados gerados
sem os brutos para conferi-los). A leitura de cadência das mesmas 72 sessões (BL157.6) está resumida no [relatório §16.7](../../viewport-real-latency-ab-report.md#167-bl1576-a-cadência-do-relógio-de-reprodução-e-o-agendamento-da-consulta);
`python3 scripts/playback-cadence.py <dir> --group auto --markdown <arquivo>` a regenera sobre os brutos.

Nada aqui contém caminhos locais, nomes de arquivos de usuário ou as nuvens do projeto real.
