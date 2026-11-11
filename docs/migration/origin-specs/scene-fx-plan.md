# Scene effects: transições, efeitos contínuos e suspense, como camada do engine

Status: **plano, revisão 3** (2026-11-02; lido por três revisores independentes em duas rodadas; cada achado
foi tratado: ver [§16](scene-fx-decisions.md) e as decisões fechadas em [§17](scene-fx-decisions.md)). Frente VGP (produção audiovisual), épico JOU-1175.

Depende do padrão de `docs/video-production/scene-engine-api-plan.md`, **já na `main`** (`2bfae8ef25`, #814): propriedades no motor, `editBundle`, módulo Node, CLI `scene`, preview fino. Este plano constrói a
camada de efeitos **nesse padrão**, para que a mesma camada sirva à biblioteca, à pipeline de geração de vídeos,
às trilhas do editor, às camadas/outline e à galeria. Todas as fases estão destravadas quanto à API (§12, §19).

## 1. Objetivo

Dar ao Scene Studio e à pipeline os efeitos que os editores de vídeo (Premiere, DaVinci Resolve, Final Cut,
CapCut) oferecem, com foco nos que servem a cenas **dark**: transições entre planos, efeitos contínuos
(aparência) e efeitos de **suspense**. Cada efeito é um dado do bundle com parâmetros tipados, resolvido por uma
função pura do frame de edição, desenhado por um compositor de passes e editável por três portas que dão o mesmo
resultado: a biblioteca (JS), a CLI `scene fx …` e o editor (trilhas, galeria, inspector).

Critério de sucesso do plano inteiro: o vídeo de 3 planos (`gen-lab/full-v2`) refeito com transições de tipos
diferentes escolhidas por um script da pipeline; os mesmos efeitos editados no Studio (arrastando a duração na
trilha); e o mesmo bundle renderizado pelo `scene render` com **o mesmo `FxFrameState` e os mesmos pixels no
mesmo ambiente de GL** (entre GPUs/drivers, dentro da tolerância dos golden stills, §9.4).

### Princípios (valem para todo efeito)

1. **Função pura do frame.** O que se vê no frame `f` depende só de `(bundle, f)`: sem relógio, `Math.random()`,
   buffers de história ou acúmulo entre frames. Ruído vem de hash inteiro de `(seed, frame, pixel)` (§7.2).
   Seek em qualquer ordem dá os mesmos pixels do play linear.
2. **Dado, não código.** Um efeito no bundle é `{ effect: "<id>", params, keys }`. O código mora no engine,
   num catálogo versionado. Bundle com efeito desconhecido é recusado, nunca ignorado em silêncio.
3. **Uma única porta de edição.** Toda edição é um comando da camada (`applyFxCommands`) ou uma property
   (`fx/...`); `editBundle` as aceita; a CLI, o harness e o Studio chamam isso e nada mais. Nenhuma lógica de
   domínio em shell.
4. **Compatível.** Bundle sem `fx` continua válido e renderiza **literalmente pelo caminho de hoje** (§8.1).
5. **Recusa com ajuda.** Cada erro tem `code`, `message` e `hint` (formas aceitas, limites, o que fazer).

## 2. Como os outros apps fazem (e o que levamos)

| Prática dos editores | O que fazemos |
| --- | --- |
| Transição vive **no corte** entre dois clipes, com tipo e duração; arrasta-se a borda para mudar a duração | Transição ancorada num corte (`fromShotId`, `toShotId`), com `durationFrames` e `alignment`; bordas arrastáveis na trilha "Transições" |
| Precisa de **handles** (material depois do fim do clipe de saída e antes do início do de entrada) | Os planos têm `instanceStartFrame` e a instância tem mais quadros que o plano; usamos esses quadros, e sem eles `handles: "hold"` congela o último/primeiro quadro (o "freeze frame" dos NLEs) |
| Efeitos de vídeo são **clipes em trilhas** com parâmetros keyframeáveis | Trilhas `fx` com clipes e `keys` por parâmetro; pilha por ordem de trilha |
| Painel de efeitos com **galeria** e busca; arrastar o efeito para o clipe ou o corte | Galeria com miniaturas, tags (`dark`, `suspense`, ...) e arrastar-e-soltar |
| Camada de ajuste vale para tudo abaixo | `scope`: `output`, `shot:<id>` ou `node` (só planos/meshes) |
| Presets ("looks") que juntam vários efeitos | `preset` = receita que **expande** em clipes comuns (não é caminho de render à parte) |
| Easing e keyframes por parâmetro | `keys` por parâmetro, mesmo `resolveKeyframes` e os easings que o engine já tem (§6.3) |

## 3. O padrão que seguimos

Três camadas finas (`scene-engine-api-plan.md` §1): **engine puro** (`@tessel/scene-engine/core`, comando
`bundle + comandos → { ok, bundle, notes } | { ok: false, errors }`, sem I/O), **módulo Node**
(`@tessel/scene-engine/node`: arquivos, trava de escrita, render) e **shells** (CLI `scene`, preview, Studio:
leem a entrada, chamam 1 ou 2, mostram; **nenhuma lógica de domínio**).

| Peça | Onde mora | Espelha |
| --- | --- | --- |
| Schema `fx` | `src/fx/fx-schema.ts` | `narration-schema.ts`, `outline-schema.ts` |
| Meta do catálogo (dados, 3 idiomas) | `src/fx/catalog/*.ts` | registro de propriedades |
| Shaders (só runtime) | `src/runtime/fx/shaders/<id>.ts` | `plane-effects-material.ts` |
| Comandos `applyFxCommands`, `explainFx` | `src/fx/fx-commands.ts`, `fx-explain.ts` | `applyNarrationCommands`, `explainNarration` |
| Resolução por frame `resolveFxFrame` | `src/fx/resolve-fx-frame.ts` | `captionsAt`, `resolveCameraAtClipFrame` |
| Propriedades `fx/...` | `src/properties/fx-properties.ts` | `key-properties.ts`, `light-properties.ts` |
| Variante `{ fx }` em `BundleEdit` | `src/edit/edit-bundle.ts` | variantes `narration`, `camera` |
| Compositor de passes (GL) | `src/runtime/fx/` | `weather-layer.ts`, `rig-layer.ts` |
| Render headless | `src/node/render.ts` | `renderBundle` |
| CLI | `bin/scene.mjs` → `scene fx …` | `scene narration …` |
| Skill | `.claude/skills/scene-fx/SKILL.md` | `scene-cli` |

Regras herdadas, sem exceção: pnpm; `pnpm type-check` (nunca `tsc`); sem ternário na lógica, sem `as`, `type` em
vez de `interface`, guard clauses, comentários em inglês que explicam o porquê; testes `.spec.ts` e nunca
`.skip`; i18n pt-br/en/es; nada de `git stash` (a lista é compartilhada entre worktrees).

## 4. Modelo de dados

Tudo opcional no bundle. Ids seguem `idSchema` da narração. **Tempo: frames de edição**, inteiros; as taxas
humanas dos efeitos (Hz, bpm) são convertidas com `bundle.fps` (racional) **dentro** de `resolveFxFrame`, e o
tempo `t` de qualquer efeito é `frameInClip / fps` (nunca relógio).

```ts
// src/fx/fx-schema.ts
type Locale3 = { "pt-br": string; en: string; es: string }
type FxParamValue = number | boolean | string | { r: number; g: number; b: number } | { x: number; y: number }   // `string` = enum ou id de referência

// Chave de parâmetro. `frame` é RELATIVO ao início do clipe (sobrevive a moveClip). easing: string validada
// contra CAMERA_EASINGS; o normalizador grava sempre o campo (padrão "linear").
type FxKey = { frame: number; value: FxParamValue; easing: string }

type FxScope =
  | { kind: "output" }
  | { kind: "shot"; shotId: string }
  | { kind: "node"; sceneId: string; nodeId: string }   // só imagePlane/supportPlane/overlay; splat = WRONG_SCOPE

type FxClip = {
  id: string                       // ÚNICO no bundle inteiro (todas as trilhas)
  effect: string
  startFrame: number               // inteiro >= 0
  durationFrames: number           // inteiro >= 1; o fim é exclusivo
  params?: Record<string, FxParamValue>
  keys?: Record<string, FxKey[]>
  scope?: FxScope                  // padrão: output
  seed?: number                    // inteiro 0..2^32-1; padrão: fnv1a32(id)
  fadeInFrames?: number            // inteiros >= 0: envelope de 0..1 sem mexer nos parâmetros
  fadeOutFrames?: number
  hidden?: true
}

type FxTransition = {
  id: string                       // único no bundle (ids de clipe e de transição não colidem)
  effect: string                   // família "transition"
  fromShotId: string
  toShotId: string                 // o plano seguinte a fromShotId em editRange
  durationFrames: number
  alignment: "center" | "start" | "end"
  params?: Record<string, FxParamValue>
  keys?: Record<string, FxKey[]>
  seed?: number
  handles: "use" | "hold" | "refuse"
  easing: string                   // do progresso 0..1 (padrão "easeInOutCubic")
  hidden?: true
}

type FxTrack = { id: string; label: string; clips: FxClip[]; hidden?: true }
type Fx = { catalogVersion: number; tracks: FxTrack[]; transitions: FxTransition[] }
```

Ids de trilha, clipe e transição compartilham **um** espaço (`DUPLICATE_ID` entre os três) e `track`, `fx` e
`transition` são reservados (`RESERVED_ID`), porque os caminhos de propriedade usam esses segmentos (§6.1).
`addTrack` num bundle sem `fx` cria `{ catalogVersion: FX_CATALOG_VERSION, tracks: [...], transitions: [] }`.

### 4.1 Regras do schema (cada uma com `code` e `hint`)

- `effect` existe no catálogo e tem a família certa (`WRONG_FAMILY`). `catalogVersion` maior que a do engine é
  recusada (`CATALOG_TOO_NEW`); menor é aceita só se nenhum efeito do bundle mudou de resultado desde então
  (tabela de mudanças no `fx-catalog.md`), senão `CATALOG_MIGRATION_NEEDED` com a dica. O registro de mudanças é **código**, não markdown:
  `FX_CATALOG_CHANGES: { version: number; effects: string[] }[]` (o `fx-catalog.md` só o espelha).
- Parâmetros: nome conhecido, tipo certo, dentro de `min`/`max`/`values` (`UNKNOWN_PARAM`, `PARAM_TYPE`,
  `PARAM_OUT_OF_RANGE`). Chaves: `frame` em `[0, durationFrames)`, crescentes, sem repetir; tipos iguais ao do
  parâmetro (`fxKeyIssues`, variante com deslocamento de `keyframeIssues`: o engine tem uma só regra de ordem,
  parametrizada pelo início do clipe). `fadeInFrames + fadeOutFrames ≤ durationFrames` (`ENVELOPE_TOO_LONG`).
- Clipes de uma trilha não se sobrepõem (`FX_CLIP_OVERLAP`); `moveClip` com `trackId` confere a trilha de
  destino. Sobrepor é outra trilha. `FX_CLIP_EMPTY` para duração < 1. `FX_CLIP_OUT_OF_RANGE` fora do timeline.
- `scope.shot`: o clipe deve estar dentro do `editRange` do plano (`FX_SCOPE_MISMATCH`, recusa; o editor
  oferece "ajustar ao plano").
- Transição: `toShotId` é o plano seguinte a `fromShotId`; no máximo uma por corte (`TRANSITION_DUPLICATE`);
  `durationFrames` cabe nos handles do alinhamento e no timeline (fórmulas em §8.4); **as janelas de cortes
  vizinhos não se intersectam** (`TRANSITION_OVERLAP`).
- `renderProfile.alphaMode !== "opaque"` com `fx` não vazio é **erro** de `explainFx` (`UNSUPPORTED_ALPHA`),
  para o `scene validate` ver antes do render. **Um `dissolve` legado conta como transição** (é desenhado pelo
  compositor, §4.2): `explainFx`/`explainFxErrors` aplicam a ele as mesmas regras (janela, `TRANSITION_TOO_LONG`,
  `TRANSITION_OUT_OF_RANGE`, `TRANSITION_OVERLAP`, `UNSUPPORTED_ALPHA`) **mesmo num bundle sem `fx`**
  (decisions #49).

### 4.2 Compatibilidade com `shot.transitionInto` (decidido, não aberto)

Hoje `transitionInto` é obrigatório em todo corte (`validateEditJoins`: `MISSING_TRANSITION`) e vale
`continuous` (exige continuidade de pose da câmera), `cut` (com `reason`) ou `dissolve` (recusado até a F2-b; aceito depois dela). Regra nova:

| Situação | Resultado |
| --- | --- |
| `transitionInto` | **continua obrigatório** em todo corte; a junção continua validada como hoje |
| `fx.transitions` num corte | é **aditivo**: só diz como desenhar os quadros em volta do corte; não substitui `transitionInto` |
| `continuous` + transição fx | válido, com aviso no `explainFx` (`TRANSITION_ON_CONTINUOUS`: os dois lados mostram a mesma instância) |
| `cut` + transição fx | válido; a razão do `cut` continua exigida |
| `dissolve` legado | passa a ser **aceito** e é lido por `normalizeTransitions` como `crossDissolve`, `alignment: "center"`, `handles: "hold"`, com `durationFrames` do próprio `dissolve`; escrever `fx.transitions` e `dissolve` no mesmo corte é `TRANSITION_DUPLICATE` |

**Feito na F2-b (decisões §18 #35).** A recusa saiu de `render-bundle.schema.ts` (o laço do `superRefine`; o
`transitionSchema` já aceitava) e de `validate/edit-joins.ts` (o ramo `DISSOLVE_UNSUPPORTED` virou um `continue`:
o `dissolve` entre **instâncias diferentes** passa sem checagem de pose, como o `cut`; `MISSING_TRANSITION` segue
exigido). Os três specs que o plano dizia afirmarem as recusas **não afirmavam** (só `fx-schema.spec.ts` afirmava,
e mudou para "aceita"); `edit-joins.spec.ts`, `render-bundle.schema.spec.ts` e `evaluate-edit-frame.spec.ts`
ganharam specs positivos. Chamadores de `validateEditJoins` conferidos: `node/viewer.ts` (harness, render),
`edit/finalize-bundle.ts` (`editBundle`), `scripts/audit-bundle.mjs`, `scripts/studio-project-api.mjs` e
`scripts/elements/full-video/build-full-bundle.mjs` só repassam a lista de problemas, então nenhum precisou mudar.
O render desenha o `dissolve` pelo compositor, via `normalizeTransitions` (mistura linear, centrada, `hold`). O
legado também entra no `TRANSITION_OVER_SPEECH`.

### 4.3 Outline

Ganha a seção **`fx`** (`OUTLINE_SECTIONS`), item `fx:<trackId>`, `ITEM_KIND_SECTION.fx = "fx"`. O que muda no
código (lista fechada, cada item com teste): `outline-schema.ts` (regex `^(light|track|node|fx):.+$`, constantes),
`outline.ts` (`sectionOfKey`, `naturalKeys`, `OutlineScope.fx?: { tracks: {id}[] }`, `knownKeys`, `flagProblem`:
`hidden` vale em grupo `fx`, `muted` não; mensagens "Items are …"), `outline/flags.ts` (`effectiveFlags` para
trilhas fx), `finalize-bundle.ts` (`pruneOutline` remove `fx:x` de trilha apagada), e no Studio
`SectionHeader.tsx`, `outlineDrop.ts`, `outlineRows.ts`, `groupContent.tsx`, `LayersPanel.tsx`. Testes:
bundle sem `fx` mantém o outline idêntico; `fx:x` sem trilha é `UNKNOWN_ITEM`; `pruneOutline` o remove.

**Entregue na F3** (`outline-fx.spec.ts`): tudo acima menos os componentes do Studio, que são da F5; a F3 só
pôs as entradas mínimas que mantêm o Studio compilando (`SectionHeader`, `describeGroup`, chaves i18n) (§18 #66).
Um grupo `hidden` da seção `fx` também esconde os clipes das trilhas dele no render (`visibleClips`, §18 #67).

## 5. Comandos da camada

Assinatura (padrão de narração/outline): `applyFxCommands(bundle, commands: readonly unknown[]) → { ok: true,
bundle, notes } | { ok: false, errors: FxCommandError[] }`. Cada comando é parseado **dentro** por
`fxCommandSchema` (zod, `discriminatedUnion("op")`). Erro: `{ code, message, hint?, command? }` (`command` =
índice na lista). Aplica em ordem sobre o resultado parcial e para na primeira recusa. `INVALID_COMMAND` tem
hint que **começa com `ops: a, b, c`** como as outras camadas e acrescenta `FX_COMMAND_SHAPES` (formas).
`bundle` é um `Pick<RenderBundle, "fx" | "shots" | "sceneInstances" | "sceneDefinitions" | "fps" |
"editDurationFrames" | "narration" | "renderProfile" | "cameraClips">`, como `applyNarrationCommands` faz. O
resultado é o bundle completo. **Hoje `applyFxCommands` só edita `fx` e não toca o outline**: o outline ainda não
tem seção `fx`; ela, a porta `{ fx }` do `editBundle` e a poda da entrada de uma trilha removida chegam juntas na
F3 (até lá não há nada a podar).

```ts
type FxCommand =
  | { op: "addTrack"; id?: string; label: string }
  | { op: "removeTrack"; trackId: string }
  | { op: "renameTrack"; trackId: string; label: string }
  | { op: "reorderTrack"; trackId: string; toIndex: number }
  | { op: "addClip"; trackId: string; effect: string; startFrame: number; durationFrames: number;
      params?: Record<string, FxParamValue>; scope?: FxScope; seed?: number; id?: string }
  | { op: "removeClip"; clipId: string }
  | { op: "moveClip"; clipId: string; startFrame: number; trackId?: string }
  | { op: "resizeClip"; clipId: string; edge: "start" | "end"; frame: number }
  | { op: "setParam"; clipId: string; param: string; value: FxParamValue }
  | { op: "keyParam"; clipId: string; param: string; frame: number; value: FxParamValue; easing?: string }  // frame relativo ao clipe
  | { op: "unkeyParam"; clipId: string; param: string; frame: number }                                      // idem
  | { op: "setEnvelope"; clipId: string; fadeInFrames?: number; fadeOutFrames?: number }
  | { op: "setScope"; clipId: string; scope: FxScope }
  | { op: "duplicateClip"; clipId: string; startFrame: number; trackId?: string; id?: string }
  | { op: "setTransition"; id?: string; fromShotId: string; toShotId: string; effect: string; durationFrames: number;
      alignment?: "center" | "start" | "end"; params?: Record<string, FxParamValue>; handles?: "use" | "hold" | "refuse";
      easing?: string; seed?: number }
  | { op: "resizeTransition"; transitionId: string; durationFrames: number; alignment?: "center" | "start" | "end" }
  | { op: "removeTransition"; transitionId: string }
  | { op: "applyPreset"; preset: string; startFrame: number; durationFrames: number; scope?: FxScope;
      overrides?: Record<string, FxParamValue> }
  | { op: "setHidden"; target: { kind: "clip" | "transition" | "track"; id: string }; hidden: boolean }
```

### 5.0 Semântica dos comandos que mexem em chaves e janelas

- `resizeClip` `edge: "start"` move o início **mantendo as chaves no mesmo instante absoluto** (`frame -= delta`;
  as que ficam `< 0` ou `≥ durationFrames` são **podadas** e listadas em `notes`); `edge: "end"` poda as que
  passam do novo fim. **`resizeClip` e `resizeTransition` ajustam e avisam**, nunca calam: se `fadeIn + fadeOut`
  passa do novo `durationFrames`, o `resizeClip` reduz `fadeInFrames` primeiro e depois `fadeOutFrames`, só até
  caber (um fade que chega a 0 some do clipe), e lista cada redução em `notes`; o `resizeTransition` poda as
  chaves da transição do mesmo modo. WHY: encolher é o propósito do comando, e recusar o lote com
  `ENVELOPE_TOO_LONG` acusaria um comando que nunca falou do envelope. Quem **diz** o envelope continua
  recusando: `setEnvelope` com fades que não cabem é `ENVELOPE_TOO_LONG` (o `addClip` não traz fades, então
  não há o que ajustar nele); a janela de transição que não cabe nos planos segue sendo erro de regra.
- `removeTrack` **apaga os clipes da trilha** (`notes` lista os ids); não há `FX_TRACK_NOT_EMPTY`.
- `duplicateClip` clona o clipe inteiro (`params`, `keys` relativas, `scope`, fades e `hidden`: um campo que o
  clipe ganhe depois vem junto) e troca só `id` e `startFrame`; mantém `seed` se explícito, senão deriva do
  novo `id` (outra tomada). Um clipe escondido continua escondido na cópia. `setHidden` de trilha **soma por OR** com `hidden` do clipe e do grupo do outline.
- Na CLI, `--scope` é `output | shot:<id> | node:<sceneId>:<nodeId>`.

- **`setTransition` substitui** a transição que já ocupa o corte (trocar o tipo ao soltar um card sobre um corte
  é um comando só: undo atômico). `TRANSITION_DUPLICATE` só vale contra o `dissolve` legado. A substituição é
  "trocar o cartão do corte", não "recomeçar": **conserva** o `id` (o `id` do comando é ignorado, com uma nota) e
  tudo o que o comando **omite**: `alignment`, `handles`, `easing`, `seed` e `hidden` ficam como estavam. Os
  padrões (`center`, `hold`, `easeInOutCubic`; [decisões §18 #8](scene-fx-decisions.md)) valem **só para
  transição nova**. Parâmetros estáticos (`params`): com o **mesmo `effect`** todos ficam e o comando os
  sobrescreve um a um; com **outro `effect`** nenhum fica. Chaves de parâmetro (o trabalho manual): com o mesmo
  `effect`, as chaves dos parâmetros que o efeito ainda tem ficam, reenquadradas à nova duração como em
  `resizeTransition` (as que ficam fora são descartadas); com outro `effect` nenhuma fica. **Todo descarte vai
  em `notes`, um por parâmetro** (o valor descartado e as chaves descartadas são notas distintas, com o nome e,
  para chaves, quantas), nunca em silêncio. Um valor estático dado no comando para um parâmetro que continua com
  chaves vira uma nota, como em `setParam` (as chaves mandam).
- **`applyPreset`**: a expansão (`expandPreset`, pura, no core) devolve `BundleEdit[]` (clipes, chaves e, quando a
  receita pede, comandos de câmera/luz). `applyFxCommands` **só aceita presets cuja expansão são `FxCommand`** e
  recusa os demais com `PRESET_NEEDS_EDIT_BUNDLE`; por `editBundle`, o ramo `fx` resolve o preset chamando
  `editBundle` recursivamente com a expansão, tudo-ou-nada, e devolve os erros com o `index` da entrada original,
  `layer: "fx"` e o hint "preset X, passo N". Assim há uma porta só por resultado.
- `explainFx(bundle) → { errors: FxIssue[]; warnings: FxIssue[] }` (só o bundle: o contexto vem de
  `fxContextOf(bundle)`), `FxIssue = { code, message, hint?,
  clipId?, transitionId?, frame? }`. **O schema e `applyFxCommands` usam só `errors`**; os avisos (custo, pilha
  com mais de 8 passes, `TRANSITION_OVER_SPEECH`, `TRANSITION_ON_CONTINUOUS`) aparecem no `scene validate` e no
  painel. `fxContextOf` deriva planos, instâncias, `fps`, duração da edição e os **intervalos de fala**: fala =
  `[startSeconds, startSeconds + durationSeconds)` dos clipes de narração que não estão `hidden`/`muted` (nem na
  trilha nem em grupo), mais os `fromMix`, convertidos em quadros por `fps`; qualquer sobreposição com a janela
  da transição dispara o aviso.
- Códigos de erro: `UNKNOWN_EFFECT`, `UNKNOWN_PARAM`, `PARAM_TYPE`, `PARAM_OUT_OF_RANGE`, `UNKNOWN_TRACK`,
  `UNKNOWN_CLIP` (também para as propriedades), `UNKNOWN_TRANSITION`, `KEYFRAME_ORDER`,
  `UNKNOWN_KEYFRAME_EASING` (os dois herdados de `keyframeIssues` por `fxKeyIssues`), `RESERVED_ID`,
  `TRANSITION_OVERLAP`, `PRESET_NEEDS_SHOT_SCOPE`, `UNKNOWN_SHOT`, `UNKNOWN_PRESET`, `UNKNOWN_SCOPE_TARGET`, `WRONG_FAMILY`,
  `WRONG_SCOPE`, `NODE_SCOPE_NOT_YET`, `REF_REQUIRED`, `FX_CLIP_OVERLAP`, `FX_CLIP_EMPTY`, `FX_CLIP_OUT_OF_RANGE`, `FX_SCOPE_MISMATCH`, `KEY_OUT_OF_CLIP`,
  `ENVELOPE_TOO_LONG`, `TRACK_INDEX_OUT_OF_RANGE`, `TRANSITION_NOT_AT_CUT`, `TRANSITION_DUPLICATE`,
  `TRANSITION_TOO_LONG`, `TRANSITION_NO_HANDLES`, `TRANSITION_OUT_OF_RANGE`, `PRESET_REFUSED`,
  `PRESET_NEEDS_EDIT_BUNDLE`, `CATALOG_TOO_NEW`, `CATALOG_MIGRATION_NEEDED`, `UNSUPPORTED_ALPHA`,
  `CAMERA_CLIP_SHARED`, `DUPLICATE_ID`, `INVALID_COMMAND`.
- **`NODE_SCOPE_NOT_YET`** (F2-b): escopo `node` ainda não é desenhado pelo compositor, então `explainFx` o recusa com este
  código (depois dos problemas de escopo que já existiam, para `errors[0]` seguir sendo o mais específico). Some na F4.
- **`REF_REQUIRED`**: um clipe de escopo `shot`/`node` cujo efeito tem parâmetro `ref` (luz, trajeto ou camada;
  ou `needs: ["light-position"|"path"]`) sem valor e sem chaves. Um `ref` não tem padrão (`paramDefault` devolve
  nada), então o erro diz **qual parâmetro**, **em qual escopo** e a dica lista os ids que a cena daquele escopo
  tem. Escopo `output` segue sendo `WRONG_SCOPE` (não há cena onde procurar o id); um id que não existe segue
  sendo `UNKNOWN_SCOPE_TARGET`. Nenhum efeito do catálogo real tem `ref` ainda: o spec usa efeitos de teste.
- **Avisos** (não bloqueiam; só `explainFx(...).warnings`): `TRANSITION_OVER_SPEECH`, `TRANSITION_ON_CONTINUOUS`,
  `TRANSITION_SPLAT_COST` (dois decks de splat: ~2x por quadro, de export), `FX_BUDGET_EXCEEDED` (mais de 8 passes **por quadro** contando os internos: §8.3),
  `FX_SCOPE_SHARED_DEFINITION` (preset de luz numa definição usada por várias instâncias).

### 5.1 `editBundle`

`BundleEdit` ganha `| { fx: FxCommand }`. Quatro pontos de `edit-bundle.ts` mudam (cada um com teste, porque
`layerOf`/`batchKey` terminam num `return "path"` implícito e um `fx` seria classificado como `path`): a união
`bundleEditSchema`, `layerOf`, `BATCHED` (o `fx` entra em lote com as outras camadas), `BUNDLE_EDIT_SHAPES`, e
`BundleEditLayer` ganha `"fx"`.

**Entregue na F3** (`edit-bundle-fx.spec.ts`): os quatro pontos, mais `applyPreset` resolvido **dentro** do
`editBundle`: `expandPreset` devolve `BundleEdit[]` (substitui `FxPresetEdit`, §18 #58), cada passo é aplicado
por recursão numa cópia, o preset todo entra ou nenhum passo entra, e a recusa mantém o índice da entrada
`applyPreset` na lista, `layer: "fx"` e a dica `preset X, step N`. `applyFxCommands` continua recusando preset
que não é só `fx` (`PRESET_NEEDS_EDIT_BUNDLE`), agora com a dica apontando o caminho real.

## 6. Propriedades

### 6.1 Caminhos (id estável, nunca o caminho da trilha)

```
fx/<clipId>/start | duration | fadeIn | fadeOut | seed | hidden     # o escopo muda só pelo comando setScope
fx/<clipId>/params/<param>                 # valor estático (kind do catálogo)
fx/<clipId>/params/<param>/keys            # trilha de chaves (kind "keys")
fx/<clipId>/params/<param>/keys/<k>/frame | value | easing
fx/track/<trackId>/label | hidden
transition/<transitionId>/duration | alignment | handles | easing | seed | hidden
transition/<transitionId>/params/<param>   |   .../params/<param>/keys   |   .../keys/<k>/frame|value|easing
```

O formato `<valor>/keys` e `<valor>/keys/<k>/…` é o de `property-paths.ts`/`properties/index.ts` (`kind ===
"keys"`), de modo que `addKeyEdit`/`removeKeyEdit` funcionam **sem mudança**. Usar só o id do clipe mantém
seleção, undo e chaves abertas estáveis quando o clipe muda de trilha.

### 6.2 Fiação do registro (lista fechada de mudanças em `src/properties/`)

`PropertyDocument.fx`; `PropertyChange { domain: "fx"; fx: Fx }` (o fx inteiro, como o domínio `camera` carrega o
clipe inteiro); ramo `fx` em `propertyDocumentOf`, `applyPropertyChange`/`changedDefinition`; códigos
`UNKNOWN_CLIP` (o mesmo que o `FxCommand` já usa; §18 #59), `UNKNOWN_TRACK`, `UNKNOWN_TRANSITION`,
`FX_REFUSED` (com `errors` aninhados no estilo `CAMERA_REFUSED`);
`Property` ganha `unit?`, `default?`, `step?`, `keyable?` (para `listProperties` dizer unidade e limites); `PropertyKind`
ganha `ref` (id de luz, trajeto ou camada), `fxVec2` (vec2 **de tela**, com mensagem própria: o `vec2` atual é
"px da imagem da camada") e `text` (o rótulo da trilha) (§18 #60); `choiceProperty` serve ao `enum`. No Studio: domínio `fx` em `editorStore` (saver por `putBundleEdits`
com update otimista e undo via `createDomain`) e `StudioSelection` ganha `fxClip` e `transition`.

**Entregue na F3** (`fx-properties.spec.ts`): a fiação do engine inteira, com todos os caminhos de §6.1
(`fx-properties.ts`, `fx-param-properties.ts`, `fx-property-write.ts`, §18 #61); a parte do Studio é da F5 (a F3
só põe um ramo `fx` "ignorado" no `editorStore` para o Studio compilar, §18 #66). Chaves são **relativas** ao
clipe (as de transição, ao início da janela); `Property.keyFrame` faz o `addKeyEdit` converter o cabeçote, e
fora do clipe/janela a gravação é `NOT_APPLICABLE`. `scope` só pelo comando `setScope`. `listProperties`,
`setProperties` e `getProperty` funcionam num bundle sem browser. Os ids reais são `fx-NN` (clipe), `track-NN`
(trilha) e `transition-NN` (transição).

### 6.3 Chaves e easing (o que o engine tem de verdade)

Os easings válidos hoje são **só `linear`, `easeInOutCubic`, `easeOutCubic`** (`camera/easing.ts`;
`keyframeIssues` recusa os outros; `applyEasing` lança). **Este plano usa só esses três** (as tabelas dizem
"subida suave" = `easeInOutCubic`, "arranque rápido e assentamento" = `easeOutCubic`). Acrescentar uma curva é
mudança separada, que afeta câmera, luz e clima e sobe a versão do catálogo. `resolveKeyframes` interpola no
quadro inteiro e segura o valor antes da primeira e depois da última chave. Valores não numéricos usam um `Mix<T>`
por tipo: `number` interpola; `{x,y}` interpola por componente; `color` interpola em **linear**; `boolean`, `enum`
e `string` mudam em degrau na chave seguinte.

## 7. Resolução pura por frame

`resolveFxFrame(bundle, editFrame) → FxFrameState`, em `src/fx/resolve-fx-frame.ts`, sem GL e testável:

```ts
type FxPass = {
  clipId: string; effect: string
  params: Record<string, FxParamValue>   // estáticos + chaves resolvidas, com easing; taxas já convertidas por fps
  weight: number                          // envelope fadeIn/fadeOut (0..1); efeito = intensity × weight
  frameInClip: number
  t: number                               // frameInClip / fps
  seed: number                            // §7.2
  events?: number[]                       // quadros (relativos ao clipe) de pulsos/surtos, para efeitos com `needs: ["events"]`
  pathProgress?: number                   // 0..1 no trajeto, para `needs: ["path"]`; o runtime projeta o ponto na tela
  scope: FxScope
}
type FxSide = {
  shotId: string
  editFrame: number            // o quadro de edição REAL: luz, clima e relâmpago rodam no relógio da edição e não são limitados
  instanceFrame: number        // limitado a [0, instance.durationFrames - 1]
  cameraClipFrame: number      // limitado ao clipe de câmera (última pose)
  heldInstance: boolean        // congela só rig, reveal e performance overrides (tudo que depende de instanceFrame)
  heldCamera: boolean          // congela só a câmera
}
type FxTransitionState = {
  transitionId: string; effect: string
  params: Record<string, FxParamValue>
  progress: number             // 0..1 já com easing
  frameInWindow: number; t: number
  events?: number[]
  from: FxSide; to: FxSide
  seed: number
}
type FxTransform2D = { dx: number; dy: number; rollDeg: number; scale: number }
type FxFrameState = {
  transition: FxTransitionState | null
  /** Passes e transforms por escopo shot/node, agrupados por deck (um deck fora de transição; dois dentro). */
  decks: { shotId: string; passes: FxPass[]; transform2D: FxTransform2D | null }[]
  /** Passes de escopo output, na ordem de pilha. */
  passes: FxPass[]
  /** Só os clipes `transform` de escopo `output`; os de escopo `shot` estão em `decks[i].transform2D`. */
  transform2D: FxTransform2D | null
}
```

Ordem de aplicação (fixa, documentada): efeitos de escopo `shot`/`node` por deck (passes e depois o
`transform2D` do deck) → transição → efeitos de escopo `output` (na ordem das trilhas) → `transform2D` de nível raiz
→ conversão de saída. Um clipe `shot:X` que não cobre os quadros de handle do deck X é resolvido no quadro **mais
próximo dentro do clipe** (mantém parâmetros e `weight` do último quadro), para o efeito não sumir de repente no
corte; isso vale também para os clipes `transform` ([decisões §18 #7](scene-fx-decisions.md): só o clipe que encosta na borda do plano).

**`transform` e escopo.** Um `transform` de escopo `shot` aplica **só ao deck daquele plano**: um tremor no plano A
treme os pixels de A e nunca os de B enquanto os dois estão na tela durante a transição (se tremesse a saída
inteira, o plano que entra tremeria antes de ser "dele"). O `transform2D` de nível raiz é **só** dos clipes de
escopo `output`. `transform` com escopo `node` é **recusado** (`WRONG_SCOPE`: é a transformação do quadro final,
não há nó a mover), por `explainFx` e portanto por `addClip`/`setScope`; o resolvedor o ignora se chegar a um
bundle não validado.

### 7.0 Janela da transição e `progress` (fórmulas fechadas)

Seja `cut` = `shots[to].editRange.startFrame` e `N = durationFrames`. Início da janela `w0`: `center` →
`cut - floor(N/2)`; `start` → `cut`; `end` → `cut - N`. Para o quadro de edição `i ∈ [w0, w0 + N)`:
`frameInWindow = i - w0` e `progress = easing((frameInWindow + 0.5) / N)`. Fora da janela `progress` é 0 (só A)
antes e 1 (só B) depois, e `FxFrameState.transition` é `null`. Nenhum quadro da janela é A ou B puro.

### 7.1 Tipos de efeito (resolve a objeção das camadas de câmera)

Cada efeito do catálogo tem `kind`:

| `kind` | O que é | Exemplos |
| --- | --- | --- |
| `pass` | Passe de tela cheia (fragment shader) | `vignette`, `bloom`, `glitch`, as transições |
| `geometry-pass` | Passe que desenha geometria instanciada (nunca N partículas por pixel num quad) | `dust` |
| `transform` | **Transformação 2D do quadro final** (`dx`, `dy`, `rollDeg`, `scale`), calculada em `resolveFxFrame` e aplicada pelo compositor (no deck do plano para escopo `shot`; no último passe para escopo `output`); **não** mexe na câmera nem em `cameraClips` | `cameraShake`, `breathingZoom`, pulso de escala do `heartbeat` e do `stinger` |
| `preset` | Receita que expande em clipes, chaves e comandos de outras camadas | os de suspense (§9.3) |

Motivo: as camadas da câmera (`MAX_CAMERA_LAYERS = 3`) são receitas em segmentos do clipe de câmera, **não têm
roll nem scale de imagem**, recusam keyframes, pertencem ao **clipe** (compartilhável entre planos) e não ao
quadro de edição; usá-las faria o efeito mutar dado. `transform` é puro por quadro e tem roll e scale reais.
Efeitos `transform` declaram `dx`/`dy` em **px a 720p**; `resolveFxFrame` os entrega **assim, sem escalar**
(`FxTransform2D` e `FxFrameState` são sempre a 720p) e o compositor os escala **uma única vez**, por
`altura do renderProfile / 720` (decisions #50: escalar nos dois lugares fazia 1080p mover 2,25x). O compositor os
aplica na ordem **escala em torno do centro → roll → translação**, com `clamp-to-edge` fora do quadro. Não tremem as
legendas (compostas depois, no Node) e o `projection-audit` (que lê só as matrizes efetivas) não os vê: documentado.
Presets que de fato precisam da câmera (`vertigo`) editam o **clipe principal** (que aceita keyframes) pela
porta `{ camera }` do `editBundle` e recusam com `CAMERA_CLIP_SHARED` se o clipe serve a mais de um plano.

### 7.2 Determinismo (definições, não frases)

- **Semente**: `seedBase = clip.seed ?? fnv1a32(UTF-8(clip.id))` (idem para a transição), inteiro 0..2^32-1. O meta do
  efeito declara `reseed: "static" | "per-frame"`: `static` (`inkBleed`, `burn`, `smokeFog`, `shadowSweep`: a
  forma não ferve) usa `seed = lowbias32(seedBase)`; `per-frame` (`filmGrain`, `glitch`, `staticNoise`, `flicker`)
  usa `seed = lowbias32(seedBase ^ frameInClip)`. Mover o clipe não muda o ruído (depende de `frameInClip`).
  `lowbias32` com `Math.imul` em JS e `uint` em GLSL ES 3.00; `fnv1a32` sobre bytes UTF-8; **vetores de teste**
  (entrada→saída) no spec valem para JS e GLSL.
- **Ruído em shader**: `highp`, hash inteiro (`uint`), `fbm` sobre esse hash; sem `sin`/`fract` de float em
  `mediump`. Posições de pulsos (`lightningFlash`, `heartbeatCut`) e de surtos (`glitch`) saem de `resolveFxFrame`
  em inteiros (`events`, por uma função pura `resolveEvents(params, durationFrames, seed, fps)` declarada no
  meta), não do shader.
- Mesmo `FxFrameState`, mesmo ambiente de GL: pixels idênticos entre execuções e entre ordens de seek. Entre
  ambientes (GPU/driver/ANGLE/SwiftShader): dentro da tolerância dos golden stills (§9.4).

### 7.3 Testes de resolução

Ouro por efeito (parâmetros resolvidos em quadros escolhidos, incluindo bordas do clipe, chaves com easing,
envelope e conversão de taxa por `fps`); propriedade: seek aleatório = linear em `FxFrameState`.

## 8. Render

### 8.1 Compositor de passes (`src/runtime/fx/`)

```
cena A ─► RT_A ─► [efeitos shot/node do deck A] ─┐
                                                   ├─► [transição] ─► RT_X ─► [efeitos output, ping-pong] ─► [saída: transform2D, conversão, dither] ─► canvas
cena B ─► RT_B ─► [efeitos shot/node do deck B] ─┘      (só durante a transição; fora dela há um deck só)
```

- **`FxFrameState` vazio nunca entra no compositor.** Com `passes: []`, `transition: null`, `transform2D: null` e nenhum deck com passes ou `transform2D` o
  runtime executa **literalmente o código de hoje** (`renderer.render(scene, camera)` ou `splatStage.render`):
  garantia bit a bit sem depender de o compositor "se comportar". O compositor só é construído quando o bundle
  tem `fx` não vazio.
- O compositor só roda a cadeia (decks, transição, passes, `transform2D`) quando `shouldPresent()` é verdadeiro
  (o `SplatStage.render` re-ordena e só apresenta na última tentativa). O **último passe escreve em
  `renderer.domElement`**, porque `capturePng` lê o canvas; isso é critério testável do S1.
- **Alvo do deck (decidido pelo S1, [relatório](scene-fx-s1-report.md), §17 #24)**: cada deck desenha em
  `createDeckTarget` (`runtime/deck-target.ts`): **RGBA8, 4 amostras, marcado como superfície de display** (sRGB), o que
  dá o **mesmo hash do canvas** em quadros de splat, de splat com chuva e de planos (também com os looks da cena). O alvo
  linear em `HalfFloatType` que este plano descrevia **falhou** na medida: o Spark e o three misturam em linear nele e o quadro
  difere do canvas em 3,9 a 10,8 níveis em média (até 74), o que seria um pulo de tom nas bordas da janela de transição.
  Logo, **o deck** fica em valores codificados (a codificação do canvas); o que vem depois dele (ver abaixo) decodifica uma vez,
  trabalha em linear e codifica uma vez. A **codificação de saída** é a sRGB, ou a curva Rec.709 (**não** a sRGB) quando
  `renderProfile.colorSpace` é `rec709-video` (o `bundle-integrity` ainda recusa esse perfil, §18 #42), mais
  **dither** (ruído triangular de hash, `±0.5/255`, por canal) para evitar banding de vinheta e grão, porque a captura
  headless é um canvas de 8 bits (`toDataURL`). Antes da F2-b este texto dizia que transição e passes operariam sobre valores
  codificados e a conversão só existiria para `rec709-video`. **Medido na F2-b (desvio, §18 #40):** depois do deck o compositor trabalha em **luz linear**: um passe de entrada
  decodifica a sRGB do deck para `HalfFloatType` linear, transição e passes operam ali (mistura de transição e cores dos
  efeitos em luz linear), e **um único** passe de saída codifica (sRGB, ou a curva Rec.709 para `rec709-video`) e aplica o
  dither. Todos os 256 níveis de uma rampa voltam idênticos pela cadeia (`scripts/fx-lab-check.mjs`). `renderer.outputColorSpace` e `toneMapping` (hoje
  `SRGBColorSpace` e nenhum) seguem como estão.
- **Capacidade**: no início o compositor testa `EXT_color_buffer_float`/`EXT_color_buffer_half_float` (presentes no Chrome do harness,
  S1 (v)); sem eles falha com mensagem clara, ou cai para `UnsignedByteType` nos alvos intermediários, caso em que o golden usa tolerância maior (4/255), há aviso e o **export final
recusa** a menos de `--allow-8bit`. O alvo do deck é RGBA8 e não depende das extensões.
- **Antialias**: o canvas de hoje tem MSAA e o alvo do deck usa **`samples: 4`** (decisão do S1, §17 #26: é o que o hash igual
  exige); `samples: 0` só no rascunho (`--fx-quality draft`) dos decks de splat (25 contra 58 ms por quadro). Quadros
  **sem** `fx` são idênticos por construção (primeiro ponto).
- **Alpha**: com `alphaMode !== "opaque"` (`straight`/`premultiplied`) o compositor **recusa** `fx` com
  `UNSUPPORTED_ALPHA` nesta primeira entrega (a pipeline atual é opaca); tratar alpha fica fora de escopo.
- Cada efeito `pass` é um `ShaderMaterial` em `FullScreenQuad`; sem estado entre frames (ping-pong só dentro do
  frame; nada de feedback, trails ou motion blur por acúmulo). Efeitos de escopo `node` desenham o nó isolado num
  alvo à parte e compõem de volta (planos e meshes; splat = `WRONG_SCOPE`, porque o Spark desenha todos os splats
  num passe só).
- Diferença esperada e registrada: **na etapa do deck nenhuma** (S1 medido: 0 pixels, com e sem os looks da cena); a diferença
  entre um quadro com `fx` e o sem `fx` vem só dos passes. Um teste compara um quadro com `fx` vazio de efeito (apenas o caminho do
  compositor, `intensity: 0`) contra o sem `fx` e registra a diferença máxima aceita (esperada: 0 nos decks, no máximo o arredondamento
  de um alvo intermediário).

### 8.2 Dois planos ao mesmo tempo (a parte mais arriscada: S1 vem antes de tudo)

`ThreeSceneRuntime` constrói **uma** definição de cena por vez (`activeDefinitionId`, `ensureSceneBuilt`),
**cria o próprio `WebGLRenderer` no construtor** (com o canvas), e o `SplatStage` instala um `SparkRenderer` na
cena **preso àquele renderer** (`SplatStage.create(scene, renderer, …)`; `render` ordena por câmera e espera o
`sortedCenter` bater). Não há hoje ponto de injeção de renderer nem prova de que o Spark desenha num
`WebGLRenderTarget`. Logo, dois decks **não** é um detalhe: é um refactor do runtime. Fica como **pré-requisito
de F2 (spike S1, com relatório antes de qualquer outro trabalho de F2)**, com critério de saída medido:

1. Extrair `SceneDeck`/injetar o `renderer` em `ThreeSceneRuntime` e criar `renderToTarget(editFrame, target)`
   a partir de `drawEditFrame` (que hoje termina em `renderer.render` direto no canvas); `prepareForInstance` já existe.
2. Provar o Spark dentro de `WebGLRenderTarget`, com hash de pixel igual ao do canvas direto num quadro de splat.
3. Medir ms/quadro (um deck, dois decks, sort do Spark por deck) e o risco de `MAX_SORT_ATTEMPTS`/timeout.
4. Ganchos a criar: renderer injetável; `renderToTarget`; `FxCompositor.draw(frameState)` chamado pelo runtime só
   quando há `fx`; o caminho sem `fx` fica **literalmente** o código atual.

Fallback se o renderer compartilhado não servir: um canvas por deck e cópia por `texImage2D`. **Não é cópia
barata**: é `readPixels`/upload 720p por quadro e sai de HalfFloat para 8 bits (perde o argumento de linear);
documentar o custo medido. Restrição declarada: **transição entre dois planos *splat* pode ser inviável de
início**; degradação documentada = `hold` ou `cut` com aviso no `explainFx`, até o
S1 provar o contrário (**provou**: ver o resultado abaixo; a degradação não se aplica, e o aviso virou `TRANSITION_SPLAT_COST`: custo, não recusa). Caso "mesma definição de cena nos dois lados": o mesmo runtime desenha duas vezes com
`instanceFrame`s diferentes; teste de ouro obrigatório (rig, reveal e clima são aplicados por quadro e parecem
puros, mas precisam de prova).

**Resultado do S1 (2026-11-02, F2-a; medido numa máquina, Chrome 156 / ANGLE Metal / M3 Pro; detalhe e números em
[scene-fx-s1-report.md](scene-fx-s1-report.md))**:

| Critério | Veredito |
| --- | --- |
| Spark num `WebGLRenderTarget`, hash igual ao canvas | **passa com o alvo canvas-equivalente** (RGBA8, 4 amostras, `createDeckTarget`: hash igual em splat, splat com chuva e planos); **falha com o alvo linear HalfFloat do §8.1 original** (3,9 a 10,8 níveis de média, máx 74) |
| Dois `ThreeSceneRuntime` num renderer, dois alvos no quadro | **passa** (splat+planos e splat+splat, 16 combinações, 0 vazamentos) |
| ms/quadro e sort | **parcial**: 1 passada de sort no máximo (sem risco de `MAX_SORT_ATTEMPTS`); um deck de splat 57,6 ms, splat+planos 64,5 ms, **splat+splat 123,5 ms** (fallback medido: 68,1 ms no par splat+planos, não adotado); não é tempo real com splat, é export |
| Último passe em `renderer.domElement`, `capturePng` lê | **passa** (PNG idêntico ao direto, 3 de 3) |
| HalfFloat e `samples` | **passa** (extensões presentes; `samples: 4`, `0` só como rascunho de splat) |

**Fim da janela (F3, [decisões #68](scene-fx-decisions.md))**: o segundo runtime não é jogado fora com a
cena dentro. No primeiro quadro fora da janela, se ele construiu a definição desse quadro, o runtime primário
**adota** a cena construída (troca de `BuiltScene` entre os dois, sem recarregar textura nem splat) e o segundo,
que passa a guardar a cena velha, é descartado. Os overrides do Studio (luz, clima, chão, looks, outline) e os
rastros de rig chegam ao segundo runtime ao criá-lo e a cada mudança durante a janela ([#77](scene-fx-decisions.md)),
então a cena adotada já está vestida como o deck de entrada: nada é reconstruído na adoção, e o rig só é refeito
se foi editado enquanto o segundo ainda construía ([#78](scene-fx-decisions.md)). Medido em Chrome (`scripts/fx-lab-check.mjs`): numa sequência A→B→C com duas
janelas o carregador é chamado **uma vez** por textura e os quadros planos depois de cada janela têm os mesmos
pixels de um runtime que construiu a cena sozinho.

Consequências: um renderer compartilhado, decks desenhados **em série por renderer** (uma fila por `WebGLRenderer`, além do
`frameBusy` de cada runtime; chamar os dois decks juntos é seguro), sem fallback e com `TRANSITION_SPLAT_COST` no lugar do
"não suportado" (§18 #34). Ganchos prontos
(`renderer` injetável, `renderToTarget`, `createDeckTarget`) com o caminho sem `fx` **idêntico** (baseline: 4 fixtures).

### 8.3 Orçamento

- Meta: 1280x720 a 24 fps, **≤ 8 passes de efeito + 1 transição** por frame, **contando os passes internos**
  (`bloom` custa 3 a 4: extração + 2 blurs + soma). `cost` declarado por efeito (`low` ≤ 4 amostragens de textura
  por pixel; `medium` ≤ 16; `high` > 16 ou blur com raio grande). O `explainFx` avisa quando a soma passa do
  orçamento; o export nunca reduz qualidade em silêncio. A soma é **por quadro**, como o compositor desenha:
  passes dos clipes de `output` + passes de cada deck na tela (dois durante a transição, um fora dela; um clipe
  `shot`/`node` só conta no deck que o desenha, inclusive o quadro congelado nos handles) + o custo declarado da
  transição daquele quadro. O aviso `FX_BUDGET_EXCEEDED` aponta a **primeira faixa** de quadros acima de 8, com
  o intervalo e o total mais alto dentro dela.
- Unidades em px valem a 720p e são **escaladas pela altura do `renderProfile`**, para o bundle não mudar de
  aparência em outra resolução. A escala acontece **só no compositor** (`pxScale`, inclusive a translação de
  `transform2D`); o resolvedor nunca a aplica (decisions #50).
- F4 mede ms/quadro por efeito no ambiente de CI e no local e **relata** (sem limite de falha na primeira
  entrega; o limite entra depois de haver a medida).

### 8.4 Handles e quadros congelados

Duas funções puras novas, no F1: `resolveShotAtOffset(bundle, shot, deltaFrames) → FxSide`, com
`deltaFrames = editFrame - shot.editRange.startFrame` (negativo antes do plano, `≥ comprimento` depois; dentro do
range dá **o mesmo** resultado de `resolveShotAtEditFrame`, sem lançar, com teste), e
`evaluateShotFrame(bundle, shot, { instanceFrame, cameraClipFrame })`, extraída de `evaluateEditFrame` (que passa a
chamá-la; teste de que o resultado dentro do range é idêntico ao de hoje, por `digestResolvedState`). Motivo:
`resolveShotAtEditFrame` só mapeia quadros dentro do `editRange` e `evaluateEditFrame` **lança** se
`instanceFrame >= instance.durationFrames` ou `clipFrame < 0`; nenhum serve ao deck inativo.

- Plano de saída, `k` quadros depois do fim: `instanceFrame = instanceStartFrame + comprimento + k`; se
  `≥ instance.durationFrames`, `heldInstance` e `instanceFrame = durationFrames - 1`.
- Plano de entrada, `k` quadros antes do início: `instanceFrame = instanceStartFrame - k`; se `< 0`,
  `heldInstance` e `instanceFrame = 0`.
- `cameraClipFrame = cameraInFrame ± k`, limitado a `[0, última pose do clipe]` (`heldCamera`); os dois clamps
  são **independentes**. `editFrame` não é limitado (a janela precisa caber no timeline).
- Quadros que cada lado precisa (`N` = duração): `center` → saída `ceil(N/2)` depois do corte, entrada
  `floor(N/2)` antes; `start` → saída `N`, entrada 0; `end` → saída 0, entrada `N`. `k ∈ [0, n-1]`.
  `handles: "use"` recusa (`TRANSITION_NO_HANDLES`) se faltar quadro **da instância ou da câmera** (os dois
  grampos são independentes, então os dois são conferidos): a mensagem diz qual plano e quantos quadros faltam, e
  entre parênteses **se faltou a instância, a câmera ou ambas** (e quanto de cada); a dica diz o que fazer com
  cada fonte (dar mais quadros da instância do plano; estender o clipe de câmera). A câmera de um plano tem
  `cameraInFrame` quadros antes do primeiro e `cameraClipFrames - (cameraInFrame + comprimento)` depois do
  último. `"hold"` aceita as duas faltas congeladas e marca `heldInstance`/`heldCamera`; `"refuse"` recusa
  sempre que faltar, **igual a `use`** ([decisões §18 #6](scene-fx-decisions.md): nenhum cálculo os diferencia; a dica do `refuse` diz isso).
- `TRANSITION_TOO_LONG`: o plano de saída precisa ter ao menos os quadros que a janela usa **dentro** dele
  (`center`: `floor(N/2)`; `end`: `N`; `start`: 0) e o de entrada os que usa dentro dele (`center`: `ceil(N/2)`;
  `start`: `N`; `end`: 0).
- `TRANSITION_OVERLAP`: as janelas de cortes vizinhos não se intersectam.

### 8.5 Áudio

Transições e efeitos não mexem na mix; o `audioMixPlan` continua o mesmo. Transição em cima de fala gera aviso
(`TRANSITION_OVER_SPEECH`) no `explainFx`. Efeitos sonoros (stingers, risers) ficam fora de escopo; a tag
`sfx-hint` fica para uma fase futura.

## 9. Catálogo de efeitos

O catálogo é **código versionado**, dividido em duas peças com um spec que garante o 1:1 entre elas:

- `FxEffectMeta` (em `src/fx/catalog/<id>.ts`, vai no `core`, leve): id, família, `kind`, rótulos e resumos nos 3
  idiomas, `tags`, `moods`, `useWhen`, `typicalDurationFrames`, `params`, `needs`, `cost`, `padding`, `sample`.
- `FxEffectShader` (em `src/runtime/fx/shaders/<id>.ts`, só no runtime): o GLSL. O schema do bundle importa só o
  meta; o `core` nunca carrega GLSL.

```ts
type FxParamDef =
  | { id: string; kind: "number"; min: number; max: number; default: number; step?: number; unit?: "px" | "frames" | "deg" | "ratio" | "bpm" | "hz" }
  | { id: string; kind: "boolean"; default: boolean }
  | { id: string; kind: "enum"; values: string[]; default: string }
  | { id: string; kind: "color"; default: { r: number; g: number; b: number } }
  | { id: string; kind: "vec2"; unit: "screen" | "velocity"; min: number; max: number; default: { x: number; y: number } }   // screen = fração da tela
  | { id: string; kind: "ref"; ref: "light" | "path" | "layer" }   // valor = id (string) dentro da cena do escopo shot/node
type FxEffectMeta = {
  id: string; family: "transition" | "look" | "suspense"; kind: "pass" | "geometry-pass" | "transform" | "preset"
  label: Locale3; summary: Locale3; tags: string[]
  moods: string[]                  // "suspense" | "calm" | "epic" | "dark" | "glitch" | "organic" | "film"
  useWhen: Locale3                 // uma frase: quando escolher este efeito
  typicalDurationFrames: number
  params: FxParamDef[]
  reseed?: "static" | "per-frame"   // quando needs inclui "seed"
  needs: ("two-inputs" | "seed" | "events" | "light-position" | "path")[]   // light-position e o ponto do path na tela são resolvidos pelo runtime (precisam da câmera e dos nós)
  cost: "low" | "medium" | "high"
  padding?: number
  sample: { transition?: Record<string, FxParamValue>; clip?: Record<string, FxParamValue> }
}
```

`catalogVersion` sobe quando o **resultado** de algum efeito muda; `FX_CATALOG_CHANGES` (código) registra cada
mudança e o `fx-catalog.md` a espelha. Todo parâmetro é *keyable* (qualquer `kind` menos `ref`; `boolean`, `enum` e
`string` mudam em degrau, §6.3).

**Parâmetros completos e `intensity`**: o catálogo acrescenta a todo efeito um parâmetro `intensity` (0..1,
padrão 1; a tabela só o lista quando o padrão é diferente) e o efeito aplicado é `intensity × weight`. Onde uma
linha das tabelas abaixo omite faixa ou padrão, o implementador usa estas convenções e as grava no meta:
`softness` 0..1 (0.1); `angle`/`startAngle` 0..360 (0); `center`/`position`/`focus` vec2 `screen` (0.5, 0.5);
`scale` 0.5..16 (3); `speed` 0..4 (1); `size` 0.5..64 px; `opacity`/`amount`/`strength`/`density` 0..1;
`drift`/`wind` vec2 `velocity` -2..2 (0, 0); cores sem padrão dado = preto (sombras/tinta) ou branco (luz);
`direction` = enum `up|down|left|right` (com `angle` opcional 0..360 que o sobrepõe); `lift`/`gamma`/`gain` =
cores neutras (preto/cinza médio/branco); `closeAt`/`cutAt`/`peakAt` 0..1 (0.5); `light`/`pathId` = `ref`. O
spec do catálogo recusa efeito com parâmetro sem faixa ou sem padrão.
Convenção: tempo em frames; ângulos em graus; posições em fração da tela (0..1, origem no canto superior
esquerdo); cores em linear 0..1; todo efeito tem `intensity` (0..1) como multiplicador global. Os rótulos vivem
**só no catálogo** (o Studio lê `label[locale]`; nada de duplicar chaves no i18n do app).

### 9.1 Transições (`family: "transition"`, `kind: "pass"`, duas entradas)

`p` = `progress` (0..1, já com easing). `A` = plano de saída, `B` = plano de entrada; mistura em linear.
`cost: low` salvo indicação. Translação (push/slide/zoom) usa `clamp-to-edge` fora do quadro (nunca borda vazia
transparente).

| id | O que faz | Como funciona | Parâmetros (padrão) |
| --- | --- | --- | --- |
| `cut` | Corte seco, só para ter o corte como dado | `p < 0.5 ? A : B` no shader | — |
| `crossDissolve` | Mistura clássica `[calm]` | `mix(A, B, p)` em linear | `curve: enum linear/smooth (smooth)` |
| `filmDissolve` | Dissolve que clareia o meio como a película `[film]` | `pow(mix(pow(A,g), pow(B,g), p), 1/g)` com `A`,`B` lineares; `g = 1` é o `crossDissolve`; `g > 1` deixa o meio mais claro | `gamma: 1..2.4 (1.6)` |
| `additiveDissolve` | Os dois somam luz no meio `[epic]` | `A*(1-p) + B*p + min(A+B, 1)*bloom*sin(pi*p)`; valores > 1 sobrevivem no alvo HalfFloat e só a conversão final limita | `bloom: 0..1 (0.35)` |
| `dipToBlack` | Escurece até preto e reabre `[dark]` | `p<0.5`: `A*(1-2p)`; `p≥0.5`: `B*(2p-1)`; `hold` segura o preto | `hold: 0..0.5 (0.1)`, `color (preto)` |
| `dipToWhite` | Clarão branco `[epic]` | igual, com branco | `hold (0.05)`, `color (branco)` |
| `dipToColor` | Passa por uma cor (sangue, azul-lua) `[dark]` | igual, com cor livre | `color`, `hold` |
| `fadeThroughVignette` | Escurece pelas bordas até o centro e reabre `[dark]` | máscara radial `smoothstep(r - soft, r, dist)` fechando em `p` | `center: vec2`, `softness: 0.05..0.6 (0.3)`, `color (preto)` |
| `wipeLinear` | Cortina em linha `[calm]` | `dot(uv - 0.5, dir) < (p - 0.5)*len`, borda suavizada | `angle: 0..360 (0)`, `softness: 0..0.5 (0.05)` |
| `wipeRadial` | Relógio `[calm]` | ângulo do pixel em torno do centro contra `p*2π` | `center`, `startAngle`, `direction: cw/ccw`, `softness` |
| `iris` | Íris que abre/fecha `[epic]` | distância ao centro, com forma, contra raio `p*maxR` | `shape: circle/diamond/eye`, `center`, `softness`, `invert` |
| `blinds` | Persianas/grade `[calm]` | máscara por faixa `fract(uv.x*n)` com atraso por faixa | `count: 2..40 (12)`, `angle`, `stagger: 0..1 (0.5)`, `softness` |
| `push` | A entrada empurra a saída `[calm]` | desloca A e B por `±dir*p` com paralaxe opcional | `direction: 4 + angle`, `parallax: 0..1 (0)`, `overshoot: 0..0.2 (0)` |
| `slide` | B desliza sobre A (A parado) | desloca só B | `direction`, `shadow: 0..1 (0.3)` |
| `zoomCross` | Zoom entra em A e sai de B `[epic]` · `medium` | A escala `1+p*z`, B escala `1+(1-p)*z`, mix por `p`, blur radial opcional | `amount: 0..1.5 (0.6)`, `center`, `blur: 0..1 (0.4)`, `direction: in/out` |
| `whipPan` | Pan rápido com borrão de movimento `[epic]` · `medium` | amostra N vezes ao longo de `dir` com peso gaussiano; A sai e B entra no pico | `angle`, `strength: 0..1 (0.7)`, `samples: 8..32 (16)`, `peakAt: 0.3..0.7 (0.5)` |
| `defocusDissolve` | Os dois desfocam no meio `[suspense]` · `high` | blur de disco com raio `sin(pi*p)*R`, mix no pico | `radius: 0..40 px (18)`, `samples: 8..32 (16)` |
| `glitch` | Blocos que deslocam, divisão RGB, linhas `[glitch][suspense]` | blocos por `hash(seed, floor(t*rate), bloco)`; canais deslocados em direções diferentes; A→B por bloco | `blocks: 4..64 (24)`, `shift: 0..1 (0.25)`, `rgbSplit: 0..30 px (10)`, `rate: 1..30 hz (12)`, `lines: 0..1 (0.3)` |
| `staticNoise` | Estática de TV entre A e B `[glitch][dark]` | mix A/B com ruído de hash em cinza + scanlines | `grain: 0..1 (0.8)`, `scanlines: 0..1 (0.4)`, `tint: color (cinza frio)` |
| `shockwave` | Onda circular que distorce e revela B `[epic]` · `medium` | deslocamento radial `sin((dist - p*R)*k)*amp` num anel; B dentro do anel | `center`, `ringWidth: 0.02..0.4 (0.12)`, `amplitude: 0..0.1 (0.04)`, `chromatic: 0..1 (0.3)` |
| `inkBleed` | Tinta preta se espalha e revela B `[dark][organic]` · `medium` | máscara `fbm(uv*scale, seed) < p*1.3`, borda escurecida e cor de tinta | `scale: 1..12 (4)`, `edge: 0..0.3 (0.08)`, `ink: color`, `octaves: 2..6 (4)` |
| `smokeFog` | Névoa densa cobre A e se desfaz sobre B `[dark][organic]` · `medium` | `fbm` deslocado por `p` (nunca por relógio) como opacidade | `density: 0..1 (0.8)`, `scale (3)`, `drift: vec2`, `fog: color` |
| `burn` | Papel queima e revela B com brasa na borda `[dark][organic]` · `medium` | máscara `fbm < p`, banda de borda mapeada para preto→laranja→amarelo, emissivo somado | `scale (3)`, `edgeWidth: 0.02..0.2 (0.07)`, `ember: color`, `glow: 0..2 (1)` |
| `lightningFlash` | Clarão de relâmpago em 2–3 pulsos e corte para B `[dark][suspense]` | posições dos pulsos vêm de `resolveFxFrame` (inteiros, da semente); no pico mostra A brilhante com inversão parcial, depois B | `pulses: 1..4 (2)`, `peak: 0..4 (2.5)`, `tint: color (azul pálido)`, `invertAmount: 0..1 (0.2)` |
| `eyeBlink` | Pálpebras fecham e reabrem `[suspense][dark]` · `medium` | duas barras elípticas com borda suave fechando em `p<0.5` e abrindo em `p>0.5`, com blur e vinheta; A→B no fechamento | `closeAt (0.5)`, `softness: 0..0.3 (0.12)`, `blur: 0..20 px (8)`, `lids: color` |
| `heartbeatCut` | Dois pulsos de escala+brilho e corte `[suspense]` | pulsos nas posições de `resolveFxFrame` (`bpm` aproximado ao quadro), vinheta vermelha, corte em `cutAt` | `bpm: 40..140 (70)`, `amount: 0..0.1 (0.04)`, `flash: color`, `cutAt (0.6)` |
| `shadowSweep` | Onda de sombra atravessa o quadro `[dark]` | máscara linear/radial com borda `fbm`, cor da sombra | `angle`, `softness`, `shadow: color`, `grain: 0..1 (0.3)` |
| `lumaWipe` | Revela B pela luminância de A ou de uma rampa `[film]` | `step(luma(A), p)` com borda | `source: enum a/b/ramp`, `softness`, `invert` |

**Estado (F4-a): as 28 transições da tabela estão implementadas** (as 6 da F2 mais as 22 da F4-a; o catálogo tem 34
efeitos com os 6 looks da F2). Cada uma tem meta em `src/fx/catalog/<id>.ts`, shader em
`src/runtime/fx/shaders/<id>.ts`, golden e a checagem 1:1 meta↔shader. Desvios e escolhas, com o porquê em
[scene-fx-decisions.md](scene-fx-decisions.md) §18 (#82 a #93):

- `cut` não declara humor (é o corte como dado, não uma sugestão); `slide` declara `calm` (a tabela não dava).
- `whipPan` e `defocusDissolve` são **`cost: high` com 2 estágios** (`internalPasses: 2`): o borrão é um segundo
  estágio sobre a tira/mistura, não `N` leituras de cada plano. `whipPan` ficou `high` (a tabela dizia `medium`) e
  `shadowSweep` ficou `medium`, por contagem de leituras (a regra: low ≤ 4, medium ≤ 16, high > 16).
- `glitch`: `rate` vira a cadência **irregular** de rajadas (`resolveEvents`, em quadros inteiros, da semente), não um
  `floor(t*rate)` de relógio; o desenho dos blocos vem de uma dobra dos `events`, estável na janela.
- `lightningFlash` e `heartbeatCut` cortam para B **num quadro inteiro** (o último clarão / o `cutAt` em quadros do
  tamanho da janela), não por `progress`, para o corte cair no pulso com qualquer easing.
- `dipToColor` tem padrão (vermelho-sangue) onde a tabela só diz `color`; `fadeThroughVignette`, `shadowSweep`,
  `eyeBlink` e os demais cobertos pela tabela usam os padrões escritos nela.
- `inkBleed`/`smokeFog`/`burn`/`shadowSweep` usam `fbm` de inteiros (`chunks/noise.ts`, sem relógio), forma
  estática por semente, e cada valor derivado de hash tem uma **checagem GLSL×JS** (`FxHashCheck`) rodada no
  navegador por `scripts/fx-lab-check.mjs`.

### 9.2 Efeitos contínuos (`family: "look"`, uma entrada, empilháveis)

Cada um é um clipe numa trilha `fx`, com `scope`, `keys` e envelope. `t = frameInClip/fps`; ruído por hash.

| id | `kind` | O que faz | Como funciona | Parâmetros (padrão) | Custo |
| --- | --- | --- | --- | --- | --- |
| `vignette` | pass | Escurece as bordas `[dark][suspense]` | `1 - smoothstep(r, r+soft, dist)` com forma elíptica | `radius: 0..1.5 (0.75)`, `softness: 0.05..1 (0.45)`, `intensity (0.7)`, `color`, `roundness: 0..1 (0.6)`, `center: vec2` | low |
| `filmGrain` | pass | Grão de filme `[film]` | hash por `(seed, frame, pixel/size)`, mistura em luminância (mais nas sombras) | `amount: 0..1 (0.25)`, `size: 0.5..4 px (1.4)`, `luma: 0..1 (0.7)`, `colored: boolean` | low |
| `colorGrade` | pass | Lift/gamma/gain, contraste, saturação, temperatura `[film]` | em linear: `(c*gain + lift)^gamma`, matriz de saturação, balanço de branco | `lift/gamma/gain: color`, `contrast: -1..1`, `saturation: 0..2 (1)`, `temperature: -1..1`, `tint: -1..1` | low |
| `bleachBypass` | pass | Dessatura e endurece `[dark]` | mistura com a luminância (overlay) + contraste | `amount: 0..1 (0.6)`, `contrast: 0..1 (0.3)` | low |
| `moonlight` | pass | Look frio de luar `[dark]` | azul nas sombras, realce frio nos médios, dessatura leve | `strength: 0..1 (0.7)`, `shadowTint: color`, `desaturate (0.25)` | low |
| `candlelight` | pass | Look quente de vela `[dark]` | âmbar nas altas luzes, sombras profundas | `warmth: 0..1 (0.7)`, `shadowDepth (0.5)` | low |
| `bloom` | pass | Brilho em altas luzes `[epic]` | extrai `luma > threshold`, blur gaussiano separável em 2 passes (meia resolução), soma | `threshold: 0..1 (0.7)`, `radius: 1..64 px (24)`, `intensity (0.6)`, `tint` | medium (3–4 passes internos) |
| `halation` | pass | Halo avermelhado em volta de luzes `[film]` | bloom só do canal vermelho com raio maior | `threshold (0.8)`, `radius (36)`, `intensity (0.4)` | medium |
| `chromaticAberration` | pass | Franjas de cor nas bordas `[glitch]` | amostra R/G/B com deslocamento radial (3 amostras) | `strength: 0..30 px (6)`, `falloff: 0..2 (1)`, `center` | low |
| `lensBlur` | pass | Desfoque por distância **na tela** (tilt-shift/radial), **não** por profundidade `[suspense]` | blur de disco com raio por distância ao plano focal | `focus: vec2`, `range: 0..1 (0.25)`, `maxRadius: 0..30 px (12)`, `mode: tilt/radial` | high |
| `mist` | pass | Névoa de tela em camadas `[dark][organic]` | `fbm` por `t` com cor pelo gradiente vertical | `density: 0..1 (0.35)`, `scale (2.5)`, `drift: vec2`, `color`, `height: 0..1 (0.5)` | medium |
| `godRays` | pass | Raios a partir de uma luz `[epic]` | blur radial a partir da luz projetada na tela com limiar; **a posição da luz é resolvida pelo runtime** (precisa da câmera do quadro), `needs: ["light-position"]`; exige `scope: shot`/`node` (a cena sai do plano) | `light: ref(light)`, `threshold (0.55)`, `length: 0.1..1 (0.7)`, `decay: 0.5..0.99 (0.94)`, `color`; 48 amostras fixas | medium |
| `lightLeak` | pass | Vazamento de luz quente `[film]` | gradiente animado por `t` com `fbm`, modo `screen` | `color`, `size`, `position: vec2`, `intensity (0.5)` | low |
| `dust` | pass | Poeira flutuando `[calm][organic]` | até 64 partículas somadas por pixel, posição `f(hash(seed,i), t)` com sway (desvio #122: não há geometria instanciada) | `count: 4..64 (40)`, `size: 0.5..10 px`, `speed`, `sway`, `color` | medium |
| `cameraShake` | transform | Tremor de câmera à mão `[suspense]` | soma suave de ruído 1D em `dx,dy,rollDeg` (em `transform2D`); **não** toca a câmera | `amplitude: 0..40 px (6)`, `frequency: 0.2..8 hz (1.6)`, `roll: 0..3 deg (0.4)`, `decay: boolean` | low |
| `breathingZoom` | transform | Respiração lenta de escala `[suspense]` | `scale = 1 + a*sin(2π t/period)` em `transform2D` | `amount: 0..0.05 (0.012)`, `periodFrames: 48..400 (160)` | low |
| `fade` | pass | Fade de/para uma cor sobre o plano, **como clipe** (abre de preto, fecha em branco, flash) `[dark]` | `mix(c, color, amount)` com `amount` keyável | `color (preto)`, `amount: 0..1 (1)` | low |
| `scalePunch` | transform | Pulso de escala (susto, batida) `[suspense]` | `scale = 1 + amount*pulse(t)` com subida/descida em frames; vários pulsos por `events` | `amount: 0..0.3 (0.08)`, `riseFrames: 1..12 (2)`, `fallFrames: 1..24 (6)` | low |
| `letterbox` | pass | Barras de cinema | máscara na razão alvo | `ratio: 1.33..2.76 (2.39)`, `softness: 0..8 px (0)`, `color` | low |
| `scanlines` | pass | Linhas de varredura e VHS `[glitch]` | seno nas linhas, jitter horizontal por hash, bleed de croma | `lines: 0.1..1.5 ratio (0.5)` (fração da altura/720), `intensity (0.25)`, `jitter: 0..1 (0.15)`, `chromaBleed: 0..8 px (2)` | low |
| `flicker` | pass | Cintilação de exposição `[suspense]` | multiplicador `1 + a*(noise1D(t*rate+seed)-0.5)`; "blackouts" opcionais | `amount: 0..1 (0.12)`, `rate: 1..30 hz (11)`, `blackouts: 0..1 (0)` | low |
| `heatHaze` | pass | Ondulação de calor `[organic]` | deslocamento UV por `fbm` animado por `t`, mais forte numa faixa | `amplitude: 0..20 px (4)`, `scale`, `speed`, `height: 0..1 (0.5)` | low |
| `inkPrint` | pass | Gravura/halftone `[film]` | quantiza luminância em tramas + linha de borda (Sobel) | `dots: 40..400 (160)`, `edge: 0..1 (0.3)`, `paper: color` | medium |

**Estado (F4-b1): as 15 linhas da tabela acima que não dependem de luz projetada nem de geometria estão implementadas**
(`bleachBypass`, `moonlight`, `candlelight`, `halation`, `lensBlur`, `mist`, `lightLeak`, `scanlines`, `flicker`,
`heatHaze`, `inkPrint`, `fade` e os transforms `cameraShake`, `breathingZoom`, `scalePunch`), mais `ink`, `dream` e
`film` como `look` do catálogo (§19.2 itens 2 e 5). O catálogo tem 54 efeitos, 51 com golden (`godRays` e `dust` entraram na F4-b2; `dust` como `pass`, ver decisões #122). **Falta** `flashlightReveal` e os presets (F4-c). Desvios, com o porquê em [scene-fx-decisions.md](scene-fx-decisions.md) §18
(#112 a #119): `heatHaze` e `lightLeak` ficaram `medium` (a tabela dizia `low`); `halation` declara
`internalPasses: 4`; `scalePunch` ganhou `pulses` e `gapFrames`; `cameraShake`, `breathingZoom` e `scalePunch` não têm
shader nem golden de imagem (o golden é numérico); `scanlines`, `inkPrint` e `candlelight` tiveram o `sample`/a
fórmula ajustados olhando a imagem.

### 9.3 Suspense e cenas dark (`family: "suspense"`, `kind: "preset"`)

Presets **não são caminhos de render**: `expandPreset(preset, args, context) → BundleEdit[]` (puro, no core, com
testes) expande em clipes, chaves e comandos de outras camadas; o que sobra no bundle é só dado editável.
Sorteios (`possessedGlitch`) usam a semente **na expansão**; o resultado é gravado em clipes fixos e a semente
deixa de importar depois. `bpm` vira frames por batida arredondado ao quadro (`round(fps*60/bpm)`).

| id | O que faz | Receita (o que expande) | Parâmetros |
| --- | --- | --- | --- |
| `creepingVignette` | O escuro fecha das bordas ao centro | `vignette` com `radius` keyado de `1.1` a `0.35` (`easeInOutCubic`), `softness` subindo; `flicker` leve no fim | `from`, `to`, `peak: 0..1 (0.8)` |
| `heartbeat` | Pulso de vinheta e escala no ritmo de um batimento | `vignette` com `intensity` em duplo pulso por `bpm` + clipes `scalePunch` nas batidas (`events`) | `bpm: 40..160 (72)`, `amount: 0..0.1 (0.03)`, `rise: boolean` |
| `lightsOut` | As luzes caem e o escuro engole | exige `scope: shot` (a cena sai do plano; `scope: output` é `PRESET_NEEDS_SHOT_SCOPE`): chaves de intensidade das luzes da **definição** dessa cena até `floor` (comandos `{ set: scene/<sceneId>/light/<id>/intensity }`; aviso `FX_SCOPE_SHARED_DEFINITION` se a definição serve a mais de uma instância), mais `vignette` e `moonlight` | `floor: 0..1 (0.05)`, `frames`, `flickerFirst: boolean` |
| `candleSnuff` | A vela apaga, escuro curto, a próxima acende | `scope: shot`: chaves de intensidade das luzes até zero e de volta + um clipe `fade` de preto no meio (sem transição: presets nunca criam transições) | `snuffFrames`, `relightFrames`, `blackHold` |
| `slowDrain` | A cor escoa devagar | `colorGrade.saturation` keyada até `to` + `bleachBypass` | `to: 0..1 (0.15)` |
| `flashlightReveal` | Só um facho de luz revela a cena, seguindo um trajeto | máscara de luz circular cujo `center` **segue um `path` do bundle**: `resolveFxFrame` entrega `pathProgress` (puro, do `progress` do clipe) e o **runtime** projeta o ponto do trajeto (px da imagem da camada) na tela com as matrizes do quadro, como `light-position`; exige `scope: shot`/`node` | `pathId: ref(path)`, `radius: 0.05..0.6 (0.18)`, `ambient: 0..0.4 (0.05)`, `tint` |
| `stinger` | Susto: clarão de 3 quadros + zoom seco | clipe `fade` branco curto, `scalePunch` (1→1.12 em 3 frames), `chromaticAberration` no pico | `flashFrames: 1..6 (3)`, `zoom: 0..0.3 (0.12)`, `shake: 0..1 (0.5)` |
| `vertigo` | Dolly zoom: o fundo estica | **comandos de câmera no clipe principal** (aceita keyframes): afasta a câmera e compensa o `fov` (`fov' = 2*atan(w/(2*d'))`); fica na aproximação do easing escolhido e o `explain` da câmera mede o desvio; recusa com `CAMERA_CLIP_SHARED` | `strength: 0..1 (0.6)`, `direction: in/out`, `frames` |
| `shadowCreep` | Uma sombra de borda avança com textura orgânica | `vignette` com `roundness` baixa + `mist` escura nas bordas | `from/to`, `organic: 0..1 (0.6)` |
| `possessedGlitch` | Surtos curtos de glitch em intervalos irregulares | N clipes curtos de `chromaticAberration`/`scanlines`/estática em quadros sorteados pela semente | `bursts: 1..12 (4)`, `maxFrames: 1..8 (4)`, `intensity (0.7)`, `seed` |
| `whisperFocus` | O desfoque fecha em volta de um ponto da tela | `lensBlur` radial com `focus` keyado + vinheta fraca (sem mapa de profundidade: não promete foco por distância) | `focus: vec2`, `frames`, `maxRadius` |
| `coldOpen` | Abertura fria: preto → grão → luar | clipe `fade` de preto com `amount` keyado 1→0, `filmGrain`, `moonlight`, `letterbox` | `grain`, `ratio` |

### 9.4 Regras para criar um efeito e golden stills

1. Um arquivo de meta, um shader, um `.spec.ts` ao lado com: resolução de parâmetros (padrão, limites, chaves) e um
   **golden still** (PNG pequeno, 320x180). A ferramenta de golden nasce no F2 (`scripts/fx-golden.mjs`, sobre
   `render-pilot.mjs`/`viewer-harness.mjs`); `scene fx preview` (F3) a reaproveita.
2. O `sample.frames` do meta diz quais quadros (contados do começo do clipe ou da janela da transição) entram no golden, que é uma **faixa** deles lado a lado (§18 #37). Goldens ficam **no git** (`packages/scene-engine/src/fx/catalog/__golden__/<id>.png`); atualizam-se só com
   `UPDATE_GOLDENS=1`. **Tolerância fixa**: diferença máxima de 2/255 por canal e no máximo 0,5 % dos pixels acima
   disso; em ambiente igual espera-se zero. Rodam no passo de render (não no vitest puro).
3. Shader sem estado, sem relógio, ruído por hash inteiro `highp`. Declarar `cost`, `padding`, `needs`, `moods`,
   `useWhen`, nomes e resumos nos 3 idiomas, `typicalDurationFrames` e `sample`.
4. Subir `catalogVersion` se mudar o resultado de um efeito existente e registrar no `fx-catalog.md`.

## 10. Integração com cada ponto pedido

### 10.1 Biblioteca (engine puro, `@tessel/scene-engine/core`)

Exports novos: `fxSchema`, `fxCommandSchema`, `FX_COMMAND_SHAPES`, `applyFxCommands`, `explainFx`,
`resolveFxFrame`, `resolveShotAtOffset`, `normalizeTransitions`, `expandPreset`, `FX_CATALOG`, `fxEffect(id)`,
`listFxEffects({ family?, tag?, mood? })`, **`describeFxCatalog({ locale, family?, mood?, maxChars })`** (texto
compacto para prompt de LLM: id, quando usar, parâmetros e limites, custo; com teto de tamanho),
**`suggestTransition(bundle, fromShotId, { mood })`** (pura; devolve `{ effect, durationFrames, alignment,
handles, params? }`: filtra as transições do catálogo pelo humor; prefere as que cabem com `handles: "use"`,
depois o menor `cost`, depois a `typicalDurationFrames` mais perto do que o corte permite (`2 × handles`
disponíveis); empate pelo id em ordem alfabética; sem candidato, `crossDissolve` com `handles: "hold"`), tipos `Fx*`. Nada de I/O nem GL. `src/core.ts` nunca importa
de `src/node/`.

### 10.2 `editBundle` e propriedades

§5.1 (as quatro edições em `edit-bundle.ts`) e §6.2 (a fiação do registro). `scene set` e `getProperty` sobre um
bundle funcionam sem browser. **Entregue na F3.**

### 10.3 Outline (camadas) e trilhas

- Seção `fx` do outline (§4.3). Trilha oculta não é desenhada nem no preview nem no export.
- Trilhas do editor, componentes irmãos de `CameraTrack`/`KeysTrack`/`FollowTrack`/`RigTrack`:
  - **`TransitionsTrack`**: bloco por corte com tipo e duração, **bordas arrastáveis** (`resizeTransition`) e um
    ponto de **alinhamento** arrastável ao longo do corte; corte sem transição mostra "+" (abre a galeria
    filtrada em transições). Soltar um card da galeria sobre um corte faz `setTransition` (que substitui).
  - **`FxTrack`** (uma faixa por trilha): clipes arrastáveis e redimensionáveis, empilháveis em trilhas, losangos
    de chave por parâmetro (reusa `keyTracks`), indicador de envelope e de custo (cor por `cost`).
  - Tudo por `setProperty`/`editBundle` com update otimista e undo (`createDomain`).
- **Como a edição na timeline foi entregue (F5, etapa 2a)**: em vez de `setProperty`, um domínio `fx` no
  `editorStore` (`createDomain<Fx>`, uma chave para a camada toda) com `applyFx` (mostra, sem gravar), `commitFx`
  (`explainFx`; um passo de undo; salva), `editFx` (comandos `applyFxCommands`) e `removeSelectedFx` (Delete).
  Salva a camada inteira em `PUT /bundle-fx` (`applyPropertyChanges` com `{ domain: "fx" }`, o mesmo portão
  `finalizeBundle`), porque desfazer restaura um valor, não um comando. A conta do arrasto é pura e testada:
  `fxClipDrag.ts` (mover/redimensionar: arredonda para frame, mínimo 1, dentro do edit, redimensionar para na
  vizinha; mover por cima de outro clipe é *reportado* e vira fantasma de erro) e `fxTransitionDrag.ts` (o corte
  não se move e só existem três alinhamentos, então a borda arrastada escolhe, por busca, o par
  (alinhamento, duração) cuja borda fica mais perto do ponteiro; a outra borda só desempata; limite =
  `maxTransitionFrames` do alinhamento escolhido). Um arrasto recusado pelo motor não é mostrado e, ao soltar,
  volta ao que estava, com o motivo na linha de status. A dissolução legada de um plano (`legacy`) só se vê e se clica.
- **F5, etapa 2b — painel de detalhe de toda camada** (entregue): clicar em um clipe, uma transição ou um plano da
  timeline abre um painel **gerado** do registro de propriedades e do catálogo (`describeFxInspector`,
  `describeShotDetail`; Studio: `describeFx`, `describeShot`, `useTimelineInspector`). Cabeçalho do clipe (trilha,
  escopo, início, duração, fade in/out, semente com "nova semente", escondido) e da transição (duração,
  alinhamento, handles, easing, semente, escondido); um bloco por parâmetro com o controle do tipo (slider com
  unidade e `step`, interruptor, seleção, cor, ponto, referência a luz/caminho/camada), **chave no playhead** e
  **restaurar**; nome, família (cor), resumo e "quando usar" no topo, no idioma da interface. Cada edição é UM
  passo de undo, com prévia ao vivo, e a recusa aparece no painel. O spec monta o modelo dos 59 efeitos
  (controle, rótulo nos 3 idiomas, faixa). O plano e a chave de follow ganharam painel/campo de frame. Mapa
  "camada → o que o painel mostra → caminho no registro" e a fronteira core/Studio em `studio-core-boundary.md`;
  decisões #128–#134 em `scene-fx-decisions.md`.
- **`FxValidationPanel`** (novo, irmão do `RigValidationPanel`, não generalização dele): lista `explainFx`
  (erros e avisos), com clique que leva o playhead.

### 10.4 Galeria (entregável)

- Biblioteca: o Studio importa `listFxEffects`/`describeFxCatalog` direto do `core` (**sem rota** `GET /fx-catalog`).
- `scene fx gallery --out <dir>` renderiza uma miniatura animada (WebM curto ou sequência de PNG) por efeito sobre
  as placas de teste do engine (duas imagens neutras para transições; uma ilustração de cena dark para looks), e
  grava `gallery.json` = `{ catalogVersion, effects: { <id>: { hash, file, bytes } } }`. `hash` = hash do meta e
  do shader do efeito: miniatura velha é detectável.
- Local: `packages/scene-engine/output/fx-gallery/` (gitignored). O harness serve só essa pasta, com confinamento
  de caminho (`GET /fx-gallery/*`), a única rota nova de leitura.
- **Sem galeria gerada** (CI, primeira execução): o card mostra um quadro estático renderizado ao vivo pelo
  compositor sobre as placas, ou o painel diz "rode `scene fx gallery`"; nunca uma grade vazia.
- Studio: painel **Efeitos** (grade de cards, miniatura ao passar o mouse, busca, filtros por família/tag/humor,
  resumo e parâmetros principais). Arrastar um card: sobre um corte = transição; numa trilha = clipe; sobre uma
  camada = escopo `node`. Clique duplo aplica no playhead com `typicalDurationFrames`.

### 10.5 CLI `scene` (e módulo Node)

Convenção da fase 6 da API (`scene <camada> <bundle> <verbo>`), raiz `fx` para tudo; comandos de catálogo não
recebem bundle:

```
scene fx catalog [--family transition|look|suspense] [--tag dark] [--mood suspense] [--json] [--pretty]
scene fx show <effectId>                       # parâmetros, limites, custo, exemplo de comando
scene fx describe --locale pt-br [--mood m] [--max-chars N]     # texto compacto para prompt
scene fx preview <effectId> [--params '{…}'] [--frames N] --out <dir> [--force]
scene fx gallery --out <dir> [--force]
scene fx <bundle> list                         # trilhas, clipes, transições
scene fx <bundle> apply '<FxCommand[]>' [--write]
scene fx <bundle> explain                      # erros e avisos
scene fx <bundle> preset <presetId> --start N --frames N [--scope '{…}'] [--set k=v …] [--write]
scene fx <bundle> transition list | set <fromShotId> <toShotId> <effectId> --frames N [--align a] [--set k=v …] [--write] | suggest <fromShotId> --mood m
scene show <bundle> --layer fx                 # --layer ganha "fx"
scene render <bundle> --out <dir> [--frames 120,240,…] [--contact-sheet]   # opções aditivas entregues pela F3 deste plano
```

Saída: um objeto JSON em stdout (`--pretty` indenta), exit 0 quando `ok`, 1 quando recusado (JSON de erro), 2 em
uso incorreto; **nada é gravado sem `--write`**; `--out` é o opt-in de `preview`/`gallery` e **nunca sobrescreve
sem `--force`**; falta de Chrome/`ffmpeg` para `preview`/`gallery` é exit 2 com mensagem clara. `scene validate`
inclui `explainFx`. Módulo Node: `renderBundle` usa o compositor; `renderFxPreview`, `buildFxGallery`
(`src/node/fx-gallery.ts`).

**Entregue na F3**: tudo acima **menos `scene fx gallery` e `buildFxGallery`** (F6; o comando responde exit 2).
Em `src/node/`: `fx-preview.ts` (`renderFxPreview`), `render-frames.ts` (`renderFrames`), `image-sheet.ts` (tira,
folha de contato e a recusa de sobrescrever), `fx-lab-bundles.ts` (placas e bundle de amostra, antes só no
script do laboratório; §18 #65). `--frames` quer dizer coisas diferentes por verbo (§18 #62). `validate` só
ganha a chave `warnings` quando há aviso (§18 #63). Sem presets reais antes da F4, a CLI lê os de teste de
`SCENE_FX_PRESETS` (§18 #64). A CLI só importa `core` e `node`.

### 10.6 Preview (harness) e Studio

- **Verificado na F3**: `grep -nE "applyFxCommands|resolveFxFrame|expandPreset|crossDissolve|dipToBlack"
  packages/scene-engine/scripts/studio-harness.mjs apps/scene-studio/src` não acha nada.
- O harness não ganha lógica de efeito: o Studio salva por `PUT /p/<id>/bundle-edit` (`putBundleEdits`, já
  existente) e lê a galeria por `GET /fx-gallery/*`.
- Para não conflitar com a outra sessão que commita direto na `main` em `StudioEditor.tsx`, `editorStore.ts`,
  `projectsRegistry.ts` e `studio-harness.mjs`: **o código novo vai em arquivos novos** (`fxSlice.ts` como
  `rigSlice`/`pathSlice`, `describeFxClip.ts`/`describeTransition.ts` no inspector, `FxMenu.tsx`,
  `TransitionsTrack.tsx`, `FxTrack.tsx`, `FxValidationPanel.tsx`, `FxGallery.tsx`), e os arquivos compartilhados
  recebem **um único commit mínimo de montagem** (hunks pequenos), depois de `git fetch && git merge origin/main`
  e commitando por caminho. `StudioSelection` ganha `fxClip` e `transition`.
- `window.__sceneStudio` ganha `listFx`, `applyFx`, `explainFx` como **uma linha cada, sem lógica** (chamam
  `listFxEffects`, `editBundle`+salvar e `explainFx` do core).
- Menu **Efeitos** no cabeçalho (Adicionar efeito ▸ por família, Adicionar transição, Aplicar preset, Abrir galeria),
  irmão de Rig e Seguir; inspector gerado do catálogo (`buildLevaSchema`) com botão de chave por parâmetro. i18n:
  rótulos e resumos vêm do catálogo; só as strings de interface ficam no i18n do app (pt-br/en/es).

### 10.7 Pipeline de geração de vídeos

Exemplo `packages/scene-engine/examples/fx-scene.mjs` (extensão do `narrated-scene.mjs` da API, que precisa
existir): abre o bundle, usa **`suggestTransition`** por humor da fala (a tabela humor→efeito é o dado `moods`/`useWhen`
do próprio catálogo, sem duplicar em arquivo do exemplo nem na skill), aplica `coldOpen` no primeiro plano,
`creepingVignette` antes do clímax e `stinger` no corte, valida com `explainFx`, renderiza e imprime o caminho.
Um teste executa o exemplo num bundle pequeno, com o render desligado quando falta `ffmpeg` (registrado no log,
sem `.skip`).

## 11. A skill

`.claude/skills/scene-fx/SKILL.md`, com o frontmatter (`name`, `description` com gatilhos) das outras skills do
repo e a estrutura de seções de `scene-cli` (que a fase 7 da API cria; se a F6 começar antes, usar o formato do
§7 da API). Linkada de `scene-cli`, de `scene-element-recipes` e do `scene-engine-index.md`. Conteúdo:

1. Quando usar (transição entre planos, look, suspense) e o que **não** usar (efeito de dentro da cena, como a
   escrita no papel: isso é `scene-element-recipes`).
2. As três camadas e o que cada comando chama (CLI ↔ `editBundle` ↔ `applyFxCommands`).
3. Todos os comandos `scene fx …` com um exemplo cada e os equivalentes em JS.
4. Formas de `FxCommand` e `BundleEdit`; caminhos `fx/...` e `transition/...`; como listá-los (`scene props`).
5. Catálogo por família e a tabela "quando usar" (humor → efeito → duração típica) **gerada de `moods`/`useWhen`**
   (um comando imprime a tabela; a skill cola a saída), e limites de custo.
6. Erros comuns (`TRANSITION_NO_HANDLES`, `FX_CLIP_OVERLAP`, `PARAM_OUT_OF_RANGE`, `PRESET_NEEDS_EDIT_BUNDLE`…) e o
   que fazer.
7. Receitas: abertura dark; subir suspense até um susto; transição por humor numa pipeline; trocar o look de uma
   cena; revelar por lanterna seguindo um path; como medir (`explain`) e conferir quadros.
8. Regras de qualidade: determinismo (sem relógio), um efeito por intenção, pilha ≤ 8 passes, nenhuma transição
   sobre fala sem aviso, olhar quadros reais (contact sheet) antes de aprovar.

A skill é escrita **depois** de os comandos existirem: cada exemplo é executado e a saída colada.

## 12. Fases, dependências e critérios

Atualização (rev. 4): a API **entrou na `main`** (`2bfae8ef25`): `src/properties/`, `editBundle`, módulo Node,
`bin/scene.mjs`, `scene-engine-api.md` e a skill `scene-cli` existem; F3 e F6 deixam de esperar por ela. Os
nomes e assinaturas reais (de `scene-engine-api.md`) valem sobre o que este plano supôs, e a F3 começa por reconciliar
os dois (§19).

| Fase | Entrega | Depende de | Critério de aceite |
| --- | --- | --- | --- |
| **F1** Schema, meta do catálogo, comandos, resolução pura | `fx-schema`, `applyFxCommands`, `explainFx`, `resolveFxFrame`, `resolveShotAtOffset`, `normalizeTransitions`, `fxKeyIssues`, contrato `FxEffectMeta` e meta dos 4 efeitos de referência (`crossDissolve`, `dipToBlack`, `vignette`, `filmGrain`; o spec 1:1 meta↔shader vale a partir da F2); `evaluateShotFrame`; **sem GL e sem tocar a validação de `dissolve`**. **Primeiro passo do F1**: gravar o **baseline de pixels** a partir da `main` limpa (hash de N quadros do fixture técnico `generate-technical-fixture.mjs`, que está no repo, e do `full-v2`; arquivo versionado `packages/scene-engine/src/fx/__baseline__/pixels.json`) | nada | Unitários por `op` e por código de erro; **fixtures de bundles existentes continuam válidos**; `evaluateEditFrame` idêntico ao de hoje por `digestResolvedState`; seek aleatório = linear em `FxFrameState`; vetores do `lowbias32`/`fnv1a32`; ouro de `resolveShotAtOffset` nas bordas (`instanceFrame < 0`, `≥ duração`, câmera fora do clipe) e das janelas por alinhamento |
| **F2** Compositor e conjunto central | **F2-a feita (2026-11-02)**: S1 ([relatório](scene-fx-s1-report.md), §8.2), renderer injetável, `renderToTarget`, `createDeckTarget`, baseline com os três looks. **F2-b feita (2026-11-02)**: `FxCompositor` (entrada, passes, transição, saída com dither), pool de decks (um segundo `ThreeSceneRuntime` só durante uma transição entre definições diferentes), integração em `ThreeSceneRuntime.renderEditFrame` (Studio, harness e render usam o mesmo caminho, sem lógica de efeito neles); **meta e shader** dos 12 efeitos (spec 1:1): `crossDissolve`, `dipToBlack`, `dipToWhite`, `wipeLinear`, `push`, `zoomCross`, `vignette`, `filmGrain`, `colorGrade`, `bloom`, `chromaticAberration`, `letterbox`; `scripts/fx-golden.mjs` com 12 goldens; o `dissolve` legado aceito e desenhado (§4.2); `NODE_SCOPE_NOT_YET`. Não houve extração de `SceneDeck` (§18 #32) | F1 | **Baseline** (gravado na F1, com os fixtures dos três looks gravados na F2-a antes dos ganchos): o render sem `fx` dá os mesmos hashes do `pixels.json` (**verificado depois da F2-b: 20+20+17+17 quadros idênticos**); golden stills (**12/12, diferença 0 em duas execuções**); cadeia de identidade (efeitos a `intensity: 0` ≤ 1/255 contra o caminho direto); transição entre dois planos reais (`crossDissolve` e `wipeLinear` em scribe→hands e hands→manor do `full-v2`); `pixel-determinism` com `fx` (`scripts/fx-determinism.mjs`): ordens de seek diferentes **e** hash por quadro igual entre execuções |
| **F3** Integração | `fx-properties` + fiação (§6.2), `{ fx }` em `BundleEdit` (§5.1), seção `fx` do outline (§4.3), `scene fx …`, `renderBundle` com `fx`, `scene render --frames`; Studio salva por `bundle-edit` | F2 **e** API fases 1, 2, 3, 4, 5 e 6 **na `main`** | Cada comando CLI roda numa cópia de bundle com a saída colada; recusa com exit 1; `grep` de prova abaixo; specs de navegador existentes passam |
| **F4** Catálogo completo | Todas as transições, os efeitos contínuos e os presets das tabelas §9, em lotes por família | F2 (shaders) e F3 (presets via `editBundle`) | Cada efeito com meta + shader + `.spec.ts` + golden + `sample` + 3 idiomas; ms/quadro por efeito no relatório; `scene fx catalog` lista todos |
| **F5** Studio | `TransitionsTrack`, `FxTrack`, `FxGallery`, `FxMenu`, inspector gerado, `FxValidationPanel`, i18n, `fxSlice`/`StudioSelection` | F3 | Arrastar borda de transição muda `durationFrames` e dá undo; soltar um card sobre um corte cria/troca a transição em **um** passo de undo; `scene-studio-fx.spec.mjs` (cabeçalho copiado do `scene-studio-rig.spec.mjs`, `--bundle <cópia>`, bundle de teste com `fx` em `src/testing/`, não em `output/`) e os existentes passam; prints do editor |
| **F6** Pipeline, galeria, skill, docs, vídeo | `scene fx gallery`, `fx-scene.mjs`, skill `scene-fx`, `scene-engine-fx.md`, `fx-catalog.md`, vídeo `full-v3` (assets em `~/www/.data/scene-engine/gen-lab/full-v2`, fora do repo, reproduzíveis por `build-full-bundle.mjs`) | F4, F5 e API fases 3, 5, 7 e 8 na `main` | Vídeo renderizado por script e conferido por quadros (1 por segundo, tamanho cheio, 3 quadros por transição); skill com todos os exemplos executados |

Antes de cada fase: `git fetch origin main && git merge origin/main`, preservando o trabalho de outras sessões;
commits por caminho, convencionais (`feat(video): …`/`ref(video): …`). Cada fase termina verde (type-check,
testes do engine e do Studio, lint, format). Nenhum bundle com `fx` entra em fixture compartilhada antes de a F1
estar em todos os worktrees (um engine sem `fx` descartaria a chave ao regravar o bundle: ver §15).

`grep` de prova (F3): `grep -nE "applyFxCommands|resolveFxFrame|expandPreset|crossDissolve|dipToBlack" packages/scene-engine/scripts/studio-harness.mjs`
**não** acha nada além de roteamento; a mesma busca em `apps/scene-studio/src` acha só chamadas ao
`@tessel/scene-engine/core`/`/node`, nunca lógica de efeito.

## 13. Como isto é implementado e revisado

1. **Plano**: revisado por dois revisores independentes (render/determinismo; padrão/UX/pipeline); achados e
   tratamento em [§16](scene-fx-decisions.md).
2. **Implementação**: Cursor (Sonnet 5.5, esforço alto), uma fase por vez, na `main` atual (checkout compartilhado),
   sem commit nem push (o shell do Cursor injeta um trailer proibido); o líder commita por caminho, sem menção a IA.
3. **Revisão de cada fase**: skill `thermo-qodex:review` (OpenQodex + `code-review`) sobre o diff da fase com
   `--base origin/main`, mais a verificação própria do líder (§14) e abrir os PNG/MP4 reais.
4. **Correção**: achados voltam para o mesmo implementador; nova rodada e nova revisão; a fase só fecha com zero
   achados bloqueantes e os critérios do §12 cumpridos com saída real colada.
5. **Fechamento**: o líder re-roda a verificação final e só então mescla.

## 14. Verificação final (o líder re-roda)

- Type-check (`pnpm exec nx run-many -t type-check -p @tessel/scene-engine @tessel/scene-studio`), `pnpm test` no
  engine e no Studio, lint e format: verdes.
- `pilot:pixel-test` e `pixel-determinism` com e sem `fx`; o baseline do `full-v2`.
- `scene fx catalog|show|describe`, `scene fx <bundle> apply|preset|explain|transition set` numa cópia de bundle,
  com a saída colada.
- Specs de navegador: os existentes e `scene-studio-fx.spec.mjs`; prints do editor (trilha de transições, trilha
  de efeitos, galeria, inspector).
- Render do `full-v3`: contact sheet 1 quadro/s, 3 quadros por transição em tamanho cheio, `ffprobe` do áudio.
- O `grep` de prova da §12.
- Relatório: fases, commits, contagem de testes, o que ficou de fora e por quê.

## 15. Riscos e fora de escopo

- **Dois planos ao mesmo tempo com splat**: S1 com critério de saída; degradação documentada (`hold`/`cut` +
  aviso) enquanto não provado.
- **Engines antigos descartam `fx`**: `renderBundleSchema` não é estrito; um worktree sem `fx` que regrave o
  bundle remove a chave em silêncio (e um outline com `fx:` é recusado, esse sim em voz alta). Mitigação: F1 sobe
  antes de qualquer bundle com `fx` entrar em fixture compartilhada; `catalogVersion` protege só quem já conhece `fx`.
- **GPU do export headless**: usa o Chrome do sistema; medir ms/quadro e a capacidade de HalfFloat no início;
  `--fx-quality draft` apenas para rascunho, nunca no export final.
- **Efeitos que prometem mais que entregam** (`lensBlur`, `whisperFocus`): os resumos dizem o que fazem de
  verdade (por distância na tela, sem profundidade).
- **Planos sem handles**: `hold` documentado; o editor mostra o aviso na trilha.
- **Mudança de aparência por versão**: `catalogVersion` e golden stills.

Fora de escopo: alpha no compositor (`UNSUPPORTED_ALPHA`), motion blur real por acúmulo, partículas com
simulação, efeitos que dependem de profundidade de verdade, áudio (stingers/risers), transições 3D com geometria
(cubo, virar página), exportação para formatos de NLE (EDL/XML), IA gerando efeitos novos, novas curvas de easing
(mudança separada).

## 16. Tratamento dos achados da revisão 1

Cada achado das duas rodadas de revisão do plano e a seção que o resolve estão em [scene-fx-decisions.md](scene-fx-decisions.md) (§16), em uma tabela por achado.

## 17. Decisões fechadas na revisão 3 (nada aberto para o implementador)

As 28 decisões numeradas (`§17 #1` a `§17 #28`: fórmula do `progress`, escopo por deck, handles, `setTransition` sobre um corte ocupado e as demais; `#24` a `#28` são as que o S1 fechou) estão em [scene-fx-decisions.md](scene-fx-decisions.md) (§17); toda citação `§17 #N` do plano e do código é desse arquivo.

## 18. Desvios da implementação

Cada desvio da implementação em relação ao plano (`§18 #1` em diante, incluindo as correções das rodadas de revisão da F1, os da F2-a, `#31` a `#34`, e os da F2-b, `#35` em diante) está em [scene-fx-decisions.md](scene-fx-decisions.md) (§18), com onde o plano diz, o que foi feito e por quê.

## 19. Estado da `main` e reconciliação (revisão 4, 2026-11-02)

Depois da revisão 3 a `main` ganhou duas coisas que este plano precisa reconhecer.

### 19.1 A API entrou (`2bfae8ef25`, #814)

`editBundle`, `properties/*` no engine, `@tessel/scene-engine/node`, `bin/scene.mjs` (`scene …`), harness fino, a skill
`scene-cli` e `docs/video-production/scene-engine-api.md`. Consequências: (a) F3 e F6 não esperam mais; (b) o desvio
#5 da F1 (`FxPresetEdit` próprio por não haver `BundleEdit`) se resolve na F3 trocando por `BundleEdit`; (c) a F3
**começa conferindo cada suposição da §5.1, §6 e §10.5 contra o código real** (nomes, assinaturas, formato do erro,
`finalizeBundle`, `BundleEditLayer`) e corrige o plano onde divergir, antes de escrever código.

### 19.2 Já existe uma camada de "looks" de cena (`0237950a70`, `b46f6efbb0`)

O que existe: `sceneDefinition.effects` (`effects/scene-effects.ts`, objeto **estrito** `{ ink?, dream?, film? }`, cada um
0..1, ordem fixa ink → dream → film), a propriedade `scene/<id>/effects/<look>`, a rota `/bundle-effects`, o
`PostEffects` (`runtime/post-effects.ts`: alvo half-float **multisampled** + um shader que aplica os looks, e o caminho
direto de antes quando nenhum está ligado) e o item "Efeitos" no painel de camadas do Studio (`describeEffects.ts`).
Decisões para **reconciliar sem duplicar**:

1. **O compositor da F2 estende `runtime/post-effects.ts`, não o substitui.** Reaproveita a criação do alvo half-float,
   o `samples` (MSAA já resolvido; o S1 fixou 4, §8.1) e a guarda "sem looks, caminho direto". O S1
   (dois decks e splat) continua sendo o risco real.
2. **`ink`, `dream` e `film` entram no catálogo** como efeitos `look` (`kind: "pass"`), cada um com os parâmetros que o
   shader já tem como constantes (raio do Kuwahara, intensidade do glow, etc., com faixa e padrão). Os trechos de GLSL
   são **fatorados em chunks compartilhados** (`runtime/fx/shaders/chunks/`) usados pelo `PostEffects` e pelos
   passes do catálogo: **uma fonte só**; o `PostEffects` não muda de pixel (critério abaixo).
3. **`scene.effects` continua válido e com o mesmo significado**: o *look de base* da cena, aplicado primeiro (mais
   interno), depois os decks, a transição e os passes `output`. Não vira clipe `fx` automaticamente e não é migrado:
   o que é estático por cena fica nessa porta; o que tem tempo, chaves, escopo ou transição fica no `fx`.
4. **Sem colisão de nomes**: propriedades `scene/<id>/effects/<look>` (cena) e `fx/...`/`transition/...` (camada fx);
   `BundleEditLayer` ganha `"fx"`, e `{ set: "scene/…/effects/…" }` continua sendo `set`.
5. **Aviso novo `FX_DUPLICATES_SCENE_LOOK`**: um clipe `fx` de `ink`/`dream`/`film` num plano cuja cena já liga o mesmo
   look empilha o efeito duas vezes; o `explainFx` avisa (não bloqueia).
6. **Studio**: o item "Efeitos" do painel de camadas passa a se chamar **"Looks da cena"** (i18n, F5) para não colidir
   com o menu e o painel **Efeitos** do `fx`; o inspector dele não muda de comportamento e ganha um atalho "Abrir
   galeria".
7. **Critério novo na F2**: o baseline de pixels (§12) inclui **um bundle de cena com os três looks ligados** (força
   0.5), e o `fx-pixel-baseline.mjs verify` compara esses quadros depois da fatoração dos shaders: **idênticos**.
   **Gravado na F2-a**, antes dos ganchos do runtime: `technical-fixture-looks` (obrigatório) e `full-v2-looks` (opcional),
   derivados em memória por `scripts/fx-looks-fixture.mjs`; ambos idênticos depois do refactor.
   O spec de navegador `scene-studio-effects.spec.mjs` existente continua passando sem mudança.
8. A F4 cobre os looks existentes como efeitos de catálogo e **não** adiciona um segundo `ink`/`dream`/`film` com outro
   visual: o catálogo expõe exatamente os shaders já em produção.

**Estado depois da F2-b (§18 #48):** os itens 1 e 2 **não** foram executados como escritos. O compositor é novo
(`runtime/fx/`) e **não estende** `PostEffects`, e os GLSL dos looks de cena **não** foram fatorados em chunks
compartilhados: `post-effects.ts` não mudou, então o critério do item 7 vale por construção (o baseline dos looks segue idêntico:
20+20+17+17 quadros). Fatorar passa a ser da F4, junto com `ink`/`dream`/`film` no catálogo (item 8).

**Mescla com a `main` (2026-11-02):** o `PostEffects` foi movido pela `main` para `runtime/post-effects/post-effects.ts` (shader montado em
`assemble.ts`, um módulo GLSL por look); a mescla e as decisões estão em [scene-fx-decisions.md §19](scene-fx-decisions.md) (#94 a #103). Onde este
plano diz `runtime/post-effects.ts`, leia `runtime/post-effects/post-effects.ts`.
