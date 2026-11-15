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
- **Prova (Windows, head de código desta entrega).** `scene_fx_transition` 106 testes e `pilot_dissolve_corpus` 10, nos
  dois perfis de `serde_json` (com e sem `preserve_order`); core completo com 2 falhas de base (`scene_gizmo_rotation`,
  `scene_gizmo_scale`, também na base sem alteração); GPUI `scene_fx` 80. 18 mutantes essenciais mortos.
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
