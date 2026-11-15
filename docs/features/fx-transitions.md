# F064 — Transições entre planos: `crossDissolve` (BL082.4, P4b)

## Objetivo e limites

`fx.transitions[]` veste um **corte** entre dois planos vizinhos com uma janela de quadros em que o monitor e o export
misturam os **dois decks** (a imagem de cada plano antes da saída) em luz linear. O único efeito de transição da
primeira entrega é `crossDissolve`. Quem calcula é o core; o plano carrega o resultado; o runtime só desenha.

**Estado (S1a, BL082.4.2): fundação de domínio no core, sem habilitar nenhuma transição pública.** Existem este documento,
o corpus independente da S0 (`scripts/pilot-dissolve-corpus.py`, `scripts/pilot_dissolve_analytic.py`,
`dissolve-cases.json`, `dissolve-manifest.json`) e, desde a S1a, o modelo de documento, a leitura, a validação completa
(`explain`), os avisos e a aritmética de janela em `crates/core/src/directed_scene/fx/` (relatório e mapa de caminhos:
[p4b-s1a-transition-admission-report.md](../migration/p4b-s1a-transition-admission-report.md)). **Tudo isso fica atrás de
um portão de admissão**: toda entrada pública (importar, anexar, abrir projeto, restaurar pacote, candidato de edição)
lê com a política `Refuse`, e um `transitions` não vazio continua `FX_TRANSITION_NOT_YET`, com a mesma mensagem. Só os
testes e o corpus do core têm a política `Accept` (`#[doc(hidden)]`, **não** é barreira de segurança). Continuam sem
existir: runtime/shader/export (S2), operações/gestos/inspetor (S3), UI (S4). **S1b (BL082.4.3) adiciona, só no core e
só interno:** a avaliação de dois decks, `ScenePlan.transition` e a capacidade `fxTransition` (ver «S1b» no fim e o
[relatório](../migration/p4b-s1b-evaluation-report.md)); a admissão pública segue fechada. O catálogo público, o seletor
de clipe e os planos **sem transição não mudam** (3 looks, `catalogVersion` 1, `PLAN_VERSION` 1; 14 capacidades no Rust,
13 na lista do JS). Plano de trabalho (versão 2):
[p4b-cross-dissolve-plan.md](../migration/p4b-cross-dissolve-plan.md), lido no head `2395719`.

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
comparado** (`ORIGIN_GOLDEN_NOT_COMPARABLE`). O manifesto lista hoje **33 fontes: 21 atestadas e 12 lidas só na ponta**: as 5 originais
(`fx-explain-effects`, `fx-explain-context`, `fx-ids`, `narration-schema`, `catalog/vignette`) e 7 da referência analítica
(`scene-lighting`, `weather-model`, `resolve-camera-clip`, `resolve-performance`, `types/fps`, `plane-effects-material`,
`three-scene-runtime`). Original: 26 (21 + 5).

## Contrato do core / biblioteca (fixado pela S0; **implementado na S1a atrás do portão**, sem avaliação nem plano)

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
de instância = `instanceStart`; de câmera = `cameraIn` se `cameraIn <= quadrosCam`, senão0 (ou `MAX` sem comprimento conhecido); caudas = `max(0, total − (início + comprimento))`. Um clipe `shot`
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
aceita só `true` ou ausente. Decisão do coordenador: manter essa regra nas transições e recusar `false` estruturalmente na S1a; isso não muda os looks existentes.

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

### Portão de admissão e modelo (S1a)

- **Política única** `TransitionAdmission::{Refuse, Accept}`, guardada no `DirectedScene` (`admission()`). O padrão e a
  única política de toda entrada pública é `Refuse` (`import_str/_with`, `import_value/_with`, anexar, abrir, restaurar
  pacote, `render`). O candidato de **toda** edição (`scene.edit`, `scene.fx.edit`) herda a política da cena
  (`DirectedScene::reimport`): uma edição nunca abre nem fecha o portão. `Accept` e os importadores `*_admitting` são
  `#[doc(hidden)]`, existem para os testes e o corpus do core e **não** têm flag, variável de ambiente, socket nem
  arquivo que os alcance. O teste `boundary` varre as fontes de produção de `crates/*/src` com o lexer do Rust
  (`proc-macro2`, só dev-dependency; sem comentários nem itens `cfg(test)` **provadamente** falsos; o nome de um arquivo
  nunca o exclui, só a declaração por um módulo de teste; `cfg` ambíguo conta como produção; falha de lexer fecha) e
  exige: nenhuma menção fora do importador, nenhuma passagem de `Accept` dentro dele e o inventário fechado dos pontos
  que criam ou reidratam uma cena (todos `Refuse`). É **auxiliar textual, não barreira de segurança**: não vê macros que
  geram código, `include!`, `cfg_attr` nem `#[path]` no próprio módulo em linha. A prova que não depende do scanner são os testes
  dinâmicos `reload` (abrir, recarregar e anexar por arquivo recusam e mantêm estado, revisão e histórico).
- **Modelo:** `Fx.transitions: Vec<FxTransition>` (`id`, `effect`, `from_shot_id`, `to_shot_id`, `duration_frames`,
  `alignment`, `handles`, `easing`, `params`, `seed`, `hidden`), preenchido só sob `Accept` e só quando nada foi recusado;
  `Fx::to_json` **não muda**. Tabela própria `transition_catalog()` (só `crossDissolve`, família `transition`,
  `Reseed::Never`, `curve` como `ParamKind::Choice` e `intensity` 0..1) separada do catálogo público.
- **Leitura estrita:** `durationFrames` e `seed` são inteiros JSON (`12.0`, `12.5`, booleano e texto são recusados;
  `seed` é `uint32`); `hidden` aceita `true` ou ausente (`false` é `INVALID_VALUE`); `keys` presente, mesmo `{}`, é
  `FX_TRANSITION_KEYS_NOT_YET`; `FX_LIMIT` é julgado antes de ler as entradas; ids compartilham o espaço de trilhas e
  clipes; campos desconhecidos viram avisos `UNKNOWN_FIELD` e os bytes do documento são preservados.
- **Janela:** `i64` verificada, `div_euclid(2)` (piso); bordas 0 e `edição` valem; `D` fora de `[1, edição]` é recusada
  antes de qualquer conta; `available_frames` nunca é negativo.
- **Avisos** (nunca recusam): `TRANSITION_ON_CONTINUOUS` e `TRANSITION_SPLAT_COST`, ignorando transições `hidden`
  (`DirectedScene::transition_warnings`).
- **Divergências registradas** (relatório da S1a): o dissolve legado é recusado antes (`DISSOLVE_UNSUPPORTED`), e o
  oráculo lista `TRANSITION_DUPLICATE` ao lado dele; um id reservado **e** duplicado dá só `RESERVED_ID`; as recusas de
  efeito/família/parâmetro/easing/enums só aparecem junto com as do bundle depois que o bundle em si é válido.

## CLI headless

**S1a:** só a recusa. `scene.attach` com `transitions` não vazio responde `invalid_bundle` com `FX_TRANSITION_NOT_YET`
em `tessel-headless` real (saída 1 numa sessão com pedido recusado, respostas iguais às da biblioteca), sem mudar o
documento, a revisão nem o histórico; `scene.fx.catalog` lista só os 3 looks. Nenhuma operação, leitura ou erro novo
de transição é exposto. A leitura pública (células L por `tessel-headless`) segue na S2A. **O oráculo Python não é CLI
headless do editor.**

## UI e CLI da janela

**S1a:** só o caso negativo (T32). A janela recusa o mesmo bundle pelo handler compartilhado (`Command::Scene(Attach)`),
não muda cenas, revisão nem histórico, e a seção «Efeitos» mantém as 3 ações `add:` sem `crossDissolve`. Nenhum diálogo,
controle ou nó novo. Controles de transição: **Pendente** (S4b). Nenhum nó, `Command`, parser ou controle existe;
nada é habilitado. Os nós propostos
(`scene.fx.transitions`, `scene.fx.cut:<de>|<para>`, `scene.fx.transition:<id>`), o rótulo «Suavização» para
`intensity` e os motivos legítimos de `disabled_reason` estão no plano (seções 9.3 e 11). Entre S2A e S4b o painel não
tem controle de transição (lacuna registrada), e um documento com transição não quebra o painel dos clipes.

## Evidências e status

Situação por superfície (S1a): **L implementado (modelo, leitura, `explain`, janela, avisos; só com a política `Accept`
dos testes) · H só a recusa (`tessel-headless` real) · UI só a recusa e a ausência de controle · Oráculo Python:
implementado e validado no Windows (abaixo)**. Portão público: até a S2A a admissão recusa `transitions` com
`FX_TRANSITION_NOT_YET`. Evidência da S1a **por head de código** (teste `crates/core/tests/scene_fx_transition.rs`:
46 testes em `5e21e09`, 47 em `449054a`, **60 em `517946a`**; `scene_fx` 25; 2 em `scene_project::admission_tests`;
GPUI `scene_fx` 80, 2 deles em `scene_fx_transition_tests`; corpus lido em Rust sem Python) e mutantes detectados: no
relatório da S1a, seção «Reforço de 517946a». Um número de um head não vale para outro.

**S0 — o que roda, versão original** (Windows x64, Python **3.13.17 real**, nenhum stub; ver o relatório
[p4b-s0-dissolve-corpus-report.md](../migration/p4b-s0-dissolve-corpus-report.md)):

| Comando | Resultado |
|---|---|
| `python scripts/pilot-dissolve-corpus.spec.py` e `python -O …` | **161 testes (versão original; 167 hoje, ver o fim do documento): 159 ok e 2 pulados** (os 2 exigem `TESSEL_ORIGIN_PATH`); com o clone da origem apontado, **os 161 ok** |
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

Códigos: `0` S0 · `1a` S1a · `1b` S1b · `2` S2 · `2A` ativação · `3a`/`3b` S3 · `4a`/`4b` S4 · `5` S5. **As células abaixo
estão Pendentes**, exceto a coluna `X` marcada `0` (corpus da S0, **oráculo**, não feature) e, **desde a S1a (BL082.4.2)**,
as células `1a` de T01, T02, T03 (avisos `ON_CONTINUOUS`/`SPLAT_COST`), T05, T28 (limites do documento) e o negativo `○`
de T32 em L, H e G (`1a ✓` no texto abaixo da tabela). `T02` em H/G, e as demais colunas, seguem como na tabela.
`○` é controle negativo do portão e `→2A` que a célula se inverte na ativação.

| ID | Caso | L | H | G | W | P | R | E | X |
|---|---|---|---|---|---|---|---|---|---|
| T01 | Documento mínimo, ida e volta em bytes | 1a | 2A | | | | | | |
| T02 | Cada recusa (família, easing, parâmetro, shot, adjacência, duplicata, duração, janela, longa, handles, sobreposição, `keys`, id, `catalogVersion`, limite) | 1a | 2A | 4b | 4b | | | | |
| T03 | Avisos `ON_CONTINUOUS`, `SPLAT_COST` (1a); `FX_PASS_BUDGET` dois decks (1b) | 1a, 1b | 2A | 4b | 4b | | | | |
| T04 | `hidden` e ausência = corte seco com os bytes de hoje | 1b | 2A | | | 2 | 2 | | 0 |
| T05 | Janela: alinhamento/duração, bordas | 1a | 2A | | | | | | 0 |
| T06 | FPS racional; relógio do vizinho em f/fps; X: estados temporais **e** sondas analíticas de posição/cor em planos planos (30000/1001, `lit-ntsc-center`); textura só na S2 | 1b | 2A | | | | 2 | | 0 |
| T07 | Progresso; `smooth` com `intensity` 0/0,5/1 = 162/165/168 | 1b | 2A | | | | | | 0 |
| T08 | Planos sem transição idênticos ao `main` | 1b | 2A | | | | 2 | | |
| T09 | Plano na janela, `requires`, adulterações | 1b | 2A | | | | | | |
| T10 | Hold por instância/câmera; X: sondas analíticas de posição projetada e cor de hold misto, instância presa com câmera correndo e o inverso; textura só na S2 | 1b | 2A | | | | 2 | | 0 |
| T11 | Clipe `shot` na borda segura o quadro | 1b | 2A | | | | 2 | | 0 |
| T12 | Passe de saída depois da mistura | 1b | | | | 2 | 2 | | 0 |
| T13 | `filmGrain` de saída: mesma semente com e sem transição | 1b | 2A | | | | 2 | | 0 |
| T14 | Orçamento conta os dois decks e a mistura | 1b | 2A | 4b | | | | | |
| T15 | Runtime sem `fxTransition` recusa | | | | | 2 | 2 | | |
| T16 | X: cor de 8 bits e posição projetada de hold misto e luz real (cenas planas, §abaixo); quadro **texturizado** só se torna real na S2 | | | | | 2 | 2 | | 0 |
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

**Evidência S1a anterior à revisão de72c9fe2 (BL082.4.2, ainda A validar):** T01 L, T02 L, T03 L (avisos), T05 L, T28 L (limites do documento), T32 L/H/G
(portão público recusa, estado intacto, catálogo de 3 looks, sem controle novo). Sem lock de GPU: nada aqui mede desempenho.
`scene_gizmo_rotation` e `scene_gizmo_scale` falham **também** na base (diferença de último bit e cisalhamento de gizmo,
fora desta entrega); a suíte do workspace inteiro não foi rodada (só `--no-run`).

**A validar / lacunas registradas:** (a) as tolerâncias e os tetos são contrato pré-registrado, sem pixel nem plano
medidos; (b) `hidden: false` é recusado nas transições (S1a ✓) e a ordem de used_by será qualificada por deck; (c) a divergência de veredito do alpha
aguarda a revisão de design (Q3); (d) reprodução Python Windows/Mac passou em `9399642`, Linux não foi rodado;
(e) o pin não existe no clone Windows, mas foi verificado no Mac; isso não executa a origem;
(f) BL167.5 permanece **parcial** (só a preparação do interpretador): build release, provas vivas e janela D3D11
seguem dependendo de BL167.1–.4.

### Revisão independente da S0 e correções do coordenador

A revisão em `173f5f4` confirmou as fórmulas por Decimal independente, mas **não aprovou S0**.
O coordenador corrigiu `--require-pin`: sem origem explícita é exit 4; mesmo com todos os blobs
presentes por id, a ausência do commit impede comprovar pertencimento ao pin e continua exit 4.
O modo de sucesso identifica pin verificado ou atestação, sem chamar texto presente de análise
estrutural: check_against é só presença textual; hashes fixam o conteúdo, não executam TS.
A CLI `digests` exige `--origin` (uso inválido: exit 2). Python legítimo da Store em execução pode
ter alias de tamanho 0; um stub sem intérprete não executa o script e falha antes (9009).
Um vetor `intensity: 2.0` fixa a mensagem numérica integral `2`; a paridade de textos numéricos fora
dos vetores do corpus não é prometida como implementação completa de String(number) do JS.

**A validar (P2-2, tratado em [p4b-s0-dissolve-corpus-report.md](../migration/p4b-s0-dissolve-corpus-report.md) §10):** a referência analítica
de hold misto e do relógio real da luz **existe agora** (ver abaixo), mas aguarda revisão independente. Nada foi renderizado;
quadro texturizado só se torna real na S2; o aceite de S0 depende da revisão.
A linha F064 foi acrescentada à matriz com todas as superfícies de produto pendentes.

Na atualização do coordenador, os **166 testes** passaram no macOS com Python 3.14.3,
nos modos normal e `-O`, com origem. `generate --check` reproduziu os arquivos byte a byte;
`verify --origin … --require-pin` confirmou o commit do pin neste clone (21 fontes do pin
e 5 só da ponta). Isso atualiza a evidência histórica de 161 testes acima, sem provar o motor.
O plano original lido pela autora foi `2395719`; a versão final de PR58, fonte `002fe9f`, integrada em `1796107` e contida em
`58157ab`, foi incorporada à seção analítica por leitura do documento, **sem afirmar prova**.

### Referência analítica X de hold misto e relógio real da luz (P2-2)

`scripts/pilot_dissolve_analytic.py` (só `math` e `fractions`; não importa o script do corpus) gera a seção `analytic_probes` de
`dissolve-cases.json` (`kind: origin-formula`): 12 cenas sintéticas de **cor plana**, 116 quadros, **451 sondas** de posição
projetada e de cor de 8 bits (1/255 por canal, \|média Δ\| ≤ 0,25, os portões pré-registrados do corpus, H.264 16 importado de F016 e
nunca calibrado por resultado). As sondas ficam ≥ 6 px de qualquer borda, em área plana.

| Regra verdadeira (D14, D15, D17) | Hipótese errada (mutante) que a cena precisa recusar |
|---|---|
| instância e câmera são grampeadas **cada uma** no seu clipe, cada uma com `held` próprio | pinagem conjunta dos relógios; instância/câmera trocadas |
| o deck vizinho usa o início, `instance_start` e `camera_in` **do seu plano** | vizinho nos relógios do primário |
| a mistura é `a·(1−w)+b·w` com `a` = plano antes do corte | decks trocados |
| o relógio da luz é o quadro de edição **real** f/(num/den), mesmo com instância e câmera presas | luz no relógio da instância, do plano, do clipe de câmera, no quadro de edição limitado, no fps nominal |

Cada cena declara o que deve rejeitar (separação ≥ 4 códigos em ≥ 3 sondas de ≥ 2 quadros) e o que deve concordar
(≤ 1 código); o gerador recusa (`ORACLE_ANALYTIC_*`) uma cena sem poder de recusa. Cenas: hold misto nos dois lados do corte com
janelas `center`/`end`/`start`, o limite do primeiro quadro preso, controles sem hold e com os dois presos, luz com os dois
relógios presos, `instance_start`/`camera_in` distintos do início do plano (relógio de edição contra relógio do clipe), 30000/1001 com a
janela em 1794..1806 e posição mais luz juntas. A spec (`scripts/pilot_dissolve_analytic.spec.py`, 36 testes em `9399642`, 39 na correção abaixo) refaz tudo com um avaliador
independente (`ctypes`, matriz do Three.js, `Fraction`, `Decimal`), aplica o portão a produtos falsos (os que implementam cada hipótese
falham; os controles e a verdade passam; um produto que trunca falha pelo viés em todas as cenas) e roda 19 mutantes do próprio oráculo.

**O que isto é e o que não é.** A S0 não tem Rust, JSONL, UI nem motor de transição: estas sondas são o **oráculo** que a S2 deve
reproduzir. **Quadros texturizados só se tornam reais na S2**; o gate `FX_TRANSITION_NOT_YET` permanece até a S2A. Nada foi renderizado,
medido nem executado; a origem não foi executada nem seu golden comparado. **A validar** pela revisão independente.

**`hidden` (contrato futuro do destino):** uma transição aceita `hidden: true` ou ausente; `hidden: false` explícito é recusa estrutural
na S1a, independente de `explain`. A entrada interna `Accept` da S1a já valida isso; a entrada
pública continua recusando qualquer transição com `FX_TRANSITION_NOT_YET`, antes dessa leitura.

**Resultados da entrega `9399642`** (Windows x64, Python 3.13.17; históricos após a correção abaixo): spec do corpus 167 testes (offline `OK (skipped=2)`; com `TESSEL_ORIGIN_PATH`
`OK`), spec analítica 36 testes `OK`, ambas também com `-O`; `generate --check` byte a byte; `verify` offline e `--origin` exit 0; `--require-pin`
sem origem e com origem sem o commit do pin exit 4; `digests` sem `--origin` exit 2. Hashes: `dissolve-cases.json`
`e5f40f70ccdb069196224d45c8df09cbaec7539eedc06ea350e65de7966cd011` (539 508 bytes), `dissolve-manifest.json`
`5368450f0df266a5d4375adafcd73a82ccf9bc9d519ff2138ae0dc1129e644dd` (27 892 bytes); os números das seções acima (161 testes, 26 fontes) são o histórico da versão original.

### Correção após a revisão R2: representabilidade e alcance do X

A revisão **não aprovou** `9399642`: as lâmpadas de intensidade 24 excediam o limite 20 do
core/runtime, tornando seis cenas de luz inalcançáveis no destino. A correção usa intensidade
**20**, acrescenta às leituras do destino os limites de intensidade e de quatro luzes e testa
todos os kits: intensidade `[0,20]`, cor/flicker `[0,1]`, raio positivo, números finitos e até
quatro luzes. É correção de representabilidade; fórmulas e limiares de rejeição não mudaram.
Os kits agora trazem `initialFovDeg: 50` e saída 640×360 explicitamente.

**Posição:** `centre_px` e `rect_px` são coordenadas diagnósticas, sem tolerância própria de
aceitação em pixels. O aceite de S2 compara estados temporais e cores nos pixels prescritos;
as hipóteses de posição são discriminadas pelas sondas de cor. Isso não alega precisão medida
de posição em GPU.

**Luz só no vizinho:** a nova spec isola esse erro mantendo o primário correto. Em `lit-end-held`,
as hipóteses LIN/LSR/LCC/LCE têm pelo menos três sondas em dois quadros; em `lit-start-held`,
LIN/LCC também. Com intensidade 20, foram 8/12/8/5 sondas em 2/3/2/2 quadros no primeiro caso,
e 7 sondas em 3 quadros no segundo. As janelas `center` não demonstram isoladamente esse poder:
podem rejeitar pelo primário ou separar poucas sondas em um único quadro, abaixo do portão de dois quadros. O erro de fps nominal é
global. S2 deve respeitar esse alcance e comparar os estados de cada deck separadamente.

O teste de câmera ganhou o quadro exato 40 e dois segmentos com `previousEnd` e com retorno
à pose `initial`. O mutante `<` → `<=` não ultrapassa o fim de um clipe único porque `ease`
limita a progressão a `[0,1]`; ele é detectado no limite entre segmentos que voltam à pose
inicial. Isso corrige a interpretação de overshoot da revisão, mantendo uma regressão útil.
`shot_numbers` foi removido e o nome da separação mínima dos decoys foi esclarecido. A diferença
de cerca de 1e-14 entre segundos racionais do destino e divisão da origem não altera os códigos
de 8 bits destes vetores. O módulo analítico ainda exige separação em módulos se crescer na S2.

A R3 independente aprovou a S0 no head `e4078ff`, com corpus 167 e analítico 39 passando
no Windows, normal e `-O`, mais controles negativos de intensidade e fronteira de câmera.
A aprovação cobre oráculo e contrato; todas as superfícies de produto continuam Pending,
sem engine, golden da origem ou textura executados. O relato completo está no relatório, §12.


### S1a: revisão e estabilidade dos diagnósticos

A revisão de72c9fe2 confirmou o portão e as regras de domínio, mas não aprovou os gates:
E0583 no módulo do teste movido foi corrigido com path explícito. Essa correção ainda exige
reexecução Windows. Os erros de params das transições agora têm ordem lexical por chave,
independente de serde_json/preserve_order; o documento não é reordenado. A ordem difere da
ordem de autoria da origem, mas o veredito e os valores não mudam. Looks mantêm a regra atual,
registrada como dívida em BL082.4.13.

**Reforço de `517946a` (BL082.4.2).** E0583 foi introduzido pelo coordenador (movimentação do teste de catálogo sem
`#[path]`) e corrigido em `449054a`; a correção de produção verificada antes **não é prova de integração**: cada head tem
a sua prova (relatório, seção «Reforço de 517946a»). Em `517946a`, com a árvore limpa: fmt, `check` e `clippy
--all-targets` do workspace, `scene_fx` 25, `scene_fx_transition` 60, GPUI `scene_fx` 80, processo `tessel-headless`
real, `--no-run` do workspace e `build -p tessel-editor` saíram 0; o core completo (`--no-fail-fast`) tem 1524 ok e as 2
falhas de gizmo que já existem na base, sem relaxá-las. O workspace inteiro **não** foi executado (claims de mídia,
BL167.6). Acrescentou-se: testes dinâmicos de abertura/recarga/anexo por arquivo; scanner com lexer real; `TOO_LONG` por
lado (mata a troca antes↔depois); janelas em ordem cronológica inversa válidas e sobrepostas reais (matam a remoção da
ordenação e da checagem de sobreposição); e o teste da ordem dos params nos perfis Cargo, em que o núcleo isolado é
**cego** (o mutante «sem ordenação» só morre sob `preserve_order`). A ordem lexical dos params das transições diverge da
ordem de autoria da origem e foi mantida. Nada aqui afirma runtime, GPU, Mac ou interface física.

**Pendente:** nova revisão independente; core completo com 2 falhas de base; workspace inteiro não rodado; S1b, S2, S3 e S4.
Status de produto de F064: **P** em todas as superfícies. BL082.4.2 permanece **A validar** até essa revisão.

**Correção posterior ao reforço (BL082.4.2).** O scanner distingue expressão
`const {}` de declaração `const NAME = ...;`, para não remover a expressão de
produção seguinte a um bloco sob `cfg(test)`. A regressão inclui os controles
com/sem ponto e vírgula e referência também condicionada a testes. A execução
Windows deste ajuste está pendente na revisão R2; os resultados de `517946a`
continuam atribuídos àquele head. Nenhum comportamento de produto mudou.

**Revisão independente R2 concluída em `a752f0a`.** S1a aprovada no seu escopo
interno. No Windows: fmt/check/clippy/workspace no-run/build passaram;
`scene_fx_transition` 61, `scene_fx` 25 e GPUI 80 passaram, com processo headless
real, perfis `preserve_order` e mutantes discriminantes. O core completo teve
1525 resultados ok e 2 falhas de gizmo da base. O workspace inteiro, Mac, runtime,
GPU e UI física não foram executados nesta revisão. Os resultados substituem a
pendência de execução do ajuste `const {}` acima, cuja regressão falha sem ele.

A coleta de IDs automáticos de looks ainda não inclui IDs das transições. Sob
`Accept`, uma transição `fx-1` ou `fx-track-1` pode causar `DUPLICATE_ID` ao
adicionar um look sem ID: recusa segura e hoje inalcançável pela entrada pública.
A correção e seus testes ficam na S3, registrada em BL082.4.15. Esta pendência
não bloqueou o parecer S1a. F064 permanece P e o portão público fechado até S2A.

## S1b (BL082.4.3): avaliação de dois decks e `ScenePlan.transition` — interna, a validar review

**Concluída parcial interna, a validar review.** Só o core; a admissão pública continua `Refuse` e o `tessel-headless`
real continua recusando `transitions` com exit 1. S2 e S3a não começaram; BL082.4.15 segue Pendente para a S3.
Detalhe, mutantes, divergências e comandos: [p4b-s1b-evaluation-report.md](../migration/p4b-s1b-evaluation-report.md).

- **Relógios (D14/D15/D17).** `evaluate_shot_at_offset` (`temporal/shot_offset.rs`) avalia um shot nomeado num quadro
  fora do seu intervalo: instância e câmera são presas **separadamente**, com `held` só quando o grampo mudou o quadro
  (clipe sem quadros: sempre preso em 0); luz e cena usam o quadro **real** de edição à taxa racional. O clipe de plano do
  vizinho usa `clamp(f, shot.start, shot.end−1)`; um clipe no meio do shot não entra. A semente de ruído do vizinho usa o
  quadro real (exceção registrada).
- **Estágios.** `Fx::resolve_frame` (`fx/frame.rs`): passes do topo, passes do vizinho (cada deck), mistura
  (`progress` suavizado, `amount = clamp01(intensity)`) e saída, uma vez. Orçamento (`fx/budget.rs`): só aviso.
- **Plano.** `scene_plan/transition{,_json,_validate}.rs`: ver [scene-plan.md](scene-plan.md). Recursos por deck, união
  derivada com `used_by` qualificado, `requires` ganha `fxTransition`, tetos 4 MiB / 20 000 itens / profundidade 1.
- **Prova histórica (Windows, commit `73775c3`).** `scene_fx_transition` 106 testes e `pilot_dissolve_corpus` 10, nos
  dois perfis de `serde_json` (com e sem `preserve_order`); core completo com 2 falhas de base (`scene_gizmo_rotation`,
  `scene_gizmo_scale`, também na base sem alteração); GPUI `scene_fx` 80. 18 mutantes essenciais mortos.
  Esses gates não comprovam os patches posteriores. O limite de cabeça de câmera (.24) tem
  114 testes e revisão Windows R3 em `4199af0`, descrita no fechamento abaixo.
- **Não provado / A validar:** pixels reais, GPU, runtime e shader (S2), Mac, origem (`ORIGIN_NOT_EXECUTED`), comparação
  com ouro (`GOLDEN_NOT_COMPARABLE`), aceite humano. A linha de pixels do corpus é um cálculo de referência
  (decodificação sRGB, mistura, codificação, meio para cima), **não** saída de GPU.

**Validação Mac da S1b (BL082.4.16).** No head73775c3 passaram106testes de
transição e10do corpus; a baseline de compatibilidade falhou porque só tinha
perfis Windows. Os perfis Macdebug/release foram capturados posteriormente
sobre a basee3c20cb sem mudança de produção, preservando as referências
Windows. Candidato+dados corrigidos passam106/10e baseline emdebug/release.
Esta é prova de compatibilidade de plano/core, sem runtime oupixelsreais,
e não resolve as divergências de kitsX (BL082.4.17) em revisão.

### Reforço do coordenador após review S1b

BL082.4.18/.19/.22: o validador recusa a relação de quadro com overflow, qualifica erros do deck
vizinho (incluindo versão e splat), e edit/inspect/gesto/UI usam o mesmo orçamento com shots e mistura.
Regressões novas cobrem tempo real diferente, clipId repetido entre decks e luzes no teto de itens.
A adaptação dos três kits lit-held passa a conferir também câmera e heldCamera de A; só os estados de
instância que dependem do comprimento adaptado ficam sem comparação. BL082.4.17 adiciona referência
representável separada; o acervo S0 permanece intacto. Nenhuma dessas correções abre a admissão.
Os testes novos e os gates do novo head ainda precisam de execução e review. F064 permanece P.

O reforço encontrou uma distinção adicional (BL082.4.23): o contador de câmera do plano primário
legado pode passar do último frame de uma câmera curta, enquanto o avaliador por offset informa
o frame preso. No caso adaptado A em f23 são23 e11. O teste valida cada contrato separadamente;
não declara igualdade dos estados completos desse deck. A referência nova deve usar comprimentos
representáveis e a S2 precisa manter essa distinção explícita. O resultado vermelho inicial consta
no relatório S1b, sem ser promovido a prova verde.

No reforço93fb635, Mac nativo passou core1594/0/7, transições112/corpus10/baseline1 com
preserve_order, GPUI80 e fmt/check/clippy/build; dependências do core sem gpui/wry.
Sete mutantes em61efdba, cujos arquivos de produção permanecem idênticos, foram detectados.
O resultado inicial vermelho e o ajuste do teste de câmera estão no relatório S1b §§6–7.
Windows do novo head ainda a validar; não herdar R1 de73775c3 nem declarar produto entregue.

A baseline release do mesmo93fb635 também passou1/1. Os gates acima pertencem ao código93fb635; o fechamento posterior muda só docs.

A revisão R2 Windows aprovou d2c1861 no escopo interno, com gates e mutantes; o construtor
continua sem contexto de deck em seus erros (BL082.4.19), diferente do validador/leitor.
Um comentário tardio de handles foi reproduzido e está em correção (.24): se cameraIn excede
o comprimento conhecido, não existe cabeça contígua de quadros reais. `use`/`refuse` recusam
quando a janela pede essa cabeça; `hold` permanece válido e prende no último frame.
Câmera começando exatamente no end exclusivo ainda tem os frames anteriores reais.
O acervo S0 não foi alterado; gates/review do novo código são separados da aprovação R2.

Limite .24: Mac896 passou core1596/0/7,114transições/10corpus/1baseline,GPUI80,check.
Após ajustar só a forma da asserção do teste, ffa1f9c passou114+10+1,clippy/build.
A rechecagem Windows do limite é separada da R2 de d2c1861 e continua pendente.

R3 Windows aprovou4199af0 no escopo interno:114/10/1 emEXPECT0/1,admission4,
clippycore/tree/check, mais temporal20/handles21/plan27/splat15/oracle3 e4mutantes.
A cabeça contígua corrigida é contrato Tessel mais estrito que a fórmula inicial do plano.
Limite conhecido antes deS3a: em cameraIn além do fim, details.lacks.camera/dica ainda usa
before-head_real e pode subestimar a extensão necessária (60/48/before6 sugere6, faltam12)
ou ser conservadora (49/48 sugere6,1basta). A admissão recusa corretamente; corrigir diagnóstico
emBL082.4.24/S3a antes da ativação pública. Não se abriu o portão nem se mudou esse diagnóstico
neste fechamento documental. Build/GPUI/release não foram reexecutados pelaR3.

### X representável para hold do vizinho (BL082.4.17)

A extensão nova, scripts/pilot_dissolve_representable.py e seu corpus, adiciona7cenários válidos,
98quadros e390sondas sem alterar a S0. Instâncias e câmeras cobrem o próprio shot; hold ocorre
só no vizinho fora dele, evitando a adaptação12/30 e o contador primário legado(.23). Os5testes
Rust importam cada bundle pelo core real/Accept e mantêm a recusa pública. Nenhum consumidor
S1b/S2 ou pixel real foi executado. R1 Windows aprovou93246d1; Mac reproduziu37specs normal/-O,
14arquivos byte a byte e5Rust. Os ajustes posteriores dos nits precisam dos próprios gates.

Os papéis estão SOMENTE em frames[].primary/neighbour, pois mudam no corte. A convenção
asset_resolution mapeia assets/<nome>.png do bundle para dissolve-representable-assets/<nome>.png
junto ao corpus; o consumidor deve resolver/copiar o basename e conferir o hash do manifesto.
Rust compara alinhamento/duração importados e transition_window com o oráculo; o teste de imports
Python usa uma allow-list de stdlib e sintaxe3.9, sem sys.stdlib_module_names. Python3.9 real não
foi executado pelo coordenador; o atributo ausente foi simulado no intérprete atual (Mac e Windows R2:37testes).

Hold da câmera não se distingue em pixel se a pose já satura. held_camera/camera_clip_frame só
se provam por ESTADO no consumidor S1b. UNH concorda em pixel nas cenas estáticas; instance_frame
o distingue. O viés≤0,25 desta referência é agregado por cenário com≥24sondas únicas; o texto
canônico fala em sondas do quadro. S2 deve declarar a granularidade antes da primeira execução
do engine, sem retunar limiar depois. F064 produto continua P; origem não executada, sem golden.

No reforço026b72a, Mac passou37normal/-O e37com metadata opcional de3.10 ausente,14arquivos
reproduzidos, verify com pin em blobs e5Rust/clippy do alvo. Dois mutantes novos de duração e
alinhamento morreram; dados restaurados. Windows pós-correções a validar. Gramática3.9 foi
parseada; não houve execução de Python3.9 real nem de origem/renderer.

R2 Windows aprovou93037e0:37normal/-O e37sem metadata opcional,14arquivos,
verify/blobs/pin ausente4,5Rust/clippy, mutantes alinhamento/duração/janela mortos.
A verificação recebeu ainda um guard de inventário: generate--check/verify agora recusam
qualquer entrada não declarada na pasta exclusiva de assets; não apagam arquivos extras.
Mac38specs normal/-O passaram com o novo controle. Dados de sondas/bundles/limiares/PNGs
não mudaram. R3 Windows aprovou `06c8ea9`: 38 specs normal/-O, 14 arquivos,
verify offline/blobs, pin estrito ausente com exit 4, Rust5, fmt e clippy do core,
tree sem GPUI/wry. O PNG extra falha nas duas portas sem ser apagado; remover o
filtro numa cópia faz a regressão falhar. A origem permaneceu intacta e não foi
executada. Mac e Windows são provas separadas; não há consumidor S1b/S2 nem pixel real.

Consistência posterior (`1de7bb4`): `generate` também verifica a saída após gravar
e retorna 1 se houver arquivo extra, sem apagá-lo. Mac39 specs normal/-O e controle
de escrita por blobs reais passaram; dados/limiares/Rust intactos. A rechecagem
Windows dessa pequena mudança de I/O é separada da R3 de `06c8ea9` e foi aceita
em `9bafc43`: 39 specs normal/-O, check14 e verify passam; geração real por blobs
retorna 0 limpa e 1 com extra preservado, assim como check/verify dessa saída.
O mutante da comparação falha por asserção. Nenhum Rust/build/GPU reexecutado
nessa rodada; os cinco Rust continuam sendo prova da R3. Origem não executada,
pin ausente no Windows não verificado; consumidor S1b/S2 e pixels pendentes.

## S3a (BL082.4.5, BL082.4.15, BL082.4.24): edição de `crossDissolve` no core — admissão **fechada**

Plano de execução: [p4b-s3a-editing-plan.md](../migration/p4b-s3a-editing-plan.md). Esta seção fixa as decisões de API
**antes** do código que as implementa; ela especializa o §9 do [plano canônico](../migration/p4b-cross-dissolve-plan.md).
O portão público continua `Refuse`: nenhuma operação abaixo é alcançável com sucesso por `scene.attach`, `scene.fx.edit`,
`tessel-headless` ou pela janela. Só testes com a política interna `Accept` (herdada pelo candidato) as exercitam.

### Operações (`scene.fx.edit`, mesmo `FxOp`, mesmo candidato)

| Op (`op`) | Campos | Efeito no documento |
|---|---|---|
| `set_transition` | `fromShotId`, `toShotId`, `effect`; `id?`, `durationFrames?`, `alignment?`, `handles?`, `easing?`, `params?`, `hidden?` | Upsert **por corte** (par de shots exatos) |
| `resize_transition` | `id`, `durationFrames`; `alignment?` | Troca a janela; o importador julga vizinhança, edição, sobreposição e handles |
| `set_transition_param` | `id`, `param`, `value` | Um parâmetro do catálogo da transição (`curve`, `intensity`) |
| `set_transition_hidden` | `id`, `hidden` (booleano) | `true` grava `hidden: true`; `false` **omite** o membro |
| `remove_transition` | `id` | Remove; id ausente é `UNKNOWN_TRANSITION`, nunca sucesso silencioso |

- **Sem aliases.** Os nomes são os do documento (`fromShotId`…); `from_shot_id` é recusado como campo desconhecido.
  Inteiros são estritos (`12.0`, `12.5`, texto e booleano são recusados na leitura da op como `invalid_op`).
- **Upsert existente** conserva `id` e todo campo omitido, inclusive os membros desconhecidos da transição (os de
  `params` são recusados pelo importador, `UNKNOWN_PARAM`, então não existem num documento válido). `params` faz
  *merge* só das chaves enviadas. Um valor **igual** ao do documento (comparado já tipado: `1` e `1.0`) não reescreve a
  grafia autorada; um parâmetro que o documento **não** grava (lê-se como o padrão) e que a op envia com o valor padrão é
  uma mudança: o padrão passa a estar escrito. Um `id` explícito diferente do existente é recusado (`TRANSITION_ID_MISMATCH`): o Tessel **não**
  segue a spec de origem aqui (ela ignora o `id` do comando com uma nota).
- **Upsert novo** grava `id`, `effect`, `fromShotId`, `toShotId`, `durationFrames`, `alignment`, `handles`, `easing` e
  `params` completos (`curve`, `intensity`) com os padrões `center`, `hold`, `easeInOutCubic`, `smooth`, `1`; a duração
  padrão é `default_transition_duration` (24 típica, cortada a `maxTransitionFrames`); máximo 0 recusa com
  `TRANSITION_TOO_LONG`. Um valor explícito ilegal **nunca** é cortado para ser aceito. `id` omitido é gerado como
  `tr-<n>`.
- **Namespace de ids (BL082.4.15).** Trilhas, clipes **e transições** dividem o espaço; a verificação de duplicata é
  exata e a de reservados (`track`, `fx`, `transition`) não distingue maiúsculas. O alocador de ids de `add_clip` e de
  trilha passa a considerar as transições (antes, uma transição `fx-1` podia colidir com um clipe gerado).
- **Um só candidato por lote.** Todas as ops do lote atuam numa cópia JSON; **um** `reimport` (política herdada) julga o
  resultado. Uma op inválida recusa o lote inteiro; `ops: []` é identidade; ida-e-volta líquida (criar e remover) não
  modifica nada; um no-op marca `noop` na mudança e não cria revisão nem entrada de undo; a edição real cria **uma**
  entrada de undo. `dry_run` roda o mesmo caminho e não grava; recusa em dry-run continua recusa.
  `base_revision` obsoleta é `stale_revision` antes de qualquer cópia. Teto de 1024 ops antes da cópia (existente).
- **`FxChange` de transição:** `track` vazio, `clip` = id da transição, `transition` = `Some(id)`; o JSON da mudança
  troca `track`/`clip` por `transition` e mantém `index`, `action`, `target`, `before`, `after`, `noop`.
  O ponteiro usado para atribuir problemas a ops é `/fx/transitions/<i>`.
- **Gesto de look.** `scene.fx.gesture.*` recusa uma op de transição com `gesture_op_not_allowed` e mensagem própria,
  sem mudar candidato, intenção nem revisão (a recusa fica registrada como a mais recente, como a de qualquer op de
  gesto recusada). Não existe gesto de transição (S3b/S4b).
- **`UNKNOWN_SHOT` de op.** Um shot que a edição não tem recusa o `set_transition` com a lista dos shots; a frase nomeia
  a transição pelo `id` pedido ou, na falta dele, pelo que seria gerado (`tr-<n>`), pois num corte com shot ausente não
  existe transição.
- **N/A:** troca de efeito (só existe `crossDissolve`); `keys`/escopo de nó de transição (`FX_TRANSITION_KEYS_NOT_YET`);
  comando de editar shots (P5/BL082.5, ver abaixo).

### Dois níveis de código e o envelope de erro

| Nível | Onde aparece | Exemplos |
|---|---|---|
| **`CommandError.code`** (envelope) | `error.code` da resposta de `scene.fx.edit` | `invalid_bundle` (candidato recusado), `invalid_op`, `unknown_transition`, `transition_id_mismatch`, `stale_revision`, `gesture_op_not_allowed` |
| **`Issue.code`** (regra) | `error.details.issues[].code` e `error.details.errors[].code` | `FX_TRANSITION_NOT_YET`, `TRANSITION_OUT_OF_RANGE`, `TRANSITION_TOO_LONG`, `TRANSITION_NO_HANDLES`, `UNKNOWN_SHOT`, `FX_LIMIT`, `DUPLICATE_ID` |

- Uma recusa do **candidato** (`invalid_bundle`) traz `details.issues[]` (formato **existente**, intacto) e agora também
  `details.errors[]`: a mesma lista na forma `{code, message, hint, transitionId?, shotId?, details}`, em **inglês**
  (o envelope não tem `locale` nesta fase; não se alega seleção pública de idioma no JSONL).
- Um erro **da op** (antes do candidato: `UNKNOWN_TRANSITION`, `TRANSITION_ID_MISMATCH`, `UNKNOWN_SHOT`, `TOO_LONG` sem
  espaço…) traz o código do envelope em minúsculas e `details.errors[]` com **um** item cujo `code` é o do nível de
  regra. Sucesso e no-op não ganham `errors` vazio. Erros de **look** não ganham `errors[]` nesta fase (formato igual).
  Essa projeção começa após o parsing das ops tipadas. Erros de sintaxe/forma no parser,
  como alias `from_shot_id` ou duração fracionária, continuam `invalid_op` sem `errors[]`.
- Projeção pura, uma só função do core para biblioteca, dry-run e futuro inspector:
  `directed_scene::fx::project_problem(problem: &RawProblem<'_>, locale: Locale) -> TransitionProblem` e
  `project_issues(issues: &[Issue], locale: Locale) -> Vec<TransitionProblem>`. `Locale` é o do Kit/core (`pt-br`,
  `en`, `es`). Os textos moram numa tabela do core indexada por código (e por `código:variante` quando a mensagem muda
  de forma, ex.: `TRANSITION_OUT_OF_RANGE:duration_below_one`); um teste enumera **todos** os códigos e variantes e exige
  as três línguas com os mesmos marcadores `{…}`. Um código **fora** dessa tabela não cai num texto inventado: devolve o
  texto original do importador (inglês) e `details.untranslated = true`. Problemas estruturais genéricos (tipo, campo
  ausente, valor inválido de campo livre) usam um texto localizado com `{path}` e levam a razão original, em inglês,
  em `details.reason`.
- `details.args` (aditivo, **não** substitui `hint`) leva os números e ids de que o texto precisa; é a entrada da projeção.
  O corpus mantém code/message/transitionId/lacks. Uma adaptação explícita de vocabulário
  troca apenas `setTransition` por `set_transition` na dica de TRANSITION_DUPLICATE, pois
  o comando camelCase do piloto não existe neste editor. O fixture fixado não é regravado.
- Código de recusa de janela de overlay: `transition_window` (nível de **gesto**, ver predicado abaixo), com texto nas
  três línguas na mesma tabela.

### BL082.4.24: diagnóstico de handles com `cameraIn` além do fim

A cabeça real contígua exige `cameraIn >= before` **e** `cameraIn <= total` (câmera de `total` quadros); as duas faltas
são **independentes** e o reparo de cada uma é diferente. `details.lacks[]` ganha, só quando a falta de fim existe,
`cameraEnd = cameraIn − total` (junto de `cameraIn` e `cameraFrames`); `camera` passa a ser **só** a falta de início,
`max(0, before − cameraIn)`, e `instance` não muda. Sem `cameraEnd` o formato e os valores continuam byte a byte os do
corpus. A dica oferece o reparo de cada falta: (início) estender o clipe de câmera em `camera` quadros antes do início
(`extend its camera clip … by N frames before its start`); (fim) estender o clipe de câmera em `cameraEnd` quadros no fim. Exemplos: `cameraIn 60`, câmera 48, `before 6` →
`cameraEnd 12` (acrescentar 6 **não** basta); `49/48` → `cameraEnd 1`; `48/48` válido; `cameraIn < before` → só a falta
de início (estender o fim não resolve); as duas faltas juntas (`cameraIn 4`, câmera 3, `before 6`: `camera 2`,
`cameraEnd 1`) pedem os dois reparos, nenhum basta sozinho; `before 0` não exige cabeça, **nem** a falta de fim (uma
janela alinhada ao início não lê nada antes do corte). A admissão **não** é enfraquecida.

**Análogo na cauda (BL082.4.25), corrigido na S2-C0a (seção abaixo):** a cauda do plano que sai tinha a mesma
subestimativa. Com `cameraIn 40` e câmera de 60 quadros num plano de 48, a janela pede 6 quadros após o fim e a dica
mandava estender o clipe em 6, mas é preciso `cameraIn + duração + after − total = 34`; estender 6 continuava
recusado. Agora a dica e o campo aditivo `cameraExtend` dizem 34.

### Predicado de janela (§9.6, sem consumidor ainda)

`directed_scene/fx/window_guard.rs`: `Fx::overlay_refusal(&self, shots: &[Shot], edit_duration_frames: u64, edit_frame:
i64, target: OverlayTarget) -> Option<WindowRefusal>` (o esboço do plano não tinha `shots`/duração, necessários para
escolher a janela). Reaproveita a **mesma** escolha de `resolve_frame` (`frame::active`: transição visível, janela
`[início, fim)` meio-aberta). Transição `hidden` não recusa. Sem estado e sem ordem. Nenhum overlay real é ligado: o
gizmo, o press e as alças continuam como estão; a S4a consome esta decisão.

**Alvo de edição durante a janela (registro, não implementado).** O dono de uma entidade é o seu escopo
(`FxScope::Shot(id)`, `scope_start..scope_end`); o shot de um quadro é `shot_at`. Uma propriedade estática sem chave não
depende do quadro; uma chave de look vale em quadro **explícito** dentro do `editRange` do dono. Uma entidade do deck
vizinho sem quadro explícito válido é recusada com `neighbor_deck_frame_out_of_shot` (código **proposto**, ainda sem
produtor); nunca se grava uma chave fora do `editRange` do plano dono.

### Shot removido ou encurtado (P5 inexistente)

Não existe comando de editar `shots[]`. A S3a prova só pelo importador do candidato: substituir ou anexar um documento
sem o shot citado, com o shot encurtado ou não vizinho é recusado com o código da regra e **nada** é cortado nem
consertado. **N/A:** edição de shots e reavaliação de transições pela P5 (BL082.5).

### Prova Mac da integração S3a

No head limpo `33323a2` (main79ed312 incorporada sem conflito de código), core
completo:1666/0/7 em65 resultados, com lib605/transições170/corpus10/X5/baseline1.
Com preserve_order:170/10/5/1; desktopFX80/80; fmt/check/clippy-all-targets/no-run/
build passaram e tree sem GPUI/wry. Windows continua como prova separada: autor934
relatou dois gizmos da base vermelhos; revisão independente integrada pendente.
Sem workspace global executado, runtime, GPU, origem ou UI física. F064 produto P,
Refuse fechado; .25 (dica da cauda) e consumidor do predicado continuam pendentes.

### Correções após a revisão Windows R1

R1 validou o escopo interno em33323a2, com dois gizmos da base vermelhos e sem
workspace global verde. Achado BL082.4.26: ida e volta numérica no mesmo lote
(`intensity` autorado1 →0.5→1) escrevia1.0 e gerava revisão/undo. O fechamento
restaura a grafia anterior de parâmetros presentes cujo valor tipado voltou ao
original, preservando a distinção entre parâmetro omitido e default escrito.
Vale também se outra propriedade do mesmo lote mudou; defaults não são apagados.
As provas novas incluem documento/revisão/histórico em dry-run e commit, hints
traduzidos, erro de look em lote misto e orçamento candidato ao adicionar/remover
transição. Esses ajustes ainda precisam de gates próprios; não herdam a R1.

BL082.4.28: contagens obrigatórias ausentes/inválidas na projeção preservam
mensagem/hint originais e marcam `untranslated`; zero válido não é ausência.
A atribuição de erro retém o tipo clip/transição ao reutilizar um ID num lote.
A dica TRANSITION_DUPLICATE usa `set_transition` nas três línguas e no importador,
com a adaptação de vocabulário do corpus registrada acima.

### Aceite interno S3a após R2 Windows

R2 aceitou512685a/códigob097979:175/10/5/1 debug e preserve_order,lib606,
desktopFX80,gatesfmt/check/clippy/no-run/build0. Dois mutantes R1 agora mortos;
controle repetido após restauração. Mac1673/0/7 é prova separada. Sem execução
do workspace global, runtime/GPU/UI física; Refuse fechado e F064 produto P.

Ressalvas de cobertura: guarda effect/from/to e cameraEnd inválido sem testes
permanentes discriminantes, embora probes confirmem o comportamento; E13 novo
provava edit/list/inspect, não frame/gesto. Naquela S3a, reutilização de ID no mesmo
tipo ainda podia atribuir erro à remoção anterior(.29); a C0a abaixo corrige esse
caso por gerações e acrescenta a cobertura permanente de frame/gesto e contagens.
No-op líquido relata alterações intermediárias por op, sem revisar/gravar o
resultado líquido. A dica da cauda(.25), pendente naquela S3a, também é corrigida
na C0a abaixo; o relatório S3a§13 conserva a evidência histórica.

## S2-C0a (BL082.4.19, .25, .26, .27, .28, .29): pré-requisitos do core — admissão pública **fechada**

Entrega só do core (sem UI, sem engine, sem Domain novo, sem `Accept` em produção). O produto F064 continua `P` e a
admissão pública continua `Refuse`. Contrato fixado **antes** do código; evidências e mutantes no relatório
[p4b-s2-c0a-report.md](../migration/p4b-s2-c0a-report.md).

### BL082.4.29: proveniência de uma entidade num lote

Um id não é uma identidade. Dentro de **um** lote, `add_clip` e o `set_transition` que escreve um corte novo **criam**
uma geração de `(tipo, id)`; `remove_clip`/`remove_transition` a encerram; os demais ops pertencem à geração viva.
Um problema que o importador acha na entidade do candidato é atribuído (`ops` do problema e `index` do erro) só aos ops
da geração que **sobrevive** no fim do lote. Os ops de uma geração removida não recebem ponteiro: se `x` é criado,
removido e criado de novo, só o terceiro op responde por `x`. Um op sem efeito (no-op) continua sem ponteiro. A regra é
privada ao pipeline de edição (`scene_edit/fx_lineage.rs`): nenhuma chave de documento, nenhum tipo público e nenhuma
mudança nas respostas de um lote aceito; a distinção clip/transição da .28 continua valendo. Não é rastreamento causal
por propriedade: um id removido e recriado é outra entidade, mesmo no mesmo tipo e no mesmo corte.

### BL082.4.19: contexto do deck vizinho no plano

`ScenePlan::build_with` e `PlanTimeline::build_with` qualificam, **por deck**, todo erro dos passos do **vizinho**
(avaliar o frame do offset, `Prepared::new`, domínios não suportados, montagem, transformação de splat) com o
`within_other` que já existia na validação do plano: o ponteiro vai para `/transition/other…` e o erro é `Invalid`
(os tipos `Frame`/`Unsupported`/`SplatTransform` do vizinho viram texto sob esse ponteiro, como já ocorre na leitura do
JSON). Os passos do deck de cima não mudam: o mesmo erro, no mesmo tipo e ponteiro. Sem transição na janela nada muda.
Os dois construtores respondem igual; o prefixo nunca é aplicado duas vezes.

### BL082.4.25: reparo da cauda de câmera que sai

Com `cameraIn + duração + after > total`, estender o clipe em `camera = after − tail_camera` não basta quando a câmera
já termina **antes** do fim do shot (`tail_camera` é saturado em 0). O reparo exato do fim do clipe é
`cameraExtend = max(0, cameraIn + duração + after − total)` (aritmética checada; num estouro de `i64` satura em
`i64::MAX` **só** no caminho de recusa, nunca grava nada). Contrato:

- `details.lacks[]` do lado `after` ganha `cameraExtend` **somente** quando difere do `camera` legado; `camera`,
  `instance` e o resto mantêm os valores e os bytes de hoje. Sem a divergência o formato é byte a byte o do corpus.
- A dica da cauda manda estender o clipe em `cameraExtend` quadros após o fim. A mensagem continua dizendo o que falta
  à janela (`camera`), não o reparo.
- `details.args.lacks[]` leva sempre `cameraExtend` (igual a `camera` quando não diverge); a projeção localizada usa-o
  no reparo, nas três línguas. Um `cameraExtend` presente e inválido (não inteiro ≥ 0) preserva mensagem/dica originais
  e marca `untranslated` (regra da .28); ausente, vale `camera`. O lado `before` não muda (`cameraEnd` da .24).
- A admissão **não** muda: o que era aceito/recusado continua; só o número do reparo fica certo. Exemplo (`cameraIn 40`,
  câmera 60, shot 48, `after 6`): `cameraExtend 34` ⇒ câmera de 94 aceita, 93 e 66 recusadas.

**Divergência explícita do corpus:** três linhas do oráculo comparam a dica antiga da cauda
(`corpus-center-12-use-lacks-camera-and-head`, `corpus-center-12-refuse-reads-as-use`: 6→11;
`corpus-start-12-use-needs-only-the-tail`: 12→17), porque a câmera do `shot-a` (40 quadros, `cameraIn 5`) termina 5
quadros antes do fim do shot. O oráculo Python e as fixtures **não** foram regravados; o consumidor Rust declara essas
três linhas como divergência conhecida da dica (a lista de lacks, a admissão e o veredito continuam iguais).

### BL082.4.26, .27, .28: lacunas permanentes de teste

Provas permanentes novas (sem mudar produção): a guarda de `effect`/`fromShotId`/`toShotId` na restauração de grafia
(caso real num lote + teste defensivo do helper), `cameraEnd` e toda contagem inválida (texto, negativo, fracionário,
nulo, `3.0`) e o orçamento de 9 passes dentro da janela e 8 fora, em `scene.fx.frame`, nas respostas de orçamento e
num gesto sobre um look. As ressalvas da S3a sobre esses pontos ficam fechadas pela suíte, não por inspeção.

### Validação Mac da S2-C0a

Em9f7cc83 limpo, código igual ao autor37fdeaf: core1699/0/7 em65 resultados,
lib616/transições192 inclusos; preserve192/10/5/1 e desktopFX80/80 passaram.
Fmt/check/clippy-all-targets/no-run/build0, árvore sem GPUI/wry. Workspace global
não executado, sem engine/GPU/desempenho/origem/UI física. Windows independente
pendente e dois gizmos da base Windows continuam registrados como vermelhos.
Prova por head no relatório S2-C0a§11; produto P e Refuse fechado.

### Aceite Windows independente da C0a

R1 aprovou9f7cc83:192/10/5/1 em debug/preserve/release,lib615,desktopFX80,
fmt/check/clippy/no-run/build0. Mac1699/0/7 é separado. .19/.25/.26–.29 estão
implementadas e validadas no escopo descrito, com quatro costuras defensivas .19
sem prova de alcançabilidade por cena importada (relatório§12). Sem core completo
ou workspace global executado nessa revisão Windows; gizmos da base não ficam verdes.
Refuse/F064P e C0b/H/R/S2A pendentes; não é aceite de produto/render.

### BL082.4.4, .17, .20, .21, .23: relógios do plano com transição (S2-C0b)

O plano de uma janela passa a declarar `alignment` e um bloco `timing` obrigatório (cena, instância e câmera dos dois
shots, mais os relógios amostrados): contrato em [scene-plan.md](scene-plan.md) e no plano
[p4b-s2-c0b-plan.md](../migration/p4b-s2-c0b-plan.md). Uma **única** regra pura de relógio
(`directed_scene::ClockSample`/`DeckClocks`: bruto, amostrado `clamp(bruto, 0, L−1)`, preso `L == 0 ∨ amostrado ≠ bruto`)
alimenta o avaliador de deslocamento, o montador e o validador; o último quadro real não é preso, o comprimento de câmera é
o da maior faixa (principal ou camada) e o primário guarda os quadros brutos (a câmera de um primário curto continua em
bruto 23 / amostrado 11 no caso legado). O wire experimental sem `alignment`/`timing` é recusado em
`/transition/alignment` e `PLAN_VERSION` continua 1; planos sem transição não mudam um byte.

BL082.4.20: cada instância deve cobrir todo o próprio shot, tanto no primário quanto no vizinho.
O validador verifica `instanceStartFrame + (shotEndFrameExclusive − shotStartFrame) <= instanceDurationFrames`
nos dois lados e recusa no ponteiro `/transition/timing/<lado>/instanceDurationFrames`.
Hold da instância do vizinho continua válido fora do próprio shot; câmera curta mantém seu contrato.
Esse reforço posterior à PR71 corrige a checagem que alcançava apenas o primário.

Provas: um plano **válido** de exatamente 20.000 itens é aceito em `validate`, `from_json` e `from_json_str`;
um passe ou uma luz a mais é recusado em `/transition`. O texto de exatamente 4.194.304 bytes é aceito por
`from_json_str`, e um byte a mais é recusado nessa porta textual (BL082.4.21); a referência representável do PR65
(`dissolve-representable-cases.json`) é lida como **estado** pelo plano real, sem Python e sem pixel (BL082.4.17).
Continua fora: o gate público (`Refuse`), runtime/shader/export, UI e qualquer pixel. Evidências, mutantes, limites e os
patches do coordenador: [p4b-s2-c0b-report.md](../migration/p4b-s2-c0b-report.md).

### BL082.4.4: carregadores do host leem a união dos dois decks (S2-H)

Os quatro carregadores de recursos do renderer (`SceneResourceMap::from_root`, `SceneSlots::resolve`/`prepare`,
`scene_operation::approved_map` e `scene_export::resource_map`) passam a percorrer `ScenePlan::resource_union()` em vez
de `plan.resources`: um `assetId` que só o deck vizinho (`transition.other`) usa deixa de ser perdido, e um `assetId`
compartilhado é resolvido, aprovado e carregado **uma vez**. A aprovação continua por `(cena do projeto, Asset(assetId))`;
nenhum id de nó, `scene_definition_id`, `scene_instance_id`, shot ou papel de deck entra na chave. Metadados divergentes
para o mesmo `assetId` seguem recusados na validação (`/transition/other/resources/N`) **antes** de resolver, abrir
arquivo ou tocar a raiz. Todo erro atribuível a um recurso carrega `details.resource = {asset_id, used_by:[{deck, node_id}]}`
e a mensagem nomeia o(s) deck(s). Falha estrutural de uma entrada nomeia só o deck afetado;
um conflito real da união nomeia ambos. O código do erro não muda (`resource_unknown` continua `resource_unknown` no
`approved_map`). Erros globais (raiz inexistente, teto total) não recebem atribuição inventada. O custo de splats conta
**cada uso qualificado** dos dois decks (ids de nó iguais em decks diferentes contam duas vezes) e os bytes de um arquivo
compartilhado contam uma vez. Custo de imagem ou soma total não representável é recusado com `resource_budget`,
inclusive com orçamento `u64::MAX`, antes de mudar slots; a conta de eviction também é verificada. Continua **sem** admissão pública (`Refuse`): os
testes usam a admissão oculta `Accept` somente em `crates/render/tests`. Plano, mutantes, limites e os patches do
coordenador: [p4b-s2-host-resource-report.md](../migration/p4b-s2-host-resource-report.md).

H foi aprovada internamente pela R2 Windows emd7c52b7 (código05f37da, testesfe14cfc):
render76, core216/10/5/8/1 isolado/preserve e gates de compilação0. Export Windows10/12
só por shasum de BL167.3; Mac8a489bf12/12 é prova separada do caminho antigo. R/S2A,
pixels de transição e todas as superfícies de produto de F064 continuam pendentes.
