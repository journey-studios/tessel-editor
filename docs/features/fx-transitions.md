# F064 — Transições entre planos: `crossDissolve` (BL082.4, P4b)

## Objetivo e limites

`fx.transitions[]` veste um **corte** entre dois planos vizinhos com uma janela de quadros em que o monitor e o export
misturam os **dois decks** (a imagem de cada plano antes da saída) em luz linear. O único efeito de transição da
primeira entrega é `crossDissolve`. Quem calcula é o core; o plano carrega o resultado; o runtime só desenha.

**Estado desta entrega (S0, BL082.4.1): somente contrato e oráculo.** Existem este documento, o corpus independente
(`scripts/pilot-dissolve-corpus.py`, `dissolve-cases.json`, `dissolve-manifest.json`) e a spec Python. **Não existe
Rust, JSONL, runtime nem UI de transição.** Hoje qualquer `transitions` não vazio continua `FX_TRANSITION_NOT_YET`; o
catálogo, o seletor de clipe e os planos **não mudam**. Plano de trabalho (versão 2):
[p4b-cross-dissolve-plan.md](../migration/p4b-cross-dissolve-plan.md), lido no head `2395719`. Um contrato em
documento não é feature implementada.

Fica fora (cada item é BL próprio): as outras transições do piloto, `keys` em transições (D10,
`FX_TRANSITION_KEYS_NOT_YET`, BL082.4.9), escopo `node`, ripple/roll, o dissolve legado (`transitionInto: dissolve`,
faixa S6 própria: nada do acervo antigo é editado), narração e `TRANSITION_OVER_SPEECH` (N/A, BL017/BL026).

## Fonte, proveniência e o que não é

Origem (somente leitura; **nunca instalada, construída nem executada**: `ORIGIN_NOT_EXECUTED`):
`packages/scene-engine/src/` de `tesselapp`. A prova é "fórmulas transcritas por Python independente", como em
`pilot-fx`; **não** é paridade com a origem. O manifesto separa três coisas, que não se misturam:

| Camada | O que é | Estado |
|---|---|---|
| **Pin** `d40ac3f7c0` (nomeado pelo plano e por `source-feature-inventory.md`) | O snapshot do piloto | **Não é commit do clone Windows** (`PIN_COMMIT_ABSENT`); a autora não o leu |
| **Ponta** `365579076782c1fc19c6464f3bc0199b30c0ab8f` | O que a autora (Windows) leu por `git rev-parse`/`cat-file`: id do blob, sha256 e bytes de cada fonte, **conferidos** por `verify --origin` | `origin-blob verified` |
| **Atestado do coordenador (Mac)** | 21 blobs do pin comparados com a ponta por objetos Git: **19 idênticos, 2 diferentes** | Relato, **não reconferido** pela autora (`PIN_BLOB_UNVERIFIED` para os dois que diferem) |

Os dois que diferem são **notas, nunca ajuste silencioso**: `fx/fx-schema.ts` (só um comentário, `CLIP_OVERLAP` →
`CLIP_PAST_END`) e `schema/render-bundle.schema.ts` (**comportamento da narração**: na ponta importa
`editFrameToSeconds` e chama `explainNarration`). O contrato vale **sem narração** (o editor não tem narração).
`origin-blob verified` **não** é `origin executed`. O golden `crossDissolve.png` da origem **não foi lido nem
comparado** (`ORIGIN_GOLDEN_NOT_COMPARABLE`). O manifesto lista 21 fontes atestadas e 5 lidas só na ponta
(`fx-explain-effects`, `fx-explain-context`, `fx-ids`, `narration-schema`, `catalog/vignette`).

## Contrato do core / biblioteca (fixado pela S0; **Pendente** no código)

Cada item abaixo tem vetor no corpus, com a **proveniência** da expectativa: `origin-formula` (lida do texto da origem e
avaliada em Python), `origin-spec-vector` (número que o spec da própria origem fixa), `destination-contract-candidate`
(o que o plano propõe onde difere da origem) ou `destination-hardening` (rigor a mais; códigos `ORACLE_*`, que **não**
são códigos de produto).

### Formato

```json
{ "id": "tr-1", "effect": "crossDissolve", "fromShotId": "shot-a", "toShotId": "shot-b", "durationFrames": 12,
  "alignment": "center", "handles": "hold", "easing": "easeInOutCubic",
  "params": { "curve": "smooth", "intensity": 1 }, "seed": 0 }
```

Mesmos nomes de campo do piloto (D01); o rascunho `{type, fromShot, toShot}` de [effects.md](effects.md) está
**superado**. Padrões de uma transição nova: `center`, `hold`, `easeInOutCubic`, `smooth`, `intensity 1`; duração
**omitida** = 24 **cortada** ao que os dois planos suportam (`maxTransitionFrames`; se for 0, `TRANSITION_TOO_LONG`);
duração **dada** que não cabe é recusada, nunca cortada.

### Janela e aritmética

`C` é o quadro do corte, `D` a duração. `center`: `[C − ⌊D/2⌋, +D)`; `start`: `[C, C+D)`; `end`: `[C−D, C)`;
`antes = max(0, min(fim, C) − início)`, `depois = D − antes`. Em `C = 48`: `center` D=12 → [42,54); D=5 → [46,51);
D=1 → [48,49); `start` D=12 → [48,60); `end` D=12 → [36,48).

- `i64` com operações **verificadas**; `1 ≤ D ≤ duração da edição` **antes** de qualquer conta; início < 0, fim > edição
  ou overflow é `TRANSITION_OUT_OF_RANGE`. **Uma janela inválida é recusada, nunca saturada nem cortada para dentro.**
- Inteiros **estritos** (nada de `12.5`, `12.0`, booleano nem texto); `seed` é `uint32`, sem coerção; inteiros acima de
  2^53 − 1 são recusados (`ORACLE_*`).
- `max(0, …)` é **semântico** só na disponibilidade de quadros reais (cauda de plano que acaba antes do shot).

### Progresso, peso e cor

`t = (f − início + 0,5)/D` (meio do quadro); `progress = ease(t)` é o que o **plano guarda**; `shaped =
mix(progress, smoothstep(progress), curve == smooth)`; `weight = mix(progress, shaped, clamp01(intensity))` é interno ao
shader e **nunca** é guardado. Saída = `mix(mix(a,b,progress), mix(a,b,shaped), amount)`, com `alpha = 1`.

- **`intensity = 0` não é identidade** nem "sem easing": mistura pelo `progress` já eased (162, não 168).
  Com `curve = linear` os dois `mix` coincidem e `intensity` não altera a saída.
- D = 12, `easeInOutCubic`, `smooth`, `intensity 1` (peso em 6 casas): k=0 → 0,000289 / 0,000000; k=1 → 0,007813 /
  0,000182; **k=3 → 0,099248 / 0,027595**; k=5 → 0,385127 / 0,330723; k=6 → 0,614873 / 0,669277; k=11 → 0,999711 /
  1,000000. Simetria `p(k) + p(11−k) = 1`.
- Canal 200 → 40, k=5: decodificados 0,577580 e 0,021219; peso 0,330723; mistura 0,393579; codificado 168,39 → **168**.
  `smooth` com `intensity` 0 / 0,5 / 1 → **162 / 165 / 168**; `linear` → 162 nas três.
- Mistura **em luz linear** (IEC 61966-2-1) sobre decks **de 8 bits** com saída arredondada (meio para cima). A origem
  mistura half-float; a diferença é **declarada** (`DECKS_8_BIT`), não escondida. Esses vetores são documentais (do
  plano e recalculados em Python), **não** saída de nenhum motor.

### Qual quadro cada deck desenha

O deck vizinho tem **três relógios** (D17): instância e câmera **presas** de forma **independente**
(`clamp(inicioInst + Δ, 0, durInst − 1)`, `clamp(camIn + Δ, 0, quadrosCam − 1)`, mais `heldInstance`/`heldCamera`) e o
**relógio de cena** (luz com cintilação) no **quadro real da edição** `f`, também em `hold`. Disponibilidade: cabeça
de instância = `instanceStart`; de câmera = `cameraIn`; caudas = `max(0, total − (início + comprimento))`. Um clipe `shot`
que **encosta** na borda do plano segura o quadro mais próximo; um clipe no meio não entra (`shotClipFrame`).
`handles`: `use` e `refuse` exigem quadros **reais** de instância **e** de câmera, contados à parte (e `refuse` lê
exatamente como `use`); só `hold` aceita congelados.

**Tracer bullet** (corpus `valid-cut-instance-offsets`, `center` D=12 `hold`, corte C=40): janela [34,46); quadro 37 →
`frameInWindow` 3, `primary: "from"`, vizinho `shot-b` com `heldInstance` e `heldCamera` verdadeiros, `progress`
0,099248, peso 0,027595. O `renderProfile` desse bundle é `straight`: o caso é **contrato Tessel** (saída opaca), **não**
"aceito pela origem".

### Recusas tipadas e ordem

Conferência por transição, na ordem da origem (`fx-explain-transitions.ts`): estrutura e tipos → efeito/família
(`UNKNOWN_EFFECT`, `WRONG_FAMILY`) e parâmetros (`UNKNOWN_PARAM`, `PARAM_TYPE`, `PARAM_OUT_OF_RANGE`) → easing
(`UNKNOWN_KEYFRAME_EASING`) → shots (`UNKNOWN_SHOT`) → adjacência (`TRANSITION_NOT_AT_CUT`) → duplicata
(`TRANSITION_DUPLICATE`, também contra um dissolve legado declarado) → duração → janela (`TRANSITION_OUT_OF_RANGE`) →
comprimento (`TRANSITION_TOO_LONG`) → handles (`TRANSITION_NO_HANDLES`, que diz **qual** falta, quantos quadros e o
remédio) → sobreposição (`TRANSITION_OVERLAP`). `WRONG_FAMILY` (look em `transitions[]`) **não** é `UNKNOWN_EFFECT`.
Acrescentados pelo Tessel: `FX_TRANSITION_KEYS_NOT_YET` (D10) e `FX_LIMIT` (no máximo `shots − 1` transições, antes do
custo). Ids: mesma regra de `fx-ids` (padrão ASCII, reservados sem distinguir maiúsculas, duplicado). `hidden`: a origem
aceita só `true` ou ausente; o exemplo do plano escreve `hidden: false`, que a origem recusa — **decisão pendente da S1a**.

### Plano de dois decks (contrato candidato, 7.3)

`other` é um `ScenePlan` com a **sua** tabela de recursos. Os ids de nó **colidem entre decks** (no bundle do tracer,
`root`, `backdrop`, `figure` e os assets `backdrop`/`figure` existem nos dois): nenhum identificador atravessa decks.
`resource_union()` é **derivada, não serializada**, ordenada por `assetId`, com `used_by` **qualificado por deck**; o mesmo
`assetId` com conteúdo diferente é plano inválido (`/transition/other/resources`); `requires` ganha `fxTransition`.
A ordem interna de `used_by` é **proposta do oráculo** (`primary` e depois `other`, nós em ordem crescente); a S1b a confirma.

**Tetos** (5.3), **contrato pré-registrado, não medido, A validar**: `MAX_TRANSITION_PLAN_BYTES = 4194304` (4 MiB),
`MAX_TRANSITION_PLAN_ITEMS = 20000` (nós + camadas + splats + passes + recursos + luzes dos **dois** decks), profundidade
**1** (`other.transition` é recusado sem recursar). Iguais no Rust e no JS; no limite aceita, um acima recusa com o
caminho. São limites de **entrada hostil**, não restrição de cenas reais; a S1b mede os planos do corpus e só reabre um
número por emenda de contrato documentada.

### Alpha (D20): divergência de **veredito**, não paridade

A origem recusa `alphaMode ≠ opaque` quando há `fx` não vazio **ou dissolve legado** (`UNSUPPORTED_ALPHA`, decisões #19,
#46 e #49), mesmo sem `fx`. O Tessel aceita e a saída da mistura é **opaca**, hoje já para os três looks. O corpus
contém um caso `opaque` que a origem aceita e um caso `straight` que a origem **recusa**; o destino aceita pelo contrato
declarado. **Nunca** se escreve "aceito pela origem" para o `straight`. A saída com `alpha = 1` é **assumida** (o shader
da origem escreve 1,0), não medida. O dissolve legado segue `DISSOLVE_UNSUPPORTED` até a S6 (faixa nova, acervo antigo
**inalterado em bytes**). Registrada também em [effects.md](effects.md).

### Tolerâncias (**contrato; não se mexe depois de ver um resultado**)

- **Monitor e renderer:** diferença máxima de **1 código (1/255)** por canal **e** viés médio com sinal
  `|média(Δ)| ≤ 0,25` código sobre as sondas do quadro (≥ 24 por canal). Só vale se a referência vê os mesmos 8 bits dos
  decks e arredonda a saída; sem o viés, uma porta que **trunca** em vez de arredondar erra 1 código em metade das sondas
  e passa pelo 1/255 (na spec, 216 sondas: máximo 1 código e média −0,444 para a porta que trunca, 0,0 para a correta; o viés a recusa). Compara o destino com **este oráculo**, não com os pixels
  da origem.
- **Export H.264:** **16/255**, importado de [F016](effects.md) (texto vigente: «tolerância 16, maior mudança 69 —
  discrimina», relido em 2026-11-14) e de `CODEC_TOLERANCE` em `crates/render/tests/scene_fx_export.rs`. Racional:
  quantização do codec e subamostragem de croma. **Não** deriva do pior valor observado (7/255 é observação, não
  critério). Se o limite deixasse de discriminar o mutante, reabre-se como emenda de contrato, nunca como ajuste.
- Excesso é **FAIL**. **Nenhum pixel do destino foi produzido ou validado** por esta entrega; pré-registrar não é validar.

## CLI headless

**Pendente** (S1a/S1b/S2A): não há operação, leitura nem erro de transição em `tessel-headless`. A leitura pública
reproduz as células L por `tessel-headless` real só na S2A. **O oráculo Python não é CLI headless do editor.**

## UI e CLI da janela

**Pendente** (S4b). Nenhum nó, `Command`, parser ou controle existe; nada é habilitado. Os nós propostos
(`scene.fx.transitions`, `scene.fx.cut:<de>|<para>`, `scene.fx.transition:<id>`), o rótulo «Suavização» para
`intensity` e os motivos legítimos de `disabled_reason` estão no plano (seções 9.3 e 11). Entre S2A e S4b o painel não
tem controle de transição (lacuna registrada), e um documento com transição não quebra o painel dos clipes.

## Evidências e status

Situação por superfície: **L Pendente · H Pendente · UI Pendente · Oráculo Python: implementado e validado no Windows
(abaixo)**. Plano de portão público: até a S2A a admissão recusa `transitions` com `FX_TRANSITION_NOT_YET`.

**S0 — o que roda** (Windows x64, Python **3.13.17 real**, nenhum stub; ver o relatório
[p4b-s0-dissolve-corpus-report.md](../migration/p4b-s0-dissolve-corpus-report.md)):

| Comando | Resultado |
|---|---|
| `python scripts/pilot-dissolve-corpus.spec.py` e `python -O …` | **161 testes: 159 ok e 2 pulados** (os 2 exigem `TESSEL_ORIGIN_PATH`); com o clone da origem apontado, **os 161 ok** |
| `python scripts/pilot-dissolve-corpus.py generate --check` | reproduz os dois JSON byte a byte; não escreve nada |
| `… verify` / `verify --origin PATH` | offline e contra os blobs da ponta: ok; idempotente |
| `… verify --origin PATH --require-pin` | **exit 4** (`PIN_UNVERIFIED`): o pin não pode ser conferido neste clone |

Controles negativos: o spec **recusa o stub da Microsoft Store** (exit 5, `INTERPRETER_NOT_REAL`, nunca um *skip*) e
imprime `sys.version`; mais de 30 mutantes (inclusive «`amount` ignorado», «`smooth` ignorado», `+0,5` removido, easing
ignorado, truncamento, `⌊D/2⌋`→`⌈D/2⌉`, `start`/`end` trocados, grampo conjunto de instância e câmera, relógio de edição
grampeado, borda de `shotClipFrame`, `handles` trocados, disponibilidade negativa, janela cortada em vez de recusada,
coerção de inteiros e de `seed`, regex de id, veredito de alpha, família, `used_by` sem deck e conflito de recurso
aceito) mudam o corpus **e**, as de fórmula, os vetores à mão; adulteração (número do corpus, hash do manifesto, blob de
fonte, bloqueio removido, `kind` trocado) falha. **Limites:** não é o corredor de mutantes da S5; o `--check` entre
hosts (libm do `pow`) **não** foi rodado no Mac; nenhuma GPU, desempenho, ponteiro físico ou áudio; a origem não foi
executada; **nenhum Rust foi alterado**.

### Células por fase (plano, seção 12; célula vazia = não se aplica)

Códigos: `0` S0 · `1a` S1a · `1b` S1b · `2` S2 · `2A` ativação · `3a`/`3b` S3 · `4a`/`4b` S4 · `5` S5. **Todas as células abaixo
estão Pendentes**, exceto a coluna `X` marcada `0`, que o corpus desta S0 cobre como **oráculo** (não como feature).
`○` é controle negativo do portão e `→2A` que a célula se inverte na ativação.

| ID | Caso | L | H | G | W | P | R | E | X |
|---|---|---|---|---|---|---|---|---|---|
| T01 | Documento mínimo, ida e volta em bytes | 1a | 2A | | | | | | |
| T02 | Cada recusa (família, easing, parâmetro, shot, adjacência, duplicata, duração, janela, longa, handles, sobreposição, `keys`, id, `catalogVersion`, limite) | 1a | 2A | 4b | 4b | | | | |
| T03 | Avisos `ON_CONTINUOUS`, `SPLAT_COST` (1a); `FX_PASS_BUDGET` dois decks (1b) | 1a, 1b | 2A | 4b | 4b | | | | |
| T04 | `hidden` e ausência = corte seco com os bytes de hoje | 1b | 2A | | | 2 | 2 | | 0 |
| T05 | Janela: alinhamento/duração, bordas | 1a | 2A | | | | | | 0 |
| T06 | FPS racional; relógio de cena do vizinho em `f/fps` | 1b | 2A | | | | 2 | | 0 |
| T07 | Progresso; `smooth` com `intensity` 0/0,5/1 = 162/165/168 | 1b | 2A | | | | | | 0 |
| T08 | Planos sem transição idênticos ao `main` | 1b | 2A | | | | 2 | | |
| T09 | Plano na janela, `requires`, adulterações | 1b | 2A | | | | | | |
| T10 | `hold` por instância/câmera; ids de nó e assets colidentes | 1b | 2A | | | | 2 | | 0 |
| T11 | Clipe `shot` na borda segura o quadro | 1b | 2A | | | | 2 | | 0 |
| T12 | Passe de saída depois da mistura | 1b | | | | 2 | 2 | | 0 |
| T13 | `filmGrain` de saída: mesma semente com e sem transição | 1b | 2A | | | | 2 | | 0 |
| T14 | Orçamento conta os dois decks e a mistura | 1b | 2A | 4b | | | | | |
| T15 | Runtime sem `fxTransition` recusa | | | | | 2 | 2 | | |
| T16 | Pixel: cor constante e texturizado por quadro | | | | | 2 | 2 | | 0 |
| T17 | Resize e perda de contexto na janela | | | | | 2 | 2 | | |
| T18 | Erro de um deck e dispose dos dois | | | | | 2 | 2 | | |
| T19 | Seek = sequencial; cache do vizinho | | | | | 2 | 2 | | |
| T20 | Edição, undo, dry-run, texto pt-br/en/es | 3a | 3b | 4b | 4b | | | | |
| T21 | Gesto begin/update/commit ≡ `scene.fx.edit` | 3b | 3b | 4b | 4b | | | | |
| T22 | Documento sem o shot / shot encurtado recusado | 3a | 3b | | 4b | | | | |
| T23 | Salvar, reabrir, pacote, intercâmbio | 3b | 3b | | 4b | | 3b | | |
| T24 | UI sintética (GPUI) | | | 4b | | | | | |
| T25 | Janela real por `scripts/ui` | | | | 4b | 4b | | | 0 |
| T26 | Predicado de janela; hit ≡ press | 3a, 4a | 4a | 4a | 4a | | | | |
| T27 | Export: sub-intervalo = completo; codec com tolerância declarada | | | | | | 2 | 2, 5 | 0 |
| T28 | Corrupto e limites; `other` de profundidade 2 e «bomba» | 1a, 1b | 2A | | | | | | |
| T29 | Mutantes (corredor `mutate.py`) | 5 | | | | 5 | 5 | | |
| T30 | Splat → splat e junta `continuous` | 1b | | | | 2 | 2 | | |
| T31 | Cache da mistura: só `intensity` remistura sem redesenhar | | | | | 2 | 2 | | |
| T32 | Portão público (8.4) | 1a○→2A | 1a○→2A | 1a○→2A | | | 2○ | | |
| T33 | Alpha: `straight` aceito e desenhado opaco | 1b | | | | 2 | 2 | | 0 |

**A validar / lacunas registradas:** (a) as tolerâncias e os tetos são contrato pré-registrado, sem pixel nem plano
medidos; (b) `hidden: false` e a ordem de `used_by` aguardam decisão de design; (c) a divergência de veredito do alpha
aguarda a revisão de design (Q3); (d) o `--check` em macOS/Linux não foi rodado; (e) o pin não é verificável neste
clone; (f) BL167.5 permanece **parcial** (só a preparação do interpretador): build release, provas vivas e janela D3D11
seguem dependendo de BL167.1–.4.
