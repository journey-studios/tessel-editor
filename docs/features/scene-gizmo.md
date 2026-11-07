# F055 — Gizmo de translação no canvas (BL015, BL042)

## Objetivo e limites

Um gizmo inicial de seleção e **translação** para cenas dirigidas. Ele é **desenhado e acertado
por um canvas 2D dentro da página da WebView** (`runtime/src/overlay.js`), nunca por elementos
GPUI sobre a WKWebView (a view nativa pinta acima de qualquer elemento GPUI). O ponteiro vira
intents que o **core valida e aplica**; a página não decide nada além de "onde, em coordenadas da
imagem, o usuário apertou".

- Handles: eixos `x`, `y`, `z` e um handle central `screen` (move no plano que encara a câmera).
- Seleção por ponteiro: apertar sobre um plano de imagem seleciona (`additive` alterna); apertar
  em área vazia da imagem limpa a seleção; apertar num handle mantém a seleção.
- O que é editado: a propriedade vetorial existente `…/node/<id>/position` (`localPosition` do
  documento). Não há rotação, escala, snapping, múltiplas seleções nem grupos nesta fatia: com
  mais de um selecionado o overlay fica oculto (`multiple_selection`).
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
`ba0d5dc`); **nenhum gate foi executado sobre o head desta PR** (ver "Evidências e status": o que
foi lido, o que foi executado e o que está BLOCKED).

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
canvas), e uma exceção do desenho vira um `error` próprio (`overlay_failed`): nunca `frame.failed`, e
`done`/`presented`/export/headless não regridem.

**Medição proporcional (não alegada como "custo 0"):** `runtime/scripts/perf/run.mjs --variants
current,baseline --baseline-bundle <bundle da main>` (sob o lock de GPU) compara este head com um
baseline sem overlay. As páginas do harness usam `__tesselPost`, portanto não têm overlay neste head: a
comparação mede o custo residual do gate. **Não executada** (BLOCKED por disco, ver "Execução no head").

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
| `pointercancel`, `lostpointercapture` com press aberto | cancela (`pointer_cancel`) |
| Novo press com outro "aberto" | descarta o arrasto cujo release se perdeu e começa limpo |
| Resize da janela/painel durante o arrasto | **não termina**: coordenadas são da imagem; a página reenvia o layout |
| Edição externa/undo no meio do arrasto | `gesture_invalidated`: o candidato antigo nunca é aplicado, o arrasto morre |
| Cancel sem nada aberto | no-op (não é erro); `changed: false` |

### Precisão das respostas (S1)

- `scene.pointer.up` **sem arrasto aberto** é no-op (`noop: true`, `changed: false`) e **não** liquida
  um gesto de slider do inspetor que outro tenha aberto; só o release de um arrasto do próprio gizmo
  (ou `scene.translate`, uma edição real) o faz. Teste: `a_release_with_no_drag_does_not_commit_somebody_elses_slider_gesture`.
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
`unknown_element`. Um comando recusado deixa documento, histórico e revisão como estavam.

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
  `handle`, `gesture`) e `monitor.viewport.state.overlay` (suportado, ligado, contadores, último
  layout e último `drawn`). Não há botão para clicar: o equivalente semântico é
  `./scripts/ui op '{"op":"scene.pointer.down",…}'`. O resumo (`summary.scene_interaction.drag`)
  mostra o arrasto aberto.
- **Ponteiro físico**: ops semânticas **não provam** um mouse real; a prova com o mouse físico na
  janela nativa continua em BL043/BL044.

## Evidências e status

Testes **declarados** no código (contados na fonte). A coluna "Execução" é a única que vale como
prova; o bloco "Execução no head" abaixo registra o que de fato rodou.

| Superfície | Teste declarado | Execução |
|---|---|---|
| Biblioteca/core | `crates/core/tests/scene_gizmo.rs` (37 funções de teste) e unit `state::tree`: geometria contra oráculo próprio da câmera, arrasto ≡ `scene.translate`, uma entrada de undo, pai rotacionado/escalado, recusas invariantes, seleção por ponteiro, políticas de cancelamento; **novos** (revisão da PR #25): chave de memo do overlay (`the_overlay_key_moves_with_…`), ramos `degenerate_axis`/`degenerate_pointer` em `Anchor`, fronteira exata de `OUTSIDE_LIMIT` (aceita em ±8, recusa além), `abandoned.reason == "replaced_by_press"` e um só gesto restante, release sem arrasto que não liquida slider alheio (S1) | BLOCKED (cargo, disco) |
| CLI headless | `a_json_lines_session_runs_the_whole_gizmo_flow_in_process` (`headless::run_session`, sem janela) e `scripts/scene-gizmo-proof.py` (superfície headless) | BLOCKED (build, disco) |
| Página/canvas | `runtime/test/overlay.test.mjs` (11, puros) e `runtime/test/gizmo-overlay.test.mjs` (14, Chrome real): mouse do puppeteer, pixels do canvas, Escape, blur, hidden, `pointercancel`, **captura perdida de verdade** (`releasePointerCapture`), botão direito/médio e segundo ponteiro, resize, hover, overlay inválido recusado inteiro; **M1**: overlay inativo sem layout/canvas, falha do desenho isolada do frame, página headless sem overlay | puros: 11/11 executados; Chrome (14): **BLOCKED** (disco; os 9 anteriores rodaram em head antigo e não valem para este) |
| Host/ponte | `viewport_host_tests` (52; 2 novos): portões e contadores do overlay, reload = época nova | BLOCKED (build, disco) |
| Janela (GPUI) | `scene_gizmo_tests` (28): mensagem → comando único, Escape/foco/oculto, mensagem velha silenciosa, invalidação (inclui `overlay_run` com `gesture_invalidated` por edição que não passa pelo `execute`), **M2/M4**: runtime sem feature/overlay desligado/reprodução sem plano, adoção da resposta, uma computação por move, época e seleção no carimbo | BLOCKED (build, disco) |
| Janela real (binário próprio, HOME curto, só `get`/`set`/`invoke`/`op`) | `scripts/scene-gizmo-proof.py` (superfície nativa); a lógica do script (M3) tem `scripts/scene-gizmo-proof.spec.py` (28) | spec: 28/28 executados; a prova nativa/headless real: BLOCKED (build, disco) |

**Mutações direcionadas (T1/M3).** `scripts/scene-gizmo-proof.py`: 11 mutações da lógica da prova, 10 mortas e 1 sobrevivente
equivalente (lista vazia de superfícies: a recusa de nome desconhecido já a recusa), registro em
`/tmp/tessel-s8-m1-m4-evidence/logs/m3-mutations.log`. As mutações dos testes **Rust** (limite, ramos degenerados,
`replaced_by_press`, `gesture_invalidated`) estão descritas em cada teste (o que deve falhar se o ramo sumir)
mas **não foram executadas**: sem cargo não há como matar mutantes de Rust; nenhum "morto" é alegado.

### Execução no head

Correção da revisão 5815117082 da PR #25 (achados M1–M4, E1, T1, S1) sobre o head `88babd5`. Todas as medições
abaixo são desta árvore (HEAD e árvore, ambiente, comando, cwd, UTC, saída e disco de **cada** log ficam no
manifesto `/tmp/tessel-s8-m1-m4-evidence/manifest.txt`, com sha256 dos logs). As execuções anteriores
(`7936a36`: runtime 166/166, overlay 9+9 em Chrome real) são **história** em
`/tmp/tessel-s8-recovered-evidence/`: o runtime mudou (`overlay.js`, `boot.js`, `index.js`), então não valem para este head.

| Verificação | Resultado |
|---|---|
| `cargo fmt --all -- --check` | passou |
| `git diff --cached --check` | passou |
| `scripts/scene-gizmo-proof.spec.py` (lógica da prova, M3) | 28 testes, passou |
| `node --test runtime/test/overlay.test.mjs` (puro) | 11/11 |
| `node --check runtime/test/gizmo-overlay.test.mjs` | sintaxe ok (o teste em si é Chrome: BLOCKED) |

**BLOCKED (não executado):** `cargo check --workspace --all-targets`, `clippy -D warnings`,
`cargo test --workspace --locked --no-fail-fast` (core, GPUI e host: **todo o Rust novo está sem compilar**),
build de `tessel-editor` e `tessel-headless`, `./scripts/runtime setup`, `runtime/test/gizmo-overlay.test.mjs` e a suíte
do runtime em Chrome real, `runtime/scripts/perf/run.mjs` (medição M1/M2), `scripts/scene-gizmo-proof.py`
(headless e janela real), `cargo tree -p tessel-editor-core`. Motivo: disco livre ≈ 9,6 GiB, **abaixo do piso de
12 GiB** e dos 16 GiB exigidos para build pesado (medição no manifesto); nada foi apagado nem contornado. Nenhum binário
foi construído, então não há hash de binário para este head. **Não há PASS de Rust, Chrome, perf ou nativo nesta entrega.**

Procedimento da medição proporcional (M1/M2) para quando houver disco: construir o bundle da `main` (`59b8efc`)
como baseline e rodar, sob `scripts/directed-scene-proof/with-gpu-lock.py`,
`node runtime/scripts/perf/run.mjs --variants current,baseline --baseline-bundle <bundle da main>`; reportar a razão
com o intervalo, nunca "custo 0"; o custo do adaptador (planos por move/tick) lê-se nos contadores `computed`/`adopted`/
`memo_hits`/`held` de `monitor.viewport.state.overlay`.

Status: **a validar**. Implementado; neste head só rodaram fmt, os testes puros do runtime (11) e a lógica da prova (28);
Rust, Chrome, perf e as provas nativas/headless estão **BLOCKED** (disco); **sem
prova com ponteiro físico** nem captura visual do gizmo sobre a WKWebView (BL043/BL044). Limitações: só
translação; um nó por vez; sem snapping; handles só em coordenadas de mundo (não locais); aspecto da
imagem vem da câmera da cena.

Harmonização: **S4** (identidade de ativos) — o gizmo só endereça nós pelo `entity` do F039
(`scene/<cena>/node/<id>`), sem tocar slots/assets; **S6** (propriedades) — usa a propriedade
`position` do catálogo existente via `EditOp::set`, então um catálogo ampliado vale sem mudança;
**S7** (Spark) — nenhum acoplamento: planos de imagem são o único alvo de seleção por ponteiro.

Veja [arquitetura](../core-architecture.md), [interação de cenas](scene-interaction.md) e
[template](_template.md).
