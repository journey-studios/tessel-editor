# P4b S2-R0 — relatório: leitor interno do plano de dois decks

Fatia **R0** de BL082.4.4 / F064 (`docs/features/fx-transitions.md`): um leitor JS **puro e interno** do wire C0b
(`ScenePlan.transition`), mais a recusa que faltava na entrada de produção. Contrato fixado antes do código em
[p4b-s2-r0-parser-plan.md](p4b-s2-r0-parser-plan.md). Windows nativo (Git Bash real, Node v26.12.1, rustc 1.99.1), base
`a61b7a9`, origem `journey-studios/tessel-editor`. **Isto não é aceite de produto nem fecha R/S2:** não há renderização, GL,
pixel, bridge, compositor, mudança de `hello`, flag nem ativação.

## O que ficou como estava (afirmado por teste, não por intenção)

- `parseDirectedLayer` (a entrada de produção) continua **recusando** todo plano com `transition`: com `fxTransition` em
  `requires` é `unsupported` com `details.missing: ["fxTransition"]` (como antes); **sem** ela é agora `invalid_plan` em
  `/requires` (a recusa que faltava). Planos sem a chave: mesmo resultado, mensagem e ordem.
- `SCENE_CAPABILITIES` (JS) = 13 nomes, sem `fxTransition`; `Capability` (Rust) = 14; admissão pública `Refuse`; F064 produto **P**.
- `runtime/dist/runtime.js` não contém o leitor (nada de produção o importa; só os próprios módulos e os testes).
- Intocados (diff contra `a61b7a9` só lista os arquivos abaixo): `compositor.js`, `assets.js`, `splat-stage.js`, bridge/worker,
  shaders, `output-fx.js`, `crates/core/src`, media, desktop, `render/src`, `Cargo.*`, `package.json`/lockfile, fixtures e
  goldens (`docs/migration/fixtures`: 0 arquivos no diff). Backlog, matriz e índice: só patches abaixo.

## Commits (por SHA)

| SHA | Conteúdo |
| --- | --- |
| `b4cd13a` | Contrato (plano) antes do código. |
| `7253fa5` | Extração **mecânica** do leitor de deck: `runtime/src/directed-deck.js` (novo) de `parseDirectedLayer`; `directed-scene.js` reexporta os nomes movidos. |
| `4fb2672` | Guarda: `Object.hasOwn(plan, 'transition')` sem `fxTransition` ⇒ `invalid_plan` em `/requires`, logo depois da conferência de `requires` e antes do corpo; uma pinagem em `directed-scene.test.mjs`. |
| `c9563bc` | Leitor de transição: `directed-transition{,-fields,-timing,-decks,-limits}.js`. |
| `e529482` | Testes JS e fixtures feitas à mão (`directed-transition{,-refusals,-limits}.test.mjs`, `directed-transition-fixtures.mjs`). |
| `b57304e` | Interoperabilidade: `directed-transition-interop.mjs` e `crates/render/tests/scene_transition_parser.rs`. |
| `ca234c1` | Dois mutantes sobreviventes da primeira bateria (M03, M34) ganham teste; M15b é equivalente. |
| (este) | Docs: este relatório, `fx-transitions.md`, `scene-plan.md`. Só docs; árvore de código idêntica a `ca234c1`. |

Todos os gates abaixo rodaram em `ca234c1` (a árvore de código do head é a dele).

## API

`runtime/src/directed-transition.js`:

- `parseTransitionPlan(plan, references)` — objeto (o que `JSON.parse` ou a bridge entregam).
- `parseTransitionPlanText(text, references)` — texto JSON: mede os bytes UTF-8 do texto **recebido** antes do `JSON.parse`;
  texto que não é JSON (truncado incluído) ⇒ `invalid_plan` com `details.pointer === ''` (a mensagem diz `Scene plan /`).
- `TRANSITION_CAPABILITIES` (= `SCENE_CAPABILITIES` + `fxTransition`), `MAX_TRANSITION_PLAN_ITEMS` (20 000),
  `MAX_TRANSITION_PLAN_BYTES` (4 194 304).
- Retorno: `{ primary, other, mix, window, timing, clocks, passes, references }`. `primary`/`other` têm a forma de
  `parseDirectedLayer` (`output, fx, camera, planes, splats, lights, resources`); `mix` = `id, effect, fromShotId, toShotId,
  primary, easing, curve, intensity, progress, amount`; `window` = `startFrame, durationFrames, frameInWindow, alignment,
  cutFrame, editFrame`; `clocks.{primary,other}.{instance,camera}` = `{raw, sampled, held}`; `passes` = `primary` (shot do
  primário), `other` (shot do vizinho), `output` (só do topo). Recusa = `RuntimeError` tipado (`invalid_plan` com
  `details.pointer` e mensagem `Scene plan <ponteiro>: …`; `unsupported`/`unsupported_plan_version` como a produção). Nada
  parcial é devolvido e a entrada nunca é escrita (os testes leem um plano congelado em profundidade e comparam o JSON depois).
- Ordem das recusas: plano/kind/versão, `transition` própria, tetos (antes de ler qualquer elemento), profundidade 1,
  parcialidade e capacidades do topo, campos e `timing` por chaves/tipos, corpo e identidade do topo, vizinho (cabeçalho,
  corpo, identidade) com `withinOther`, relações entre decks, relógios/tempos, passes, recursos.

## Extração

`directed-deck.js` recebeu, sem mudar o corpo, os helpers de campo, `splatMatrixFault`, `SPLAT_SIMILARITY_TOLERANCE`,
`MAX_POINT_LIGHTS`, `parseResourceReferences` e `readDeckBody(plan, requires)` (de `/output` a `/lights`, na ordem de
antes); `directed-scene.js` os reexporta e chama. Prova de preservação: uma bateria de controle de **4665 casos** (planos
válidos e todas as recusas da produção) com resultado serializado, idêntica byte a byte antes e depois (SHA-256 do controle
`172E8D40…B47027`); os 65 testes puros de `directed-scene*`/`output-fx`/luz da base seguiram verdes em cada commit.
`output-fx.js` **não** mudou: `parseOutputFx(raw, requires, frame)` já devolve uma lista em ordem de plano; o prefixo do
vizinho e a recusa de `scope: output` nele são de quem chama.

## Limites JS ≠ Rust (honestos)

- Inteiros do JS são `Number.isSafeInteger`; relações usam BigInt e relógios brutos fora de `2^53−1` são **recusados**, nunca
  arredondados. O Rust lê `u64`/`i64`: um plano com contadores acima de `2^53−1` é válido no core e recusado aqui.
- O leitor julga **coerência, não procedência**: um plano inteiramente consistente e forjado descreve outra cena. A igualdade
  exata de `requires` com a união do core não é recalculada (só a cobertura: o do topo cobre o do vizinho).
- `from_json(Value)` do core e `parseTransitionPlan(objeto)` não medem bytes; só o texto tem teto. A bridge não promete o
  teto de bytes na R0 (nenhuma entrada de transporte foi ligada).
- Não resolve slots, não abre arquivo, não verifica hash: a resolução de recursos continua sendo de
  `resolveSceneResources`/host (fora da R0).
- Nenhuma medição de custo/desempenho; o lock de GPU não foi tomado.

## Texto × objeto

| | `parseTransitionPlan` | `parseTransitionPlanText` |
| --- | --- | --- |
| Entrada | objeto | texto UTF-16 do JS (medido em UTF-8) |
| 4 194 304 bytes | não mede | `= 4194304` lido; `+1` ⇒ `/transition` antes do parse |
| 20 000 itens, profundidade 1 | sim | sim |
| Não-JSON/truncado | n/a | `invalid_plan`, ponteiro `''` |
| Chave com escape (`"\u0074ransition"`) | n/a | bytes do texto recebido (mais longo que o `JSON.stringify` do objeto) |

## Casos e mutantes

Fixtures JS feitas **à mão** com a aritmética em comentário (`directed-transition-fixtures.mjs`; 11 cenários: quadro 36 e 42,
C−1 e C, centro ímpar D5, `start`, `end`, D1, 30000/1001, câmera de 0 quadros e último quadro real; `raw 41 / sample 39`).
O resultado do leitor é comparado a literais, não a uma função do código sob teste. Contagens em `ca234c1`:
`directed-transition.test.mjs` 19, `…-refusals.test.mjs` 79, `…-limits.test.mjs` 10 (108 novos) + a pinagem da guarda; `scene_transition_parser.rs` 4.

Cobertura: wire antigo (recusado em `/transition/alignment`), cada uma das 17 chaves e dos 16 campos de `timing`
obrigatórios, chaves desconhecidas, tipos inválidos (float, bool, string, `null`, `NaN`, `±Infinity`, negativos, `2^53`,
array/objeto), nomes de protótipo (`constructor`, `toString`, `__proto__` por texto, campo só herdado ausente), outra versão/output/FPS/bundle
no vizinho, `scope: output` no vizinho, `clipId` duplicado, `assetId` conflitante (e ids de nó iguais **válidos**),
`primary` trocado (com os dois decks concordando, só o corte recusa), quadro no fim exclusivo do shot, corte/janela
adulterados, amostra/`held` falsos (`false` incluído), relógios ligados entre si, `Ratio` errado, instância curta no
próprio shot, 20 000 itens aceitos/+1 recusado, lights e neighbour contados, 4 MiB exatos/+1 em texto com 2/3/4 bytes e
substituto solitário, chave com escape, bomba aninhada em texto e em objeto (100 000 níveis), objetos cíclicos.

Interop com o core real (`scene_transition_parser.rs`, Node **obrigatório**; ausente é `expect` ⇒ falha, nunca skip):
12 quadros da janela × 3 conteúdos (`plain`, `sky`, `sky-and-splats`), comparados ao plano tipado (mistura, janela, relógios
brutos/amostrados/presos dos dois decks, contagens e recursos de cada deck); quadros fora da janela recusados em
`/transition`; os tetos iguais aos do core; **28 planos adulterados** recusados pelo core (`ScenePlan::from_json_str`) e
pelo leitor **no mesmo ponteiro e código**; o texto de exatamente o teto aceito/recusado pelos dois.

Mutantes (`agent-tools/mutate.mjs`, um por vez, bytes restaurados com hash conferido, mtime tocado, controle limpo depois de
cada um; suíte: os três arquivos novos + `directed-scene.test.mjs`, 117→118 testes). **45 mutantes**: 44 mortos, 1 equivalente.

| Mutante | Resultado |
| --- | --- |
| M01 guarda de `transition` sem `fxTransition` removida | morto (2) |
| M02 vizinho sem prefixo; M03 prefixo duas vezes; M04 deck de cima prefixado como vizinho | mortos (M03 só depois do teste unitário de `withinOther`, ver abaixo) |
| M05 profundidade removida | morto |
| M06 vizinho fora da contagem; M07 `lights` fora; M07b `fx` fora; M08 `>`→`>=` itens | mortos |
| M09 `>`→`>=` bytes; M10 bytes por `length`; M10b par substituto 3; M10c 2 bytes contado 1; M10d 3 bytes contado 2; M10e substituto solitário 1; M28 porta de texto sem medir | mortos |
| M11 progress sem +0,5; M12 sem easing; M13 amount × progress; M27 tolerância 1e-6 | mortos |
| M14 fim exclusivo aceito; M17 cobertura de shot omitida; M23 centro arredonda para cima; M29 `end` como `center`; M30 janela fora da edição | mortos |
| M15 vizinho com raw em vez de sample; M15c câmera do primário com sample; M24 câmera no relógio da instância | mortos |
| M16/M16b/M16c `held` falso/verdadeiro ignorado; M21 clipe sem quadros não preso; M22 último quadro real preso | mortos |
| M18 output no vizinho; M19 `clipId` duplicado; M20 asset conflitante; M32 output do vizinho; M33 versão do vizinho; M35 cobertura de `requires`; M36 `fxTransition` no vizinho | mortos |
| M25 `transition` herdada aceita; M26 razão não reduzida; M31 amostra não conferida; M34 primário sem checar o corte | mortos (M34 só depois do teste novo) |
| **M15b** primário guarda `sampled` em vez do `raw` da instância | **equivalente** (não contado como morto) |

Primeira bateria: 3 sobreviventes (M03, M15b, M34). M03 e M34 ganharam teste em `ca234c1` (um teste unitário de
`withinOther` sobre um erro já prefixado; os dois ids de shot concordando com o `primary` declarado, de modo que só o corte
recusa) e morreram; M15b é equivalente porque a cobertura do shot pela instância (M17, verificada antes) garante
`raw ∈ [0, L)` para o primário, então `raw` e `sampled` da instância são o mesmo número em qualquer plano aceito.
`directed-scene.test.mjs` mata M01 junto com o teste de produção.

## Gates (Windows nativo, `ca234c1`)

Executados:

- `./scripts/runtime setup` (pnpm frozen) — ok; `./scripts/runtime build` — ok (`dist/runtime.js` 4199 KiB, sem o leitor).
- Node puro: `directed-scene`, `directed-scene-light`, `directed-scene-splats`, `output-fx`, `fx-handles-overlay` e os três arquivos novos: **182 passam, 0 falham**.
- `cargo fmt --all -- --check` 0; `./scripts/cargo check --workspace --all-targets --locked` 0; `clippy --workspace --all-targets --locked -- -D warnings` 0;
  `test --workspace --no-run --locked` 0 (só compila); `build -p tessel-editor --locked` 0. Alvo mbx padrão, `--locked`, sem
  clean/gc/lanes/target-dir.
- `scene_transition_parser`: 4/4 (debug).
- Core, **debug e release**, `--locked`: `scene_fx_transition` 216 (inclui compat, public, reload, boundary/scan, timing,
  ceiling, corpus), `scene_plan_transition_baseline` 1, `pilot_dissolve_corpus` 10, `pilot_dissolve_representable_import` 5,
  `pilot_dissolve_representable_state` (X) 8: todos verdes nos dois perfis.

Lidos / não executados: `pnpm test` global (tem testes de browser que exigem Chrome/GL; nenhum teste de browser ou de pixel
foi rodado), `cargo test --workspace` completo (**não** declaro workspace verde: os dois gizmos Windows da base seguem sem
verde e não foram tocados nem executados aqui), o ramo "Node ausente" do teste de interop (só lido: `locate(...).expect(...)`
panica; não consegui tirar o Node da máquina sem alterar o ambiente — Program Files é um fallback), Mac, origem
(`ORIGIN_NOT_EXECUTED`), GPU/desempenho/lock, UI física, aceite humano. N/A: pixels, shader, bridge/worker, CLI/UI (nenhuma
superfície de edição mudou).

## Ownership

Arquivos novos/alterados (todos desta entrega, nenhum fora do ownership da R0): `runtime/src/directed-deck.js`,
`directed-transition*.js` (5), `runtime/src/directed-scene.js`, `runtime/test/directed-transition*` (4 + fixtures + interop),
`runtime/test/directed-scene.test.mjs` (uma pinagem), `crates/render/tests/scene_transition_parser.rs`, `docs/migration/p4b-s2-r0-parser-plan.md`,
este relatório, `docs/features/fx-transitions.md`, `docs/features/scene-plan.md`. Todos < 1000 linhas; nenhum arquivo acima de 1000 linhas cresceu.
`AGENTS.md`, `docs/backlog.md`, `docs/features/README.md` e `docs/README.md` não foram editados.

## Patches literais para o coordenador (IDs existentes: BL082.4.4, F064; sem ID novo)

1. `docs/backlog.md`, linha `BL082.4.4`: na coluna de estado, trocar `R pendente` por
   `R0 leitor interno JS integrado a validar; R restante pendente`; e **acrescentar ao fim da célula de notas**:

   `R0 (ca234c1): leitor puro do plano de dois decks (parseTransitionPlan/Text, runtime/src/directed-transition*.js), extração mecânica do leitor de deck com controle de 4665 casos idêntico, guarda de transition sem fxTransition em /requires; 108 testes JS novos, interop com o core real (12 quadros × 3 conteúdos e 28 planos adulterados no mesmo ponteiro), 45 mutantes: 44 mortos, M15b equivalente. Windows: runtime build, Node puro 182/0, fmt/check/clippy/no-run/build0, core 216/1/10/5/8 em debug e release. Produção inalterada: JS13/Rust14, Refuse, F064 P. Sem render/GL/pixel/bridge/hello/flag, sem Mac, sem workspace global, sem browser. [Relatório R0](migration/p4b-s2-r0-parser-report.md) por SHA.`

2. `docs/features/README.md`, linha `F064`: **acrescentar ao fim da célula de status**, antes do `|` final:
   `R0/ca234c1: leitor JS interno do plano de dois decks (puro, sem produto) e guarda de /requires; produção ainda recusa toda transição, JS13/Rust14, Refuse, F064 P. [Relatório R0](../migration/p4b-s2-r0-parser-report.md).`

3. `docs/README.md`, após o item `P4b S2-H`:
   `- [P4b S2-R0 — leitor interno do plano de dois decks](migration/p4b-s2-r0-parser-report.md): leitor JS puro do wire C0b com tetos, relógios e interop com o core; sem renderização, produção ainda recusa transições.`
