# P5 / S0 — relatório (BL082.5.1)

Entrega da fatia S0 da [P5](p5-shot-duration-plan.md): contrato F065, auditoria dos campos adiados, conclusão da D07,
referência Python independente, corpus e spec. Só docs e Python da biblioteca padrão; nada em `crates/`, `runtime/` nem
em produção. Status do produto: **planejado (P)**; nenhuma superfície foi tocada.

## O que foi feito

| Entrega | Arquivo |
|---|---|
| Contrato, auditoria, D07 e o que o oráculo é | [`docs/features/shot-duration.md`](../features/shot-duration.md) (F065) |
| Referência (`trim_shot`, `move_cut`, `set_shot_duration`, `set_edit_duration`, `explain`) | `scripts/shot-trim-reference.py` |
| Corpus gerado pela referência | `docs/migration/fixtures/shot-trim/cases.json` (163 casos) e `manifest.json` |
| Spec (unittest) | `scripts/shot-trim-reference.spec.py` |
| Este relatório | `docs/migration/p5-s0-report.md` |

A referência partiu de um rascunho interrompido (1705 linhas, nunca executado), conferido contra os dois documentos da
origem e os arquivos do schema/validação, executado e corrigido. Ela não leu Rust, exceto a lista `DEFERRED_FIELDS`
(`crates/core/src/directed_scene/mod.rs:81-100`); a única leitura de `crates/` está no spec (`AuditTests`), que compara
a lista com a auditoria.

## Cobertura do corpus por linha da matriz (§7 do plano)

Total: 163 casos = 157 de ops (119 aceitos, 38 recusados) + 6 de `explain`. Um caso pode cobrir mais de uma linha. Cada
recusa com `limitFrame` tem um gêmeo `@limit` (17 gêmeos) com a op aplicada nesse quadro; o gerador falha
(`ORACLE_LIMIT_MISMATCH`) se o gêmeo recusar de novo pelo mesmo código. Seis lotes, seis casos com `keep_absolute`,
quatro com `shrink_transitions`, os três valores de `content` (explícitos: 18 `anchor`, 13 `follow`; o resto usa o
padrão `auto`).

| T | Casos | aceitos / recusados | Observação |
|---|---|---|---|
| T01 | 17 | 15 / 2 | |
| T02 | 23 | 18 / 5 | |
| T03 | 37 | 29 / 8 | `anchor`, `follow`, `auto`; `AUTO_RESERVE` |
| T04 | 13 | 9 / 4 | |
| T05 | 14 | 11 / 3 | `CONTINUOUS_JOIN` |
| T06 | 0 | — | só S1 (portão de pose, D05): fora do oráculo |
| T07 | 11 | 5 / 6 | `LOCKED` |
| T08 | 6 | 6 / 0 | sem recusa: a regra é nunca recusar (instância cresce) |
| T09 | 9 | 6 / 3 | `NO_HEAD_FRAMES` |
| T10 | 16 | 5 / 11 | |
| T11 | 8 | 8 / 0 | sem recusa na regra (D09) |
| T12 | 13 | 10 / 3 | `keep_absolute`; recusa pelo portão |
| T13 | 11 | 11 / 0 | |
| T14 | 17 | 12 / 5 | `TRANSITION_TOO_LONG`, `shrink_transitions` |
| T14b | 5 | 3 / 2 | reserva da transição de entrada (#29) |
| T15 | 8 | 8 / 0 | sem `Ok → Err` do leitor (não modelado) |
| T16 | 5 | 5 / 0 | só `weatherKeys` (a lista não tem outro campo a recusar) |
| T17 | 2 | 1 / 1 | `SHOTS_OPTION_NOT_YET` |
| T18 | 0 | — | S3 (lote atômico de adaptador) |
| T19 | 6 | `explain` | handles, travado, avisos de áudio |
| T20 | 0 | — | S3 |
| T21 | 17 | 17 / 0 | casos `@limit` mais a propriedade do spec |
| T22–T24 | 0 | — | fora do oráculo (catálogo, pixels, export) |

Recusas por código no corpus: `SHOT_TOO_SHORT` 6, `LOCKED` 5, `CANDIDATE_REJECTED` 5, `TRANSITION_TOO_LONG` 5,
`SHOTS_NOT_ADJACENT` 4, `CONTINUOUS_JOIN` 3, `NO_HEAD_FRAMES` 3, `EDIT_TOO_LONG` 3, `SHOT_NOT_FOUND` 3,
`SHOTS_OPTION_NOT_YET` 1. `SHOTS_LAYER_NOT_YET` tem **zero** casos: a lista de adiados não tem campo que o acione.

## D07 / Q2

Os quadros acrescentados pela instância que cresce são avaliados como **reais**; `heldInstance` só existe na ponta da
branch de efeitos e vale para o outro deck de uma transição. **A Q2 não é disparada.** Evidência com arquivo e linha na
seção D07 de [F065](../features/shot-duration.md#d07--q2--quadros-da-instância-que-cresce). Ressalva: o código do piloto
de #6 não está no clone (`ORIGIN_NOT_EXECUTED`).

## Auditoria (resumo)

17 campos auditados (todos de `DEFERRED_FIELDS` menos `lighting`); tabela completa com formas e linhas em F065.

| Destino | Campos |
|---|---|
| remapeia como dado simples (quadros da edição) | `weatherKeys` |
| fica, relógio da instância | `rigs`, `nodes[].reveal` |
| fica, segundos da edição (não anda por D13) | `narration` |
| fica, sem tempo no dado | `weather`, `effects`, `outline`, `metadata`, `contactPlaneId`, `frameAudit`, `weatherGround`, `paths`, `nodes[].surface`, `nodes[].normalMap`, `nodes[].sceneDepth`, `nodes[].relief`, `nodes[].rigBind` |
| recusa com `SHOTS_LAYER_NOT_YET` | nenhum |

A auditoria **muda a D12 na prática**: `SHOTS_LAYER_NOT_YET` é inalcançável com a lista de hoje (o código fica no
contrato para um campo futuro).

## Comandos e resultados

Interpretador: Python 3.13.17 de `%LOCALAPPDATA%\Programs\Python\Python313` (não o stub da Loja). Não foi rodado cargo.

| Comando | Saída | Exit |
|---|---|---|
| `python scripts/shot-trim-reference.spec.py` | 40 testes, OK | 0 |
| `python -O scripts/shot-trim-reference.spec.py` | 40 testes, OK | 0 |
| `python scripts/shot-trim-reference.py --check` | `check ok: 163 cases, 2 files reproduced byte for byte, nothing written` | 0 |
| `python scripts/shot-trim-reference.py --verify-origin <clone local da origem>` | `origin blobs verified (32); the origin was NOT executed` | 0 |
| `python scripts/gizmo-perf/hygiene.spec.py` | 13 testes, OK | 0 |
| `python scripts/backlog-tables.spec.py` | 19 testes, OK | 0 |

O spec tem quatro camadas: vetores à mão (39 expectativas escritas como literais; #3, #6, #8–#13, #15, #16, #18, #21,
#28, #33, D09); a propriedade T21 (sementes 20261115 com 220 sequências e 7 com 120) contra um verificador de validade
escrito separadamente da referência; 14 mutantes (um ripple que não move os seguintes, uma cauda que não poda keys,
etc.), todos reportados, mais um controle com o portão da referência cego (o verificador independente o pega porque
`keep_absolute` deixa documentos inválidos); e corpus, manifesto, auditoria contra `DEFERRED_FIELDS`, `--check` com
adulteração, e higiene (sem `assert` nu, sem dado de máquina, sintaxe 3.9 por AST).

## Limites e pontos abertos

1. **`ORIGIN_NOT_EXECUTED`.** O piloto (`src/shots/`) não existe no clone da origem; o oráculo é transcrição, nunca
   saída da origem. Os arquivos de efeitos (`fx`, `resolveShotAtOffset`, `heldInstance`) só existem na ponta
   `3655790` da branch `t3/scene-engine-pilot`; foram lidos lá, além do pin `d6adac15` (declarado no manifesto, flags
   `PIN_HAS_NO_FX` e `PLAN_COMMANDS_ABSENT_FROM_CLONE`).
2. **`AUTO_RESERVE` (ambiguidade da #29).** O texto escreve `min(d, instanceStartFrame, cameraInFrame)` menos a reserva;
   o oráculo tira a reserva dos quadros de cabeça que existem. Os casos marcados `AUTO_RESERVE` mostram a diferença. O
   coordenador deve decidir antes da S1 (não há código a executar).
3. Escolhas do oráculo onde a origem não diz (`oracle_choices` no corpus): fronteira dos pontos, emenda no meio de um
   clipe, container vazio de luz, ordem de recusas, `LOCKED` com `auto`, chaves de transição, aviso de áudio.
4. **Não modelado:** o portão de pose da junção contínua (`CONTINUOUS_*_JUMP`, T06), a recusa `Ok → Err` de
   `Lighting::read` (D11), pixels e export (T23/T24), `NARRATION_PAST_EDIT`.
5. **Sem recusa no corpus** em T08, T11, T13, T15 e T16: ou a regra não recusa, ou o recusar depende do que o oráculo
   não modela (T15). T06, T18, T20, T22–T24 ficam para as fatias seguintes.
6. **Python 3.9** só é verificado por AST (`ast.parse(feature_version=(3, 9))`); não há 3.9 neste Windows.
7. **Desvio de branch:** `feat/p5-s0-shot-trim-contract` já existia e estava em uso no worktree do rascunho
   interrompido; usei `feat/p5-s0-shot-trim-contract-r2`, a partir de `origin/docs/p5-shot-duration-plan` (`de83087`).
   O worktree do rascunho não foi tocado.
8. Nada foi enviado nem aberto como PR: a revisão e a publicação são do coordenador.

## Patch para o coordenador

Não editei `docs/backlog.md`, `docs/features/README.md` nem a matriz.

### Linha de F065 em `docs/features/README.md`

Colunas: UI, CLI da janela, CLI headless, Lib, entrada compartilhada (status P, pendente).

```
| F065 | [Duração e cortes dos planos da cena dirigida (P5, BL082.5)](shot-duration.md) | P | P | P | P | `trim_shot`, `move_cut`, `set_shot_duration`, `set_edit_duration` (opções `mode`, `content`, `keep_absolute`, `shrink_transitions`; `with_narration` recusado) e `scene.shots.explain`, ainda sem código. S0 entregue: contrato, auditoria dos 17 campos adiados (só `weatherKeys` remapeia; `SHOTS_LAYER_NOT_YET` inalcançável com a lista de hoje), D07 conferida (quadros reais; Q2 não disparada), referência Python `scripts/shot-trim-reference.py` e corpus de 163 casos com `--check`, spec 40 testes normal e `-O`; `ORIGIN_NOT_EXECUTED` (o piloto não está no clone). Ponto aberto: `AUTO_RESERVE` (#29). |
```

### `docs/backlog.md`

`BL082.5.1` (linha do plano): status **A validar** (implementado e verificado nesta rodada, aguarda a revisão do
coordenador; não marcar Concluído antes dela). Acrescentar à coluna de aceite/detalhes:
"Entregue em `feat/p5-s0-shot-trim-contract-r2` (commits `039ebce`, `1153176` e o dos docs): F065
(`docs/features/shot-duration.md`), `scripts/shot-trim-reference.py` com `--check` e `--verify-origin`, spec 40 testes
normal e `-O`, corpus 163 casos, auditoria dos campos adiados; Q2 não disparada (D07: quadros reais);
`ORIGIN_NOT_EXECUTED`; pendente: decidir `AUTO_RESERVE` (#29) antes da S1."

`BL082.5.3` (S2): a auditoria **muda a D12**. Trocar "remapeamento dos campos adiados" por "remapeamento de
`weatherKeys` (único campo da lista com tempo na edição; mesma regra das keys de luz, #15)"; remover de T16 o ramo
`SHOTS_LAYER_NOT_YET` (sem caso na lista de hoje, o código fica no contrato). Em `BL082.5.2` o aceite "recusas D12/D13"
passa a "recusa D13 (`SHOTS_OPTION_NOT_YET`); `SHOTS_LAYER_NOT_YET` só existe como código, sem caso".

Novo item sugerido (uma linha, ID novo a atribuir pelo coordenador): decidir a leitura de `AUTO_RESERVE` da #29;
dependência BL082.5.1; aceite: a decisão registrada em F065 e o corpus regenerado com `--check`.
