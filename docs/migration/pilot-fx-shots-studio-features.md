# Piloto `t3/scene-engine-pilot`: tudo o que foi construído, para levar ao Tessel Editor (Rust)

Estado: inventário de funcionalidades para migração. Não altera código. Escrito em 2026-11-07 a partir do branch
`t3/scene-engine-pilot` do repositório `tesselapp` (HEAD `d40ac3f7c0`, worktree `/Users/wallyssonnunes/.t3/worktrees/tesselapp/t3-5de3852f`).
Complementa [README.md](README.md) (inventário da `main`, itens `MIG100`–`MIG105` do piloto só em nível de catálogo):
**este arquivo é o detalhe desse delta**. Fontes de verdade no código: `packages/scene-engine` (o CORE) e `apps/scene-studio` (a casca).
Planos e decisões de origem: `docs/video-production/scene-fx-plan.md`, `scene-fx-decisions.md` (decisões #1–#134), `scene-shot-trim-plan.md`,
`scene-shot-trim-decisions.md`, `studio-core-boundary.md`.

## 0. Como ler e a regra que organiza tudo

**Regra de arquitetura (a mais importante para o port):** toda a lógica mora no core como funções puras; o editor é só uma casca.

- Comandos puros: `lista de comandos + bundle → bundle novo | erros {code, message, hint}`. Sem I/O, sem relógio, sem `Math.random`.
- `editBundle` aplica comandos de várias camadas (`{ fx }`, `{ shots }`, câmera, narração…) e valida uma vez no fim (`finalizeBundle`).
- Um **registro de propriedades** por caminho (`fx/<clipId>/start`, `transition/<id>/duration`, `scene/<id>/light/...`) é a API de leitura/escrita única.
- Os **gestos** do editor (arrastar um clipe, aparar um plano, mover uma chave) são funções puras do core (`src/gestures`): o que o mouse propõe, o core
  decide (recusa, aperta ao quadro legal mais próximo, devolve o plano de comandos). A UI só desenha e entrega o ponteiro.
- O **painel de detalhe** é gerado do registro + catálogo (`describeFxInspector`): um efeito novo aparece sem código de UI.
- Uma CLI `scene` expõe os mesmos comandos e gestos, com JSON no stdout e códigos de saída 0/1/2, e nada grava sem `--write`.
- Teste de fronteira: `studioCoreBoundary.spec.ts` impede o Studio de reimplementar regra do core.

Consequência para o Rust: portar o core (modelo, comandos, resolvedor por quadro, gestos, registro, inspector) para `crates/core` e deixar a GPUI como
casca; o GLSL/Three.js vira o equivalente do renderer do destino. Cada item abaixo diz **onde está na origem**.

Contagens medidas no branch: catálogo = **59 efeitos** (31 looks + 28 transições); 19 comandos de efeito, 4 comandos de plano; engine 2907 testes, Studio 240.

---

## 1. Camada de efeitos (`bundle.fx`)

Origem: `packages/scene-engine/src/fx/*` (modelo, comandos, validação, resolvedor), `src/fx/catalog/*` (catálogo), `src/runtime/fx/*` (GPU).

### 1.1 Modelo
- `Fx { catalogVersion, tracks[], transitions[] }`. Opcional no bundle; um bundle sem `fx` segue o caminho direto, idêntico ao baseline (pixel baseline).
- **Trilha** (`FxTrack`): `id`, `label`, `hidden`, `clips[]`. Uma trilha toca um efeito por vez (sem sobreposição no tempo: `FX_CLIP_OVERLAP`);
  efeitos que se sobrepõem ficam em trilhas empilhadas. A ordem das trilhas é a ordem de composição nas passadas de saída.
- **Clipe** (`FxClip`): `id`, `effect` (id do catálogo), `startFrame`, `durationFrames`, `scope`, `params` (valores fixos ou **chaves**), `fadeInFrames`,
  `fadeOutFrames`, `seed`, `hidden`. Seed padrão = `fnv1a32(id)`.
- **Escopo** (`FxScope`): `output` (a imagem final inteira), `shot:<shotId>` (só o plano), `node:<sceneId>:<nodeId>` (um nó; **ainda recusado**, `NODE_SCOPE_NOT_YET`).
- **Parâmetros**: `number` (min/max/default/step/unit px|frames|deg|ratio|bpm|hz), `boolean`, `enum`, `color` (rgb), `vec2`, e **referências** a luz/caminho/nó
  (`lightRefDef`, ver §1.5). Px é declarado a 720p e escala por `altura/720` só no compositor.
- **Chaves de parâmetro** (`keys`): lista `{frame, value, easing}` por parâmetro, com `keyParam`/`unkeyParam`; o resolvedor interpola (`fx-keys.ts`).
- **Envelope**: `fadeIn/fadeOut` em quadros, com intensidade `0→1→0`; todo efeito é **exatamente a identidade em `amount <= 0`**.
- **Transição** (`FxTransition`): `id`, `effect`, `fromShotId`, `toShotId`, `durationFrames`, `alignment` (`center`|`start`|`end` em relação ao corte), `handles`
  (`hold` segura quadros além da ponta do plano), `easing`, `params`, `seed`, `hidden`. Fica **ancorada num corte** entre dois planos vizinhos
  (`TRANSITION_NOT_AT_CUT`, `TRANSITION_DUPLICATE`, `TRANSITION_OVERLAP`, `TRANSITION_TOO_LONG`, `TRANSITION_NO_HANDLES`, `TRANSITION_OUT_OF_RANGE`).
  A janela (`fx-windows.ts`): `center` atravessa o corte (o quadro ímpar vai depois), `start` começa no corte, `end` termina no quadro anterior; o plano de saída
  precisa fornecer `framesAfterCut` quadros de cauda e o de entrada `framesBeforeCut` de cabeça (handles).
- **Dissolve legado** (`shot.transitionInto`) continua válido e é normalizado para uma transição `crossDissolve` (`fx-normalize-transitions.ts`).

### 1.2 Validação (`explainFx`) — códigos estáveis
Erros: `UNKNOWN_CLIP`, `UNKNOWN_TRACK`, `UNKNOWN_TRANSITION`, `UNKNOWN_SHOT`, `UNKNOWN_PRESET`, `UNKNOWN_SCOPE_TARGET`, `UNKNOWN_KEYFRAME_EASING`, `RESERVED_ID`,
`DUPLICATE_ID`, `WRONG_FAMILY`, `WRONG_SCOPE`, `FX_SCOPE_MISMATCH`, `REF_REQUIRED`, `FX_CLIP_OVERLAP`, `FX_CLIP_EMPTY`, `FX_CLIP_OUT_OF_RANGE`, `KEY_OUT_OF_CLIP`,
`ENVELOPE_TOO_LONG`, `TRACK_INDEX_OUT_OF_RANGE`, os `TRANSITION_*` acima, `PRESET_*`, `CATALOG_TOO_NEW`, `CATALOG_MIGRATION_NEEDED`, `UNSUPPORTED_ALPHA` (o compositor só aceita
perfil opaco), `CAMERA_CLIP_SHARED`, `INVALID_COMMAND`, `NODE_SCOPE_NOT_YET`.
Avisos: `TRANSITION_OVER_SPEECH`, `TRANSITION_ON_CONTINUOUS`, `TRANSITION_SPLAT_COST`, `FX_BUDGET_EXCEEDED` (**orçamento de 8 passadas** no quadro mais carregado,
`fx-explain-budget.ts`), `FX_SCOPE_SHARED_DEFINITION`, `FX_DUPLICATES_SCENE_LOOK`.
Toda recusa traz `code`, `message` e `hint` ("o que fazer"). A mensagem de sobreposição cita o efeito e o rótulo da trilha (`vignette (fx-01) and godRays (fx-04) are on the
same track "look" ...`), não só ids.

### 1.3 Comandos de efeito (`FxCommand`, 19 ops, `fx-command-*.ts`, tabela `op → handler`)
`addTrack`, `removeTrack`, `renameTrack`, `reorderTrack`, `addClip`, `removeClip`, `moveClip` (também troca de trilha), `resizeClip` (borda `start`|`end`),
`duplicateClip`, `setParam`, `keyParam`, `unkeyParam`, `setEnvelope`, `setScope`, `setHidden` (clipe, trilha ou transição), `setTransition`, `resizeTransition`,
`removeTransition`, `applyPreset` (existe; o catálogo de presets está vazio, ver §9).

### 1.4 Resolução por quadro (`resolveFxFrame`, puro e determinístico)
`bundle + quadro → FxFrameState`: quais clipes/transição estão ativos, intensidade com envelope, valores de parâmetro (chaves interpoladas), decks (A/B), passadas de
saída em ordem de trilha. Ruído por `hashPixel(seed, pixel)` em `uint` (`lowbias32`), os mesmos inteiros no JS (`fx-hash.ts`) e no GLSL; **sem relógio, sem `Math.random`, sem hash em float**.
Pulsos e rajadas (glitch, lightningFlash, heartbeatCut) são quadros inteiros vindos de `resolveEvents`. Cache por quadro (`resolve-fx-frame-cache`).

### 1.5 Passadas ancoradas numa luz ou num ponto da tela
`godRays`, `smoke*`, `fire`, `flameLife`, `lightLeak`, `candlelight`, `dust` apontam para uma **luz da cena por id**. `locatePassPoints` (runtime) projeta a luz à tela
com as matrizes efetivas da câmera (`projectToScreen`, `lightScreenPoint` em `fx-screen-point.ts`) e entrega `FxPass.screenPoint` ao compositor. Luz atrás da câmera ou ausente = não desenha.
Defeito corrigido e coberto por spec: o compositor recebia os decks não localizados (todas as passadas de luz saíam em branco); agora recebe `located.decks`.

---

## 2. Catálogo de efeitos (59)

Fonte: `src/fx/catalog/<id>.ts` (contrato: nome/resumo/quando usar em pt-br, en, es; tags; moods; duração típica; parâmetros com faixa e padrão; custo; o que precisa; amostra para
a galeria) e o GLSL em `src/runtime/fx/shaders/<id>.ts`. O contrato é dado puro (`defineFxEffect`, validado por `fx-effect-meta.spec.ts`); o shader nunca é carregado pelo schema.
A tabela abaixo foi **gerada dos arquivos do catálogo** (parâmetros = ids declarados; faixas e padrões estão em cada arquivo).
`kind: pass` = uma passada de tela cheia; `kind: transform` = transformação 2D do quadro (`transform-motion.ts`), sem passada de GPU extra.

### 2.1 Looks (31) — passam sobre o quadro (escopo saída ou plano)

| id | nome (en) | custo | precisa | parâmetros | o que faz |
|---|---|---|---|---|---|
| `bleachBypass` | Bleach bypass | low | — | — | Pulls the colour out and hardens the contrast, like a film print with the silver left in. |
| `bloom` | Bloom | medium | — | threshold, radius, intensity, tint | A soft glow around the highlights of the frame. |
| `breathingZoom` | Breathing zoom | low | — | amount, periodFrames | The frame slowly grows and comes back, like a breath, so a still shot never feels dead. |
| `cameraShake` | Camera shake | low | seed | amplitude, frequency, roll, decay | Shakes the frame like a hand-held camera, with smooth shifts and tilt; it can die away by the end of the clip. |
| `candlelight` | Candlelight | low | — | — | A warm candle look: amber highlights and deeper shadows. |
| `chromaticAberration` | Chromatic aberration | low | — | strength, falloff, center | Colour fringes at the edges of the frame, like a cheap lens. |
| `colorGrade` | Colour grade | low | — | lift, gamma, gain, contrast, saturation, temperature, tint | Lift, gamma and gain, contrast, saturation and white balance in linear light. |
| `dream` | Dream | high | — | threshold, gain, reach, fringe, softReach, softStart, softEnd | A soft glow on the bright parts, colour fringes toward the edges and softened borders (the `dream` scene look, as a clip). |
| `dust` | Dust | medium | seed | count, size, speed, sway, color | Dust motes floating slowly in the air, catching the light. |
| `fade` | Fade | low | — | — | Mixes the shot with a colour (black, white, red) by an amount that can be keyed. |
| `film` | Film | low | seed | warmth, curve, vignette, grain | A warm film grade with a gentle contrast curve, darker corners and a fine grain that changes every frame (the `film` scene look, as a clip). |
| `filmGrain` | Film grain | low | seed | amount, size, luma, colored | Film grain that boils every frame, stronger in the shadows. |
| `fire` | Fire | medium | seed, light-position | light, coreColor, midColor, edgeColor, height, width, flicker, turbulence, speed, embers | Flames that rise from a light of the scene in dancing tongues, from the red edge to a near-white core, with optional sparks. |
| `flameLife` | Flame life | medium | seed | threshold, softness, hue, distortion, speed, scale, flicker, glow | Brings the fire already in the picture to life: it finds the flame-coloured pixels and makes them lick upward, flicker in brightness and glow warmly. |
| `flicker` | Flicker | low | seed | amount, rate, blackouts | The exposure wavers like a failing bulb or a flame, with optional blackouts. |
| `godRays` | God rays | medium | light-position | light, threshold, length, decay, color | Shafts of light that fan out from a light of the scene and cross the frame, like light through a window in fog. |
| `halation` | Halation | medium | — | threshold, radius | A wide reddish halo around strong lights, like the one on film. |
| `heatHaze` | Heat haze | medium | seed | amplitude, scale, speed, height | The frame ripples like the air over hot asphalt or a bonfire, stronger at the bottom. |
| `ink` | Ink | high | — | radius, lineWidth, lineStrength | Flat areas turn into brush strokes and the contours of shapes are drawn as ink lines (the `ink` scene look, as a clip). |
| `inkPrint` | Ink print | medium | — | dots, edge, paper | Turns the frame into a halftone engraving on paper, with outlines in ink. |
| `lensBlur` | Lens blur | high | — | focus, range, maxRadius, mode | Blurs the frame by the distance ON THE SCREEN from a point or band of focus (tilt-shift or radial). It does not use depth. |
| `letterbox` | Letterbox | low | — | ratio, softness, color | Bars that mask the frame down to the chosen aspect ratio. |
| `lightLeak` | Light leak | medium | seed | color, size, position | A warm light leak that comes in from a corner and slowly moves, like in film cameras. |
| `mist` | Mist | medium | seed | density, scale, drift, color, height | A layered mist that slides across the frame, denser at the bottom with its colour changing from top to bottom. |
| `moonlight` | Moonlight | low | — | strength, shadowTint, desaturate | A cold night look: bluish shadows, cool midtones and the colour drained a little. |
| `scalePunch` | Scale punch | low | events | amount, riseFrames, fallFrames, pulses, gapFrames | A sharp zoom pulse (a scare or a beat) that jumps in a few frames and falls back; it can repeat. |
| `scanlines` | Scanlines | low | seed | lines, jitter, chromaBleed | Tube-screen scanlines with a per-row horizontal wobble and colour bleed, like a VHS tape. |
| `smoke` | Smoke | medium | seed, light-position | light, color, glow, height, width, spread, rise, drift, scale, density, opacity | A soft column of smoke that rises from a light of the scene, widens, drifts with the wind and dissipates in the air. |
| `smokeBillow` | Billowing smoke | medium | seed, light-position | light, color, litColor, height, width, spread, rise, drift, billow, density | Dark, heavy smoke in big rolling puffs that spread as they rise, with the underside lit by the fire. |
| `smokeWisp` | Smoke wisp | low | seed, light-position | light, color, glow, height, thickness, curl, frequency, rise, drift, strands, density | A thin thread of smoke that leaves a light of the scene and draws curls that grow as it rises, like incense or a freshly snuffed candle. |
| `vignette` | Vignette | low | — | radius, softness, intensity, color, roundness, center | Darkens the edges of the frame with a soft elliptical falloff. |

Observações: `breathingZoom`, `scalePunch` e `cameraShake` são os 3 looks `transform` (mexem no quadro, não colorem). `film` é um look composto (grade quente, curva de contraste suave,
cantos escuros e grão fino que muda a cada quadro). `ink` e `inkPrint` redesenham o quadro em traços/meio-tom (o `ink` equivale ao look de cena `ink`). `flameLife` **procura os pixels
cor de chama da própria imagem** e os faz lamber para cima (não desenha fogo novo). Os looks `needs: seed` usam o hash determinístico.

### 2.2 Transições (28) — dois planos em um, só dentro da janela de uma transição

| id | nome (en) | custo | precisa | parâmetros | o que faz |
|---|---|---|---|---|---|
| `additiveDissolve` | Additive dissolve | low | two-inputs | — | The two shots add their light in the middle of the change, like a soft flash. |
| `blinds` | Blinds | low | two-inputs | count, angle, stagger, softness | Strips open one after another, like blinds, and reveal the next shot. |
| `burn` | Burn | medium | two-inputs, seed | scale, edgeWidth, ember, glow | The outgoing shot burns like paper and reveals the next with embers at the edge. |
| `crossDissolve` | Cross dissolve | low | two-inputs | curve | Blends the outgoing shot into the incoming one, in linear light. |
| `cut` | Hard cut | low | two-inputs | — | Switches from A to B halfway through the window, with no blend. |
| `defocusDissolve` | Defocus dissolve | high | two-inputs | radius, samples | Both shots go out of focus mid-change and sharpen again on the new one. |
| `dipToBlack` | Dip to black | low | two-inputs | hold, color | The outgoing shot darkens to black and the incoming one opens from it. |
| `dipToColor` | Dip to color | low | two-inputs | hold, color | The outgoing shot fades to a colour and the incoming one opens from it. |
| `dipToWhite` | Dip to white | low | two-inputs | hold, color | The outgoing shot burns out to white and the incoming one opens from the flash. |
| `eyeBlink` | Eye blink | medium | two-inputs | closeAt, softness, blur, lids | Eyelids close, the shot changes in the dark and open on the next one. |
| `fadeThroughVignette` | Fade through vignette | low | two-inputs | softness, color | The dark closes from the edges to the centre and opens again from the centre outwards. |
| `filmDissolve` | Film dissolve | low | two-inputs | — | Blends the shots like film stock: the middle of the change is brighter. |
| `glitch` | Glitch | low | two-inputs, seed, events | blocks, shift, rgbSplit, rate, lines | Shifted blocks, an RGB split and lines swap the shot in irregular bursts. |
| `heartbeatCut` | Heartbeat cut | low | two-inputs, events | bpm, amount, flash, cutAt | Two pulses of scale and red glow, and the cut lands on the last one. |
| `inkBleed` | Ink bleed | medium | two-inputs, seed | scale, edge, ink, octaves | A dark ink stain spreads and reveals the next shot behind it. |
| `iris` | Iris | low | two-inputs | shape, softness, invert | A round, diamond or eye-shaped opening reveals the next shot. |
| `lightningFlash` | Lightning flash | low | two-inputs, seed, events | pulses, peak, tint, invertAmount | Lightning flashes light the shot and the change happens on the last one. |
| `lumaWipe` | Luma wipe | low | two-inputs | source, softness, invert | The next shot shows first in the dark (or bright) parts of the picture. |
| `push` | Push | low | two-inputs | parallax, overshoot | The incoming shot pushes the outgoing one out of the frame. |
| `shadowSweep` | Shadow sweep | medium | two-inputs, seed | angle, softness, shadow, grain | A band of shadow with a ragged edge crosses the frame and swaps the shot. |
| `shockwave` | Shockwave | medium | two-inputs | ringWidth, amplitude, chromatic | A ring of distortion grows from the centre and leaves the next shot inside it. |
| `slide` | Slide | low | two-inputs | — | The incoming shot slides over the outgoing one, which stays still. |
| `smokeFog` | Smoke fog | medium | two-inputs, seed | density, scale, drift, fog | A dense fog covers the outgoing shot and clears over the incoming one. |
| `staticNoise` | TV static | low | two-inputs, seed | grain, scanlines, tint | Snow and scan lines cover the screen and swap the shot. |
| `whipPan` | Whip pan | high | two-inputs | angle, strength, samples, peakAt | The camera swishes to the next shot, with motion blur. |
| `wipeLinear` | Linear wipe | low | two-inputs | angle, softness | A straight curtain crosses the frame and uncovers the incoming shot. |
| `wipeRadial` | Radial wipe | low | two-inputs | startAngle, direction, softness | A hand turns around the centre and reveals the next shot. |
| `zoomCross` | Zoom cross | medium | two-inputs | amount, center, blur, direction | The outgoing shot zooms in as it fades and the incoming one settles from a zoom. |

`smokeFog` (névoa que cobre a saída e abre sobre a entrada) e `inkBleed` (mancha de tinta que revela o próximo plano) são **transições**, não looks.
Parâmetros comuns de direção: `direction-params.ts`. Toda transição em `intensity: 0` é a mistura linear simples. `crossDissolve` é a transição que o dissolve antigo
(`shot.transitionInto`) vira. Custos: `TRANSITION_SPLAT_COST` avisa quando o segundo runtime precisa montar uma cena de splat.

### 2.3 Fogo e fumaça (o último lote implementado)
- `smoke` (fumaça ancorada numa luz), `smokeWisp` (**fio fino com contornos que sobe e se enrola**), `smokeBillow` (**fumaça pesada, em bolas**),
  `fire` (chama procedural ancorada numa luz, `fire-motion.ts`), `flameLife` (**anima partes que já têm fogo na imagem**: movimento e brilho
  sobre a arte, sem desenhar fogo novo), `godRays` (feixes radiais a partir de uma luz), `dust` (partículas de poeira no ar).
- Ancoragem: os cinco que usam luz têm o parâmetro `ref` para uma luz da cena; ajuda comum em `shaders/chunks/light-point.ts` e `chunks/index-draw.ts`.
- Cada shader tem **verificação de hash** (`fbmCheck`/`moteDraw`): o mesmo GLSL roda no Chrome contra um gêmeo em JS (`scripts/fx-lab-check.mjs`).

---

## 3. Planos (shots): aparar, alongar, mover o corte, ripple

Origem: `src/shots/*` (comandos puros), `src/gestures/shot-*.ts` (gestos), `bin/scene-shots.mjs`. Plano em `docs/video-production/scene-shot-trim-plan.md`.
Antes do piloto a duração de um plano só nascia no gerador do bundle; agora é editável.

- **Comandos** (`ShotCommand`, via `editBundle({ shots })`): `trimShot {shotId, edge: start|end, frame, mode}`, `moveCut {fromShotId, toShotId, frame}`,
  `setShotDuration {shotId, durationFrames, mode}`, `setEditDuration {frames}`.
- **Modos**: `ripple` (padrão: a borda se move e tudo depois anda junto; `editDurationFrames` muda) e `roll` (entre vizinhos: o vizinho absorve, a duração total fica).
- **Conteúdo** (`--content`): `anchor` (a imagem fica nos mesmos quadros da cena; `NO_HEAD_FRAMES` se não houver cabeça), `follow` (a cena começa no novo início; o plano mostra mais da cauda),
  `auto` (padrão: ancora enquanto houver cabeça e segue no resto; nunca recusa por falta de cabeça).
- **Opções**: `withNarration` (desloca as falas depois do ponto), `keepAbsolute` (efeitos de saída e chaves ficam no tempo absoluto), `shrinkTransitions` (encurta a transição que não cabe).
- **O que acompanha o ripple** (`shot-layers-*.ts`): clipes de efeito `scope: shot` andam com o plano; os de `output` depois do ponto andam pelo delta (os que cruzam esticam/encurtam);
  chaves de luz, de clima, trilhas de rig e grupos do outline em quadros da edição andam; transições seguem o corte e recalculam a janela.
- **Regra de produto:** *imagem e áudio são independentes.* Mudar a duração de um plano **não move nem corta** a narração nem o áudio por padrão; a imagem pode terminar antes ou
  depois do áudio, isso é permitido. Narração além da edição vira aviso (`NARRATION_PAST_EDIT`), nunca erro; edição mais longa que o áudio é válida (o resto é silêncio).
- **Erros**: `SHOT_TOO_SHORT`, `SHOT_NOT_FOUND`, `SHOTS_NOT_ADJACENT`, `TRANSITION_TOO_LONG`, `EDIT_TOO_LONG`, `LOCKED` (`editLocked`), `NO_HEAD_FRAMES`; todos com `limitFrame` quando há um limite (o gesto usa para apertar).
- **Runtime**: alongar além do fim da cena ou do clipe de câmera **segura o último quadro**.

---

## 4. Gestos puros do core (`src/gestures`) — a lógica de arrastar sem mouse

Cada gesto recebe a proposta do ponteiro e devolve `{ok, frame/limite, comandos, avisos}`; a UI nunca decide legalidade.
`fx-clip-drag` (mover/aparar clipe de efeito, troca de trilha), `fx-transition-drag` (aparar a janela pelo início/fim, sem mover o corte), `fx-edit` (aplicar o plano),
`shot-drag`/`shot-edit`/`shot-field-edit` (aparar plano, mover corte, digitar um número), `shot-order`, `scene-spans`, `frame-range`,
`key-frame-drag`, `key-retime`, `key-path-retime` (mover uma chave, mover todas, escalar até a última; as chaves **mantêm a ordem** e param no vizinho),
e as chaves de **follow** da mão que escreve (`setFollowKey`, `retimeFollowKeys`, `setFollowProgress`).
Specs: um `*.spec.ts` por gesto, com casos limite e propriedade "o resultado sempre passa `validateBundle`".

---

## 5. Registro de propriedades e painel de detalhe

- Caminhos novos: `fx/<clipId>/{start,duration,fadeIn,fadeOut,seed,hidden,scope,track}`, `fx/<clipId>/<param>` e `.../<param>/keys`, `fx/track/<trackId>/{label,hidden}`,
  `transition/<id>/{duration,alignment,handles,easing,seed,hidden}` e parâmetros; planos `scene/shot/<id>/...` (leitura e via comando: o registro **não pode driblar o ripple**).
  Recusa sempre como `FX_REFUSED` com os erros da camada aninhados.
- **Inspector gerado** (`describeFxInspector`, `src/properties/fx-inspector.ts`): cabeçalho (trilha, escopo, início, duração, fades, semente, escondido; transição: duração,
  alinhamento, handles, easing) + um campo por parâmetro com o controle certo (`FxInspectorField` é uma união discriminada: slider com faixa/passo/unidade, liga/desliga, escolha,
  cor, ponto 2D, referência a luz/caminho, semente), padrão, `atDefault`, botão de chave no playhead e redefinir. Rótulos dos 117 parâmetros em pt-br/en/es no core
  (`fx-param-labels.ts`); um spec **falha se um id do catálogo ficar sem rótulo**. O mesmo modelo sai por `scene inspect`.
- Painel de plano (início, fim, duração, limites que se atualizam depois de cada edição), de chave de follow (quadro) e de fala da narração.

---

## 6. Runtime / renderer (Three.js hoje; é o que o destino precisa reproduzir)

Origem: `src/runtime/fx/*`, `src/runtime/three-scene-runtime.ts`. Documento: `src/runtime/fx/README.md`.

- **Fluxo do compositor** (`fx-compositor.ts`, sem estado entre quadros, recebe uma porta de render para testar sem GPU):
  `decks (RGBA8) → ingest (sRGB→linear, half float) → passadas do deck (looks com escopo de plano, depois transform2D) → transição (dois decks em um, só na janela) →
  passadas de saída (looks `output`, em ordem de trilha) → final (transform2D de saída, encode sRGB ou Rec.709, dither) → canvas`.
- **Dois decks / segundo runtime**: uma transição entre duas cenas diferentes monta um **segundo runtime** (`createSecondary`) só pela janela, com cópia dos overrides ao vivo
  (luz, outline, clima, chaves de clima, chão, looks de cena; `live-overrides.ts`), e o libera no primeiro quadro depois dela (`endWindow`, `releaseIdle` após 48 quadros sem efeito).
  Ao fim da transição a cena montada é entregue ao primário (`adoptSceneOf`) para não reconstruir (e não "pular").
  A construção do segundo runtime fica **dentro da seção exclusiva do renderer** (invariante a preservar).
- **Pool de alvos** (`fx-target-pool.ts`): alvos half float por tamanho, reaproveitados; liberados após longa ociosidade.
- **Gate** (`fx-layer.ts` `bundleHasFxLayer`): um bundle sem clipe/transição/dissolve não resolve fx e segue o caminho direto (decidido a cada quadro).
- **Regras de shader**: identidade em `amount <= 0`; opaco apenas (`UNSUPPORTED_ALPHA`); px a 720p escalados só no compositor; nada de variável GLSL com palavra reservada (um spec varre);
  todo shader que chama o hash precisa declarar `hashChecks`.
- **Pixel baseline e determinismo**: `scripts/fx-pixel-baseline.mjs verify` (um bundle sem fx continua idêntico ao baseline), `fx-determinism.mjs` (mesma entrada, mesmos pixels),
  `fx-lab-check.mjs` (GLSL x JS no Chrome).
- **Goldens**: `src/fx/catalog/__golden__/` por efeito; `UPDATE_GOLDENS=1 node scripts/fx-golden.mjs --effect <id>` regenera; `fx-golden.mjs` sem argumento é a comparação real.
  O laboratório (`src/node/fx-lab-bundles.ts` `goldenBundle`) usa uma luz `lab-light` e quatro placas (plate-a, plate-b, illustration, flames).
- **Apresentação do quadro** (Studio): o quadro é decidido no início (`presentFrame`) para o preview se mover durante o play mesmo com o compositor ocupado.
  Hook de medição `onSceneBuilt`/`[scene-build] <definitionId>: <ms> ms (secondary: …)` no console para medir o atraso na troca de cena.

---

## 7. Studio (a casca): o que o editor mostra e faz

Origem: `apps/scene-studio/src`. Toda ação abaixo chama o core (comando/gesto/propriedade); o editor desenha, entrega o ponteiro e mostra o resultado.

### 7.1 Timeline
- **Todas as trilhas de todos os planos ficam sempre visíveis** (antes só as do plano sob o playhead). Itens de outros planos aparecem esmaecidos/com tom próprio (`otherShotTone`), com tooltip
  "Plano X · …: clique para levar o cursor até ele".
- **Trilhas de efeito** (`FxTrack`): uma linha por trilha, uma barra por clipe, rótulo = nome do efeito; clipe escondido aparece apagado; trilha vazia tem linha para poder ser preenchida.
  Tooltip: efeito e duração, trilha, escopo, estado, o que o clique faz. Clique leva o cursor ao início e seleciona; **Delete** remove (guardado: só vale na timeline e fora de campo de texto, `fxDeleteKey`).
- **Arrastar um clipe** move no tempo e **soltar em outra trilha troca de trilha**; alças esquerda/direita mudam início/fim. Durante o arraste o preview mostra a proposta, uma recusa do core
  não é mostrada (fica o último quadro legal) e o motivo aparece na linha de estado. Soltar = **um passo de desfazer** e salva.
- **Transições** (`TransitionsTrack`): uma barra por corte com a janela real (centrada/início/fim); alças para mudar a duração **pelo início ou pelo fim da janela sem mover o corte**; linha
  escondida quando não há transição; Delete remove.
- **Planos** (`ShotsTrack`): uma barra por plano (rótulo = `intent`); **alça direita = aparar com ripple** (os seguintes andam, a edição cresce/encolhe, selo com duração e delta); **divisa entre dois
  planos = mover o corte (roll)**; **Alt** troca o modo (ripple ↔ roll); campo numérico no painel; clamp no quadro legal mais próximo (`limitFrame`); um passo de desfazer por arraste; todos os domínios
  que o arraste toca (efeitos, chaves de luz e de clima) são re-semeados; áudio e playhead ficam estáveis durante o arraste. Aviso visível quando a narração passa do fim da imagem ("A narração passa 0.66 s do fim da imagem"),
  sem bloquear.
- **Chaves** (`KeysTrack`): chaves de luz, de clima e de parâmetro de efeito são losangos arrastáveis (ordem preservada, ímã com Alt para desligar); mover uma, mover todas, escalar.
- **Follow / mão que escreve** (`FollowTrack`): arrastar as chaves de progresso da escrita (acelerar, começar logo ao iniciar), mover ou escalar o follow inteiro.
- **Rig e câmera**: trilhas de câmera (movimentos, trilhas paralelas, dolly + órbita), rig, caminhos, com a mesma regra de gesto no core.
- **Várias abas de projeto** (`ProjectTabs`, `projectsRegistry`) e um domínio de histórico por camada (`historyDomain`: fx, plano, luz, clima, câmera, rig), com **desfazer/refazer global** (Cmd/Ctrl+Z, Y) que vale
  com qualquer painel focado, exceto em área de texto.

### 7.2 Painel de detalhe (gerado do core, §5)
Clicar em qualquer item da timeline (clipe de efeito, transição, plano, chave de follow, fala) abre o painel com os controles certos e **todas** as props editáveis. Cada edição é um passo de desfazer, aparece ao vivo no preview
(preview em arraste de slider, commit ao soltar) e uma recusa do core aparece no painel com as palavras do motor. O seletor "Trilha" lista as trilhas **pelo que elas guardam** ("Flame life (bloom)"), não só pelo nome interno;
trilha vazia aparece como "(vazia)". A mensagem de recusa some ao selecionar outro item. Construído com Leva a partir de um `InspectorSchema` neutro (`ui/leva`): o schema é pura estrutura, o Leva é trocável — para o Rust,
basta consumir o mesmo `FxInspectorModel` com os controles GPUI.

### 7.3 Preview e reprodução
- O preview apresenta o quadro **decidido no início** do pedido, então a câmera e os efeitos acompanham o play (defeito corrigido: ficava parado até pausar).
- Reprodução continua **além do fim do áudio** (a imagem pode passar do áudio); falas empilham dentro de um corte.
- Diagnóstico de transporte e medição `[scene-build]` na troca de cena.

---

## 8. CLI `scene` (mesmos comandos e gestos da UI)

`scene <comando> <bundle> …` — um objeto JSON no stdout (`--pretty` indenta), saída 0 ok / 1 recusado (os erros estão no JSON) / 2 uso; nada grava sem `--write`.
Comandos que já existiam: `effects`, `show`, `props`, `get`, `set`, `edit`, `camera`, `narration`, `outline`, `takes`, `cut-from-mix`, `validate`, `render`, `preview`. **Novos no piloto:**

- `scene fx catalog [--family] [--tag] [--mood]`, `fx show <id>`, `fx describe [--locale] [--family] [--mood] [--max-chars]`, `fx preview <id> [--params] [--frames] --out`,
  `fx <bundle> list | apply '<comandos>' | explain | preset <id> … | transition list|set|suggest`.
- `scene shots <bundle> list | trim | move-cut | duration | edit-duration` com `--mode`, `--with-narration`, `--keep-absolute`, `--shrink-transitions`, `--content`.
- `scene keys <bundle> <path/keys> move-key | move-all | scale`, `scene follow <bundle> <rig> <follow> move-key | move-all | scale`,
  `scene inspect <bundle> clip|transition|shot <id>` (o modelo do painel).
- `scene show --layer fx|shots|…`; edição em lote `{ fx }` e `{ shots }` por `editBundle`.
- Skills que ensinam um agente a usar tudo isso: `.claude/skills/scene-cli`, `scene-element-recipes` (inclui "duração dos planos", a regra de imagem/áudio independentes e as chaves de follow), `scene-effects`.

---

## 9. Produção de cena a partir de uma imagem (o fluxo que usamos para testar)

Não é do editor, mas é o que gera os projetos que o editor abre:
- `scripts/splat/import-sharp-scene.mjs`: monta uma cena de **splat (SHARP)** a partir de uma imagem, com `--character none` para cena sem personagem, câmera com `from: "previousEnd"`.
- `scripts/elements/new-project/make-layered-stub.mjs`: bundle de um plano a partir de uma imagem (valida `--frames` e `--id`).
- Clima: chuva, poças d'água e relâmpago (`rain:0.6+puddles+lightning`); luzes com cintilação e chaves; narração local (Chatterbox, `http://127.0.0.1:8004/v1`, voz `narrador`); `scene takes`/`cut-from-mix`.
- Servidor do editor: `node packages/scene-engine/scripts/elements/rig/open-hands-editor.mjs --bundle <json> --port N --data-dir <dir>` (rotas `PUT /bundle-fx` e `PUT /bundle-shots`, ambas via `applyPropertyChanges` + `finalizeBundle`).

---

## 10. O que ainda NÃO existe no piloto (não portar como pronto)

- **`scope: node`** (efeito isolado num nó da cena): recusado com `NODE_SCOPE_NOT_YET`. Falta o compositor desenhar um nó isolado.
- **`flashlightReveal`** e a projeção de um ponto de caminho à tela.
- **Presets**: `applyPreset` existe, mas `FX_PRESETS` está **vazio**; os 13 presets de suspense (`expandPreset`) não foram escritos.
- **F6**: `scene fx gallery`, skill `scene-fx`, `scene-engine-fx.md`, `fx-catalog.md` e um render completo `full-v3`; o bundle `full-v2` usa `alphaMode: straight`, que o compositor recusa (`UNSUPPORTED_ALPHA`),
  então o perfil de render precisa virar opaco.
- **Prefetch na troca de cena**: existe um atraso pequeno ao entrar numa cena nova (o segundo runtime monta dentro da seção exclusiva); a medição `[scene-build]` está pronta, o prefetch de bytes dos ativos não.
- Limites conhecidos: o orçamento de 8 passadas por quadro; `dust` recalcula por pixel; o Studio pode sobrescrever edição de fx feita pela CLI enquanto está aberto (recarregar a aba); `setShots` redesenha duas vezes.
- Fora de escopo do trim: dividir um plano (`splitShot`), apagar/reordenar planos, estender o áudio, câmera contínua atravessando o corte, "slip".

---

## 11. Ordem sugerida para o Rust (dependências, do mais barato ao mais caro)

1. **Modelo + validação** de `Fx` (tipos, `explainFx` com os códigos e dicas, `fx-ids`, normalização do dissolve legado) e o **catálogo como dado** (as 59 metas; pode ser gerado a partir dos `.ts` ou de JSON).
2. **Comandos de efeito** (19 ops) e **de plano** (4 ops) com ripple/roll/content, sobre o modelo de bundle que o destino já tem; specs portados (os `*.spec.ts` são a especificação executável).
3. **Registro de propriedades** + **inspector gerado** (`FxInspectorModel`) + rótulos pt-br/en/es: a GPUI consome o modelo; um teste "todo efeito do catálogo tem controle e rótulo".
4. **Gestos** (`gestures/*`) como funções puras: é o que dá "arrastar legal" sem lógica na UI.
5. **Resolvedor por quadro** (`resolveFxFrame`, janelas de transição, chaves, hash `lowbias32`) com os mesmos vetores de teste (`fx-hash`, `fx-noise1d`, goldens de estado).
6. **Compositor GPU** (fluxo do §6) e os **31 looks + 28 transições**; começar por `vignette`, `filmGrain`, `crossDissolve`, `fade`, `dipTo*` e validar por golden e pelos hashes antes dos
   efeitos com luz (`godRays`, fumaça, fogo), que dependem de `locatePassPoints`.
7. **Timeline da GPUI**: trilhas de efeito, transições, planos e chaves com os gestos do passo 4.
8. **CLI** (`fx`, `shots`, `keys`, `follow`, `inspect`) sobre as mesmas funções.
9. **Validação de paridade**: usar o oráculo de migração (`scripts/migration-oracle`) e os goldens do piloto como referência; `fx-pixel-baseline`, `fx-determinism` e a comparação real de golden como gates.

Mapeamento com o inventário existente: os itens `MIG100`–`MIG105` deste README devem apontar para as seções 1–8 acima.
