# F059 — O terceiro look: `filmGrain` (BL082.1, P3)

## Objetivo e limites

`filmGrain` é o terceiro efeito do catálogo de F016 (escopo de saída): ruído triangular de filme, **reseminado a
cada quadro**, somado à imagem. Fatia completa de ponta a ponta: catálogo (rótulos pt-br/en/es), inspector (F057),
chaves e seed ([F058](fx-keys.md)), plano com seed já resolvida, desenho no runtime, monitor e export. É **3 dos 59**
efeitos do piloto: os outros 56 não foram portados e não há controle habilitado para eles.

Fora desta entrega: escopo `shot`/`node` e transições (P4), os 56 demais efeitos, GPU real, Windows e ponteiro físico.

Fonte (somente leitura, snapshot `d40ac3f7c0`): `fx/catalog/filmGrain.ts`, `runtime/fx/shaders/filmGrain.ts` e os
chunks `hash.ts`/`color.ts`, `fx/fx-hash.ts`, `fx/resolve-fx-frame-hashes.spec.ts`; blob SHA de cada fonte em
`docs/migration/fixtures/pilot-fx/grain-manifest.json` (14 fontes). A origem **não foi executada**.

## Contrato do core / biblioteca

### Catálogo

| Parâmetro | Tipo | Faixa | Padrão | Passo |
|---|---|---|---|---|
| `amount` | número | 0–1 | 0,25 | 0,01 |
| `size` | número (px a 720p) | 0,5–4 | 1,4 | 0,1 |
| `luma` | número | 0–1 | 0,7 | 0,01 |
| `colored` | **booleano** | — | falso | — |
| `intensity` | número comum | 0–1 | 1 | 0,01 |

Família `look`, `typical_duration_frames` 96, `reseed` por quadro, precisa de seed. Entra na oferta porque desenho,
inspector, plano e provas existem em todas as superfícies.

### Seed e hash inteiro (`directed_scene::fx::hash`)

- Base = seed explícita ou FNV-1a (`0x811C9DC5`, `0x01000193`) dos **bytes UTF-8** do ID do clipe.
- Seed do quadro = `lowbias32(base ^ frameNoClipe)` (`uint32` com wrap). O quadro é local ao clipe.
- `lowbias32`: `h ^= h>>16; h *= 0x7feb352d; h ^= h>>15; h *= 0x846ca68b; h ^= h>>16`.
- `hashPixel(seed, x, y) = lowbias32(lowbias32(seed ^ 0x9e3779b9) + x + (y << 16))`.
- `unitFloat(h) = (h >> 8) / 2^24`; ruído triangular = `unitFloat(h) + unitFloat(lowbias32(h ^ 0x68bc21eb)) - 1`.
  `colored` usa `h`, `lowbias32(h + 0x632be5ab)` e `lowbias32(h + 0x85157af5)` nos três canais.
- Rust usa `wrapping_*`, o GLSL usa `uint`; nunca float, `sin` ou relógio. Os vetores inteiros são **iguais bit a bit**
  entre Rust, o corpus independente em Python e o GLSL.

### Pixels

- Célula = `max(size × altura/720, 1)` em double, aplicada **uma vez** (o JS recebe o passe pronto); `gl_FragCoord`
  conta de baixo para cima, então a referência CPU inverte a linha explicitamente.
- O ruído soma no espaço **sRGB codificado**: `grained = clamp(c + ruído × (amount × 0,25 × resposta))`, com
  `resposta = mix(1, 1 - smoothstep(0,05; 0,95; luma), luma_param)` e luma `0,2126 R + 0,7152 G + 0,0722 B`; a mistura
  com a origem pelo `amount` do passe (`intensity × weight`) é em **luz linear**.
- `amount × intensity × weight = 0` devolve os bytes de entrada **exatamente** (identidade, sem ida e volta).
- O alpha da cena é preservado (o piloto escreve 1; divergência declarada).

### Plano e capacidade

O core resolve chaves, peso e seed; o passe do plano carrega `params` já amostrados e `seed` (`uint32`). A nova
`Capability::FxNoise` (`"fxNoise"`) é derivada **só** de passes com seed (os bytes dos planos de vinheta/fade não mudam).
Um runtime P2 sem ela é recusado **na negociação** (`plan.require_capabilities` / `missing_capabilities`, antes de
enviar nada); o próprio runtime recusa com `invalid_plan` (ponteiro `/requires`) um plano com ruído sem `fxNoise`, e
um passe de ruído sem seed válida. Materiais e alvos são retidos (um `ShaderMaterial` por efeito, ping-pong de dois
alvos): nenhum shader é compilado por quadro e nada de chave/seed é resolvido em JS.

## CLI headless

`scene.fx.edit` com `add_clip {effect:"filmGrain"}`, `set_param` (incluindo o booleano `colored`; um número é
`invalid_value`), as ops de chave e `set_seed` de [F058](fx-keys.md); `scene.fx.catalog` lista `filmGrain` com a
`seed`/`reseed`; `scene.plan` mostra `seed` e os `params` amostrados do quadro. Os limites da camada seguem F016.

## UI e CLI da janela

O mesmo modelo do inspector: o painel lista os campos do catálogo (`amount`, `size`, `luma`, `intensity` como
`Slider`, `colored` como `Switch` real do Kit), a seed e os controles de chave de F058. `./scripts/ui invoke scene.fx
add:filmGrain`; `ui set scene.fx.clip:fx-1 colored true`; `ui set scene.fx.clip:fx-1 seed 77`.

## Divergências declaradas em relação ao piloto

- O alpha da cena é preservado; o piloto escreve alpha 1.
- As texturas de origem já são bytes sRGB codificados (o piloto lê texturas lineares): a mistura em linear parte do valor decodificado.
- Preview em resolução diferente do export tem outra grade de células; a igualdade byte a byte só vale com as
  mesmas dimensões, seed, quadro e cor.
- O corpus é **origin-formula** (as fórmulas transcritas e conferidas por blob), não saída do motor original:
  `ORIGIN_NOT_EXECUTED` e `ORIGIN_GOLDEN_NOT_COMPARABLE` continuam abertos.

## Evidências e status

Estado: **validado neste branch** em macOS/SwiftShader (Chrome headless), sem GPU real, sem Windows, sem ponteiro
físico (BL043) e sem aceite humano. Os números e hashes do código final estão no relatório de entrega (nenhum
número de P2 é reaproveitado).

| Superfície | Prova |
|---|---|
| Corpus independente | `scripts/pilot-grain-corpus.py` (+ `.spec.py`, normal e `-O`) gera `grain-cases.json`/`grain-manifest.json`; `verify` offline e `--origin` conferem os 14 blobs; 16 mutações discriminantes no spec Python (constante de hash, soma no lugar de XOR, linha contada de cima, canto do pixel, escala de resolução dupla/nenhuma, célula sem piso de 1 px, seed que não segue o quadro ou sem hash, ruído uniforme, canais compartilhados, booleano misturado, easing da chave de partida ou curva errada, grão em luz linear, resposta de luma ignorada) e o teste Rust `the_corpus_discriminates_the_mistakes_a_port_would_make` |
| Lib (core) | `crates/core/tests/pilot_grain_corpus.rs` (vetores inteiros exatos, pixels, interpolação), `scene_fx_keys.rs`, `scene_fx.rs` (catálogo de três looks) |
| GLSL real | `runtime/test/directed-scene-grain-browser.test.mjs` (5): sondas do GLSL contra `grain-cases.json` (tolerância 1,5), recusas tipadas; `output-fx.test.mjs` (18) |
| Renderer | `crates/render/tests/scene_fx.rs`: `fxNoise` anunciada, referência, reseed por quadro (12 ≠ 13, mesmo quadro igual, seek devolve o mesmo), seed explícita, amount 0 = baseline, curva por chave, recusa de runtime P2 |
| Export | `crates/render/tests/scene_fx_export.rs`: pixels decodificados do MP4 contra a referência do core nos centros de célula (pior delta 6, mudança máxima 43 contra o baseline), hashes iguais ao `scene.render`, re-export e sub-faixa idênticos, salvar/reabrir, um undo |
| JSONL | processo real `tessel-headless` (`scene_fx_keys.rs`), plano igual ao da lib |
| Janela | `scripts/scene-fx-grain-ui-proof.py` (104 checagens no head 74dc1b9 limpo: nó, seed, `colored`, chaves por ações, easing, mover, remover animação, gesto, salvar/reabrir, export contra o grain em Python); `scene-fx-ui-proof.py` e `fx-history-smoke-proof.py` sem regressão |

Lacunas: GPU nativa e Windows; execução do motor original; ponteiro físico; os 56 efeitos restantes; escopos
`shot`/`node` e transições (P4); timeline de chaves (P6).

Rechecagem independente P3 no head limpo `74dc1b9`: workspace com runtime obrigatório
2208/0/13 (113 resultados de suíte), fmt/check/clippy/build passaram, core sem gpui/wry;
janela FX153/153, grão104/104 e histórico42/42, JSONs dirty_files=0. Probes independentes
debug/release4/4, incluindo 21060 combinações de escala. Logs: /tmp/p3rev2/. A diferença
de contagem para o relato2144/0/13 do implementador não foi explicada pelo revisor.
A revisão aprova core/UI com nits baixos, corrigidos posteriormente em docs e no ramo morto
de diagnóstico. Os gates completos continuam ancorados em74dc1b9; não foram repetidos
após esses nits. Sem GPU real/Windows/ponteiro físico/motor original executado.
