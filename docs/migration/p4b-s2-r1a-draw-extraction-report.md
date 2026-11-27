# P4b S2-R1a — relatório: extração mecânica do desenho dirigido

**Estado da integração:** produção e teste de capacidades em `e14dc514b8685bca12c52dc08f3cb72e0a0b1440`
validados no Mac nativo e aprovados por revisão independente Windows, sem bloqueio. O relato do autor abaixo pertence
à base `7d88545`; os gates da integração com `805a0c6` e a revisão estão nas seções finais. Produto P, Refuse fechado.

## Entrega original do implementador — histórico até 791b368

Fatia **R1a** de BL082.4.4 / F041 / F064. Contrato fixado antes do código em
[p4b-s2-r1a-draw-extraction-plan.md](p4b-s2-r1a-draw-extraction-plan.md) (commit `1bcb09c`). Windows nativo 10.0.26200,
checkout preparado `E:\www\tessel-p4b-s2r1-20261115` (sem clone/worktree novo), `origin` `journey-studios/tessel-editor`,
base `main` `7d88545320fd8466812c649a1877c6f0362493ae` limpa; Node v26.12.1, pnpm 10.34.9, rustc 1.99.1, Git Bash real, `mbx`
padrão; 157,8 GiB livres e 25,4 GiB de memória livre de 63,7 antes do trabalho pesado. **Isto não é aceite de produto nem
desenho de transição:** nenhum pixel de transição, shader, cache, orçamento, router ou capacidade mudou.

## O que mudou e o que não mudou

- `prepareDirectedScene` e `sceneMaterial` saíram de `runtime/src/compositor.js` para o novo
  `runtime/src/directed-deck-draw.js` (`prepareDirectedScene(host, layer, frame)` e `sceneMaterial(host, order, alpha, index,
  texture, keep)`). A classe `Compositor` mantém os dois métodos, com as mesmas assinaturas, como wrappers de uma linha
  (`return prepareDirectedScene(this, layer, frame);` — o método continua `async`; `return sceneMaterial(this, order, ...)`).
  `materialOf` continua chamando `host.sceneMaterial(...)`, ou seja, o wrapper, pelo mesmo caminho de antes.
- Planos simples seguem pelo helper e desenham a mesma imagem. Planos com transição continuam recusados por
  `parseDirectedLayer` (nenhum código de transição foi tocado nem acrescentado). `SCENE_CAPABILITIES` segue **13** e sem
  `fxTransition`; Rust segue **14**; admissão pública `Refuse`; F064 produto **P**.
- Intocados na entrega original (`git diff --name-only 7d88545 791b368` lista só os arquivos de "Ownership"): `crates/*`, `Cargo.*`, `runtime/package.json`
  e lock, `assets.js`, `splat-stage.js`, `retention.js`, `directed-scene.js`, `directed-transition*.js`, `output-fx.js`,
  bridge/worker, shaders, fixtures, goldens, baselines, dados S0/X.
- O único import removido de `compositor.js` é o bloco inteiro de `./directed-scene.js` (os oito nomes só eram usados nos
  corpos movidos), a exceção mecânica prevista. `imageTexture` (assets.js) e `RuntimeError` (protocol.js) continuam em uso no
  resto de `compositor.js` e são importados de novo no módulo novo. O módulo novo não importa `compositor.js` e não
  há ciclo (importa `assets.js`, `directed-scene.js`, `protocol.js`); não tem estado global.

## Commits (por SHA)

| SHA | Conteúdo |
| --- | --- |
| `1bcb09c` | Plano R1a completo, antes de qualquer mudança de código. |
| `02f16a3` | **Extração mecânica**: `directed-deck-draw.js` (novo) e wrappers/import em `compositor.js`. Nada além disso. |
| `bdd38bd` | Cinco controles de borda pelo runtime real (`runtime/test/directed-scene-draw-seam.test.mjs`); passam na base `7d88545` e depois da extração. |
| docs finais | Este relatório e as notas de F041/F064 (só docs). A árvore de código é a de `bdd38bd`. |

Os commits do harness chegaram com um trailer de coautoria; foram saneados antes de qualquer push, com a mesma árvore
(`git rev-parse HEAD^{tree}` igual antes/depois) e sem force-push nem reescrita de commit publicado.

## Comparação de fonte: corpos antigos × novos

Base `7d88545` (`compositor.js` blob `339dc28c74bb…`, 1166 linhas) contra `02f16a3` (`compositor.js` blob `e27206580895…`,
**1054 linhas**; `directed-deck-draw.js` blob `2c4350ccf10c…`, **133 linhas**).

| | corpo antigo (linhas dentro das chaves) | corpo novo | SHA-256 do corpo normalizado (antigo = novo) |
| --- | --- | --- | --- |
| `prepareDirectedScene` | 88 | 88 | `0fb5936fdbfe9bd28e0677453c7e305cabd3886397ae8df940c00bcc4600fbd3` |
| `sceneMaterial` | 12 | 12 | `c143d7c93dd116aa82faf5eddfaeaa6fbf19bfad805af891f39f3b91632edd9c` |

Normalização **única**: tirar dois espaços de indentação do corpo antigo e trocar `this.<nome>` por `host.<nome>`; nada mais
(comentários, ordem, expressões e `await` são comparados byte a byte). Resultado: **idêntico nos dois**, exit 0. Diferenças
declaradas, e só estas: (1) a assinatura (`async prepareDirectedScene(layer, frame) {` → `export async function
prepareDirectedScene(host, layer, frame) {`; idem `sceneMaterial`) e a chave de fechamento; (2) indentação; (3) `this.` →
`host.` em 16 acessos de `prepareDirectedScene` (`sceneTarget` ×2, `retain`, `sceneMaterial`, `limits` ×3, `splatStage`,
`renderer` ×2, `discardFailedStage`, `fxPipeline`, `counters` ×3, `fx`, `dropFxPipeline`) e 5 de `sceneMaterial` (`retain`,
`materials` ×2, `counters` ×2); as duas ocorrências restantes de "this" no corpo antigo são a palavra "this" de comentários
("for this frame only", "for this frame"), mantidas; (4) o comentário de documentação do método foi para o módulo, e o do
wrapper é de uma linha; (5) o bloco de imports. Diff de `compositor.js` na extração: as quatro únicas linhas adicionadas são
o import (`import { prepareDirectedScene, sceneMaterial } from './directed-deck-draw.js';`), o comentário de uma linha e os dois
`return`. Delta de linhas do compositor: **−112** (exigido ≤ 0); o módulo novo está bem abaixo de 1000. A comparação é de
texto: **não** prova pixels (os controles abaixo são o que prova) e **não** compara stacks, que passam a apontar para o módulo
novo.

Reprodução (a partir da raiz; `git show` lê a base):

```js
// node compare.mjs — normalised bodies of the two methods, base compositor.js × directed-deck-draw.js
import { execFileSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import fs from 'node:fs';
const old = execFileSync('git', ['show', '7d88545320fd8466812c649a1877c6f0362493ae:runtime/src/compositor.js'], { encoding: 'utf8' }).split('\n');
const neu = fs.readFileSync('runtime/src/directed-deck-draw.js', 'utf8').split('\n');
const body = (lines, start, indent) => {
  const s = lines.findIndex((l) => l === start);
  return lines.slice(s + 1, lines.findIndex((l, i) => i > s && l === `${indent}}`));
};
const norm = (lines, dedent) => lines.map((l) => (dedent ? l.replace(/^ {2}/, '') : l).replace(/\bthis\.(?=[A-Za-z_])/g, 'host.')).join('\n');
for (const [name, a, b] of [
  ['prepareDirectedScene', '  async prepareDirectedScene(layer, frame) {', 'export async function prepareDirectedScene(host, layer, frame) {'],
  ['sceneMaterial', '  sceneMaterial(order, alpha, index, texture, keep) {', 'export function sceneMaterial(host, order, alpha, index, texture, keep) {'],
]) {
  const x = norm(body(old, a, '  '), true);
  const y = norm(body(neu, b, ''), false);
  console.log(name, x === y, createHash('sha256').update(x).digest('hex'));
}
```

Bundle: `runtime/dist/runtime.js` (ignorado pelo Git) vai de 4 300 037 B (SHA-256 `23182C11…5CBF1`, base) para 4 299 867 B
(`646D1A9E…7228C`, extração); a diferença de 170 B é a do wrapper e da indentação, e o build é determinístico (o mesmo hash
saiu em todos os rebuilds medidos).

## O que a extração preserva (e por qual prova)

Ordem do preflight (`parseDirectedLayer` → `resolveSceneResources` → registro `used` → abort → `containSize` → alvo `ds:` →
`buildDirectedScene` → teto de splats → leases com abort a cada um → `buildSplatScenes` → estágio → desenho → FX do shot e da
saída → telemetria), recusa antes/depois de cada ação, chaves `ds:<order>`, `ds:<order>:<alpha>:<index>`, `dsfx:<order>:<slot>`,
MSAA final, depth, contadores, textura devolvida (`{ texture, width, height, fit: false, flipV: true }`), campos de telemetria,
posse e momento do descarte (tudo em `frame.keep` até o fim do frame, inclusive leases; nenhum `finally` novo; nada liberado entre
decks), `discardFailedStage`/`dropFxPipeline`: **corpo idêntico pela comparação acima**; comportamento observado pelos
controles existentes (Node/Chrome abaixo) e pelos cinco novos.

### Controles novos (bdd38bd) — porque os existentes não discriminavam a borda

Antes de escrever teste, rodei uma bateria de **19 mutantes** do módulo novo (um por propriedade que o contrato manda
preservar) contra os controles **existentes** (10 arquivos, 112 testes: os quatro browser dirigidos, `retention-browser`,
`directed-scene{,-splats,-light}`, `output-fx`, `splat-lineages`). 8 morreram e **11 sobreviveram**: a recusa de abort do
preflight, o registro `used`, a ausência do campo de splats, as chaves `ds:`/`dsfx:`, a contagem de reuso de material, o descarte
do material por-frame e o MSAA do alvo não tinham controle. Os cinco testes de `directed-scene-draw-seam.test.mjs` (Chrome
real, os métodos do próprio compositor embrulhados; **não** conhecem o helper) fixam essas bordas e **passam na base
`7d88545`** (checkout temporário de `compositor.js` da base, build, 5/5; depois restaurado) e na extração (5/5):

1. alvos `ds:0` (tamanho, sem argumento de MSAA/depth ⇒ MSAA final com depth), `dsfx:0:0`/`dsfx:0:1` com `0`/`false`, ordem
   alvo → material → pipeline → alvos de passe; duas camadas dirigidas independentes no quadro = `ds:0`, `ds:1`, ordem dos materiais e
   `directed_scene_layers` = 2; tamanho = ajuste `contain` da saída do plano (160×45 para 32:9);
2. campos de telemetria ausentes sem splat/FX/shot e `directed_scene_fx` = 1 só com FX;
3. com retenção: material guardado sob `ds:0:opaque:0`, reusado e contado (`material_reused` +2 = plano + composição) e não
   descartado no fim do frame; sem retenção: o frame constrói e **descarta** o que construiu e nada fica no cache;
4. quadro superado **antes** da construção responde `cancelled` (`The frame was superseded`) sem pedir alvo/material/passe;
5. slot trocado depois de preparar o plano cancela o quadro (`details.asset`) e o quadro ainda descarta o que construiu.

| Mutante (módulo novo) | Só controles existentes | Com os cinco novos |
| --- | --- | --- |
| M01 recusa de abort do preflight removida | sobreviveu | **morto** |
| M02 teto de splats depois das leases | morto | — |
| M03 lease sem `keep` | morto | — |
| M04 lease liberada logo após tomada | morto | — |
| M05 `used.set` removido | sobreviveu | **morto** |
| M06 telemetria `fx_shot` removida | morto | — |
| M07 `directed_scene_splats` sempre escrito | sobreviveu | **morto** |
| M08 `cached.result = null` removido | sobreviveu | sobrevive (sem efeito observável: o espaço `ds:` nunca recebe um `result`; **equivalente por leitura, não por prova**) |
| M09 `discardFailedStage` removido | sobreviveu | sobrevive (precisa de falha de desenho de splat) |
| M10 `dropFxPipeline` removido | morto | — |
| M11 chave `ds:` trocada | sobreviveu | **morto** |
| M12 chave `dsfx:` trocada | sobreviveu | **morto** |
| M13 `material_reused` sem contar | sobreviveu | **morto** |
| M14 `material.map` sem atribuir | morto | — |
| M15 textura sempre por-frame | morto | — |
| M16 material por-frame sem `keep` | sobreviveu | **morto** |
| M17 alvo em MSAA de preview | sobreviveu | **morto** |
| M18 `directed_scene_layers` removido | morto | — |
| M19 abort depois de cada lease removido | sobreviveu | sobrevive (precisa de plano com vários splats e cancelamento entre leases) |

Resultado: 19 mutantes, 16 mortos, 3 sobreviventes (M08 equivalente por leitura; M09 e M19 são lacunas de controle do caminho
de splat, **anteriores à extração**, registradas para o coordenador/R1b; não foram cobertas aqui por exigirem fixtures de splat
que não existem para essa borda). É a bateria desta fatia, não soma de outras fases; cada mutante rodou com o bundle
reconstruído e o arquivo restaurado ao final (`git status` limpo; hash do bundle voltou a `646D1A9E…`).

## Gates executados (Windows nativo)

Cargo sempre por `./scripts/cargo` (Git Bash real, `mbx` padrão), `--locked`, sem `CARGO_TARGET_DIR`/target-dir/lane/clean/gc.
Runtime: `./scripts/runtime setup` (pnpm frozen + build oficial) **antes** de qualquer teste de browser; build oficial repetido
(`./scripts/runtime build`, exit 0) antes da rodada final de Node. `TESSEL_REQUIRE_RUNTIME=1` em todo teste que usa Chrome
(skipped 0 em todas as execuções; nenhum skip contado como verde).

| Gate | Head do código | Resultado |
| --- | --- | --- |
| `./scripts/runtime setup` | `02f16a3` | exit 0; `runtime.js` 4199 KiB |
| Node, **misto**: `directed-scene`, `-light`, `-splats`, `output-fx`, `fx-handles-overlay` (5 arquivos; inclui Chrome) | `bdd38bd` | 74 passam, 0 falham, 0 skipped |
| Node, **puro**: `directed-transition{,-refusals,-limits,-boundaries}` (4 arquivos) | `bdd38bd` | 121 passam (120 + 1) |
| Node, os nove arquivos juntos (contagem de base: 195) | `bdd38bd` | **195/195**, 0 falhas, 0 skipped |
| Node, browser Chrome real: `directed-scene-browser`, `-fade-`, `-fx-`, `-grain-browser`, `retention-browser` (retain on/off) | `02f16a3` | 40/40 |
| idem + `directed-scene-draw-seam` (5 novos) + `splat-lineages`, `scene-warm`, `preview-sort` (leases/estágio) | `bdd38bd` | **71/71**, 0 skipped |
| `cargo fmt --all -- --check` | `02f16a3` | exit 0 |
| `check --workspace --all-targets --locked` | `02f16a3` | exit 0 |
| `clippy --workspace --all-targets --locked -- -D warnings` | `02f16a3` | exit 0 |
| `test --workspace --no-run --locked` | `02f16a3` | exit 0 (só compilou) |
| `build -p tessel-editor --locked` | `02f16a3` | exit 0 |
| core isolado `TESSEL_EXPECT_PRESERVE_ORDER=0`: `scene_fx_transition` 216, `pilot_dissolve_corpus` 10, `_representable_import` 5, `_representable_state` 8, `scene_plan_transition_baseline` 1 | `02f16a3` | todos verdes |
| mesmo conjunto com `--features serde_json/preserve_order`, `EXPECT=1` | `02f16a3` | 216/10/5/8/1 verdes |
| render, runtime real: `scene_transition_parser` (interop) 5, `scene_fx` 8, `scene_fx_gesture_frames` 1, `scene_fx_handles` 2, `scene_fx_shot_scope` 8, `scene_light_frames` 3, `scene_splat_frames` 9 | `02f16a3` | 36 passam |
| render: `scene_frames` | `02f16a3` | 7 passam, **1 falha pré-existente** (abaixo) |

O Rust foi executado em `02f16a3`; `bdd38bd` só acrescenta um arquivo de teste JS e não toca nada que o Cargo compila (o `dist` do
runtime, lido do disco pelos testes de render, tem o mesmo hash `646D1A9E…` em ambos).

### Falha histórica do teste de capacidades — corrigida na integração (BL168.11)

`crates/render/tests/scene_frames.rs::the_runtime_advertises_every_capability_the_core_can_ask_for_and_nothing_it_cannot_parse`
**falhava na base `7d88545` e na candidata original do mesmo jeito** (verificado com `compositor.js` da base e bundle da base, hash
`23182C11…`): o runtime anuncia as 13 capacidades e o teste exige `Capability::ALL` do core (14, com `FxTransition`). É o
descasamento JS13/Rust14 que o contrato mantém de propósito; o teste não foi atualizado quando o core ganhou o 14º nome.
O implementador não alterou o teste nem a lista na entrega original (decisão 8 do plano).
Na integração, o coordenador corrigiu apenas o teste em `e14dc51` (BL168.11), sem alterar
anúncio ou negociação: 8/8 no Mac e Windows. Não é regressão da extração nem dívida atual do teste.

### Lido / não executado / N/A

- Não executados: `pnpm test` global do runtime, `cargo test --workspace` completo e core completo/ledger/stress/áudio (o
  `core/src` está intocado; **não declaro workspace verde**; os dois gizmos Windows da base e o `shasum` de BL167.3 seguem
  separados, não tocados), `scene_export*` de render (BL167.3, fora do escopo), Mac, origem (`ORIGIN_NOT_EXECUTED`),
  benchmark/FPS/GPU física/lock de desempenho, áudio ouvido, UI física, aceite humano, pixel de transição.
- N/A: CLI/UI/lib (nenhuma superfície de edição mudou); shader/bridge/hello/capacidades (intocados, JS13/Rust14).
- Chrome/SwiftShader é evidência funcional do **caminho antigo** de planos simples, não de transição.
- Prova nativa Mac e revisão Windows independente são do coordenador; nada aqui herda a prova R0.

## Ownership da entrega original

Novos: `runtime/src/directed-deck-draw.js`, `runtime/test/directed-scene-draw-seam.test.mjs`,
`docs/migration/p4b-s2-r1a-draw-extraction-{plan,report}.md`. Alterados: `runtime/src/compositor.js` (−112 linhas, 1166 → 1054),
`docs/features/scene-renderer.md`, `docs/features/fx-transitions.md`. `AGENTS.md`, `docs/backlog.md`,
`docs/features/README.md` e `docs/README.md` **não** foram editados pelo implementador até `791b368`.
O coordenador os atualizou na integração, junto do teste `crates/render/tests/scene_frames.rs`.
Nenhum ID novo. O compositor integrado continua com delta −112 contra a main `805a0c6` (1192→1080).

## Patches originais — histórico, consolidados na integração (IDs existentes)

1. `docs/backlog.md`, linha `BL082.4.4`: na coluna de estado, trocar `desenho/cache/lifecycle R e S2A pendentes` por
   `R1a extração mecânica do desenho entregue a validar; desenho/cache/lifecycle R1b/c e S2A pendentes`; e **acrescentar ao
   fim da célula de notas** (antes do `|` final):

   `R1a (código 02f16a3, testes bdd38bd): prepareDirectedScene/sceneMaterial movidos mecanicamente para runtime/src/directed-deck-draw.js, wrappers no Compositor (compositor.js 1166→1054, −112); corpos comparados por SHA-256 normalizado (this→host/indentação): idênticos. Cinco controles novos de borda (chaves ds:/dsfx:, MSAA, telemetria, materiais, cancel/replaced) passam na base 7d88545 e na extração; 19 mutantes do módulo: 16 mortos, 3 sobreviventes (M08 equivalente por leitura; M09/M19 lacunas de splat preexistentes). Windows: runtime setup+build, Node 195/195 e 71/71 browser (TESSEL_REQUIRE_RUNTIME=1, skip 0), fmt/check/clippy/no-run/build 0, core 216/10/5/8/1 EXPECT0 e preserve_order EXPECT1, interop 5, render de FX/luz/splat 36 verdes. Observação preexistente: render scene_frames (capacidades) falha igual na base por JS13≠Rust14, não corrigida. Sem pixel de transição, shader, cache, router, capacidades; JS13/Rust14, Refuse, F064 P; sem Mac/workspace global/GPU/origem. [Relatório R1a](migration/p4b-s2-r1a-draw-extraction-report.md).`

2. `docs/features/README.md`, linha `F041`: **acrescentar ao fim da célula de status**, antes do `|` final:
   `R1a/02f16a3: o desenho do plano dirigido vive em runtime/src/directed-deck-draw.js (extração mecânica, mesmo comportamento; compositor.js −112 linhas); sem mudança de produto. [Relatório R1a](../migration/p4b-s2-r1a-draw-extraction-report.md).`

3. `docs/features/README.md`, linha `F064`: **acrescentar ao fim da célula de status**, antes do `|` final:
   `R1a/02f16a3: extração mecânica do desenho dirigido atual (sem desenho de transição, sem cache/lifecycle novo); produção ainda recusa toda transição, JS13/Rust14, Refuse, produto P. [Relatório R1a](../migration/p4b-s2-r1a-draw-extraction-report.md).`

4. `docs/README.md`, após o item `P4b S2-R0`:
   `- [P4b S2-R1a — extração do desenho dirigido](migration/p4b-s2-r1a-draw-extraction-report.md): prepareDirectedScene/sceneMaterial em módulo próprio, comportamento idêntico e controles de borda; sem desenho de transição.`

## Limites

R1a não resolve nada do que a R0 deixou (limites de accessors/Proxy/toJSON e diferenças JS/core seguem em BL082.4.4) e não
antecipa R1b/c: nada de dois decks, `from/to` em chaves de material, liberação de lease entre decks, cache T31, orçamento de
1 GiB, nove alvos, capacidade 14 ou router. A tolerância dos controles de pixel existentes não mudou e nenhum golden, fixture ou
baseline foi regenerado.

## Integração do coordenador com main805a0c6

PR77: merge `7086105` tem pais `791b368` e `805a0c6`. O único conflito era de imports em compositor.js:
foram preservados directed-deck-draw, LINE_PITCH_ROWS e grading. Os corpos de prepareDirectedScene
e sceneMaterial na main805a0c6 são idênticos aos do helper após a normalização acima, com os
mesmos hashes `0fb5936f…` / `c143d7c9…`. O compositor da main tem 1192 linhas e o integrado 1080 (−112).
As mudanças da main em títulos/grading e core são integração, não produção nova da extração.

BL168.11, já no backlog, corrige o contrato do teste scene_frames: exige o conjunto exato de
Capability::ALL menos FxTransition reservada. Não altera runtime, anúncio, guardas nem a lista
do core. A falha histórica 7/8 da entrega original permanece registrada acima; os gates deste
ajuste e do merge são registrados separadamente abaixo. Revisão Windows independente aprovada em e14dc51.
As seções anteriores de scene_frames, ownership e patches descrevem a entrega original, não o head integrado.


## Mac nativo do coordenador — head integrado e14dc51

macOS 26.6.2 arm64, Node 22.23.3, Rust 1.99.1, clone de validação limpo em
`/Volumes/Extreme Pro/www/tessel-pr63-mac-20261114`, origin `journey-studios/tessel-editor`.
Fetch normal e HEAD destacado em `e14dc514b8685bca12c52dc08f3cb72e0a0b1440` antes dos gates.
Disco inspecionado: externo ~1 TiB livre, interno ~74 GiB. Logs/manifesto de SHA-256 em
`~/.cache/tessel-pr77-mac-20261115` (poucas centenas de KB). Nenhuma mutação nesta rodada.

| Gate executado neste head | Resultado |
| --- | --- |
| Setup oficial e build do runtime antes de browser | exit 0 |
| Nove arquivos Node da entrega (grupo misto, inclui browser) | 195/195, falhas/cancelados/skips 0 |
| Browser: dirigido/fade/FX/grain, retenção, seam, splat-lineages, warm, preview-sort, grading e títulos acentuados | 74/74, falhas/cancelados/skips 0 |
| `fmt --all -- --check` | exit 0 |
| `check --workspace --all-targets --locked` | exit 0 |
| `clippy --workspace --all-targets --locked -- -D warnings` | exit 0 |
| Core isolado EXPECT=0: transições/corpus/X import/X estado/baseline | 216/10/5/8/1, todos passam |
| Mesmo conjunto com preserve_order EXPECT=1 | 216/10/5/8/1, todos passam |
| Render com TESSEL_REQUIRE_RUNTIME=1, serial | 44/44: scene_frames8, FX8, gesto1, handles2, shot_scope8, luz3, splat9, interop5 |
| `test --workspace --locked --no-run` | exit 0, só compilou |
| `build -p tessel-editor --locked` | exit 0 |
| `cargo tree -p tessel-editor-core` | 0 ocorrências de gpui/wry |

`scene_frames` inclui a igualdade exata das capacidades com a exceção reservada de BL168.11:
8/8 neste head. Não se transfere o 7/8 histórico do autor nem se altera o runtime para passar.
O merge preserva grading/layout da main; seus testes de browser foram executados junto da seam.
As contagens dos grupos Node são separadas e podem se sobrepor; não são soma de testes únicos.
Check/clippy/no-run/build usam o cache compartilhado mbx e não alegam recompilação integral.
Nenhum CARGO_TARGET_DIR, lane, clean ou gc manual. O wrapper pode coletar seu cache automaticamente.

Não executados: workspace global, core completo/gizmos da base Windows, release, áudio, UI física,
GPU física, benchmark/FPS, origem ou pixels de transição. Chrome/SwiftShader e os oráculos antigos
provam o caminho simples existente. M08/M09/M19 da bateria do autor continuam com o alcance declarado;
não houve repetição dos 19 mutantes no Mac. Revisão Windows independente do head e14dc51 aprovada:
[PR77 R1 Windows: extração integrada, grading e capacidades reservadas](t3-thread://v2/33595e42-547d-42b7-92a7-cb4acb430dd7/mcp:launch:08c519672eae1c909da50fe56f00002ad1bc3f9513c540288d167e7266aaed01).

A consolidação posterior deste resultado toca só docs; produção, testes, locks e dados permanecem
idênticos a e14dc51. A aprovação da extração não implementa R1b/c nem abre o portão.


## Revisão independente Windows R1 — e14dc51, aprovada sem bloqueio

Revisão executada por Cursor Sonnet 5.5/defaults, sem subdelegação, no checkout preparado
`E:\www\tessel-p4b-s2r1-20261115`, origin `journey-studios/tessel-editor`. Fetch e FF normais
791b368→e14dc51, árvore limpa no fim. Código de e63e705 e desta consolidação é idêntico ao
revisado; o revisor leu apenas parte dos docs de e63e705. Correções de relato são do coordenador.

O revisor confirmou byte a byte os dois corpos contra a main805a0c6, os hashes normalizados,
o delta1192→1080, imports sem ciclo e a preservação integral de grading/layout. Os cinco testes
seam passam também com o compositor da main805a0c6; são controles do comportamento anterior.

| Executado independentemente no Windows, e14dc51 | Resultado |
| --- | --- |
| Runtime setup/build oficial, TESSEL_REQUIRE_RUNTIME=1 | exit 0, bundle SHA prefix54D71677 |
| Nove arquivos Node | 195/195:121 puros+74 mistos, nenhum skip |
| Browser dirigido/retention/seam/splat/warm/preview-sort | 71/71 |
| Grading e títulos acentuados da integração | 3/3 |
| fmt/check workspace all-targets/clippy-Dwarnings/no-run/build desktop | exit 0 |
| Core transições/corpus/X import/X estado/baseline, EXPECT0 e preserve EXPECT1 | 216/10/5/8/1 nos dois perfis |
| tree core | sem gpui/wry |
| Render dirigido: frames/interop/FX/gesto/handles/shot_scope/luz/splat | 44/44 (8/5/8/1/2/8/3/9) |
| Render da integração main: grading/accented_titles/title_layout | 9/9 (2/3/4) |

BL168.11: o teste `the_runtime_advertises_every_drawable_capability_and_keeps_transitions_reserved`
mantém parse estrito e igualdade exata dos vetores ordenados. Quatro mutantes C01–C04 morrem:
anunciar fxTransition cedo, omitir fxShotScope, duplicar camera e omitir camera. Não é teste de
subconjunto e não aceita lista vazia, duplicata, capacidade desconhecida ou ativação prematura.

### Bateria própria do revisor, separada dos 19 do autor

| Resultado | Quantidade | Alcance |
| --- | --- | --- |
| Mortos pelos testes existentes | 12 | H05/H06/H07/H08/H09/H12/H14/H15/H16/H19/H20 por asserção; H17 por travamento |
| Mortos só pelos probes descartáveis do revisor | 6 | H01/H02/H03/H04/H10/H13 |
| Mortos pelo teste real de capacidades | 4 | C01–C04, por asserção |
| Equivalentes por leitura | 2 | H11 abort antes de registrar used; H18 wrapper sem async (helper já async) |

Total24:21 por asserção,1 por travamento,2 equivalentes por leitura. H17 (wrapper sem keep)
travou a suíte; o revisor encerrou só a própria árvore de processos. Não conta como sumário
limpo nem falha por asserção. Após cada restauração: bytes/hash conferidos, mtime atualizado,
bundle recompilado. Controle final: Node195, browser+probes80 e scene_frames8 passam.

Os seis probes ficam em `C:\Users\wally\tessel-review77\probe.test.mjs` (logs1,3MB no mesmo
diretório persistente), passam no código novo e no compositor da main805a0c6. Registrados em
BL082.4.4 para adoção como regressões na R1b; ainda não entraram nos testes commitados:

- P1: releaseSplatMesh antes de lease.release.
- P2/P3: abort entre leases e descarte do estágio após falha (M19/M09 do autor).
- P4: invalidação de cached.result (M08) com estado injetado. Equivalente no produto atual,
  onde ds: não recebe result; o probe não prova um caso produtivo alcançável.
- P5: três passes de FX consultam cada alvo uma vez por slot.
- P6: descarte da textura temporária sem retenção.

As lacunas são anteriores à extração; identidade de corpo é a evidência de preservação.
O coordenador arquivou o parecer completo no cache local; o arquivo-fonte dos probes continua
no caminho Windows do revisor, sem afirmar cópia local ou adoção já executada.

Uma execução acidental da suíte runtime completa no Windows deu546 testes,545 passaram e1 skip
Unix. Está em node-FULL-runtime-suite-accidental.log; é relato extra e não um gate545/545 sem skip.
Não houve workspace global, core completo/gizmos, ledger/stress, áudio, scene_export, origem,
GPU física/FPS nem pixel de transição. Sem mudança de tolerância, corpus ou golden.

Ressalvas de docs integradas: estado não confunde merge da main na branch com PR já incorporada;
scene_frames/ownership originais estão rotulados como história; BL168.11 nomeia o teste novo;
F041 registra1192→1080 da integração. F064 continua P em todas as superfícies, Refuse fechado.
