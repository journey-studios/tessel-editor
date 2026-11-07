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
| `pointercancel`, `lostpointercapture` com press aberto | cancela (`pointer_cancel`) |
| Novo press com outro "aberto" | descarta o arrasto cujo release se perdeu e começa limpo |
| Resize da janela/painel durante o arrasto | **não termina**: coordenadas são da imagem; a página reenvia o layout |
| Edição externa/undo no meio do arrasto | `gesture_invalidated`: o candidato antigo nunca é aplicado, o arrasto morre |
| Cancel sem nada aberto | no-op (não é erro); `changed: false` |

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
| Biblioteca/core | `crates/core/tests/scene_gizmo.rs` (32 funções de teste) e unit `state::tree` (`the_gizmo_node_exists_only_with_a_scene_and_follows_the_drag`): geometria contra oráculo próprio da câmera, arrasto ≡ `scene.translate`, uma entrada de undo, pai rotacionado/escalado, recusas invariantes, seleção por ponteiro, políticas de cancelamento | BLOCKED (cargo, disco) |
| CLI headless | `a_json_lines_session_runs_the_whole_gizmo_flow_in_process` (`headless::run_session`, sem janela) e `scripts/scene-gizmo-proof.py` (superfície headless) | BLOCKED (build, disco) |
| Página/canvas | `runtime/test/overlay.test.mjs` (9, puros) e `runtime/test/gizmo-overlay.test.mjs` (9, Chrome real): mouse do puppeteer, pixels lidos do canvas, Escape, blur, hidden, `pointercancel`, resize durante o arrasto, hover, overlay inválido recusado inteiro | executado: 9/9 e 9/9 (ver "Execução no head") |
| Host/ponte | `viewport_host_tests` (3 novos): negociação do feature, deduplicação, reload esquece o overlay | BLOCKED (build, disco) |
| Janela (GPUI) | `scene_gizmo_tests` (17): mensagem → comando único, Escape/foco/oculto, mensagem velha silenciosa, invalidação | BLOCKED (build, disco) |
| Janela real (binário próprio, HOME curto, só `get`/`set`/`invoke`/`op`) | `scripts/scene-gizmo-proof.py` (superfície nativa) | BLOCKED (build, disco) |

### Execução no head

Head testado: `7936a36` (árvore `09e1bf31f291fb58a89981e9bfc48f97ae53488c`), árvore inteira limpa, Rust 1.99.1, disco livre ≈ 12,4 GiB
(abaixo dos 16 GiB exigidos para qualquer build pesado). Logs em `/tmp/tessel-s8-recovered-evidence/logs/`.

| Verificação | Resultado |
|---|---|
| `cargo fmt --all -- --check` | passou |
| `git diff --check` do commit `ba0d5dc` ao head | passou |
| `scripts/scene-gizmo-proof.spec.py` (lógica pura da prova) | 7 testes, passou |
| `./scripts/runtime setup` | passou (`runtime.js` 4129 KiB) |
| `runtime/test/overlay.test.mjs` (puro) | 9/9 |
| `runtime/test/gizmo-overlay.test.mjs` (Chrome real, sob o lock de GPU, `TESSEL_REQUIRE_RUNTIME=1`) | 9/9 |
| Suíte completa do runtime (`pnpm test`, Chrome real, lock de GPU, `TESSEL_REQUIRE_RUNTIME=1`) | 166/166, 0 falhas, 0 pulados |
| `cargo tree -p tessel-editor-core --locked --offline -e normal,build` | 14 linhas, sem `gpui`/`wry` |

**BLOCKED (não executado, BL015.6):** `cargo check --workspace --all-targets`, `clippy -D warnings`,
`cargo test --workspace --locked --no-fail-fast` (testes do core, GPUI e host), build de `tessel-editor` e
`tessel-headless`, `scripts/scene-gizmo-proof.py` (headless e janela real) e a mutação dos testes. Motivo:
disco livre abaixo de 16 GiB; o piso de 12 GiB não foi cruzado nem contornado. Nenhum binário foi
construído, então não há hash de binário para este head. Os 166/166 de uma tentativa anterior em
`/tmp/tessel-s8-final-evidence` eram de uma árvore suja e não valem como evidência.

Status: **a validar**. Implementado; o runtime/página foi executado, mas os gates Rust e as provas
nativas/headless estão **BLOCKED** no head da PR; **sem
prova com ponteiro físico** nem captura visual do gizmo sobre a WKWebView (BL043/BL044). Limitações: só
translação; um nó por vez; sem snapping; handles só em coordenadas de mundo (não locais); aspecto da
imagem vem da câmera da cena.

Harmonização: **S4** (identidade de ativos) — o gizmo só endereça nós pelo `entity` do F039
(`scene/<cena>/node/<id>`), sem tocar slots/assets; **S6** (propriedades) — usa a propriedade
`position` do catálogo existente via `EditOp::set`, então um catálogo ampliado vale sem mudança;
**S7** (Spark) — nenhum acoplamento: planos de imagem são o único alvo de seleção por ponteiro.

Veja [arquitetura](../core-architecture.md), [interação de cenas](scene-interaction.md) e
[template](_template.md).
