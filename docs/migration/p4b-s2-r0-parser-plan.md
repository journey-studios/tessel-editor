# P4b S2-R0 — leitura interna do plano de dois decks

Data: 2026-11-15. Base: main `a61b7a9ef3c3e269c5c672c37c1f89d08cff1746`
(PR73/H integrada). Backlog único: BL082.4.4. Contratos: F064, ScenePlan e plano
canônico P4b §§5.3, 7.3, 8.1–8.4, 12. Este documento fixa o contrato antes do código;
o backlog concentra status/atribuição. Não declara S2 inteira entregue.

## Objetivo e fronteira

Entregar a leitura pura e testável do wire C0b, reutilizando o leitor de deck atual,
e fechar uma recusa ausente. O helper interno de transição ainda não será ligado ao
compositor, bridge, worker ou a qualquer entrada pública de desenho. Não haverá
`allowTransition`, flag de ambiente, alteração do hello ou bypass para desenhar.

JS continua com 13 capacidades, sem `fxTransition`; Rust mantém 14. O catálogo público
continua com os três looks. `Refuse` público e F064 produto P permanecem. Nenhum Domain,
comando, shader, GPU, UI, export, cache ou lifecycle novo nesta PR.

A H foi aceita por Windows R2 em d7c52b7 (código05f37da, testesfe14cfc), com render76,
core216/10/5/8/1 e export10/12 por shasum/BL167.3. Mac separado: export12/12 no mesmo
código, render80 no reforço de testes. Nenhuma dessas provas valida a R0.

## Decisões anteriores ao código

1. A entrada de produção `parseDirectedLayer` continua recusando uma transição válida
   por `unsupported`, `details.missing: ["fxTransition"]`, antes de desenho. Um campo
   próprio `transition` sem `fxTransition` em `requires` deve dar `invalid_plan` em
   `/requires`; não pode ser ignorado nem virar desenho apenas do primário. Isso fecha
   uma lacuna do parser atual, registrada sob BL082.4.4. Transição ausente preserva
   resultados, mensagens e ordem das recusas existentes.
2. A nova API é interna de módulo, sem entrada de bridge: `parseTransitionPlan(plan,
   references)` para objeto e `parseTransitionPlanText(text, references)` para texto.
   Ambas retornam dados de desenho dos dois decks e dados de mistura já resolvidos.
   Não chamam compositor, não carregam assets, não recalculam poses nem editam timeline.
   Os nomes podem ser ajustados por clareza, documentados antes do código respectivo.
   Sem propriedade própria transition, essa API dedicada recusa em /transition. Separar
   negociação de capacidades do leitor de estrutura de deck compartilhado.
   Não adicionar opções para abrir a entrada de produção. Reconhecer a forma interna
   do efeito não é anunciá-lo no hello nem servir um frame com ele.
3. Um único parser de deck verifica os mesmos campos de desenho do caminho puro atual.
   Extrair esse parser mecanicamente, com controles dos testes existentes. O vizinho
   usa esse mesmo leitor, com ponteiro prefixado uma única vez por `/transition/other`.
   Erros de outro deck preservam código, detalhes existentes e acrescentam o contexto
   preciso; o erro do topo conserva o contrato anterior. Não copiar o leitor inteiro.
4. O helper de transição exige o wire C0b: `alignment` e `timing` obrigatórios, sem
   defaults para o formato experimental antigo. Exigir chaves/tipos conhecidos da
   transição e de timing; nomes são dados próprios, sem lookup no prototype.
   `effect` só `crossDissolve`, `primary` from/to, flags booleanos, easing reconhecido,
   params em ordem/tipo do catálogo (`curve`, `intensity`), nenhum parâmetro extra.
5. `progress` já vem eased e `amount` vem da intensidade; o runtime só confere coerência.
   Conferir os dois contra o contrato (tolerância fixa1e-9), sem usar amount=intensity
   vezes progress. Conferir janela/corte/alinhamento, papel do primário, frame na janela
   e no shot primário, IDs, comprimentos compartilhados, cobertura do próprio shot,
   relógios raw/sample/held independentes e tempos Ratio exatos pelo FPS racional.
   O primário conserva os contadores brutos; o other conserva amostras. L=0 e último
   quadro real seguem C0b; não alterar as regras do core nem fabricar novos estados.
6. JS aceita somente inteiros `Number.isSafeInteger` nas quantidades inteiras lidas,
   com sinais conforme o campo. Isto é subconjunto explícito do i64/u64 do Rust; recusar
   dados não representáveis, nunca arredondá-los em admissão. JSON.parse perde a grafia
   12/12.0: não afirmar que objetos JS distinguem lexemas numericamente iguais. Usar
   aritmética exata/verificada nas relações (BigInt local, se necessário), sem conversão
   silenciosa de intermediários grandes, overflow aceito ou comparação de duas falhas.
7. Teto de 20000 itens sobre os seis arrays de AMBOS os decks: nodes/layers/splats/fx/
   resources/lights. Pré-checar comprimentos antes de ler elementos; teto exato aceita
   um plano estruturalmente válido, +1 recusa. Profundidade1: detectar own
   `other.transition` antes de descer, sem recursar; bomba recusa tipada, sem RangeError.
8. Os 4194304 bytes valem só para a API textual, medidos em UTF-8 no texto recebido.
   Não usar JSON.stringify(objeto) como medida do texto original. O wrapper textual é
   dedicado ao plano de transição e verifica bytes antes do parse; truncamento recebe
   erro tipado. O bridge atual recebe objeto ou parseia mensagem inteira: esta PR NÃO
   promete o teto na entrada de transporte, cuja integração fica para R1/desenho.
   Testar texto exato e +1, inclusive caracteres multibyte e chaves escapadas.
9. Conferir versão/output/FPS/tempos reais iguais nos decks, identidade de shot, lista
   de passes (shot primário, shot vizinho, output uma vez só no topo), clipId único entre
   decks, requires coerente com ambos e fxTransition, recursos separados por deck.
   Mesmo assetId com metadata conflitante recusa; IDs de nós iguais entre decks são
   válidos. Não inventar uma chave de nó global nem substituir a união do core no host.
10. APIs são puras: não mutar plano/references/arrays, nem ordenar a entrada no lugar;
    recusas não deixam dados parciais. O resultado pode conter Maps como o parser atual.
    Reutilizar erros RuntimeError/invalid_plan e os códigos existentes, sem TypeError
    para null, prototype names, arrays inválidos, números não finitos ou bombas.
11. Nada altera dados S0/X, Python, PNG, manifesto, golden ou baseline. Fixtures JS
    próprias podem ser escritas à mão, com contas independentes. A interoperabilidade
    deve também consumir ScenePlan construído pelo core real sob Accept só de teste.
12. Antes da PRIMEIRA medição de pixel de transição, a próxima fatia R deve fixar o
    aceite: manter máximo1 código/canal e viés absoluto<=0,25 POR QUADRO (canônico).
    Agregado por cenário da extensão X será apenas evidência adicional, nunca substituto
    do gate por quadro. Poucas sondas podem tornar o teste mais estrito; falha não autoriza
    retune posterior. R0 não executa nem compara pixels e não resolve isso por medição.
13. O desenho/cache seguinte precisa fixar a política agregada de retenção ANTES do código:
    targets=2 atual é limite flexível (trim conserva entradas usadas no frame). Não elevar
    limites silenciosamente nem declarar orçamento estrito só pelo LRU ou tamanho por alvo.
    Essa decisão permanece sob BL082.4.4, fora da implementação R0.

## Ownership e commits

- Novos módulos puros coesos `runtime/src/directed-transition*.js` e helper de deck/checks
  se a extração exigir; novos arquivos abaixo de1000linhas, preferindo limites bem menores.
- `runtime/src/directed-scene.js`: extração do leitor compartilhado e recusa do campo
  transition omitido de requires; conservar SCENE_CAPABILITIES e contrato de render puro.
- `runtime/src/output-fx.js`: só costura mecânica necessária para reuse/prefixo de parser,
  sem alterar shaders/fórmulas/semântica de passes existentes. Justificar no relatório.
- `runtime/test/directed-transition*.test.mjs` e helpers próprios; mudanças mínimas nos
  testes antigos para pinagem da recusa. Não enfraquecer testes ou catálogo existentes.
- Novo teste em `crates/render/tests/scene_transition_parser.rs` (ou módulo irmão coeso):
  core constrói planos de teste sob Accept e Node os lê, sem GPU e sem export. Tratar
  dependência de Node explicitamente; o gate obrigatório deve falhar se Node ausente,
  sem contar skip como prova. Não adicionar Accept a produção nem ferramenta pública.
- Docs próprios: este plano, relatório R0 novo, F064 (`fx-transitions.md`) e `scene-plan.md`.
  Backlog/matriz/índice: somente patch literal para o coordenador, com IDs existentes.
- Intocados: crates/core/src, media, desktop, render/src, compositor.js, assets.js,
  splat-stage.js, bridge/worker, shaders, Cargo.*, package/pnpm lock, fixtures/goldens.
- Primeiro commit: contrato e API antes do código. Extração mecânica em commit separado,
  depois leitor de transição/guarda, testes de integração e relatório. Não reescrever
  commits publicados, não force-push, mensagens sem assistentes/Co-authored-by.

## Casos e mutantes mínimos

- Planos simples pelos testes atuais; JSON serializado e resultado de parse preservados.
- Transição completa from/to, C-1/C, center ímpar, start/end, D1 e 30000/1001; raw23/sample11.
- Antigo wire sem alignment/timing; todos os tipos inválidos; keys/params/prototype names;
  outra versão/output/FPS, output no other, clipId duplicado, assetId conflitante.
- primary trocado, quadro exatamente no fim exclusivo do shot, corte/janela adulterados,
  sample/held falsos, relógios ligados entre si, Ratio errado, instance curta no próprio shot.
- 20000 itens válidos aceitos/+1 recusado; lights e other no contador; 4MiB exatos/+1 no
  texto; bomba aninhada e cyclic object recusados sem recursão. Entradas intactas na recusa.
- Mutantes: guarda de missing-requires removida; primário/other prefixados errado; profundidade
  removida; exclusão other/lights do contador; > por >=; byte multibyte contado por length;
  progress sem +0,5/easing, amount vezes progress, fim exclusivo aceito, raw/sample trocados,
  held falso ignorado, cobertura de shot omitida, output no other, clipId duplicado ignorado,
  asset conflito ignorado. Mutantes equivalentes são separados, não classificados como mortos.
- Interop com core real e controle da entrada de produção ainda unsupported/missing, sem GL.

## Gates e entrega

1. Confirmar Windows nativo, checkout preparado, origin/base a61b7a9, árvore limpa, espaço
   e memória; ler AGENTS, skill, backlog, F064 e contratos citados. Não criar clone/worktree.
2. `./scripts/runtime setup` (pnpm frozen); Node tests puros novos e antigos de
   directed-scene/output-fx/luz/contratos pertinentes; build oficial do runtime. Não anunciar
   sucesso de pnpm test global se executar só filtros. Nenhum teste browser/pixel nesta fatia.
3. Cargo pelo `./scripts/cargo`, target mbx padrão: fmt, check/clippy workspace-all-targets
   locked-Dwarnings, test workspace-no-run, build desktop, core compat/public/reload e
   transições/corpus/X/baseline nos dois perfis; teste render de interoperabilidade Node.
   Core completo pode continuar com os dois gizmos Windows da base; não mascarar nem
   chamar workspace global de verde. No-run só compila. Sem clean/gc/lanes/target-dir.
4. Mutantes um de cada vez, restaurar bytes/hash e tocar mtime; controle limpo após cada
   restauração para não herdar artefato alterado. Não tocar processos/TEMP alheios.
5. Relatório por SHA com API, extração, limites JS/Rust, text/object, casos/mutantes,
   executado/leitura/N/A, ownership e patches BL082.4.4/F064/índice. Sem IDs novos por rotina.
6. PR contra main, push normal; link_pull_request como próxima ferramenta após criar/push.
   Não fazer watch/poll/merge. Última linha: PR URL, head SHA, checks/reviews vistos uma vez.
   Revisão independente Windows e validação Mac pertencem ao coordenador, não herdar gates.

## Contrato final de nomes, tipos, API e ponteiros (fixado ANTES do código)

Esta seção só detalha os fatos acima; não os altera. Foi escrita depois de ler
`runtime/src/directed-scene.js`, `output-fx.js`, `crates/core/src/scene_plan/{transition_json,
transition_validate,temporal/*}.rs`, `fx/{window,keys,transition_catalog}.rs` e `time.rs`. Os nomes
são os que o código usa; uma mudança posterior é registrada no relatório com o motivo.

### 1. Módulos (todos puros: sem GL, DOM, bridge nem relógio)

| Arquivo | Papel |
| --- | --- |
| `runtime/src/directed-deck.js` (novo) | O leitor de deck compartilhado, extraído mecanicamente de `parseDirectedLayer`: helpers de campo (`invalid`, `objectAt`, `arrayAt`, `stringAt`, `finiteAt`, `positiveAt`, `integerAt`, `matrixAt`, `vec3At`), `splatMatrixFault`, `SPLAT_SIMILARITY_TOLERANCE`, `MAX_POINT_LIGHTS`, `parseResourceReferences` e `readDeckBody(plan, requires)` (de `/output` até `/lights`, na ordem atual). |
| `runtime/src/directed-scene.js` | Mantém `SCENE_CAPABILITIES`, `parseDirectedLayer` (versão/kind, parcial, capacidades, **guarda nova**, corpo, referências), `parseSceneIdentity` e o desenho. Reexporta os nomes movidos. |
| `runtime/src/directed-transition.js` (novo) | A API: `parseTransitionPlan`, `parseTransitionPlanText`, `MAX_TRANSITION_PLAN_ITEMS`, `MAX_TRANSITION_PLAN_BYTES`, `TRANSITION_CAPABILITIES`. Só orquestra, na ordem da seção 3. |
| `runtime/src/directed-transition-fields.js` (novo) | Chaves/tipos de `transition` e de `timing`, efeito, easing, parâmetros, `progress`, `amount`, janela e `frame` de cada deck. |
| `runtime/src/directed-transition-timing.js` (novo) | A regra de relógios (`DeckClocks`) e os tempos exatos em BigInt, e as relações do `timing` (porta de `scene_plan/temporal/validate.rs`). |
| `runtime/src/directed-transition-decks.js` (novo) | Relações entre os dois decks: versão, output, fonte, shots, passes, `clipId`, recursos, cabeçalho do vizinho e o prefixo `/transition/other`. |
| `runtime/src/directed-transition-limits.js` (novo) | Contagem de itens, profundidade e bytes UTF-8. |

`output-fx.js` **não** precisa mudar: `parseOutputFx` já recebe `(raw, requires, frame)` e devolve uma lista na ordem do
plano, de modo que o prefixo do vizinho e a exclusão de `scope: output` no vizinho são feitos por quem chama. Se uma
costura mecânica se mostrar necessária, ela é justificada no relatório.

### 2. Guarda da entrada de produção

`parseDirectedLayer` ganha, **logo depois** de a lista `requires` ter sido conferida contra `SCENE_CAPABILITIES`:
`Object.hasOwn(plan, 'transition')` ⇒ `RuntimeError('invalid_plan')` em `/requires` («a plan with a transition must
require `fxTransition`»). Depois dessa checagem `requires ⊆ SCENE_CAPABILITIES`, que não tem `fxTransition`; portanto um
plano com `transition` e **com** `fxTransition` já é recusado antes, como hoje, por `unsupported` com
`details.missing: ["fxTransition"]`. Nada novo é aceito; plano sem a chave não muda resultado, mensagem nem ordem.

### 3. API e ordem das recusas

```js
parseTransitionPlan(plan, references)       // objeto; references = layer.resources (array do host)
parseTransitionPlanText(text, references)   // texto JSON; mede os bytes antes do parse
```

Ambas devolvem (e nunca devolvem nada parcial; recusa é `RuntimeError`):

```js
{
  primary: { output, fx, camera, planes, splats, lights, resources },   // forma de `parseDirectedLayer`
  other:   { output, fx, camera, planes, splats, lights, resources },   // o vizinho, lido pelo mesmo leitor
  mix: { id, effect: 'crossDissolve', fromShotId, toShotId, primary: 'from'|'to',
         easing, curve, intensity, progress, amount },                    // progress/amount já resolvidos
  window: { startFrame, durationFrames, frameInWindow, alignment, cutFrame, editFrame },
  timing: { editDurationFrames, from: DeckTiming, to: DeckTiming },
  clocks: { primary: DeckClocks, other: DeckClocks },                     // { instance|camera: { raw, sampled, held } }
  passes: { primary: [...], other: [...], output: [...] },                // shot do primário, shot do vizinho, saída (só no topo)
  references,                                                             // Map, como `parseDirectedLayer`
}
```

Ordem (a primeira falha é a devolvida; a ordem só é normativa onde um teste a fixa):

1. `plan` é objeto (`/`); `kind` (`/kind`); `planVersion` (`unsupported_plan_version`, como a entrada de produção).
2. Propriedade **própria** `transition` ausente ⇒ `invalid_plan` em `/transition`; não objeto ⇒ `/transition`.
3. Teto de itens (seção 5), antes de ler qualquer elemento: `/transition`.
4. Profundidade 1: `other` é objeto com propriedade própria `transition` ⇒ `/transition/other/transition`, sem descer.
5. Parcialidade (`complete`/`unsupported`: `unsupported`, igual à produção) e capacidades do topo
   (`TRANSITION_CAPABILITIES = SCENE_CAPABILITIES + fxTransition`; faltar `fxTransition` ⇒ `invalid_plan` em `/requires`;
   desconhecida ⇒ `unsupported` + `details.missing`). Negociação separada da leitura de deck.
6. Campos da transição (chaves exatas, tipos, `effect`, `primary`, `easing`, `params`, janela, `progress`, `amount`, flags)
   e `timing` por chaves/tipos: seção 4.
7. Corpo do topo: `readDeckBody(plan, requires)`; frame/source/output do topo.
8. Cabeçalho do vizinho (`kind`, versão, `complete`/`unsupported`, `requires` próprios — sem `fxTransition` — e cobertos pelo do
   topo) e corpo do vizinho: `readDeckBody(other, other.requires)` com o erro reescrito por `withinOther`.
9. Relações entre decks, `timing` (formato, janela, identidades, relógios, tempos exatos), `clipId`, `scope: output` no vizinho
   e união de recursos.

`withinOther(erro)` prefixa **uma única vez**: `invalid_plan` ganha `pointer` e mensagem `Scene plan /transition/other<ponteiro>: …`;
`unsupported_plan_version` vira `invalid_plan` em `/transition/other/planVersion`; um erro `unsupported` (parcial, capacidade,
escopo, matriz de splat) conserva `code` e `details` e ganha `details.deck = 'other'` e, quando tem `pointer`, o ponteiro
prefixado. O erro do deck de cima não ganha prefixo, nem `deck`.

### 4. Ponteiros, tipos e fórmulas

- Chaves exatas de `transition` (17): `id effect fromShotId toShotId primary startFrame durationFrames alignment timing
  frameInWindow easing progress amount params heldInstance heldCamera other`. Chave desconhecida ⇒ `/transition/<chave>`
  (`unknown field`); ausente ⇒ mesmo ponteiro (`missing field`). O wire antigo (sem `alignment`/`timing`) é recusado em
  `/transition/alignment`, sem default. Chaves de `timing`: `editDurationFrames from to`; de cada lado: `shotStartFrame
  shotEndFrameExclusive instanceStartFrame instanceDurationFrames cameraInFrame cameraClipDurationFrames
  sampledInstanceFrame sampledCameraClipFrame`, ponteiro `/transition/timing/{from|to}/<campo>`. Presença sempre por
  `Object.hasOwn`: `constructor`, `toString` e `__proto__` não existem como campo.
- Inteiros: `Number.isSafeInteger`, não negativos nos campos de quadro e de contador; `startFrame` ≥ 0; `durationFrames` ≥ 1;
  números fracionários, bool, string, `null`, `NaN`, `Infinity` e negativos recusam no próprio ponteiro. Toda conta entre eles é BigInt.
- `effect === 'crossDissolve'`; `primary ∈ {from, to}`; `alignment ∈ {center, start, end}`; `easing ∈ {linear,
  easeInOutCubic, easeOutCubic}`; `params` = `[["curve", "linear"|"smooth"], ["intensity", n]]`, `n` finito em `[0, 1]`
  (`/transition/params`, `/transition/params/<i>`); `heldInstance` e `heldCamera` booleanos; `id`, `fromShotId` e `toShotId`
  strings não vazias, os dois shots diferentes (`/transition/toShotId`).
- `frameInWindow ∈ [0, durationFrames)`; `startFrame + frameInWindow === frame.editFrame` (`/transition/frameInWindow`).
- `progress` finito e em `[0, 1]`, `|progress − ease((frameInWindow + 0.5) / durationFrames)| ≤ 1e-9`; `ease` é o do core
  (`apply_easing`: prende `t` a `[0, 1]`; `4t³`, `1 − (2 − 2t)³/2`, `1 − (1 − t)³`). `amount` finito em `[0, 1]`,
  `|amount − clamp01(intensity)| ≤ 1e-9`. Ponteiros `/transition/progress` e `/transition/amount`. `amount` **não** é
  `intensity × progress`.
- Janela (`transition_window`): duração fora de `[1, editDurationFrames]` ⇒ `/transition/durationFrames`; `start = cut`
  (`start`), `cut − ⌊D/2⌋` (`center`), `cut − D` (`end`), `end = start + D`; fora da edição ou diferente de `startFrame` ⇒
  `/transition/startFrame`. `cut = timing.to.shotStartFrame`.
- Shots: `start < end ≤ editDurationFrames` (`…/shotEndFrameExclusive`), `from.end === cut`, `primary = from` se `f < cut`, senão
  `to` (`/transition/primary`), `f` dentro do shot do primário (`…/shotStartFrame`, `…/shotEndFrameExclusive` do primário).
- Identidades: mesma instância (`source.sceneInstanceId`) ou mesmo clipe de câmera (`frame.cameraClipId`) nos dois decks
  ⇒ mesmo comprimento (`/transition/timing/to/instanceDurationFrames`, `…/cameraClipDurationFrames`); cada instância cobre
  todo o seu shot: `instanceStart + (end − start) ≤ instanceDuration` (`/transition/timing/<lado>/instanceDurationFrames`).
- Relógios, por lado e **independentes**: `delta = f − shotStart`; `raw = start + delta`; `sampled = clamp(raw, 0, L − 1)`
  (`L = 0` ⇒ 0); `held = L = 0 ∨ sampled ≠ raw`. `sampled*` declarados são os do relógio (`…/sampledInstanceFrame`,
  `…/sampledCameraClipFrame`). Primário: `frame.instanceFrame`/`cameraClipFrame` são o `raw` (`/frame/instanceFrame`,
  `/frame/cameraClipFrame`). Vizinho: são o `sampled` (`/transition/other/frame/instanceFrame`, `…/cameraClipFrame`) e
  `heldInstance`/`heldCamera` são exatamente `held` (`/transition/heldInstance`, `/transition/heldCamera`), inclusive `false`.
- Tempos exatos: `FPS = {numerator, denominator}` inteiros em `[1, 2³²−1]`, `numerator ≤ 1000 × denominator` (a `Rate` do core;
  `/output/fps`). Tempo de um contador `n` é a razão **reduzida** `n × den / num`; `editTime`, `instanceTime` e `cameraClipTime`
  de **cada** deck são iguais a ela (razão reduzida de inteiros seguros, `/frame/…Time`, `/transition/other/frame/…Time`).
  Razão cujo valor não cabe em inteiro seguro não é aceita (subconjunto do `i64` do Rust).
- Entre decks: `kind`/`planVersion` iguais (`/transition/other/kind`, `/transition/other/planVersion`); `output` igual (largura,
  altura, `fps`, `alphaMode`, `colorSpace`; `/transition/other/output`); `bundleId` e `revisionId` iguais
  (`/transition/other/source`); `frame.editFrame` e `frame.editTime` iguais (`/transition/other/frame/editFrame`,
  `…/editTime`); `frame.shotId` do topo é o shot do primário (`/transition/primary`) e o do vizinho o outro
  (`/transition/other/frame/shotId`).
- Passes: o deck de cima pode ter passes de shot (do **seu** shot, como hoje) e de saída; o vizinho só de shot do seu
  próprio shot, qualquer `scope: output` ⇒ `/transition/other/fx/<i>/scope`; `clipId` único entre os dois decks ⇒
  `/transition/other/fx/<i>/clipId`.
- Recursos: cada deck mantém o seu `Map`; o mesmo `assetId` nos dois com `revisionId`, `contentHash`, `relativePath`, `format`,
  `widthPx`, `heightPx`, `alpha` ou `colorSpace` diferentes ⇒ `/transition/other/resources/<i>`. IDs de nós iguais entre
  decks são válidos, sem chave global de nó. A resolução contra slots carregados continua sendo de `resolveSceneResources`
  (fora da R0).
- `requires` do topo cobre o do vizinho e o seu; o do vizinho nunca traz `fxTransition` (`/transition/other/requires/<i>`);
  a igualdade exata com a união do core continua sendo regra do core, não é recalculada aqui.

### 5. Teto de itens e de bytes

- Itens: `nodes + layers + splats + fx + resources + lights` do topo e de `transition.other`, contados pelo **comprimento** de
  cada array próprio (ausente ou não array conta 0; a leitura tipada depois recusa o que não for array). `> 20000` ⇒
  `/transition` («the two decks hold N items…»); `= 20000` e estruturalmente válido ⇒ aceito. Isto roda antes de qualquer
  elemento ser lido e antes de qualquer recursão.
- Bytes: só em `parseTransitionPlanText`. Comprimento UTF-8 do texto recebido (somando 1/2/3 bytes por unidade e 4 por par
  substituto; um substituto solitário conta 3, como o `TextEncoder`), sem alocar o buffer e antes de `JSON.parse`;
  `> 4194304` ⇒ `invalid_plan` em `/transition`; `= 4194304` aceito. Texto que não é JSON (inclui o truncado) ⇒
  `invalid_plan` em `/` com «not JSON». Não é `JSON.stringify(objeto)`; o objeto não tem texto.
- Nenhuma entrada de transporte promete o teto na R0.

### 6. Interoperabilidade e arquivos tocados (registro prévio)

`crates/render/tests/scene_transition_parser.rs` constrói `ScenePlan`s reais (`Accept` só no teste, o fixture público
`valid-cut-instance-offsets`), escreve o JSON do core, e um helper Node (`runtime/test/directed-transition-interop.mjs`, sem
`.test.` no nome) os lê com `parseTransitionPlanText`/`parseTransitionPlan`; a saída é conferida pelo Rust contra o plano
tipado. Node é resolvido por `TESSEL_NODE` ou `PATH` (a mesma localização do renderer); **ausente é falha**, não skip. Sem `Cargo.*`.

Produção: `runtime/src/{directed-deck,directed-transition,directed-transition-fields,directed-transition-timing,
directed-transition-decks,directed-transition-limits}.js` (novos) e `runtime/src/directed-scene.js`. Testes:
`runtime/test/directed-transition*.test.mjs` (+ `directed-transition-fixtures.mjs`), uma pinagem em
`runtime/test/directed-scene.test.mjs`, `crates/render/tests/scene_transition_parser.rs`. Docs: este plano, relatório novo,
`fx-transitions.md`, `scene-plan.md`. Fora de ownership: tudo o que a seção «Ownership» já exclui, `docs/backlog.md`,
`docs/features/README.md` e `docs/README.md`.


## Refinamento do coordenador — arrays herdados (antes do merge)

A API de objeto recusa qualquer um dos seis campos de array herdado pelo protótipo,
mesmo se vazio: invalid_plan no próprio campo (`/resources` ou
`/transition/other/resources`, por exemplo). Sem isso, o contador pula o campo que
o leitor compartilhado ainda consome. Uma sonda em9c3a643 aceitou20.003 recursos
herdados no primário; o JSON textual não contém herança de protótipo. A guarda fica
no pré-check da transição, preservando a extração e o parser de produção sem transição.
Campos ausentes e não arrays continuam a ter a recusa de forma do leitor posterior.

A frase final sobre inteiros significa valores numéricos fracionários, não lexemas:
JSON.parse transforma12.0 e12e0 em Number12. A leitura JS aceita essas grafias quando
o valor é inteiro seguro; o leitor Rust textual pode recusá-las. Essa diferença já
estava declarada nas decisões iniciais e não é paridade lexical nem arredondamento.

Razões reduzidas são verificadas na leitura do próprio campo por MDC=1, antes da
comparação dos decks;0/1 é canônico e6/4 é recusado no campo que o declara.
