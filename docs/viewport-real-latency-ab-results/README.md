# Resultados gerados do diagnóstico de latência real (BL157.1–BL157.6)

Tabelas **resumidas** que os runners de `scripts/` geraram a partir de sessões do editor real, mais as regras fixadas antes de medir. Leitura, hipóteses e limites:
[relatório](../viewport-real-latency-ab-report.md) e [F063](../features/viewport-asset-io.md). **A latência continua não aceita**; nada aqui fecha o BL157.

## Proveniência (leia antes de citar um número)

- Os **registros brutos de sessão não estão no Git**: são JSON por sessão, parte deles com o documento do projeto real (privado), e os 72 da prova nativa somam 143 MB. Nenhum número destas pastas pode
  ser reconferido a partir do repositório; o relatório cita **só as tabelas resumidas** daqui. Quem quiser repetir roda o runner com o seu projeto (`--project`) e a regra desta pasta.
- Não são versionados, por serem gerados e inverificáveis sem os brutos: os gráficos SVG, os `manifest.json`, as listas `*.sha256` dos brutos, as análises parciais de lote (subconjuntos de
  `analysis-all-16-sessions.md`), os lotes superados do BL157.2 (`ab1`–`ab3`, `loop1`–`loop3`, `loop5`, resumidos no [relatório §11](../viewport-real-latency-ab-report.md#11-bl1572-onde-está-o-trânsito-do-quadro)),
  a rodada estreita do BL157.3, a leitura de cadência por braço e as saídas de pixels. Os runners os regeneram (`analyze --out <dir> --project <documento> [--report-dir <pasta>]`).
- Os cabeçalhos `code <sha> dirty files N` dos relatórios gerados descrevem a árvore de quem mediu, com os commits da época: `b153f257b14a` (hoje `e4e5aa2`, só a mensagem foi reescrita) e `b5ad3b824944`
  (`fb0036f`, mesma árvore) **não são ancestrais** do head. Eles são rótulo de proveniência, não link; `dirty files` diz que parte do código ainda não estava commitada na hora.
- Nas tabelas mantidas, valores sem medida aparecem como `—` e o ruído de ponto flutuante dos geradores foi arredondado (3 casas a partir de 1 ms; 6 abaixo, a resolução da fonte). Nenhum valor foi acrescentado nem recalculado.

## Arquivos

| Arquivo | O que é | Citado em |
|---|---|---|
| `analysis-all-16-sessions.md` | BL157.1: as 16 sessões do A/B do I/O (2 rodadas), por sessão, play, gap e pausa | [relatório §4](../viewport-real-latency-ab-report.md#4-ab-no-projeto-real) |
| `bl157-2/loop4-final-analysis.md`, `bl157-2/ab4.md` | BL157.2, lote final: 4 editores novos (`loop4`) e o A/B do braço `gpu_finish` (8 pares) | [relatório §11](../viewport-real-latency-ab-report.md#11-bl1572-onde-está-o-trânsito-do-quadro) |
| `first-use/full-preupload/analysis.md`, `first-use/full-warm-draw/analysis.md` | BL157.3: o A/B de cada fator no projeto real completo (4 pares × 2 sessões) | [relatório §13](../viewport-real-latency-ab-report.md#13-bl1573-pré-upload-da-textura-png-e-warm-draw-pr-41-fb0036f--docs) |
| `bundle-ab/` | BL157.4: prova nativa dos dois `runtime.js` (72 sessões), a cadeia do deadlock e a réplica do laboratório; ver o [README](bundle-ab/README.md) | [relatório §16](../viewport-real-latency-ab-report.md#16-bl1574-deadlock-do-webkit-no-decodificador-laboratório-de-webkit-real-e-prova-nativa-pr-41-após-4c16656) |
| `poll-ab/` | BL157.6: a regra do veredito, escrita antes de qualquer sessão; ver o [README](poll-ab/README.md) | [relatório §16.7](../viewport-real-latency-ab-report.md#167-bl1576-a-cadência-do-relógio-de-reprodução-e-o-agendamento-da-consulta) |

`poll-ab/rule.txt` é a única peça que o código lê: `rule_state()` em `scripts/playback-poll-ab.py` recusa começar o A/B se o arquivo não está commitado e inalterado. Não o edite sem decidir uma nova regra
antes de qualquer medida.

Nada aqui contém caminhos locais, nomes de arquivos de usuário, mídia nem as nuvens do projeto real. Os relatórios descrevem a estrutura do projeto (quadros de corte, entradas de imagem, número de quadros) e o SHA-256 do
documento, que são o mínimo para ler as tabelas.
