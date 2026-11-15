# F065 — Duração e cortes dos planos da cena dirigida (P5, BL082.5)

Status: **P (planejado) em todas as superfícies.** Esta entrega (S0, BL082.5.1) é só contrato, auditoria e oráculo;
nenhum código de `crates/` mudou e nenhum controle está habilitado. O plano é
[p5-shot-duration-plan.md](../migration/p5-shot-duration-plan.md); o relatório da S0 é
[p5-s0-report.md](../migration/p5-s0-report.md).

## Objetivo e limites

A pessoa (ou um agente) muda a duração de um plano, move o corte entre dois planos vizinhos e muda a duração da edição
inteira, sem regerar o bundle, com o mesmo resultado pela lib, pela CLI sem janela e pela UI. Os planos são os `shots[]`
da cena dirigida importada de um render-bundle 1.0; a origem é o piloto de cortes do Tessel (decisões #1–#34 em
[scene-shot-trim-decisions.md](../migration/origin-specs/scene-shot-trim-decisions.md) e
[scene-shot-trim-plan.md](../migration/origin-specs/scene-shot-trim-plan.md)).

Fora do escopo da P5: dividir, apagar, reordenar ou inserir planos, `slip`, câmera contínua entre clipes diferentes,
estender ou gerar áudio, trilha "Planos" arrastável (P6), mover narração (M5), escopo `node` (BL082.40).

Hoje **não existe** implementação: este documento fixa o contrato que a S1–S5 devem cumprir e declara o que o oráculo
da S0 prova e o que não prova (seção final).

## Contrato do core / biblioteca

### Operações

| Op (JSONL) | Parâmetros | Efeito |
|---|---|---|
| `trim_shot` | `shot`, `edge: "start"\|"end"`, `frame`, opções | move a borda para `frame` (quadro da edição; `end` é exclusivo) |
| `move_cut` | `from`, `to`, `frame`, opções | **roll**: o corte entre dois planos vizinhos vai para `frame`; a edição não muda de tamanho |
| `set_shot_duration` | `shot`, `frames`, opções | atalho de `trim_shot edge:"end"` |
| `set_edit_duration` | `frames`, opções | alonga ou encurta **só o último** plano (#21) |

Um lote é uma lista de ops aplicadas em ordem, cada uma sobre o candidato que a anterior deixou, e é **atômico**: uma
recusa deixa o documento exatamente como entrou.

### Opções (#19)

| Opção | Valores | Regra |
|---|---|---|
| `mode` | `ripple` (padrão), `roll` | `ripple` move os planos seguintes e muda a edição; `roll` mantém a edição |
| `content` | `anchor`, `follow`, `auto` (padrão) | o que acontece com o conteúdo da cena quando o lado de entrada vai para antes (#28–#32); `auto` ancora a cabeça que existe menos a reserva da transição de entrada (#29, leitura `AUTO_RESERVE` abaixo) e segue o resto; sob `editLocked`, `auto` toma a primeira política que mantém o passo: a sua mistura, depois ancorar tudo (o `move_cut` da #30), depois seguir tudo (o ripple da cabeça, onde ancorar muda o passo) (#10, #30) |
| `keep_absolute` | bool | clipes `output`, keys de luz e `weatherKeys` ficam nos quadros absolutos em vez de andar pela emenda (#12, #15) |
| `shrink_transitions` | bool | encolhe as transições presas ao corte em vez de recusar com `TRANSITION_TOO_LONG` (D10) |
| `with_narration` | bool | **sempre recusado** nesta fase com `SHOTS_OPTION_NOT_YET` (D13): narração é M5 |

### Regras fixadas pela S0

- **Instância cresce, nunca encolhe (#6).** Esticar um plano além da instância aumenta `durationFrames`, com a nota
  `INSTANCE_GREW`. O ponto de partida da instância e da câmera é o do plano; o relógio da cena não muda (#5).
- **Cadeia de emendas (#31).** Cada op vira uma lista de inserções `ins(at, n)` e remoções `del(a, b)`; toda camada em
  quadros da edição segue a mesma cadeia. Um ponto dentro do trecho removido é descartado; um ponto em `at` ou depois
  se move. A emenda fica no fim do take (#8).
- **`limitFrame` (#33)** é sempre um quadro da edição, no relógio da própria op: aplicar a op nesse quadro não recusa
  pelo mesmo código (propriedade T21, checada pelo corpus com um gêmeo `@limit` por recusa com limite).
- **`MAX_EDIT_FRAMES = 216000`** (#18): ultrapassar recusa com `EDIT_TOO_LONG` e `limitFrame`.
- **Junção contínua (#8, #30):** `follow` para dentro de uma junção contínua é `CONTINUOUS_JOIN`; `auto` nunca cai para
  `follow` numa junção contínua de entrada.
- **Clipes (D09):** clipe `shot` anda por `contentShift = Δ(startFrame − instanceStartFrame)`, é cortado nas bordas do
  plano e removido, com nota, se ficar vazio; clipe `output` segue a cadeia; keys por caso (anda inteiro = intactas;
  cabeça cortada = deslocadas e podadas abaixo de 0; cauda cortada = podadas no fim ou depois); o fade de entrada é
  reduzido primeiro. A "cabeça esticada" do plano (D09) não acontece: pela regra `BOUNDARY`, um clipe que começa no
  ponto de inserção anda inteiro. Nenhum `KEY_OUT_OF_CLIP` nem `FX_SCOPE_MISMATCH` pode sair.
- **Keys de luz e de clima (#15, D11):** depois do ponto andam pelo delta; dentro do trecho removido caem, com nota;
  lista vazia é removida (a trilha vazia de uma luz remove também o objeto `keys`).
- **Narração e áudio nunca andam** (D13, D14). `explain` avisa quando a edição passa do áudio. O áudio dura na edição
  `editEndSecondsExclusive − editStartSeconds` (o tempo `t` da edição toca a fonte em `editStartSeconds + t`).

### Respostas

Sucesso: `{changed, notes[], revision}`. Notas: `INSTANCE_GREW`, `AUTO_FELL_TO_FOLLOW`, `CLIP_REMOVED`, `KEYS_PRUNED`,
`FADE_REDUCED`, `TRANSITION_SHRUNK`, `LIGHT_KEYS_DROPPED`, `LIGHT_TRACK_REMOVED`, `WEATHER_KEYS_DROPPED`,
`WEATHER_KEYS_REMOVED`.

Recusa: `{code, index, detail, limitFrame?}` e o documento intacto. Códigos (D19):

| Código | Quando | `limitFrame` |
|---|---|---|
| `SHOT_TOO_SHORT` | o plano ficaria com menos de 1 quadro | sim |
| `SHOT_NOT_FOUND` | id de plano inexistente | — |
| `SHOTS_NOT_ADJACENT` | `move_cut` entre planos que não são vizinhos | — |
| `EDIT_TOO_LONG` | a edição passaria de 216000 quadros | sim |
| `LOCKED` | a op tiraria de passo dois planos de uma instância `editLocked` que estavam em passo: ripples que deslocam um deles, `follow` no `move_cut`, `anchor` no ripple da cabeça | — |
| `CONTINUOUS_JOIN` | a op cortaria a cabeça de quem continua o plano anterior, ou pediria `follow` para dentro de uma junção contínua | — |
| `NO_HEAD_FRAMES` | `anchor` sem quadros de cabeça | sim |
| `TRANSITION_TOO_LONG` | a transição presa ao corte não cabe (reaproveita `fx/transition_problem.rs`) | sim |
| `SHOTS_LAYER_NOT_YET` | campo adiado com tempo na edição em outra forma (D12); **hoje inalcançável**, ver auditoria | — |
| `SHOTS_OPTION_NOT_YET` | `with_narration: true` | — |
| `CANDIDATE_REJECTED` | o portão final reprova o candidato, com os achados | — |

Ordem em que o oráculo reporta duas regras que recusam a mesma op: `SHOTS_OPTION_NOT_YET`, `SHOT_NOT_FOUND`,
`SHOTS_NOT_ADJACENT`, `SHOTS_LAYER_NOT_YET`, `SHOT_TOO_SHORT`, `CONTINUOUS_JOIN`, `NO_HEAD_FRAMES`, `EDIT_TOO_LONG`,
`LOCKED`, `TRANSITION_TOO_LONG`, depois o portão. A origem não fixa essa ordem: é escolha do oráculo, a confirmar na S1.

### `scene.shots.explain`

Leitura (D16), sem efeito: `{edit, shots[], audio, warnings[]}`. Cada plano traz `id`, `intent`, `start`, `end`,
`frames`, `instance`, `instance_start`, `camera_clip`, `camera_in`, `locked`, `transition_into`
(o `type` da junção de entrada, `"cut"`, `"continuous"` ou o de uma transição; `null` no primeiro plano) e `handles` (`scene_before/after`, `camera_before/after`: quadros
disponíveis para esticar cada lado). `audio` traz `frames`, `edit_start_seconds` e `edit_end_seconds_exclusive`;
`warnings` usa `EDIT_SHORTER_THAN_AUDIO` e `EDIT_LONGER_THAN_AUDIO` (tolerância de um quadro), comparando a edição com
`frames` = (`editEndSecondsExclusive − editStartSeconds`) × fps. O nó `scene.shots` da
janela devolve o mesmo JSON.

### Pré-condições e invariantes

- Há cena dirigida com `shots[]`; a ausência dela é decidida pelos adaptadores (S3/S4), fora do planejador puro.
- Depois de uma op aceita, os planos cobrem `[0, editDurationFrames)` sem lacuna nem sobreposição, nenhum plano passa
  da sua instância, nenhum início fica negativo, e as camadas passam o portão do candidato.
- Em recusa, no-op e `dry_run`, o documento não muda; no-op não cria entrada de desfazer (`edit_fx`).

## Auditoria dos campos adiados (D12)

Auditoria da lista `DEFERRED_FIELDS` (`crates/core/src/directed_scene/mod.rs:81-100`) exceto
`sceneDefinitions[].lighting`, que é a D11. Cada linha cita o arquivo da origem no pin `d6adac15` (relido por id de
blob; `scripts/shot-trim-reference.py --verify-origin`). O relógio é o do dado: *quadros da edição* andam pela cadeia
de emendas; *quadros da instância* não andam (#5); *nenhum* quer dizer que não há tempo no dado.

| Campo | Relógio | Forma | Destino | Origem (`d6adac15`) |
|---|---|---|---|---|
| `weatherKeys` | quadros da edição | `weatherKeys[].frame`, inteiro de 0, estritamente crescente; `{frame, value, easing}` | **remapeia como dado simples** (#15: depois do ponto pelo delta; descartado no trecho removido; lista vazia removida) | `schema/render-bundle.schema.ts:133-147, 260-267`; `runtime/three-scene-runtime.ts:1373-1381`; `validate/bundle-integrity.ts:570-575`; `keyframes/keyframe-rules.ts:10-44` |
| `rigs` | quadros da instância | `constraints[].target.keys[].frame` e `follows[].progress[].frame` | fica (relógio da cena, #5; quadros além da trilha seguram) | `rig/rig-schema.ts:43-49, 68, 79`; `rig/follow-schema.ts:12-18`; `rig/rig-targets.ts:89-93`; `runtime/rig-layer.ts:72` |
| `nodes[].reveal` | quadros da instância (no `follow` que nomeia) | `{rigId, followId}` | fica | `rig/follow-schema.ts:37-44`; `schema/render-bundle.schema.ts:202` |
| `narration` | segundos da edição | `tracks[].clips[].startSeconds/durationSeconds`, `fromMix` | fica (só andaria com `with_narration`, recusado por D13) | `narration/narration-schema.ts:4-5, 24-25, 49-58` |
| `weather` | nenhum (nome com intensidade) | `"snow:0.6"`; a neve integra a intensidade de cada quadro anterior, então o que se desenha depois do ponto muda de propósito | fica | `schema/render-bundle.schema.ts:250-259`; `weather/weather-keys.ts:36-58` |
| `effects` | nenhum (a semente lê o quadro da edição) | quantidade 0..1 por olhar | fica | `effects/scene-effects.ts:4-7, 19-37` |
| `outline` | nenhum | grupos e itens: ids, rótulos, ordem | fica (#23) | `outline/outline-schema.ts:28-54` |
| `metadata` | nenhum (só `pilotFrameAudit` é lido, para recusar) | registro livre | fica | `schema/render-bundle.schema.ts:515`; `validate/bundle-integrity.ts:896-900` |
| `contactPlaneId` | nenhum | id de nó | fica | `schema/render-bundle.schema.ts:246` |
| `frameAudit` | nenhum ("frame" é o quadro da imagem) | contatos, grupos coplanares, ordem de desenho, margem em px e unidades de cena | fica | `schema/render-bundle.schema.ts:211-229, 247` |
| `weatherGround` | nenhum | `{nodeId, top}` | fica | `schema/render-bundle.schema.ts:284-290` |
| `paths` | nenhum (o progresso da pena está no `follow` do rig) | linhas em px da imagem de uma camada | fica | `path/path-schema.ts:3-6, 28-38`; `rig/follow-schema.ts:12-26` |
| `nodes[].surface` | nenhum | mapa e câmera/chão | fica | `schema/render-bundle.schema.ts:63-84, 192` |
| `nodes[].normalMap` | nenhum | mapa e fonte | fica | `schema/render-bundle.schema.ts:103-110, 196` |
| `nodes[].sceneDepth` | nenhum | grade de profundidade de um splat e sua câmera | fica | `schema/render-bundle.schema.ts:86-101, 194` |
| `nodes[].relief` | nenhum | grade de profundidade de um plano e sua câmera | fica | `schema/render-bundle.schema.ts:112-131, 198` |
| `nodes[].rigBind` | nenhum | `{rigId, mode, boneId?, blendPx?}` | fica | `rig/rig-schema.ts:83-91`; `schema/render-bundle.schema.ts:200` |

**Conclusão da auditoria.** Um único campo da lista é remapeado como dado simples: `weatherKeys`. Nenhum campo tem
tempo na edição em outra forma nem relógio incerto, então `SHOTS_LAYER_NOT_YET` não é alcançável a partir da lista de
hoje: o código fica no contrato para um campo adiado novo (a lista pode crescer), mas a S1/S2 não têm caso de recusa
dele. A D12 se mantém como decidida (Q4), com a lista fixada: `weatherKeys` remapeia; o resto fica.

## D07 / Q2 — quadros da instância que cresce

**Conclusão: reais.** Os quadros acrescentados pela #6 são avaliados como quadros reais de uma instância mais longa; a
origem não "segura o último quadro" da instância inteira. **A Q2 não é disparada:** o comportamento que o plano adotou
(D07) é o da origem, sem divergência para o dono decidir.

Evidência:

1. O avaliador nunca limita o quadro da instância do próprio plano: passa o quadro real às sobreposições e às camadas
   de rig e de reveal, e a regra de integridade faz a instância crescer para que o quadro nunca passe do fim
   (`evaluator/evaluate-edit-frame.ts:70-74, 83-87`; `runtime/three-scene-runtime.ts:1430-1438`;
   `validate/bundle-integrity.ts:817-828`; pin `d6adac15`).
2. `heldInstance` só existe na ponta da branch de efeitos (`3655790`), em `resolveShotAtOffset`: limita a
   `[0, durationFrames − 1]` e só é verdadeiro quando isso moveu o quadro; quem o lê é o **outro deck** de uma transição.
   Dentro do alcance do plano ele responde o mesmo que `resolveShotAtEditFrame`
   (`edit/resolve-shot-at-offset.ts:16-24, 35-44, 54-86`; `edit/resolve-shot-at-offset.spec.ts:89-101`;
   `docs/video-production/scene-fx-plan.md:369-376, 578-590`).
3. Quem segura por conta própria são os canais: `cycle` continua com módulo do quadro da instância (não segura),
   `event` segura o valor final por desenho, a pose baked do rig e o reveal seguram depois do fim da própria trilha
   (`performance/resolve-performance.ts:17-30, 64-82, 84-100`; `schema/render-bundle.schema.ts:308-317`;
   `runtime/rig-layer.ts:72-76`; `rig/rig-targets.ts:89-93`).

4. O próprio efeito colateral que a #6 aceita aponta para quadros reais: a instância maior aumenta os `handles` dos
   outros planos que a usam, e as transições `use`/`refuse` só contam quadros **reais** como handle
   (`fx-explain-transitions.ts:115-120`, na ponta `3655790`). Se os quadros novos fossem segurados, não seriam handle.

Ressalvas: a decisão #6 diz "o runtime segura o último quadro … `heldInstance`", o que lido solto sugere um *hold* da
instância; o código não faz isso. O código do piloto de #6 (`packages/scene-engine/src/shots/`) não está no clone
(`ORIGIN_NOT_EXECUTED`), então a leitura é do texto e do código ao redor. O oráculo não avalia quadros: D07 não muda
nenhum caso do corpus; ela pesa sobre a prova de pixels (T23, S5). O texto literal da #6 fica registrado para o dono no
relatório da S0, mesmo sem disparar a Q2.

## CLI headless

Planejado (S3): `./scripts/headless` com
`{"op":"scene.shots.edit","scene":"…","ops":[{"trim_shot":{…}}],"dry_run":false}` e
`{"op":"scene.shots.explain","scene":"…"}`. `scene.capabilities` e `scope.rs` só listam `shots.edit` em `validated`
quando a S3 fechar. **Gap atual:** nada disso existe; a S3 o implementa.

## UI e CLI da janela

Planejado (S4): seção "Planos" no painel da cena (`workstation/scene_shots.rs`), com componentes do Kit.

| Controle | Comando | Nó (`./scripts/ui`) |
|---|---|---|
| lista dos planos | `scene.shots.explain` | `scene.shots` (`get`) |
| campo "Duração" por plano | `set_shot_duration` | `scene.shot:<id>` (`set duration`) |
| campo "Corte" entre dois planos | `move_cut` | `scene.shots.cut:<from>:<to>` (`set frame`) |
| campo "Duração da edição" | `set_edit_duration` | `scene.shots` (`set edit_duration`) |
| "Modo" e "Conteúdo" | opções das ops | `scene.shots` (`set mode`, `set content`) |

Sem cena dirigida os controles ficam desabilitados com `disabled_reason`. Sem arrasto (D20) e sem diálogo novo.
Apresentação do arrasto e da trilha "Planos": N/A nesta fase (P6).

## Evidências e status

| Superfície | Status | Evidência hoje |
|---|---|---|
| Lib | P | só o oráculo da S0 (abaixo); nenhuma linha de Rust |
| CLI headless | P | — |
| UI | P | — |
| CLI da janela | P | — |

### O que o oráculo é e o que não é

`scripts/shot-trim-reference.py` (Python, só biblioteca padrão, 3.9 a 3.13) implementa as quatro ops, as opções e a
cadeia de emendas **a partir do texto** das decisões e dos arquivos da origem, sem ler o código Rust. Gera
`docs/migration/fixtures/shot-trim/cases.json` (167 casos: 160 de ops, 7 de `explain`) e `manifest.json` (SHA-256 dos
documentos e das fontes lidas, versão do gerador, `ORIGIN_NOT_EXECUTED`). `--check` regenera e compara byte a byte sem
escrever; `--verify-origin PATH` relê os blobs da origem por id. O spec
(`scripts/shot-trim-reference.spec.py`) tem vetores à mão, a propriedade T21 com semente fixa e verificador de validade
independente, controles negativos (mutantes) e higiene.

**É:** uma transcrição, escrita por quem não leu o Rust, que a S1 e a S2 devem reproduzir; onde o texto da origem é
ambíguo, a escolha está nomeada em `oracle_choices` (`AUTO_RESERVE`, `BOUNDARY`, `CHAIN_MIDDLE`, `EMPTY_CONTAINER`,
`GATE`, `LOCKED_AUTO`, `REFUSAL_ORDER`, `TRANSITION_KEYS`, `AUDIO_WARNING`).

**Não é:** a saída do piloto. `ORIGIN_NOT_EXECUTED`: `src/shots/` não existe no clone da origem (o pin `d6adac15` nem
tem `src/fx/`), então nenhum caso foi comparado com código da origem. Também não modela o portão de pose da junção
contínua (`CONTINUOUS_*_JUMP`, T06, só na S1), nem a recusa `Ok → Err` do `Lighting::read` (D11), nem pixels nem export
(T23/T24). Um caso do corpus que discorde da origem no Mac é um achado a corrigir, não uma verdade.

**`AUTO_RESERVE`, decidido pelo coordenador (2026-11-15):** a #29 escreve `a = min(d, instanceStartFrame, cameraInFrame)`
menos a reserva da transição. Vale a leitura do oráculo, `a = max(0, min(d, min(instanceStartFrame, cameraInFrame) −
reserva))`: a reserva sai dos quadros de cabeça que **existem**. É a leitura que cumpre o propósito da #29 (guardar a
cabeça que a transição de entrada desenha e ancorar o resto, como a #28 manda). A leitura literal reservaria demais
sempre que `d` é menor que a cabeça e faria o resultado pular quando uma transição `hold` vira `use`. Os casos marcados
`AUTO_RESERVE` no corpus mostram a diferença.

Tabela de cobertura por linha T da matriz do plano: ver [p5-s0-report.md](../migration/p5-s0-report.md).
