# F055 — Gizmo de translação e rotação no canvas (BL015, BL015.1, BL042)

## Objetivo e limites

Um gizmo inicial de seleção, **translação e rotação** para cenas dirigidas. Ele é **desenhado e acertado
por um canvas 2D dentro da página da WebView** (`runtime/src/overlay.js`), nunca por elementos
GPUI sobre a WKWebView (a view nativa pinta acima de qualquer elemento GPUI). O ponteiro vira
intents que o **core valida e aplica**; a página não decide nada além de "onde, em coordenadas da
imagem, o usuário apertou".

- Handles: eixos `x`, `y`, `z` e um handle central `screen` (move no plano que encara a câmera).
- Seleção por ponteiro: apertar sobre um plano de imagem seleciona (`additive` alterna); apertar
  em área vazia da imagem limpa a seleção; apertar num handle mantém a seleção.
- O que é editado: a propriedade vetorial existente `…/node/<id>/position` (`localPosition` do
  documento) com a ferramenta `translate`, e a propriedade `…/node/<id>/rotation` (o quaternion
  `localRotation`) com a ferramenta `rotate` (BL015.1, [seção própria](#rotação-de-um-nó-g1-bl0151)).
  Não há escala, snapping, múltiplas seleções nem grupos: com mais de um selecionado o overlay fica
  oculto (`multiple_selection`).
- Fora do escopo e **não declarado como entregue**: ponteiro físico nativo (BL043), popover/foco
  de teclado sobre a WebView (BL044), perfil de latência, atalhos de teclado do gizmo.

**ID.** F055. Em 2026-11-07 o `docs/features/README.md` da `main` (`59b8efc`) tem até F049 e as PRs
abertas já reservam F050 (títulos acentuados, PR #21), F051 (fades, PR #22), F052 (importação SHARP,
PR #23), F053 (cabeçalho/miniaturas, PR #24) e F054 (keyframes da câmera, PRs #20/#23). A primeira
escolha desta entrega (F052) colidia com a PR #23; F055 é o primeiro ID livre. F044 é o aceite
independente. Esta PR não toca as PRs #20–#24: quem integrar por último confere a numeração da `main`.

**Procedência.** Retomada da entrega interrompida do gizmo (WIP somente leitura em
`t3-b43670d3`, branch `t3/canvas-transform-gizmo`, commits `0074f8e` e `6084892`, que continuam como
estavam). O WIP **não é prova**. O código foi adaptado sobre a `main` viva (`59b8efc`, commit
`ba0d5dc`). Os gates foram executados em duas rodadas (`4120ca2`/`aabb589`, depois o head de código `85d6aa7`; ver
"Evidências e status": o que rodou, o que falhou e foi corrigido, e o que continua sem prova).

## Contrato do core / biblioteca

Tudo vive em `tessel-editor-core` (`scene_interaction::gizmo` para a geometria,
`scene_project::gizmo` para a execução). Sem GPUI, janela ou socket.

### Espaço do ponteiro

Coordenadas **da imagem de saída**, não do painel: `u` e `v` em `0..1` sobre a figura (câmera da
cena, `output.width × output.height`), `v` cresce para baixo. Comprimentos (handle, raio de
acerto) são medidos em alturas de imagem. Por isso **resize nunca muda o significado de um
ponteiro**: a página converte pixel do painel para imagem (`containRect`) e o core nunca vê pixel.
Um *press* precisa cair dentro da imagem; *moves* podem sair dela (até `OUTSIDE_LIMIT = 8`).

Constantes (`scene_interaction::gizmo`): `HANDLE_LENGTH = 0.14`, `HIT_RADIUS = 0.025`,
`CENTER_RADIUS = 0.03`. Convenção de câmera idêntica a `runtime/src/directed-scene.js`:
`right = normalize(forward × up)`, FOV vertical, `aspect = width/height`.

### Comandos (também ops JSON)

| Op | Comando | Efeito |
|---|---|---|
| `scene.gizmo.state` | `GizmoCommand::State` | leitura: o `Overlay` (v1) do selecionado no frame |
| `scene.gizmo.hit` | `GizmoCommand::Hit` | leitura: o que há sob um ponto (`handle`, `plane` ou `none`) |
| `scene.pointer.down` | `PointerCommand::Down` | seleciona/abre arrasto (`gesture_begin`); nunca escreve o documento |
| `scene.pointer.move` | `PointerCommand::Move` | `gesture_update` com **um** `set position` calculado do press |
| `scene.pointer.up` | `PointerCommand::Up` | `gesture_commit`: **uma** entrada de undo ("Edit scene") |
| `scene.pointer.cancel` | `PointerCommand::Cancel` | descarta o arrasto; razões `escape`, `blur`, `pointer_cancel`, `hidden`, `unavailable`, `api` |
| `scene.translate` | `GizmoCommand::Translate` | o mesmo resultado **sem ponteiro**: delta de mundo → `edit_scene` |

`scene.capabilities` lista todos (negociação aditiva por listagem).

### Identidade do overlay (carimbo)

Cada overlay leva `stamp = {scene, session, revision, frame}` **produzido pelo core**
(`EditorState::gizmo_stamp`): `session` muda com um projeto novo, `revision` com cada edição e `frame`
com o playhead; um arrasto em andamento (candidato) **não** é revisão. A página devolve o carimbo em
toda mensagem `overlay.pointer` e `overlay.drawn`; o adaptador só **compara igualdade**. Mensagem de
página recarregada, de projeto substituído, de cena trocada ou de frame anterior a um seek é
descartada e contada (`overlay.stale`); um `down` velho nunca abre arrasto e um `move`/`up` velho
**cancela** o arrasto que a página acredita ter (para o release, que também seria recusado, não
deixá-lo aberto). `cancel` é sempre aceito (só descarta). Um ack `drawn` com carimbo diferente do
overlay atual nunca vira "o que a página mostra".

O adaptador **acrescenta** ao carimbo do core dois campos que o core não conhece
(`Workstation::overlay_stamp`): `epoch`, a encarnação da página (sobe a cada `overlay_forget`: reload,
página nova), e `selection`, a seleção de entidades do overlay. Uma mensagem de uma página anterior ao
reload carrega o `epoch` velho e é recusada mesmo que `{scene, session, revision, frame}` coincidam;
um press feito contra a seleção anterior (a árvore mudou a seleção e o overlay novo ainda não chegou)
é recusado em vez de agir sobre o nó errado. Move/up/press só são aceitos com o feature negociado **e**
o overlay ligado (`overlay_supported() && overlay_enabled()`); um press recusado por isso é contado
(`stale`) e não altera nem a seleção. Um `epoch` novo no meio de um arrasto o cancela.

### Custo do overlay (M2 da revisão da PR #25)

O overlay só é planejado quando alguém pode vê-lo, **antes** de qualquer plano da cena
(`Workstation::overlay_refresh`), na ordem:

1. suporte e ligado (`overlay_enabled` = ligado **e** o runtime anunciou `overlay.gizmo`): sem o
   feature ou com o overlay desligado não há plano, nem `scene.gizmo.state`;
2. reprodução (`playback.running_frame().is_some()`): o overlay é **limpo uma vez**
   (`overlay.clear`, contador `held`) e nada é planejado por tick. Um gizmo parado sobre
   conteúdo em movimento seria falso; ao parar volta a ser calculado;
3. memo: `EditorState::gizmo_overlay_key` (carimbo do core + seleção + estado do arrasto:
   gesto, atualizações aceitas, morto) é pura e barata; chave igual à última calculada = nada a fazer
   (`memo_hits`);
4. só então `scene.gizmo.state` (`computed`).

`Workstation::execute` **adota** o `overlay` que a resposta de um comando de gizmo já traz
(`overlay_adopt`, contador `adopted`): o sync seguinte não planeja de novo, então um `move` custa um
plano (o do core), não dois. `PlanTimeline` não é usado: ele empresta a cena e não sobrevive a uma
edição (contrato do tipo), e o overlay é recalculado exatamente quando algo muda. O custo do plano no
core **não** foi escondido: a regra do core (DOM, `ScenePlan::build`) é a mesma; o que mudou é
**quando** o adaptador a chama. Os contadores ficam em `monitor.viewport.state.overlay`
(`computed`, `adopted`, `memo_hits`, `held`, `epoch`, `stale`, `sets`).

### Capacidade por transporte (M1)

O mesmo bundle `runtime/dist/runtime.js` serve o worker headless (Node/puppeteer, que usa
`window.__tesselPost`) e a WebView do desktop (`window.ipc.postMessage`). O overlay só é criado e só é
anunciado (`overlay.gizmo`) no contexto de **desktop**, ou numa página de teste que o peça por
`?overlay=gizmo` (`wantsOverlay`); a página headless não tem `#overlay`, não anuncia o feature e recusa
`overlay.*` com `unsupported`. Depois de cada frame, `refreshAfterFrame` roda **depois** de
`frame.done` e de `announcePresented`, só se o overlay está ativo (inativo = nenhum layout nem
canvas), e uma exceção do desenho vira `overlay.error` (código `overlay_failed`): nunca `frame.failed` nem o `error`
genérico, e `done`/`presented`/export/headless não regridem. O mesmo vale para o `overlay.receive` de uma página com
overlay operacional que recusa um payload (`overlay.set` inválido): `overlay.error` com `request` e código
`invalid_overlay`. `unsupported` continua sendo o `error` genérico só onde **não há superfície de overlay** (a
página headless); erros de protocolo reais (JSON inválido, versão, tipo desconhecido) e `frame.failed` seguem pelo caminho
de sempre.

**Medição proporcional (não alegada como "custo 0"):** `runtime/scripts/perf/run.mjs --engine chrome-swiftshader --suite bench
--cases image,scene --variants current,baseline --baseline-bundle <bundle da main> --passes 3 --frames 120` (sob o lock de GPU),
executada no head de código `85d6aa7` contra o bundle de `runtime/` extraído de `git archive 59b8efc` e construído fora do repositório.
Chrome 157 com SwiftShader (CPU, **não** a GPU da janela); 3 passadas A/B/A, 120 quadros, 0 erros, 60,0 fps nas 12 execuções.
O relógio do harness tem quantum de 0,1 ms. Resultado (média de `render` por passada, em ms): `image` baseline 0,222/0,252/0,266 e
atual 0,267/0,249/0,162; `scene` baseline 0,219/0,206/0,202 e atual 0,207/0,210/0,207; p50 de 0,2 ms em todos; p95 entre 0,3 e 0,6 ms.
**Leitura:** a diferença entre as variantes é menor que a oscilação entre passadas da mesma variante (a passada 3 de `image` atual foi a
mais rápida); nada acima da resolução do relógio foi medido. As páginas do harness usam `__tesselPost` e **não têm overlay** neste
head, então isto mede só o custo residual do portão de `refreshAfterFrame` (inativo = nenhum layout nem canvas): é evidência de que o
portão não regride o quadro de preview, **não** de que o desenho do gizmo custa zero. O custo do adaptador (planos por move/tick) **não**
foi medido em tempo: está provado em contagem pelos testes `a_drag_is_planned_once_per_move_by_the_core_and_never_again_by_the_sync`,
`a_state_the_page_already_has_is_not_planned_again` e `playback_clears_the_overlay_once_and_plans_nothing_per_frame`, com as mutações
correspondentes mortas (abaixo). Dados brutos e resumo: `/tmp/tessel-s8-finish-gates/perf/`.

### Arrasto morto

Um arrasto cujo gesto não pode mais ser aplicado (edição, undo, cena substituída ou desanexada no
meio) é **morto**: `EditorState::gizmo_drag_is_dead` o detecta e o adaptador o cancela
(`settle_gizmo_drag`, chamado a cada sync/tick/mensagem) sem esperar um evento do ponteiro. Também
é cancelado quando a WebView fica inativa (monitor oculto: `hidden`), quando a cena deixa de estar
no monitor ou a página perde o feature (`unavailable`) ou quando o playhead sai do frame do press
(seek). No core: `move` num arrasto morto é recusado com `gesture_invalidated` e **mantém o arrasto
aberto**; `up` e `cancel` o encerram e levam o gesto junto (nada fica bloqueando o próximo press).
O overlay de um arrasto morto desenha como "sem arrasto".

### Regra de passo da página

A página só desenha e só aceita ponteiro quando o aspecto da figura que está no canvas coincide com o
aspecto do overlay (`pictureInStep`, tolerância de um pixel do lado curto). Enquanto o canvas ainda guarda
uma figura de outra forma que a do overlay (a saída mudou de proporção e o frame ainda não chegou), ela não
desenha nem acerta nada; ao chegar o frame (`refresh`) volta a desenhar. Assim nenhuma coordenada é
traduzida por um aspecto errado.

### Como um arrasto vira edição

1. `down` num handle: o core guarda a **âncora** (raio do ponto de press contra o eixo/plano), o
   frame de edição, a matriz do nó e o `localPosition` do documento. Abre um gesto (F039).
2. Cada `move` recalcula tudo **a partir do press** (não acumula): eixo = ponto mais próximo
   entre raio e reta; `screen` = raio contra o plano com normal na direção da câmera. Casos
   degenerados (raio paralelo, atrás da câmera) são recusados ou ignorados sem escrever.
3. O delta de mundo vira delta **local**: `Δlocal = L · W⁻¹ · Δworld` (`L` parte linear da matriz
   local, `W` da mundial), somado ao `localPosition` do documento. Pai rotacionado/escalado não
   muda para onde o nó vai (teste com oráculo). Escala zero é recusada (`not_translatable`).
4. `up` confirma o gesto: **uma entrada de undo** para o arrasto inteiro, `changed` indica que o
   documento mudou (delta zero é no-op, sem entrada).

O ponto agarrado acompanha o ponteiro (não a origem do nó): apertar na ponta do eixo e mover o
ponteiro por Δ move o nó por Δ.

### Política explícita de Escape, foco e resize

| Evento | Política |
|---|---|
| Escape (página ou janela) | cancela o arrasto (`escape`); o resto do press (moves/release) é silencioso |
| Janela perde foco / `blur` | **cancela**, nunca confirma (um release pode nunca chegar) |
| Monitor oculto (outra aba, diálogo) / `visibilitychange` hidden | cancela (`hidden`); todo diálogo marca `Workflow::has_dialog`, que esconde a WebView |
| Cena sai do monitor / página sem `overlay.gizmo` / recarregada | cancela (`unavailable`); reload esquece o overlay |
| Seek: playhead diferente do frame do press | cancela (`api`) |
| Reprodução em curso | overlay limpo uma vez (`held`), sem plano por tick; o arrasto aberto é cancelado pelo seek do relógio |
| `pointercancel`, `lostpointercapture` com press aberto | cancela (`pointer_cancel`). O Chrome headless dirigido por CDP **não dispara** `lostpointercapture` depois de `releasePointerCapture` (provado em execução), então a página também confere `hasPointerCapture` no **próximo move e no release**: captura perdida = cancela, nunca `up` |
| Novo press com outro "aberto" | descarta o arrasto cujo release se perdeu e começa limpo |
| Resize da janela/painel durante o arrasto | **não termina**: coordenadas são da imagem; a página reenvia o layout |
| Edição externa/undo no meio do arrasto | `gesture_invalidated`: o candidato antigo nunca é aplicado, o arrasto morre |
| Cancel sem nada aberto | no-op (não é erro); `changed: false` |

### Precisão das respostas (S1)

- `scene.pointer.up` **sem arrasto vivo** é no-op (`noop: true`, `changed: false`) e **não** liquida
  um gesto de slider do inspetor que outro tenha aberto; só o release de um arrasto vivo do próprio gizmo
  (ou `scene.translate`, uma edição real) o faz. "Vivo" é um só critério, `SceneInteraction::live_drag`: o arrasto
  guardado cujo gesto ainda é o aberto. Quem o lê é o `pointer_up` (e `move`/`cancel`) **e** a regra de liquidação de
  `EditorState::execute` (`is_idle_gizmo_release`), então as duas não divergem: um arrasto cujo gesto outro comando encerrou
  (`scene.gesture.cancel`) fica guardado até o próximo comando de ponteiro, mas está morto, o release dele é no-op e **não**
  encerra o slider aberto depois (thread 4504060107 da PR #25). Um arrasto vivo, mesmo invalidado por uma edição
  (`gesture_invalidated`), continua liquidando como toda edição. Testes:
  `a_release_with_no_drag_does_not_commit_somebody_elses_slider_gesture`,
  `a_release_of_a_dead_drag_does_not_commit_somebody_elses_slider_gesture` (histórico, revisão e gesto do slider intactos),
  `a_release_of_a_live_drag_still_settles_the_slider_gesture`.
- `scene.pointer.move` responde `changed: true` quando um arrasto **está aberto e foi recalculado**,
  mesmo que o ponto não tenha sido aplicado (`applied: false`: raio degenerado, o candidato anterior
  fica). `changed` não diz "o documento mudou" (`document_changed` é sempre `false` num move); leia `applied`.
- `scene.gizmo.state` é uma consulta, mas **pode limpar o registro de um arrasto cujo gesto já não
  existe** (`live_drag`: o gesto foi liquidado ou descartado por outro caminho): nunca é uma edição do
  documento nem toca um gesto vivo.
- Os ramos `degenerate_axis` (press) e `degenerate_pointer` (move) são **defensivos**: um ponteiro real
  não os alcança pelos handles `x`/`y` nem pelo central (o centro vence perto do eixo `z`); estão
  provados em `Anchor` com o pixel cujo raio é paralelo ao eixo (`an_anchor_refuses_a_press_or_a_move_whose_eye_ray_runs_along_its_axis`).

### Erros tipados

`no_drag` (move/up sem arrasto), `not_translatable` (`details.reason`: sem `position`, escala zero,
não invertível), `gizmo_unavailable`, `invalid_value` (delta/ponto não finito), `gesture_active`
(outro gesto aberto, p.ex. slider do inspetor), `gesture_invalidated`, `unknown_entity`,
`unknown_element`. Um comando recusado deixa documento, histórico e revisão como estavam, **exceto** pela liquidação de um
gesto de slider aberto descrita em "Liquidação de slider aberto" (abaixo): ela acontece antes da recusa e não é desfeita.

### Fluxo Rust

```rust
state.execute(Command::Scene(SceneCommand::Gizmo(GizmoCommand::Pointer(
    PointerCommand::Down { scene: "street".into(), at: FrameAt::Frame(0), point, additive: false },
))))?;
state.execute(Command::Scene(SceneCommand::Gizmo(GizmoCommand::Pointer(
    PointerCommand::Move { point },
))))?;
state.execute(Command::Scene(SceneCommand::Gizmo(GizmoCommand::Pointer(
    PointerCommand::Up { point: Some(point) },
))))?; // uma entrada de undo
```

## CLI headless

Mesmas ops no `./scripts/headless` (JSONL), sem janela:

```json
{"op":"scene.select","scene":"street","entities":["scene/scene-a/node/figure"]}
{"op":"scene.gizmo.state","scene":"street","frame":0}
{"op":"scene.pointer.down","scene":"street","frame":0,"point":{"u":0.61,"v":0.58}}
{"op":"scene.pointer.move","point":{"u":0.66,"v":0.58}}
{"op":"scene.pointer.up","point":{"u":0.66,"v":0.58}}
{"op":"history.undo"}
{"op":"scene.translate","scene":"street","entity":"scene/scene-a/node/figure","frame":0,"delta":{"x":1,"y":0,"z":0}}
```

`scene.translate` produz o **mesmo documento** que um arrasto cujo `world_delta` seja o mesmo
(teste de igualdade).

## Rotação de um nó (G1, BL015.1)

Primeira fatia de G1 do [plano de gizmos](../migration/gizmos-spatial-editing-plan.md): **rotação de UM
nó**, com orientação local ou de mundo, reutilizando o gesto de F055 (begin/update/commit/cancel). Fica
dentro de F055 (nenhum ID novo): é a mesma superfície de seleção, overlay e arrasto com uma segunda
ferramenta. **Não entram** escala, snapping, seleção múltipla, grupos, luzes, câmera nem FX.

### Convenções (decididas antes do código, fixadas por testes)

| Tema | Regra |
|---|---|
| Representação | O documento continua com o quaternion unitário `localRotation` (propriedade `rotation`, `Mat4::compose` como no Three.js). Nenhuma representação nova, nenhum Euler gravado. |
| Unidade e sinal | Graus na API; regra da mão direita: `+90°` em torno de `+Z` leva `+X` a `+Y`. |
| Pivô | A origem do próprio nó (sua translação de mundo). Girar só altera `localRotation`: a origem não se move e os filhos giram junto. |
| Eixos | `world`: `x`, `y`, `z` da cena. `local`: os eixos do nó no mundo, `O·eᵢ` com `O = Pₙ·R` (rotação do pai vezes a rotação autorada, **sem** a escala própria): são unitários e ortogonais qualquer que seja a escala do nó. |
| Composição com o pai | `W = P·R·S`. Uma rotação de mundo `G` vira `W' = G·W`, isto é, `R' = Pₙᵀ·G·Pₙ·R`; num eixo local, `R' = R·Rot(eᵢ, θ)`. Só existe quando `P` é uma similaridade (rotação vezes **uma** escala); senão a recusa é tipada (abaixo). |
| Ângulo de um arrasto | Ângulo entre o press e o ponteiro, ambos projetados no plano do anel, em `(-180°, 180°]`, recalculado **a partir do press** a cada move (não acumula). Uma volta completa é um comando numérico ou vários arrastos. |
| Volta completa | O ângulo é reduzido módulo 360°; o que reduz a (quase) zero devolve o quaternion autorado sem mudança: **no-op, sem entrada de histórico**. |
| Anéis | Raio `RING_RADIUS = 0.10` altura de imagem (como as setas, comprimento na tela e não no mundo), polígono de `RING_SEGMENTS = 64` segmentos (65 pontos, o último repete o primeiro; `null` onde o anel passa atrás da câmera). Um anel visto de **borda** (o raio do olho quase perpendicular ao eixo: |cos| entre o raio e o eixo abaixo de 0,05) não determina ângulo: o press é recusado (`refused: degenerate_axis`) antes de abrir gesto. |
| Handle central | Não existe na ferramenta `rotate` (o centro é do `screen` da translação). |

### Comandos (também ops JSON)

| Op | Comando | Efeito |
|---|---|---|
| `scene.gizmo.tool` | `GizmoCommand::Tool` | lê e, com `tool`/`space`, define a ferramenta (`translate` padrão, `rotate`) e a orientação (`world` padrão, `local`). **Estado de sessão**, nunca uma edição: sem histórico, sem revisão. Recusada com `gesture_active` enquanto há arrasto vivo (pedir o que já está definido é leitura). Resposta: `tool`, `space`, `previous`, `tools`, `spaces`, `changed`, `document_changed:false`. |
| `scene.rotate` | `GizmoCommand::Rotate` | o mesmo resultado **sem ponteiro**: `entity`, `frame`/`time`, `axis` (`x`\|`y`\|`z`), `degrees` finito, `space` opcional (padrão: o da sessão) → `edit_scene` com **um** `set rotation`; uma entrada "Edit scene". Resposta: `axis`, `space`, `degrees`, `rotation` (o quaternion gravado), `changed`. |
| `scene.pointer.*` | `PointerCommand` | com `tool = rotate`, `down` num anel abre o gesto "Rotate", `move` faz `gesture_update` com **um** `set rotation`, `up` confirma (uma entrada de undo). Respostas trazem `tool`, `degrees`, `axis`, `space` e `rotation`. |
| `set monitor.gizmo tool\|space <valor>` | `parse_set` → o mesmo `GizmoCommand::Tool` | o nó `monitor.gizmo` da árvore (`state.tool`, `state.space`, `fields tool/space` do tipo `choice`) |

`scene.capabilities` passa a listar `scene.rotate` e `scene.gizmo.tool`. O arrasto e `scene.rotate` passam
**pelo mesmo** `RotationFrame::rotate`: o documento que um arrasto deixa é igual, bit a bit, ao de `scene.rotate`
com o `degrees`/`axis`/`space` que o arrasto reportou (teste de igualdade nas duas orientações).

### Overlay v1 (aditivo)

`tool` e `space` entram no overlay e no carimbo (`stamp.tool`, `stamp.space`): um press feito contra as setas nunca
age sobre anéis (nem sobre os eixos do outro espaço), e a chave de memo do adaptador muda com a ferramenta. Com
`tool = rotate`, cada handle tem `kind: "ring"` e `points` (65 pontos ou `null`); `metrics.ring_radius` dá o raio. Um
overlay do primeiro corte (sem `tool`) é lido como `translate`/`world`. Nó que não pode girar: `visible:false` e
`reason` com o motivo tipado (nunca um gizmo que depois recusa o press).

### Recusas tipadas (`not_rotatable`, `details.reason`)

Um comando recusado deixa documento, histórico e revisão como estavam, **exceto** pela liquidação de um gesto de slider aberto
("Liquidação de slider aberto", abaixo).

| `reason` | Quando |
|---|---|
| `animated_rotation` | um canal de performance escreve a rotação deste nó **no frame de edição** (também com valor zero). A rotação base não é a mostrada e uma edição seria deslocada pela animação; **nenhuma chave é criada em silêncio**. Pai animado, e nó com canal só de posição ou escala, **podem** girar: o `W' = G·W` vale no frame. |
| `non_uniform_parent_scale` | a cadeia de pais escala ou cisalha de forma desigual: nenhuma rotação local gira o nó em torno de um eixo de mundo, e um nó TRS não guarda cisalhamento |
| `mirrored_parent` | a cadeia de pais espelha (determinante negativo): o sentido da rotação inverteria |
| `degenerate_transform` | a matriz local ou de mundo não é invertível (escala zero) |
| `degenerate_rotation` | o quaternion autorado não é finito nem diferente de zero |
| `not_movable` | guarda defensiva da regra de translação; todos os tipos de nó atuais (`Group`, `ImagePlane`, `Splat`) são móveis, portanto este ramo não é alcançável com o catálogo atual |

Outros erros: `invalid_value` (eixo `screen`/desconhecido, graus não finitos, ferramenta ou orientação desconhecida),
`unknown_entity`, `gesture_active`, `gesture_invalidated`, `no_drag`. **Spark:** o plano já recusa, com
`unsupported`, um splat cuja matriz de mundo não é uma similaridade (`PlanError::SplatTransform`); a rotação nunca
contorna essa regra, e uma rotação aplicada a um splat com escala uniforme mantém a matriz uma similaridade (teste).
A **escala própria** não uniforme ou negativa do nó é permitida (a rotação é aplicada depois da escala no `compose`)
e não é tocada.

### Animação e a rotação do documento

A avaliação converte o quaternion em Euler XYZ, **soma** os canais de rotação e reconstrói. Por isso uma rotação
animada não pode ser editada como base sem criar um deslocamento invisível; esta fatia a recusa explicitamente
(`animated_rotation`) em vez de gravar chaves ou ignorar a animação. Girar um nó animado exigiria editar a chave
(fora desta fatia, BL015.1).

### Equivalências e superfícies

| Superfície | Como |
|---|---|
| Biblioteca | `GizmoCommand::Tool` / `GizmoCommand::Rotate` e `PointerCommand` via `EditorState::execute`; regras em `scene_interaction::gizmo::rotation` (aritmética pura, sem GPUI) e `scene_project::gizmo` (glue) |
| CLI headless | as ops acima em `./scripts/headless` (JSONL), sem janela |
| Janela | botões **Mover**/**Girar** (e **Mundo**/**Local** com Girar) no cabeçalho do monitor, cada um um `Command` `scene.gizmo.tool` com probe `control:monitor-gizmo-*`; anéis desenhados no canvas da página; o adaptador só traduz `overlay.pointer` |

### Limites desta fatia (continuam pendentes em BL015.1)

Escala, snapping, múltipla seleção e pivôs de grupo, luzes, câmera espacial e FX; **translação em eixos locais**
(a translação segue em coordenadas de mundo; o espaço só vale para a rotação); rotação de um nó cuja rotação é
animada (edição de chave); pais com escala não uniforme (recusa tipada, não suporte); o mouse **físico** na WKWebView
(BL015.2/BL043/BL044).

## UI e CLI da janela

- **Canvas**: a página cria `<canvas id="overlay">` acima de `#viewport` (`pointer-events: none`
  até `overlay.enable`). Desenha contorno do plano selecionado, setas dos eixos, handle central e
  destaca o handle ativo/sob o cursor (`cursor: grab`).
- **Ponte v1, aditiva**: página→host `overlay.pointer` (`down|move|up|cancel`, `seq`,
  `point`, `additive`, `reason`), `overlay.layout`, `overlay.drawn` (ack opt-in com
  amostras de pixel por handle, `TESSEL_VIEWPORT_OVERLAY_ACK=1`); host→página `overlay.enable`,
  `overlay.set`, `overlay.clear`. Só é enviada a runtimes que anunciam o feature
  **`overlay.gizmo`** em `ready.capabilities.features` (runtime antigo nunca recebe `overlay.*`).
- **Adaptador** (`crates/desktop/src/workstation/scene_gizmo.rs`): cada `overlay.pointer` vira um
  `scene.pointer.*` por `scene_execute` (mesmo caminho dos demais gestos); depois de qualquer
  comando o overlay é recalculado (`scene.gizmo.state`) e reenviado só se mudou. Seleção na árvore,
  valor digitado no inspetor, undo e arrasto redesenham igual. Escape, perda de foco e monitor
  oculto cancelam também pelo lado da janela.
- **Moves** são coalescidos por frame de animação na página; o release carrega a posição final.
- **Árvore** (`ui tree`): `monitor.gizmo` (só com cena anexada; estado `selection`, `dragging`,
  `handle`, `gesture`, e com BL015.1 `tool` e `space`, mais os campos `tool`/`space`) e
  `monitor.viewport.state.overlay` (suportado, ligado, contadores, último layout e último `drawn`). O gizmo em si
  não tem botão (é um canvas); a escolha da ferramenta tem (BL015.1) e o equivalente semântico do arrasto é
  `./scripts/ui op '{"op":"scene.pointer.down",…}'`. O resumo (`summary.scene_interaction.drag`)
  mostra o arrasto aberto.
- **Ponteiro físico**: ops semânticas **não provam** um mouse real; a prova com o mouse físico na
  janela nativa continua em BL043/BL044.

## Evidências e status

Testes **declarados** no código (contados na fonte). A coluna "Execução" é a única que vale como prova; o bloco "Execução no head"
registra o que de fato rodou, com o manifesto `/tmp/tessel-s8-finish-gates/manifest.txt` (comando, cwd, HEAD e árvore, ambiente, UTC,
disco, saída, sha256 de cada log e das fontes, dos scripts e dos binários). A rodada anterior continua em
`/tmp/tessel-s8-final-gates/manifest.txt` (preservada, com as falhas de então).

| Superfície | Teste declarado | Execução (head de código `85d6aa7`, árvore limpa) |
|---|---|---|
| Biblioteca/core | `crates/core/tests/scene_gizmo.rs` (38 testes em `85d6aa7`; **41** em `db02923`, com os 3 do critério de arrasto vivo; **42** em `fb268bb`, com `a_dead_drag_does_not_act_on_a_gesture_opened_after_its_own_ended`): geometria contra oráculo próprio da câmera, arrasto ≡ `scene.translate`, uma entrada de undo, pai rotacionado/escalado, recusas invariantes, seleção por ponteiro, políticas de cancelamento, chave de memo do overlay, ramos `degenerate_axis`/`degenerate_pointer` em `Anchor`, fronteira de `OUTSIDE_LIMIT`, `replaced_by_press`, release sem arrasto que não liquida slider alheio (S1), move em arrasto morto recusado onde o ponteiro não resolve | **38/38** em `85d6aa7` (1556 passaram, 0 falharam, 12 `ignored`); **41/41** em `db02923` (`cargo test --workspace --no-fail-fast --locked`: **1562 passaram, 0 falharam, 12 `ignored`**, `TESSEL_REQUIRE_RUNTIME=1`, sob o lock de GPU); **42/42** em `fb268bb` (`cargo test -p tessel-editor-core --test scene_gizmo`, controle da rodada 2; o workspace inteiro não foi repetido com o teste novo) |
| CLI headless | `a_json_lines_session_runs_the_whole_gizmo_flow_in_process` (`headless::run_session`, sem janela) e `scripts/scene-gizmo-proof.py --surfaces headless` | o teste em processo roda no `cargo test` (ok); **prova headless 56/56** com o binário `tessel-headless` do head (hash no manifesto), `final_evidence: true` na execução das duas superfícies em `85d6aa7`; **de novo 56/56** com o `tessel-headless` construído do head `964859e` (rodada 1) e do `fb268bb` (rodada 2), `final_evidence: true` (árvore limpa, sem `--allow-dirty`) |
| Página/canvas | `runtime/test/overlay.test.mjs` (11, puros) e `runtime/test/gizmo-overlay.test.mjs` (15, Chrome real): mouse do puppeteer, pixels do canvas, Escape, blur, hidden, `pointercancel`, **captura perdida de verdade** (`releasePointerCapture`, com e sem move antes do release), botão direito/médio e segundo ponteiro, resize, hover, overlay inválido recusado inteiro; **M1**: overlay inativo sem layout/canvas, falha do desenho isolada do frame, página headless sem overlay | puros **11/11** (repetido no head final; em `db02923` também 11/11 e Chrome do gizmo **16/16**, suíte completa **175/175**, `skipped 0`); Chrome do gizmo **15/15** e suíte completa do runtime **174/174**, `skipped 0`, sob o lock de GPU (rodada anterior, árvore `aabb589`): as fontes e o `runtime/dist/runtime.js` do head final têm os **mesmos sha256** do manifesto anterior, então o Chrome não foi repetido |
| Host/ponte | `viewport_host_tests` (52): portões e contadores do overlay, reload = época nova | **52/52** (parte dos 280 do desktop no `cargo test`) |
| Janela (GPUI) | `scene_gizmo_tests` (28): mensagem → comando único, Escape/foco/oculto, mensagem velha silenciosa, invalidação, **M2/M4**: runtime sem feature/overlay desligado/reprodução sem plano, adoção da resposta, uma computação por move, época e seleção no carimbo | **28/28** no `cargo test --workspace` do head final |
| Janela real (binário próprio, HOME curto, só `get`/`set`/`invoke`/`op`) | `scripts/scene-gizmo-proof.py` (superfície nativa); a lógica do script (M3) tem `scripts/scene-gizmo-proof.spec.py` (32) | **prova nativa 84/84** com o `tessel-editor` do head e o runtime construído (`final_evidence: true`: ambas as superfícies, árvore limpa, sem `--allow-dirty`); spec **32/32** em `85d6aa7` (**34/34** em `db02923`); **de novo 84/84** com o `tessel-editor` construído do head `964859e` (rodada 1) e do `fb268bb` (rodada 2), binário próprio, `HOME` curto próprio, só `get`/`set`/`invoke`/`op`, sem coordenadas e sem a janela do usuário, `final_evidence: true`, `surfaces_agree_on_the_drag: true` (`proof-final.json` de cada rodada) |
| Ponteiro físico, foco do teclado e captura sobre a WKWebView | — | **não observado** (BL043/BL044 seguem a validar; as ops `scene.pointer.*` são semânticas, não um mouse) |

**Mutações direcionadas (executadas).** 15 mutações de lógica, uma por vez, em árvore limpa, com o arquivo restaurado por `git checkout` e hash/árvore conferidos
depois (`/tmp/tessel-s8-finish-gates/mutations.jsonl`, um log por mutação em `logs/mut-*.log`, diff aplicado no cabeçalho do log; um registro "morta" exige
testes executados, falha do comando e o teste esperado entre os que falharam): `degenerate_axis`, `degenerate_pointer`, `OUTSIDE_LIMIT`, `replaced_by_press`,
S1 (`is_idle_gizmo_release`), a checagem de vida no `pointer_move`, o portão `negotiated` do `overlay_pointer`, a retenção na reprodução, o memo, o portão de
suporte do `overlay_refresh`, a adoção da resposta, `epoch` e `selection` do carimbo (Rust); o guarda de `active()` e o isolamento do `throw` em
`refreshAfterFrame` (JS). **15 mortas pelo teste certo** no head final. **Uma sobreviveu na primeira rodada** (a checagem de vida no começo de `pointer_move`
removida: nenhum teste falhou) e **não era equivalente**: num arrasto morto cujo ponteiro não resolve (raio paralelo ao eixo), a resposta era um `applied:
false` quieto em vez de `gesture_invalidated`. O teste `a_move_on_a_dead_drag_is_refused_even_where_the_pointer_resolves_to_nothing` foi acrescentado e mata
essa mutação (e a de `degenerate_pointer` também). O log da primeira rodada dessa mutação foi sobrescrito por minha reexecução com o mesmo nome (o registro
`SURVIVED` e o sha256 do log antigo seguem em `mutations.jsonl`; as demais rodadas passaram a ter nome único). Mutações anteriores preservadas: as 11 da prova Python
(M3) em `/tmp/tessel-s8-m1-m4-evidence/logs/m3-mutations.log` e a do guarda de captura perdida do `overlay.js` em `/tmp/tessel-s8-final-gates/logs/`.

### Threads CodeRabbit da PR #25 (head de código `db02923`)

Execução em `/tmp/tessel-s8-coderabbit-final-gates/manifest.txt` (comando, cwd, HEAD e árvore, UTC, disco, ambiente, sha256 dos logs, das fontes e do bundle). As provas
da rodada `85d6aa7`/`cf2aa7e` seguem como estavam e **não substituem** as deste head.

| Thread | Correção | Prova executada neste head | Limite |
|---|---|---|---|
| 4504060107 (S1, arrasto morto) | um critério, `SceneInteraction::live_drag`, lido por `pointer_up` e por `is_idle_gizmo_release` | core `a_release_of_a_dead_drag_does_not_commit_somebody_elses_slider_gesture` (gesto do slider, histórico e revisão intactos; o slider commita a própria entrada), `a_release_of_a_live_drag_still_settles_the_slider_gesture`, `a_release_of_a_live_but_stale_drag_settles_the_slider_and_is_refused` (invalidação preservada), `a_gesture_settled_elsewhere_kills_the_drag_without_reviving_it` (recusa `no_drag` preservada); core 41/41 | a mutação `rust-settle-rule-reads-stored-drag` (volta a regra para `drag().is_none()`) foi executada depois e **morta** pelo teste certo nas duas rodadas; `rust-live-drag-never-live` (`live_drag` nunca vivo) morta (24 testes, entre eles `a_release_of_a_live_drag_still_settles_the_slider_gesture`); a 4ª, extra, `rust-live-drag-always-live`, **sobreviveu na rodada 1** (teste novo, ver abaixo) |
| 4504060115 (docs, "três superfícies") | o texto separa as 3 superfícies do contrato (lib, CLI headless, UI) dos 2 processos da prova (`headless`, `native`); a lib é provada pelos testes do core, não por um terceiro processo | leitura contra `SURFACES`/`final_blockers` | só texto |
| 4504060122 (`overlay.error`) | `refreshAfterFrame` já usava `overlay.error`; agora também o `overlay.receive` com overlay operacional recusando um payload (`request`, `invalid_overlay`); `unsupported` só sem superfície de overlay; erros de protocolo e `frame.failed` inalterados; o adaptador guarda `request` no relatório | Chrome 16/16 (`an overlay payload the page refuses is its own error…`, antigo teste de payload recusado agora espera `overlay.error`, headless ainda `unsupported`); GPUI com host **`Ready` pelo handshake real** (`an_overlay_error_is_consumed…`, `an_overlay_the_page_refused_is_consumed…`, controle `a_real_protocol_error_still_fails_the_render_session`); mutações JS 3/3 mortas | o GPUI foi compilado/rodado no workspace do head (`cargo test --workspace`); a mutação `rust-overlay-error-goes-to-the-session` (o braço `overlay.error` do adaptador passa a não casar) foi executada depois e **morta** pelos 2 testes esperados nas duas rodadas |
| 4504060124 (manifesto da spec) | a spec lê o AST do script e confere o primeiro argumento literal de cada chamada `check(...)`, não o texto (que contém `REQUIRED`) | spec 34/34, 2 testes novos de regressão; mutações `py-check-renamed-without-the-manifest` e `py-check-call-removed-from-the-scenario` mortas pelo teste certo (34 testes executados) | — |

### O que o disco bloqueou, executado depois (BL015.6)

A rodada dos threads não pôde compilar o head (livre 14,86 GiB < 16 de entrada). Uma rodada contínua nova, com 16 GiB de reserva de entrada para a rodada inteira e piso
absoluto de 12 GiB vigiado durante todo o pipeline (só mataria os processos da própria rodada; nunca foi atingido), tudo sob **um** lock de GPU tomado no início (sem
lock duplo; o Cargo por `cargo-serial.py`), rodou no head limpo da PR. Entradas: `/tmp/tessel-s8-last-proof-gates/manifest.txt` (rodada 1, head `964859e`, entrada 18,94 GiB, mínimo 18,05) e `/tmp/tessel-s8-last-proof-gates/round2/manifest.txt` (rodada 2,
head `fb268bb`, entrada 17,93 GiB, mínimo 17,74). Cada gate registra comando, cwd, HEAD e árvore, sujo, ambiente, UTC, disco antes/depois/mínimo, saída e sha256 do log.

| Item | Rodada 1 (`964859e`) | Rodada 2 (`fb268bb`) |
|---|---|---|
| controle sem mutação (core `scene_gizmo`; desktop `scene_gizmo_tests`) | 41/41 e 31/31, os testes esperados `ok` | 42/42 e 31/31, idem |
| `rust-settle-rule-reads-stored-drag` | morta por `a_release_of_a_dead_drag_does_not_commit_somebody_elses_slider_gesture` | morta, mesmo teste |
| `rust-live-drag-never-live` | morta (24 testes, com `a_release_of_a_live_drag_still_settles_the_slider_gesture`) | morta (25 testes, com o teste novo) |
| `rust-overlay-error-goes-to-the-session` | morta pelos 2 testes `an_overlay_error_is_consumed…` e `an_overlay_the_page_refused_is_consumed…` | morta |
| `rust-live-drag-always-live` (extra) | **SOBREVIVEU** (log e registro mantidos) | morta por `a_dead_drag_does_not_act_on_a_gesture_opened_after_its_own_ended` |
| fmt, `diff --check`, clippy `-D warnings`, `cargo tree` do core | não repetidos (verdes em `db02923`) | passaram |
| build de `tessel-editor` e `tessel-headless` (uma só vez, após as mutações, árvore limpa) | passou; binários copiados na hora para `bins/` | passou; idem |
| `scene-gizmo-proof.py` (headless + nativo, sem `--allow-dirty`) | passou: headless 56 checks, nativo 84, `final_evidence: true` | idem |

**O mutante sobrevivente era uma lacuna de teste, não equivalente.** `live_drag` devolvia o arrasto quando *qualquer* gesto estava aberto (`gesture.id() == drag.gesture || true`); os
testes de arrasto morto só tinham nenhum gesto ou um gesto de slider (que é outro mecanismo), então nada notava. Com um gesto de cena aberto por `scene.gesture.begin` depois do
cancelamento do arrasto, esse arrasto morto agiria sobre o gesto alheio. O teste novo cobre exatamente isso (mover é `no_drag`, soltar é `noop`, o outro gesto continua aberto,
documento, histórico e revisão intactos); o código de produção já estava certo e não mudou.

**Mesmas fontes, binários próprios.** A rodada 2 muda só `crates/core/tests/scene_gizmo.rs` em relação à 1: as árvores de `crates/core/src`, `crates/desktop`, `crates/media`,
`crates/render`, `crates/audio`, `runtime`, `scripts`, `Cargo.toml` e `Cargo.lock` são idênticas às de `964859e`. Os dois builds deram o mesmo sha256 aqui (observação, não
promessa de reprodutibilidade). A prova vale para os binários copiados no manifesto; o commit de documentação que vem depois só altera `docs/`.

**Não provado (nada disto é declarado):** ponteiro físico, foco do teclado e captura do gizmo sobre a WKWebView, nem pixels sobre ela (BL043/BL044, `not_proven` do relatório da prova); o
workspace inteiro e o Chrome não foram repetidos com o teste novo (as fontes de produção e de runtime são as mesmas de `db02923`); o oráculo da S10 e as tolerâncias não foram tocados.

### Falhas encontradas ao executar (e corrigidas, sem relaxar contrato)

Esta árvore nunca havia sido compilada com os testes: a execução achou o que a leitura não achou.

1. **Três testes GPUI falhavam** (`a_press_with_the_overlay_off_or_not_negotiated_selects_nothing_and_is_counted`,
   `a_press_with_the_overlay_off_does_not_even_change_the_selection`, `an_overlay_that_is_off_does_not_plan_the_scene_either`).
   Causa: o teste escondia o monitor escrevendo `viewport.on_screen = false`; o render que segue qualquer `update` recalcula
   `on_screen` a partir do layout (`update_viewport_visibility`) e religava o overlay antes da asserção. Defeito do **teste**, o código
   do adaptador estava certo. Correção: `monitor_hidden_by_dialog` esconde pelo caminho real (diálogo, render). Os contratos
   (sem plano, sem seleção, press contado) não foram afrouxados. Rodada anterior (`/tmp/tessel-s8-final-gates/logs/cargo-test.log`).
2. **Um teste de Chrome falhava** (`a lost pointer capture … cancels the drag`): o Chrome headless dirigido por CDP não dispara
   `lostpointercapture` após `releasePointerCapture` (sonda em `/tmp/tessel-s8-final-gates/probe-capture.mjs`), então a página continuava emitindo `move` e `up`,
   que o core confirmaria. Defeito **real** da página: `overlay.js` confere `hasPointerCapture` no próximo move e no release e cancela (`pointer_cancel`). Teste
   novo para o release sem move; o guarda do release foi mutado e o teste novo falhou.
3. **`cargo clippy --workspace --all-targets -- -D warnings` falhava** (3 diagnósticos, defeitos de estilo reais, sem mudança de comportamento):
   `clippy::question_mark` em `pointer_move` (`scene_project/gizmo.rs`), e `useless_conversion` e `neg_multiply` em `crates/core/tests/scene_gizmo.rs`. Corrigidos
   (commit `fix(gizmo): Satisfy clippy…`). Logs: `cargo-clippy.log`, `cargo-clippy-r2-explore.log` (falhas) e `cargo-clippy-final2.log` (passou).
4. **A primeira prova nativa falhou** (`and shows the initial picture again`, 33 checks, `proof-native-and-headless.log`, relatório `proof-native-and-headless-FAIL1.json`):
   defeito do **script**, não do produto. A linha de base de imagem era tirada logo depois de `monitor.show_scene`, no playhead da primeira exibição, mas o arrasto, o
   commit e o undo acontecem no frame de edição; o `undo` correto nunca igualaria aquela imagem e o check "a imagem seguiu o candidato (o hash saiu da inicial)" era
   verdadeiro pelo motivo errado (mudança de frame, não o arrasto). A linha de base passa a ser tirada depois do seek convergido (`record_baseline_picture`;
   a primeira exibição fica em `shown_hash`). Quatro testes da spec fixam isso e **falham contra o script anterior**. Reexecução: 84/84.
5. **Uma mutação sobreviveu** (acima): corrigida por teste novo.

### Execução no head

Cabeçalho do manifesto (`/tmp/tessel-s8-finish-gates/env.txt`): HEAD inicial `89e4147` (árvore `2001010681ad…`, igual ao head da PR), base `main` `59b8efc` (a `main` viva continuava
`59b8efc`: nada a integrar), `rustc 1.99.1`, `node v22.23.3`, Python 3.14.3, `TESSEL_REQUIRE_RUNTIME=1`, Cargo por `cargo-serial.py`, lock de GPU
(`with-gpu-lock.py`) antes de **todo** gate que usa Chrome/runtime/janela, sem aninhar. Disco livre na entrada 19,43 GiB (≥ 16 para os pesados), mínimo durante as rodadas
16,19 GiB (piso 12); nada foi limpo nem apagado (só os 549 MiB de ativos do perf, gerados por mim). O checkout T3 estava em `2401037` (não ancestral): o commit foi preservado em
`backup/t3-66247c45-heritage-2401037` e a branch reorientada com `git reset --keep` em árvore limpa.

Commits de código desta rodada: `fix(gizmo): Satisfy clippy…`, `fix(gizmo): Compare the native pictures…`, `test(gizmo): Pin the dead-drag refusal…` (head de código `85d6aa7`).
Os commits seguintes são só documentação (a equivalência de árvore fora de `docs/` está no manifesto).

| Verificação (head de código `85d6aa7`, salvo indicação) | Resultado |
|---|---|
| `git diff --check 59b8efc HEAD`, `cargo fmt --all -- --check` | passaram |
| `scripts/scene-gizmo-proof.spec.py` | 32 testes, passou |
| `node --test runtime/test/overlay.test.mjs`, `node --check` do teste de Chrome | 11/11; ok |
| `./scripts/runtime setup` (checkout novo, sem `dist`) | passou; `runtime/dist/runtime.js` com o **mesmo sha256** do manifesto anterior |
| `cargo clippy --workspace --all-targets --locked -- -D warnings` | **passou** (depois dos 3 diagnósticos acima; primeira rodada falhou) |
| `cargo test --workspace --no-fail-fast --locked` (lock de GPU, `TESSEL_REQUIRE_RUNTIME=1`) | **passou**: 1556 passaram, 0 falharam, 12 `ignored` declarados (medições); core 285 unit + 38 do gizmo, desktop 280 (28 do gizmo, 52 do host), render com Chrome real |
| `cargo build -p tessel-editor -p tessel-editor-core --locked` | passou; os binários do head (`bins-final/`) têm **sha256 idêntico** aos de `fc999ef` (`bins/`): o diff de `fc999ef` a `85d6aa7` só toca testes e `scripts/` |
| `cargo tree -p tessel-editor-core` | só `flate2`, `serde`, `serde_json`: sem `gpui`/`wry` |
| `scripts/scene-gizmo-proof.py` (ambas as superfícies, `final_evidence: true`) | headless **56/56**, janela real **84/84** (HOME curto próprio, só `op`/`get`/`invoke`) |
| medição M1/M2 (`perf/run.mjs`, baseline da `main`) | sem diferença acima da resolução do relógio (ver "Custo do overlay") |
| mutações Rust/JS | 15/15 mortas no head final (acima) |

**Limites declarados dessa execução.** (a) Os binários usados pela prova foram construídos no head `fc999ef`; o hash idêntico no head final os atesta. (b) A prova nativa usa
requisições semânticas; **nenhum ponteiro físico, foco do teclado nem captura de tela do gizmo sobre a WKWebView foi observado** (BL043/BL044). (c) O Chrome do gizmo (15/15) e a suíte
do runtime (174/174) não foram repetidos: as fontes JS e o bundle são byte a byte os da rodada anterior (sha256 no manifesto). (d) A medição de perf roda em SwiftShader, não na GPU da janela.

Status: **a validar**. Implementado e agora executado em quase todas as superfícies pedidas: core 38/38, host 52/52, janela GPUI 28/28, workspace 1556/0, clippy `-D warnings`, prova semântica
headless 56/56 e da janela real 84/84, Chrome 15/15 e runtime 174/174, 15 mutações mortas. **Faltam** a prova com ponteiro físico, foco do teclado e captura do gizmo sobre a WKWebView
(BL015.2, BL043/BL044) e o veredito do coordenador (revisão nova depois destes gates). Limitações: um nó por vez; sem escala nem snapping; a rotação tem anéis em coordenadas de mundo **ou** locais, mas a translação
continua só em eixos de mundo; rotação animada e pais com escala não uniforme são recusados (tipado); aspecto da imagem vem da câmera da cena.

Harmonização: **S4** (identidade de ativos) — o gizmo só endereça nós pelo `entity` do F039
(`scene/<cena>/node/<id>`), sem tocar slots/assets; **S6** (propriedades) — usa a propriedade
`position` do catálogo existente via `EditOp::set`, então um catálogo ampliado vale sem mudança;
**S7** (Spark) — nenhum acoplamento: planos de imagem são o único alvo de seleção por ponteiro.

Veja [arquitetura](../core-architecture.md), [interação de cenas](scene-interaction.md) e
[template](_template.md).

Uma falha do overlay (o desenho depois de um frame, ou um payload que a página recusou) é informada por `overlay.error`
e **consumida pelo adaptador da janela fora da sessão de render** (`viewport_ipc` → `overlay_note`, antes de
`ViewportHost::on_ipc`): fica em `get monitor.viewport` → `state.overlay.error` (o último erro desde o último reload, com `request` quando veio de
uma mensagem do host) e não vira `BackendClosed`/`backend_unavailable`, não apaga capacidades nem ativos. O `error` genérico continua
fechando a sessão, como um erro de protocolo deve.

### Ajustes após revisão G1 (PR #40)

Rotação permanece **a validar** até gates do head final. O teste de quaternion quase
unitário aceita e normaliza o valor; seu nome foi corrigido para refletir isso.
O teste puro de normalização cobre zero, NaN e infinito, mas não comprova que esses
valores atravessam o importador até `RotationFrame::of`. `degenerate_rotation` é
uma guarda defensiva; `not_movable` não é alcançável pelos tipos atuais de nó.
Nenhum desses ramos é anunciado como recusa exercitada por integração.

### Liquidação de slider aberto por `scene.rotate`/`scene.translate` (regra existente, sem atomicidade)

`EditorState::execute` assenta (`settle_property_gesture`, como se o slider fosse solto) um gesto de slider do inspetor ainda
aberto **antes** de aplicar qualquer comando que tenha `history_label`: `scene.rotate`, `scene.translate`, o release de um
arrasto vivo, `scene.edit`. A liquidação acontece antes de o comando ser validado, então um `scene.rotate` ou `scene.translate`
que depois é **recusado** (`not_rotatable`, `not_translatable`, `invalid_value`, `unknown_entity`…) já liquidou o slider e essa
entrada de histórico (se o slider tinha tocado o valor) **permanece**. O texto "um comando recusado deixa documento, histórico e
revisão como estavam" vale, portanto, em relação ao estado **depois** dessa liquidação; ele só é literal quando nenhum slider está
aberto. Não há promessa de atomicidade entre "liquidar o slider" e "aplicar o comando": a regra é anterior à G1 (S1 da PR #25,
`EditorState::execute`/`is_idle_gizmo_release`), não foi alterada por esta PR e foi lida no código; **nenhum teste executado nesta
rodada a exercita para a rotação** (o `cargo test` ficou bloqueado, ver abaixo). `scene.gizmo.tool` não tem `history_label`
(não liquida), e um release sem arrasto vivo é no-op e também não liquida.

### Rodada de validação do head `ebd1f08` (PR #40): gates pesados BLOQUEADOS

Head remoto conferido por `gh pr view 40` e `git fetch`: `ebd1f08f471c84027df7a796a6dbdc56545624c9` (árvore
`17450f4457992dcd8f75141133d33f6b034a0a96`), base `origin/main` `3e48861e9914ed2fd6c71e13c62c5673f068768f` (= merge-base), PR #39 ativa
(`f8c4a7e`). Evidência em `/tmp/tessel-pr40-gates` (fora do Git; `SHA256SUMS.txt`, `manifest.jsonl` com comando, cwd, ambiente, head,
árvore, UTC, saída, disco antes/depois e sha256 de cada log; **os logs de falha e a causa do bloqueio foram preservados**).
Wrapper conferido: `scripts/cargo` → `scripts/mbx`, que só define `CARGO_TARGET_DIR` se `TESSEL_CARGO_LANE` estiver no ambiente (não estava);
nenhum `CARGO_*`/`TESSEL_*` no ambiente e `.mbx.toml` sem `target-dir`.

**Bloqueio.** Uma única tentativa **não bloqueante** do lock compartilhado de GPU/build (`fcntl.flock(LOCK_EX|LOCK_NB)` em
`/tmp/tessel-scene-gpu.lock`, `tools/holder.py`) falhou em 2026-11-10T16:34:00Z: o lock estava com outro processo
(`scripts/real-latency-ab.py`, pid 14486, e um `scene-fx-ui-proof.py` do `with-gpu-lock.py` já na fila, pid 18543). Sem nova tentativa,
sem espera, sem contorno, nenhum processo alheio tocado. Disco livre na entrada 62 GiB (reserva 16, piso 12: ok).

| Gate no head `ebd1f08` | Resultado |
|---|---|
| `git diff --check 3e48861 HEAD` | passou (exit 0) |
| `cargo fmt --all -- --check` (via `./scripts/cargo`) | passou (exit 0) |
| `node --test runtime/test/overlay.test.mjs` (puro) | **14/14**, 0 pulados |
| `scripts/scene-gizmo-proof.spec.py` (S8, módulo carregado pela prova G1) | 34/34 |
| `node --check runtime/test/gizmo-overlay.test.mjs`, `py_compile` da prova G1 | ok (só sintaxe, **não** executa o teste) |
| **fail-closed da prova G1** (`tools/failclosed.py`, sem GPU/janela/build) | **16/16**: binário ausente, binário que sai, binário que diz ok sem provar, árvore suja sem `--allow-dirty` (recusada antes de qualquer superfície), `--surfaces` vazio/repetido/desconhecido; em todos o relatório fica `ok:false`, `final_evidence:false` e um `ok:true` velho é sobrescrito; `final_blockers` (subconjunto, `--allow-dirty`, árvore suja) e `missing_checks`; cada nome de `REQUIRED` é primeiro argumento literal de um `check(...)` por AST |
| mutações JS 3/3 contra o teste puro | `js-ring-gap-not-skipped` e `js-rotate-has-a-centre-handle` mortas por `hover on the rings…`; `js-ring-polygon-too-short-accepted` morta por `a rotate overlay carries the tool, the space and the polygon…`; arquivo restaurado, sha256 e árvore iguais (`js-mutations.json`) |
| leitura estática do D1 | `degenerate_authored_quaternions_cannot_be_normalised` cobre `quat_unit` com zero, `NaN` e `∞` (puro); o teste de integração quase unitário agora se chama `a_near_unit_authored_quaternion_is_normalised_when_rotated`. **Não compilado nem executado aqui** |

**Não executado (BLOQUEADO, nenhum é PASS):** `cargo check --workspace --locked`, `clippy --workspace --all-targets --locked -- -D warnings`,
`cargo test --workspace --locked` com `TESSEL_REQUIRE_RUNTIME=1`, os alvos de rotação (`scene_gizmo_rotation`, os 4 unitários de
`rotation.rs`, `scene_gizmo_tests`), `cargo build -p tessel-editor` e `cargo build -p tessel-editor-core --bin tessel-headless`
(o `tessel-headless` é um binário do pacote `tessel-editor-core`, não de `tessel-editor`; um `-p tessel-editor --bin tessel-headless` falharia),
`cargo tree -p tessel-editor-core`, `./scripts/runtime setup` e os testes de Chrome (`gizmo-overlay.test.mjs`, suíte do runtime),
`scripts/scene-gizmo-rotation-proof.py` nas duas superfícies (**nunca executado**), as mutações Rust discriminantes da rotação
(sinal, ordem `R'`, espaço local, pai `Pₙᵀ`, redução módulo 360°, volta completa, rasante, `animated_rotation`, `mirrored_parent`,
`non_uniform_parent_scale`, recusa da ferramenta sob arrasto, carimbo `tool`/`space`, `quat_unit` zero) e a medição do overlay dos anéis visível/oculto.
`tools/measure-core-overlay.py` (custo de `scene.gizmo.state`, anéis vs oculto vs setas) e `tools/measure-page-rings.mjs`
(`overlay.set` em Chrome real, anéis vs oculto vs setas, com e sem leitura do canvas) estão **escritos e nunca executados**; quando rodarem
os limites a declarar são: build debug, round trip JSONL com sobrecarga de Python, Chrome SwiftShader (não a WKWebView da janela), relógio
em lotes e **sem** custo de composição da janela nativa. `core 38/38`, `1556/0` e `2074/0` citados mais acima ou na PR **não** são gates deste head.

**Status de G1 neste head:** rotação continua **a validar**. Um bloqueio de lock não é falha do produto nem prova dele. Nenhuma linha da
matriz foi promovida. Ponteiro físico/WKWebView segue em BL015.2/BL043/BL044. A produção nova desta rodada é só documentação, então não exige
revisão de código nova, mas os gates acima precisam rodar (sob o lock) em um head com estes mesmos fontes antes de qualquer promoção.

### Rodada de gates do head `4b690bd` (PR #40): executados sob o lock de GPU

Head remoto conferido por `gh pr view 40`: `4b690bd8642910d831e0d22dfd91a404259b0989` (árvore `898e7f86fe9a8b80374cad0b6e9fd5f15bcee673`), base da PR `3e48861`.
A `origin/main` viva já está em `e18b0cf` (PR #39, FX/inspetor) e `mergeStateStatus` da PR #40 é **`DIRTY`/`CONFLICTING`**: integrar/rebasear é decisão do
coordenador e **não** foi feita aqui (a rodada não toca FX nem o conteúdo da #39). Evidência em `/tmp/tessel-pr40-gates/r2` (fora do Git, 3,2 MiB; `manifest.jsonl` com
comando, cwd, ambiente extra, head, árvore, sujo, UTC, saída, disco antes/depois e sha256 de cada log; `SHA256SUMS.txt`). Ambiente: `rustc 1.99.1` (pin), `node v22.23.3`,
Python 3.14.3; Cargo só por `./scripts/cargo` → `mbx` com o **target padrão** (sem `CARGO_TARGET_DIR`, `--target-dir` nem `TESSEL_CARGO_LANE`); disco livre 58 GiB na entrada
(reserva 16, piso 12; mínimo visto pela prova 41,9 GiB). O checkout T3 foi levado ao head da PR (`git checkout -B … origin/t3/scene-gizmo-rotation`, árvore limpa); nenhum outro
worktree, nenhum processo alheio tocado, nenhum `mbx clean`/`gc` executado por mim.

**Sequência do lock (sem polling nem contorno).** 16:34Z: tentativa bloqueada (rodada anterior). 17:17:15Z: nova tentativa não bloqueante **adquirida** depois da liberação
confirmada pelo coordenador; o processo que a segurava (e o lote) desapareceu depois de ~3 min, no meio de `cargo check` (log `G04-check.KILLED-run1-holder-died.log`), sem
registro de liberação e com o lock livre por `lsof` e sem fila alheia na hora. Isso não foi falha do produto. 17:23:18Z: **uma** nova tentativa não bloqueante (`driver.py`,
que segura o lock e executa a fila no mesmo processo) foi **adquirida** e mantida até 18:29:58Z (liberada por arquivo `RELEASE`), sem outra tentativa. Um `lockrun.py` de outro
agente ficou esperando o lock durante a janela; não foi tocado. O `mbx` aplicou sozinho a política de GC do wrapper (`MBX_GC_MAX_TOTAL_SIZE=8GiB`) durante `G06`
(`removed 4 target directories (17.7 GiB logical …)`, log `G06-build-editor.log`): não foi comando meu, **não foi contornado**; fica registrado porque pode ter evictado cache
de outros checkouts.

| Gate | Head / árvore | Resultado |
|---|---|---|
| `git diff --check 3e48861 HEAD` | `4b690bd` | passou |
| `cargo fmt --all -- --check` | `4b690bd` | passou |
| `./scripts/runtime setup` | `4b690bd` | passou (`runtime/dist/runtime.js` sha256 `9fbaaa5f…`) |
| `cargo check --workspace --locked` | `4b690bd` | passou (a 1ª tentativa foi morta com o holder; repetida) |
| `cargo clippy --workspace --all-targets --locked -- -D warnings` | `4b690bd` | **passou** (o `Box<RotationFrame>` nunca gateado agora foi compilado) |
| builds `tessel-editor` e `tessel-editor-core --bin tessel-headless` | `4b690bd` | passaram; sha256 `732dd9b7…` e `46e7b742…` |
| `cargo tree -p tessel-editor-core` | `4b690bd` | só `flate2`, `serde`, `serde_json`; sem `gpui`/`wry` |
| `cargo test --workspace --no-fail-fast --locked`, `TESSEL_REQUIRE_RUNTIME=1` | `4b690bd` | **2075 passaram, 0 falharam, 12 `ignored`** (106 alvos), com Chrome real; core: 471 unit, `scene_gizmo` 42, `scene_gizmo_rotation` 45 |
| `./scripts/runtime test` (suíte inteira, inclui `gizmo-overlay.test.mjs`) | `4b690bd` | **218/219**: falhou `video B2: a frame superseded while it seeks is cancelled…` (`frame.done` no lugar de `frame.failed`, `video.test.mjs:208`, teste de corrida do vídeo, sem relação com o overlay); `video.test.mjs` sozinho 7/7; suíte inteira repetida **219/219**. Trato como flaky sob carga (o lote correu com load average > 50), **não** como passe limpo da 1ª execução |
| `scripts/scene-gizmo-proof.spec.py` (S8) | `4b690bd` | 34/34 |
| **`scripts/scene-gizmo-rotation-proof.py`, headless + janela real** (binário próprio, HOME curto, só `op`/`get`/`invoke`) | `4b690bd` | **`ok: true`, `final_evidence: true`, `surfaces_agree_on_the_drag: true`, árvore limpa**: headless 48 checks, janela 54 checks (a janela vê o monitor seguir o candidato e voltar ao inicial no undo; anéis pintados nas cores dos eixos). Repetida com o headless reconstruído (sha256 `a90aff9c…`) e de novo no head final: sempre `ok`. Primeira execução da prova desde que foi escrita |
| `cargo clippy -D warnings`, `fmt`, core `--lib`+`scene_gizmo_rotation`+`scene_gizmo` (472, 42, 46), `cargo tree` | `1b471d6` (testes novos, ver abaixo) | passaram |
| `cargo check`, builds, `cargo test --workspace` (`TESSEL_REQUIRE_RUNTIME=1`), prova G1 | `1b471d6`, árvore `f26e90f5c42c6006967ffc068e47df54d6099b19`, limpa | check/builds/prova passaram; workspace **2076 passaram, 1 falhou, 12 `ignored`**: falhou `killing_the_process_while_its_discard_hashes_a_large_file_leaves_a_claim_recover_takes_back` (`crates/media/tests/job_discard_claims.rs`, espera de 30 s do hash de um arquivo grande, **mídia, fora do escopo da PR**, passou no `4b690bd`); o alvo repetido sozinho passou 13/13. Não é um passe limpo do workspace no head final; é o `2075/0` do `4b690bd` mais as 2 provas novas e uma flaky de carga declarada |

**Mutações Rust discriminantes** (`mutate.py`, uma por vez, arquivo restaurado por texto e conferido por sha256 e `git status`; um mutante só vale como morto quando um teste nomeado **falha**, erro de
compilação não conta). Resultado: **15 mortas na 1ª passada e 2 sobreviventes**, ambos lacunas de **teste**, não de produção:

| Mutação | Resultado |
|---|---|
| sinal do giro; ordem de `R'`; espaço local vira mundo; `Pₙᵀ` sem transpor; sem módulo 360°; intervalo `(-180,180]` aberto; volta completa deixa de ser no-op; arrasto abaixo de −180° sem enrolar | mortas (testes da rotação: `a_node_with_its_own_negative_and_uneven_scale_…`, `local_and_world_turn_about_different_axes_…`, `zero_and_whole_turns_are_no_ops_…`, `angles_reduce_to_the_half_open_half_turn_…`, `a_negative_sweep_turns_the_other_way_…` etc.) |
| `animated_rotation` permitida; `mirrored_parent` permitido; `non_uniform_parent_scale` permitido | mortas (`a_node_whose_rotation_is_animated_is_refused_…`, `a_mirroring_parent_is_refused_typed`, `a_parent_that_scales_unevenly_is_refused_typed_…`) |
| `quat_unit` aceita zero | morta (`degenerate_authored_quaternions_cannot_be_normalised`) |
| ferramenta muda sob arrasto vivo; carimbo sem `tool`/`space`; cancelamento não descarta o arrasto | mortas (`the_tool_cannot_change_under_an_open_drag_…`, `the_tool_and_the_space_are_part_of_the_overlay_identity`, `a_drag_is_dead_when_its_gesture_can_no_longer_apply_…`, `state::tree::tests::the_gizmo_node_exists_only_…`) |
| **rasante** (`GRAZING` nunca recusa) | **SOBREVIVEU** (core 471/42/45 verdes; log `M-rust-grazing-ring-resolves-core.log` mantido): o único teste usava a câmera exatamente de lado, que `plane_hit` já recusa; a faixa `0 < \|cos\| < 0,05` não era coberta |
| **`RotationFrame::of` sem normalizar/recusar o quaternion autorado** (`let unit = authored`) | **SOBREVIVEU** (log `M-rust-authored-not-normalised-core.log`): a normalização do resultado final e a normalização das colunas dos eixos mascaram o caso quase unitário (equivalente para quaternions finitos não nulos); só zero/NaN/∞ diferia, e nenhum teste alcançava `degenerate_rotation` |

A 2ª fase de cada mutação (testes do desktop, só para sobreviventes do core) falhou por **erro do comando** (`--lib` não existe no pacote `tessel-editor`; log preservado) e não foi usada; os veredictos vêm do core. Correção, só de teste (commit `1b471d6`, produção idêntica): `a_ring_seen_almost_edge_on_is_refused_and_a_clearer_view_of_it_opens_a_drag` (câmera 0,3 acima: `degenerate_axis`; 1,0 acima:
arrasto aberto; mesmo ponto) e o unitário `a_frame_refuses_what_cannot_be_turned_with_the_typed_reason` (zero, NaN, ∞, animado, matriz de mundo e local singulares, nas duas
orientações). **Com os testes novos, as duas mutações foram mortas** (`rust-mutations-rerun-with-new-tests.json`). `not_movable` continua inalcançável: `NodeKind` só tem `Group`,
`ImagePlane` e `Splat`, todos movíveis (a guarda é defensiva e fica sem teste). **Mutações JS 3/3** do `ebd1f08` valem para este head: de `ebd1f08` a `4b690bd` só mudam 3 arquivos de `docs/`.
A recusa `degenerate_rotation` segue sem teste de integração pelo importador (só o unitário de `RotationFrame::of`).

**Medição do overlay dos anéis (BL015.12), com limites.** Core: `tools/measure-core-overlay.py`, `scene.gizmo.state` por JSONL contra `tessel-headless` em build **debug**, relógio de parede incluindo
Python e serialização, n=1500 por caso, 100 descartados, 3 execuções. A máquina estava compartilhada: a execução 2 tem p95 de dezenas de ms (ruído), a 1 e a 3 não. Mínimos estáveis: anéis visíveis
885–1310 µs, setas 197–264 µs, oculto (animado ou pai não uniforme) 166–195 µs, sem seleção 115–143 µs. p50 da execução mais quieta (3): anéis mundo 1147 µs, local 1216 µs, setas 236 µs, oculto
208/180 µs, sem seleção 177 µs. **Os anéis custam ~5× as setas e ~5–6× o oculto neste caso** (3×65 pontos de JSON e a projeção), em ~1 ms de debug; a dupla chamada de `rotation_frame` (R4) **não** foi
isolada nem otimizada. Página: `tools/measure-page-rings.mjs` em Chrome headless SwiftShader (**não** a WKWebView), `performance.now` em lotes de 200 chamadas × 15, 2 lotes de aquecimento descartados, 2 execuções
concordantes: `overlay.set` com anéis 10,5 µs/chamada (desenho gravado, sem leitura), oculto 1 µs, setas 3 µs; **forçando a rasterização** (`getImageData` 1×1 por chamada) anéis 211 µs, oculto 42 µs, setas 62–63 µs. As
medições anteriores da página (`measure-page-rings.json` e `…-guarded-run*.json`) são **inválidas**: o payload de anéis do script não tinha `from`/`to` por handle e a página o recusava (media o caminho de recusa);
a guarda acrescentada (pixels pintados, `inStep`, nenhum `overlay.error`) o revelou, o script foi corrigido e os arquivos antigos ficam só como registro do defeito. Não inclui composição da janela nativa nem GPU real.

**Limites que permanecem.** Nenhum ponteiro físico, foco de teclado nem captura sobre a WKWebView (BL015.2, BL043, BL044). Sem `.spec.py` própria da prova G1; sem caso de pai rotacionado na prova semântica
(só no core); cor conferida só nos anéis x e y na prova. Os binários da prova não são reproduzíveis byte a byte entre builds (hashes diferentes a cada compilação, cada relatório grava o do binário que usou).

**Status de G1 neste head:** rotação continua **a validar**: todos os gates pedidos foram executados e passaram (com as duas flakies de carga declaradas e repetidas), os sobreviventes viraram testes mortos, mas falta a revisão
independente do coordenador sobre este head e a prova com ponteiro físico. Nada além disso foi promovido.

### Integração da PR #40 com a `main` viva (merge `d54a3f3`, 2026-11-10)

Origem conferida: `journey-studios/tessel-editor`. `git ls-remote origin` e `git fetch origin` deram `refs/heads/main` = `e18b0cf39f5b94c71640ad397f3f21bc9781666f`
(PR #39, inspector neutro de FX e o look `fade`) e `refs/pull/40/head` = `23462bdd8aebbcceba733195b9bd105cd6143dc8`; `gh pr view 40`: draft, `DIRTY`, base `main`. O merge-base é `3e48861`.
O checkout T3 (`t3/integrate-pr40-gizmo-g1`, `172c90f`, ancestral do head da PR) foi levado a `23462bd` com `git merge --ff-only`; depois `git merge --no-ff origin/main` e **um commit de
merge** (`d54a3f3`, pais `23462bd` e `e18b0cf`, árvore `885fea733863deaa58bb686341b9087b4df6fc24`). Sem rebase, sem force, nenhum outro worktree ou checkout, raiz canônica não tocada,
nenhuma linha de FX/inspector/`fade` alterada por esta entrega (código da #39 entra só pelo merge).

**Conflitos (só dois, ambos de documentação) e como foram resolvidos por contrato:**

| Arquivo | Conflito | Resolução |
|---|---|---|
| `docs/README.md` | no fim do índice, a nota "G1 (PR #40) … a validar" contra a linha do inspector neutro/smoke de P2 | **as duas** ficam: a linha do F057 como item de lista e a nota do G1 depois dela |
| `docs/backlog.md` | no fim, o parágrafo da revisão G1 (nome do teste, resumos F055, `not_movable`) contra a seção "P2 / PR39" com `BL082.21`–`BL082.25` | **as duas** ficam, na ordem (G1, depois P2), sem editar nenhuma linha da P2 |

Auto-merge sem conflito, conferido por leitura do delta contra o head da #40: `crates/core/src/ops.rs` (parser de `scene.fx.inspect`/`fx_probe` ao lado de `scene.rotate`/`scene.gizmo.tool`),
`scene_project/command.rs` (`SceneCommand::FxInspect`/`FxProbe` ao lado de `GizmoCommand::Rotate`), `scene_project/query.rs` (`scene.capabilities` lista **as duas** famílias e o braço
de leitura de FX recebe `FxInspect`), `docs/features/README.md` (F055 com G1 e F057 coexistem, cada um com seu ID). Os dois conjuntos de comandos são aditivos e usam o mesmo
espaço único de gestos (`claim_gesture_slot`), o que o teste de interação abaixo (pendente) fixa.

**Comparação de três vias** (base `3e48861`, nosso `23462bd`, deles `e18b0cf`, resultado `d54a3f3`; script em Python 3, só leitura, sobre `git show`): IDs `BL…` e `F…`
do `docs/backlog.md` (nosso 335, deles 337, resultado 341; **0 ausentes**), de `docs/features/README.md` (146, 145, 148; **0 ausentes**) e de `docs/README.md` (13, 14, 14; **0 ausentes**); linhas
acrescentadas ou alteradas por cada lado em relação à base e ausentes do resultado: **0** nos três arquivos. Nenhum ID, feature ou evidência desapareceu. Um arquivo versionado
que um lado alterou e o outro não ficou idêntico ao do lado que o alterou (verificável com `git diff 23462bd d54a3f3 --stat` e `git diff e18b0cf d54a3f3 --stat`).

**Gates no head integrado — histórico desta rodada: NÃO executados (BLOQUEADOS), nada promovido (executados depois, sem lock, na seção seguinte).** Sequência do lock, sem sleep, polling, contorno nem kill: 18:35:51Z a **primeira** tentativa não bloqueante
(`/tmp/tessel-pr40-int/driver.py`: `LOCK_EX|LOCK_NB` em `/tmp/tessel-scene-gpu.lock` e no lock de vídeo) **adquiriu**; o processo que a segurava, iniciado por `nohup` numa chamada de ferramenta,
desapareceu com a sessão da ferramenta depois de ~10 min, durante a **compilação do core** (log `D01-newtest-core.KILLED-run1-holder-died.log`, sha256 `44d68e3b…558f`, sem `RELEASED`, sem
produto envolvido: mesmo padrão registrado na rodada anterior). 18:45:43Z uma **segunda e última** tentativa não bloqueante (agora como tarefa de segundo plano gerenciada) saiu com
`BLOCKED`: o lock é de `/tmp/tessel-bl157-1/lockrun.py` (`cargo test --workspace`, outro agente, pid 64533). Registro em `/tmp/tessel-pr40-int/lock.state`. Disco livre 48,5 GiB na entrada e 39,9 GiB
depois do que outro processo ocupou (reserva 16, piso 12); `/tmp/tessel-pr40-int` tem 44 KiB. Cargo só por `./scripts/cargo` → `mbx` com o target padrão (sem `CARGO_TARGET_DIR`,
`--target-dir`, `build.target-dir` nem `TESSEL_CARGO_LANE`); nenhum `mbx clean`/`gc`.

| Gate | Head / árvore | Cwd | UTC | Saída | Resultado |
|---|---|---|---|---|---|
| `git diff --check 3e48861 HEAD` | `d54a3f3` / `885fea73…` limpa | checkout T3 | 18:46Z | 0 | passou |
| `cargo fmt --all -- --check` (rustfmt direto, não compila) | `d54a3f3` limpa | checkout T3 | 18:46Z | 0 | passou |
| `python3 scripts/scene-gizmo-proof.spec.py` (S8) / `scene-fx-ui-proof.spec.py` (P2) | `d54a3f3` limpa | checkout T3 | 18:46Z | 0 / 0 | 34/34 e 28/28 |
| `cargo check`, `clippy -D warnings`, `cargo test --workspace` (`TESSEL_REQUIRE_RUNTIME=1`), builds, `cargo tree` do core, Chrome do runtime, prova G1 nas duas superfícies | `d54a3f3` | — | — | — | **BLOQUEADOS** (lock de GPU/build ocupado; **a compilação do merge continua não verificada**) |

Os gates de `4b690bd` e `1b471d6` acima **não** valem para o merge: o código da #39 (FX, inspector, `fade`) entrou depois deles. O que vale para o código do G1 é o histórico preservado (nada dele foi
alterado pelo merge nos arquivos do G1, ver `git diff --stat 23462bd d54a3f3 -- crates/core/src/scene_interaction runtime/src/overlay.js crates/core/tests/scene_gizmo_rotation.rs`).

**Teste de interação G1 × FX (BL015.13) — histórico desta rodada: escrito, retirado da árvore e não executado (compilado e executado depois, ver a seção seguinte).** O arquivo `crates/core/tests/scene_gizmo_fx.rs` (4 testes: um gesto de efeito aberto recusa o press num anel e a troca
de ferramenta não o toca; um arrasto de anel aberto recusa um gesto de efeito e as leituras do FX o deixam vivo; uma rotação e uma edição de efeito são duas entradas de um histórico que não se tocam em
undo/redo; uma edição de efeito sob um arrasto o mata e a soltura tardia não escreve) não pôde ser compilado sem o lock. Como a política do repositório é "sem prova, a validar" e um teste que talvez não compile
quebraria `cargo test --workspace` da PR, **não foi commitado**; cópia em `/tmp/tessel-pr40-int/scene_gizmo_fx.rs.pending` (sha256 `e12d4679…c3f0`, fora do Git: não durável) para o coordenador
recolocar e rodar quando o lock liberar. Os testes existentes que já cobrem o encontro por outro lado: `a_ring_press_while_another_gesture_is_open_is_refused_and_a_late_release_is_inert` e
`the_tool_cannot_change_under_an_open_drag_and_the_drag_is_left_alone` (G1) e `one_gesture_at_a_time_and_the_two_kinds_do_not_cross` (F016).

**Status de G1 neste head:** a PR #40 deixa de estar em conflito com a `main` (merge textual resolvido, três vias sem perda), mas a rotação continua **a validar**: não há compilação, clippy, suíte,
runtime nem prova G1 no head integrado. O próximo passo é rodar a bateria completa do head integrado quando o lock estiver livre; nenhuma flaky anterior foi reinterpretada e nenhuma foi relaxada.

### Gates funcionais do head integrado `ebf77c2` (sem lock de GPU)

**Regra nova (2026-11-10, atribuída ao usuário):** o lock de GPU/build (`/tmp/tessel-scene-gpu.lock`, `with-gpu-lock.py`) existe **somente para validações de desempenho**.
Edição, `cargo fmt/check/clippy/test/build`, o runtime Chrome funcional e as provas funcionais não esperam por ele nem o tomam; o lock de outro agente não foi apagado nem seu processo tocado,
e nenhuma medição de desempenho foi iniciada. Um script que mistura resultado funcional e de desempenho separa os dois: sem lock, o desempenho não é declarado aceito (registro em `AGENTS.md`).

Origem conferida: `journey-studios/tessel-editor`, PR #40 draft `CLEAN`, base `main` (`e18b0cf`), head recebido `3c26101`. O checkout T3 `t3/pr40-functional-gates` foi levado a `3c26101` (sem force,
sem novo worktree, raiz canônica não tocada) e recebeu um commit de código, `ebf77c2` (árvore `4cce6028930182c09c6e7a9a65ecbfdea721aee1`). Cargo só por `./scripts/cargo` → `mbx` com o target padrão (sem
`CARGO_TARGET_DIR`, `--target-dir`, `build.target-dir` nem `TESSEL_CARGO_LANE`; `CARGO_BUILD_JOBS=6` limita a CPU), `rustc 1.99.1`, `node v22.23.3`, Python 3.14.3, uma sequência finita de comandos
gerenciados pela ferramenta (sem `nohup`, fila ou polling). Disco livre 43 GiB na entrada, **mínimo 31 GiB** (reserva 16, piso 12); evidência de 472 KiB em `/tmp/tessel-pr40-fn` (fora do Git, não durável).

| Gate | Head / árvore | UTC (início) | Saída | Resultado |
|---|---|---|---|---|
| `cargo fmt --all -- --check`, `git diff --check 3c26101 HEAD` | `ebf77c2` limpa | 19:40Z | 0 / 0 | passou |
| `cargo check --workspace --locked` | `ebf77c2` limpa | 19:40Z | 0 | passou (172 s) |
| `cargo clippy --workspace --all-targets --locked -- -D warnings` | `ebf77c2` limpa | 19:43Z | 0 | passou |
| `cargo build -p tessel-editor --locked`; `cargo build -p tessel-editor-core --bin tessel-headless --locked` | `ebf77c2` limpa | 19:44Z | 0 / 0 | passou |
| `cargo tree -p tessel-editor-core --locked` | `ebf77c2` | 19:47Z | 0 | **0** ocorrências de `gpui`/`wry` |
| `./scripts/runtime setup` | `ebf77c2` | 19:47Z | 0 | passou |
| `./scripts/runtime test` (Chrome completo) — **1ª execução** | `ebf77c2` limpa | 19:47Z | **1** | **222/223**: falhou `video B2: a frame superseded while it seeks is cancelled and the next one is exact` (`frame.done` em vez de `frame.failed`) |
| `cargo test --workspace --locked`, `TESSEL_REQUIRE_RUNTIME=1` | `ebf77c2` limpa | 19:51Z | 0 | **2123 passaram, 0 falharam, 12 `ignored`**, num único passe (681 s), com `scene_gizmo_fx` 5/5, `scene_gizmo_rotation` 46/46, `scene_gizmo` 42/42 |
| `./scripts/runtime test` — **repetição integral** (não é um rerun isolado) | `ebf77c2` limpa | 20:02Z | 0 | **223/223** |
| `python3 scripts/scene-gizmo-rotation-proof.py --editor target/debug/tessel-editor --headless target/debug/tessel-headless --out …` | `ebf77c2` limpa | 20:09Z | 0 | `ok:true`, `final_evidence:true`, `surfaces_agree_on_the_drag:true`, headless **48** + janela **54** checks; relatório grava `head` = `ebf77c2`, `tree_clean:true` |
| `python3 scripts/scene-fx-ui-proof.py --binary target/debug/tessel-editor --json …` (P2, janela real, só requisições semânticas, sem coordenadas) | `ebf77c2` limpa | 20:09Z | 0 | **153/153**, inclusive a exportação MP4 comparada com o vinheta Python (8/8 quadros) |
| `scene-gizmo-proof.spec.py` / `scene-fx-ui-proof.spec.py` / `proof-surface.spec.py` | `ebf77c2` limpa | 20:10Z | 0 / 0 / 0 | 34/34, 28/28, 29/29 |

Binários usados pelas duas provas (os do HEAD, construídos pelos gates acima): `tessel-editor` sha256 `f9d28ce08fb2b83ac5797926b597a96276c678fcb752814f0e88e92fa5b885fd`,
`tessel-headless` sha256 `f4f097736a3dce5a267a348eece741e59d610735280d9293df6da9e00448cc63`. As provas rodaram pela janela isolada do script (HOME curto próprio, socket próprio); o editor do usuário
não foi tocado e nenhum processo da rodada ficou vivo. Os commits seguintes a `ebf77c2` só mudam documentação, `AGENTS.md` e o texto do docstring de um script (`git diff --stat ebf77c2 HEAD` o mostra): o código gateado é o mesmo.

**O que esta rodada achou.**

1. **Teste G1 × FX (BL015.13).** Dos 4 testes pendentes, o arquivo com valores absolutos de undo (`scene_gizmo_fx.rs.pending`) não foi usado; a versão com profundidade relativa ao histórico inicial compilou e passou.
   O `baseline` sem uso do primeiro teste seria um aviso do clippy `-D warnings`: removido, e o clippy fechou limpo. Um quinto teste cobre o F2.
2. **F2 da revisão 5847593300 — reproduzido e corrigido.** `scene.fx.list/inspect/frame` com `gesture` documentam checar cena **e tipo**, mas `fx_read_scene` só checava cena e vivacidade: o ID de um arrasto de anel
   (gesto `properties`) respondia `source:"gesture"` com o candidato da rotação (observado na primeira execução do teste novo). A correção é o mesmo `ensure_kind` dos outros verbos do gesto de efeito
   (`gesture_kind_mismatch`, `details.use` aponta a consulta sem `gesture`): a leitura continua sem escrever, o arrasto continua vivo e a consulta ao documento continua funcionando.
   Nenhuma outra regra de FX mudou; o teste `an_effect_read_names_an_effect_gesture_and_refuses_the_id_of_a_ring_drag` falhou sem a correção e passa com ela.
3. **Flaky de vídeo B2 repetida (BL015.15).** É o mesmo teste da rodada de `4b690bd` e falhou de novo na 1ª execução completa, com a máquina compartilhada em carga ~24. Não há passe limpo único do runtime neste head
   (222/223, depois 223/223 na repetição integral); nenhuma asserção nem tolerância foi relaxada, e o produto (`latestRender`) não é tocado pela PR. A hipótese de corrida do teste não está provada.

**Não feito, de propósito.** Nenhuma medição de desempenho (a regra nova só impõe o lock a ela; as medições anteriores à fusão seguem históricas); ponteiro físico/WKWebView (BL015.2/BL043/BL044); escala, luz e FX
novos; nova revisão independente (fica com o coordenador). As mutações Rust de G1 (BL015.10) continuam as do head `1b471d6`, não repetidas aqui.

**Status de G1 neste head:** rotação segue **a validar**. Os gates funcionais do head integrado estão executados e passaram, exceto que o runtime/Chrome só tem passe completo na repetição (BL015.15). Faltam a
revisão independente do coordenador, o ponteiro físico e a flaky de vídeo diagnosticada.
