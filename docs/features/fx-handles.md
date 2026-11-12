# F061 — Handles 2D da vinheta no monitor (G4.1, BL082.35)

Status: **A validar** — implementado e executado nas três superfícies (lib, headless, janela própria com comandos
semânticos), com testes de core, GPUI, JS (Chrome real), imagem/export reais e mutações. **Não** há prova com ponteiro
físico na WKWebView, nem de desempenho, nem em Windows (ver "Limites").

## Objetivo e limites

Um clipe de efeito `vignette` selecionado no inspetor, cujo intervalo contém o quadro exibido, ganha no monitor três
alças 2D e duas elipses desenhadas no canvas 2D do overlay (F055), **nunca** no quadro renderizado, no export nem no
documento:

| Alça | Parâmetro | Onde fica | Arrasto |
|---|---|---|---|
| `center` | `center` | no próprio centro | livre em 2D |
| `radius` | `radius` | `(cx + a_u(radius), cy)`: onde o escurecimento começa (elipse sólida) | `radius` |
| `softness` | `softness` | `(cx + a_u(radius + softness), cy)`: onde termina (elipse tracejada) | `softness` |

A geometria é o inverso do `vignette_shade` (`crates/core/src/directed_scene/fx/reference.rs`), em função da razão
`W/H` do perfil de saída e do `roundness`:

```
stretch = 1 + (W/H − 1)·roundness
norm    = max(√(stretch² + 1), 1)
distance(u, v) = √((2(u − cx)·stretch)² + (2(v − cy))²) / norm      (a forma do `vignette_shade`)
a_u(r) = r·norm / (2·stretch)       a_v(r) = r·norm / 2             (semi-eixos da elipse `distance = r`)
```

O valor que o ponteiro dá a um handle é a inversa (`radius = distance`, `softness = distance − radius`, centro = o
ponto), quantizada pela grade do catálogo (`quantize_control`) e presa à sua faixa (`radius` 0–1,5 passo 0,01;
`softness` 0,05–1 passo 0,01; centro 0–1 passo 0,01).

Fora do escopo desta entrega (registrado no backlog): handles de `roundness`/`intensity`/`color`; **gravar chaves pelo
handle** (BL082.37); `fade` e `filmGrain`; escopo `node` e as alças de um clipe de plano (`shot`, recusadas com
`scope_not_supported`, BL082.41/BL082.64); luzes spot/direcional; câmera espacial; snapping, seleção
múltipla e grupos; ponteiro físico na WKWebView; desempenho integrado; Windows.

## Contrato do core / biblioteca

Geometria pura: `directed_scene::fx::{VignetteShape, VignetteParams, VignetteHandle, HandleValue, hit_handle,
on_picture, vignette_params_at}` (`handles.rs`). Visão por clipe e quadro: `scene_interaction::{FxTarget,
FxHandlesView, FxOverlay}`; cola com o projeto: `scene_project/fx_handles.rs`. «Este clipe tem alças?» tem **uma** resposta,
`FxClip::handles_refusal` (`handles.rs`; `has_handles` é a ausência da recusa): `NoHandles::NotVignette` (`fx_has_no_handles`,
um efeito sem parâmetro espacial) ou `NoHandles::ScopeNotSupported` (`scope_not_supported`, uma vinheta num plano, F062).
`FxHandlesView::of` a devolve como erro tipado, a poda do alvo (`fx_clip_has_handles`, só `pub(crate)`), a ação `handles` e o
botão do inspetor a consultam; nenhum adaptador compara o id do efeito nem o escopo.

Duas operações novas (`SceneCommand::FxSelect` / `FxHandles`, `crates/core/src/ops.rs`):

- `scene.fx.select {scene, clip?}` — **estado de sessão**, não edição (nenhuma entrada de histórico, `document_changed:
  false`). Define o clipe-alvo; esvazia a seleção de entidades (`selection_cleared`) e cancela um arrasto vivo
  (`cancelled_drag`). `clip` nulo/omitido limpa. Um efeito sem handles (`fade`, `filmGrain`) é recusado como
  `fx_has_no_handles`; uma vinheta num plano (F062), como `scope_not_supported` (alvo = o clipe, `details {clip, scope}`);
  clipe inexistente, `unknown_element`.
- `scene.fx.handles {scene, clip?, frame|time, probe?{handle, point{u,v}}}` — leitura. Recusa como `scene.fx.select`
  (`fx_has_no_handles`, `scope_not_supported`, `unknown_element`). Responde `selected`, `visible`,
  `reason` (`clip_not_in_frame`, `degenerate_output`), `handles[{id,param,kind,at}]`, `disabled_handles[{id,reason,at?}]`,
  `ellipses[{id,center,rx,ry}]`, `values`, `active` (o handle do arrasto aberto **desse** clipe; outro clipe da cena responde `null`, e um arrasto que a cena
  deixou para trás também: é o `fx.active` que o overlay desenha, pelo mesmo critério `drag_can_apply`), `metrics{hit_radius,unit}`. Com `probe` faz um *dry run* do mesmo
  `set_param` que o arrasto enviaria (`probe.ok/value/would_change`).

O arrasto **reaproveita** `scene.fx.gesture.*`: o press num handle abre o gesto (`fx_gesture_begin`), cada `move` é UM
`set_param` candidato (preview ao vivo no monitor, o documento não muda), o `up` faz o commit (UMA entrada "Edit
effects", nenhuma se nada mudou). O documento final é **bit a bit** o de `scene.fx.edit` com o mesmo valor. Nenhuma
posição do ponteiro faz a camada recusar o `set_param` (o valor é quantizado e preso à faixa, e o `scene.pointer.move` já
exigiu um gesto vivo); se um dia recusasse, seria o erro tipado do `scene.pointer.move`, com o último candidato onde
estava. O único `applied:false` que um ponteiro real alcança neste arrasto é o `not_moved` (regra «Clique não é arrasto»,
abaixo); o `degenerate_pointer` é defensivo (uma saída sem tamanho já não oferece handles).

`handles` (em `scene.fx.handles` e no overlay) saem **já em ordem de prioridade** (`softness`, `radius`, `center`: o mais
externo primeiro, para o centro nunca esconder os outros quando coincidem). O core usa essa ordem para decidir o press
em empate; a página escolhe «o mais perto, empate = o primeiro da lista» e não tem regra própria.

Regras:

- **Alvo único.** `scene.select` (qualquer modo) larga o clipe (`fx_target_dropped`, `cancelled_drag`); `scene.fx.select`
  esvazia a seleção. Um press que não cai num handle oferecido segue o picking de sempre (selecionar larga o alvo; press
  no vazio limpa a seleção e larga o alvo). Trocar de alvo cancela o arrasto: não sobra arrasto-fantasma.
- **Por que o alvo não é uma entidade da seleção.** `fx_target` mora ao lado da `Selection` (estado de sessão, fora do
  documento e do histórico), não dentro dela. A `Selection` já aceita tipos que não são nós (a luz, F060), então um
  `EntityKind::FxClip` caberia e apagaria os ramos de exclusão mútua; a escolha desta entrega foi **não** tocar `address.rs`,
  `tree.rs` e os parsers de `scene.select` para uma alça de uma só família de efeito, mantendo a exclusão mútua nos poucos
  pontos onde ela muda (`scene.select`, `scene.fx.select`, o press). O desenho como entidade é a pendência **BL082.60**.
  Enquanto isso a invariante «o alvo existe no projeto e o clipe tem alças» (`FxClip::has_handles`) é imposta **onde o estado muda**, não por
  leituras: `EditorState::apply` (todo comando, também depois de um erro) e `restore` (`undo()`/`redo()` e reversão de
  transação) chamam `prune_fx_target`; uma leitura (`tree`, `summary`, `scene.gizmo.state`, `scene.selection`) nunca poda e
  nunca publica um clipe que não existe.
- **Uma só resposta para «tem alças?».** A decisão é `FxClip::handles_refusal` (`has_handles` é a sua ausência); o teste de
  unidade `the_target_predicate_says_yes_exactly_when_the_view_is_built` fixa que a poda (`fx_clip_has_handles`) diz sim
  exatamente quando `FxHandlesView::of` monta a visão, para vinheta, vinheta num plano, `fade`, `filmGrain` e clipe inexistente.
  Duas recusas tipadas: `fx_has_no_handles` (efeito sem parâmetro espacial) e `scope_not_supported` (F062, BL082.64: o clipe
  está num plano e o overlay só conhece clipes da saída; `details {clip, scope}`, alvo = o clipe). Quem põe o clipe num plano
  (`set_scope`) derruba o alvo na própria edição, sem leitura; `scene.fx.select`, `scene.fx.handles` (com e sem `probe`),
  `scene.gizmo.hit` e `scene.pointer.down` respondem pela mesma recusa ou pela ausência de handle de efeito. As alças de um
  clipe de plano seguem pendentes (BL082.41).
- **API da biblioteca.** Esta entrega mudou a superfície pública de `scene_interaction::Drag` contra a `main`:
  `Drag::handle()` devolve `Option<Handle>` (um arrasto de vinheta não tem `Handle` do gizmo; o nome do handle é
  `Drag::handle_id()`) e `Drag::tool() -> GizmoTool` deixou de existir (no lugar, `Drag::tool_id()` devolve o nome, `"fx"`
  para a vinheta); nenhum consumidor do workspace usava as versões antigas.
- **A consulta é o clique.** `scene.gizmo.hit` num ponto responde o mesmo `hit` que o `scene.pointer.down` no mesmo ponto:
  `{kind:"fx_handle", clip, handle, param}`, e `refused:"animated_param"` quando o handle está apagado. Uma consulta não
  escreve nada, não exige câmera e mantém o alvo.
- **Clique não é arrasto.** O valor de um handle é **absoluto** (a posição do ponteiro vira o valor), então uma posição
  igual ao ponto da pressão, enquanto nenhuma posição foi aplicada, responde `applied: false, reason: "not_moved"` e não
  toca o candidato (`DragAction::Fx.from`, `fx_handle_update`, que `move` e a posição do `up` dividem). Pressionar e soltar
  no mesmo ponto, com o `up` carregando o ponto, **não escreve nada** e não deixa entrada de histórico: nem quando o clique
  cai um pouco fora do centro do handle (dentro do raio de captura), nem quando o valor do documento está fora da grade do
  controle (`center.x` autorado `0,1234` não vira `0,12` por um clique). Depois que o ponteiro saiu do ponto da pressão
  toda posição conta, também a volta a ele (o usuário arrastou: o valor **daquele** ponto é o que se grava). Testes:
  `a_press_and_release_on_the_same_point_writes_nothing`, `a_press_a_little_off_the_handle_and_released_there_writes_nothing`,
  `a_drag_away_writes_the_value_of_the_point_it_ends_on_even_if_it_came_back`.
- **Alça oferecida × alça apagada.** Quando as duas alcançam o ponteiro, a **oferecida** vence em todo o seu raio de captura
  (`fx_pick` consulta `hit()` antes de `hit_animated()`), mesmo que a apagada, de um parâmetro com chaves, esteja mais perto:
  o clique abre o arrasto da oferecida em vez de responder `animated_param`. É a regra de hoje, não uma garantia de «a mais
  próxima»; a decisão está em **BL082.65** (com a conta reproduzida: 1080×1920, `roundness` 1, clique em 0,74 fica a 0,0203
  do `radius` e a 0,0084 do `softness` apagado).
- **Parâmetro com chaves** (`animated_param`): o handle fica desabilitado, aparece apagado e o press nele é respondido
  `refused: "animated_param"` sem abrir gesto nem escrever nada; não grava chaves nem usa `FX_STATIC_UNDER_KEYS`. Como todo
  press, o recusado primeiro abandona o arrasto ainda aberto (`abandoned.reason: replaced_by_press`): a liberação perdida
  não deixa um arrasto vivo que o próximo `move` ou `up` continuaria editando. Os outros handles continuam.
- **Fora do quadro** (`handle_off_picture`): não é desenhado, não captura o ponteiro e é listado em `disabled_handles`.
  Um press precisa cair no quadro (`u`, `v` em 0..1), então um press além da borda é `invalid_value` do ponteiro.
- **Gesto obsoleto**: cena trocada/desanexada, projeto substituído, undo, revisão nova, clipe removido ou o quadro fora do
  clipe matam o arrasto (e o alvo, quando o clipe deixa de existir: na própria escrita, sem esperar uma leitura). O
  release de um arrasto morto é recusado (`gesture_invalidated`) e não escreve. O overlay o desenha como arrasto nenhum e
  `scene.fx.handles` responde `active: null` pelo mesmo critério (`drag_can_apply`, o que `gizmo_drag_is_dead` nega); teste
  `the_active_handle_of_a_query_follows_the_overlay_when_the_gesture_goes_stale`.
- **Overlay.** Com alvo de efeito o overlay do nó fica oculto (`hidden: fx_target`, `visible:false`) e ganha o bloco
  aditivo `target:"fx"` + `fx{...}`. O carimbo (`gizmo_stamp`) ganha `fx_clip` e o resumo/árvore, `fx_target`/`target`.
  Durante a reprodução o overlay é limpo como o do gizmo.
- **Histórico.** O release de um arrasto de handle grava o rótulo "Edit effects" (a mesma entrada de `scene.fx.edit`).

## CLI headless

`./scripts/headless` (sessão JSONL, sem desktop):

```json
{"op":"scene.fx.select","scene":"street","clip":"v1"}
{"op":"scene.fx.handles","scene":"street","clip":"v1","frame":20}
{"op":"scene.pointer.down","scene":"street","frame":20,"point":{"u":0.50,"v":0.50}}
{"op":"scene.pointer.move","point":{"u":0.55,"v":0.50}}
{"op":"scene.pointer.up","point":{"u":0.55,"v":0.50}}
{"op":"scene.pointer.cancel","reason":"escape"}
```

`scene.pointer.*` carregam coordenadas de quadro, as mesmas do gizmo de F055 (um comando semântico, não um mouse).

## UI e CLI da janela

- Botão "alças" (`IconName::Frame`, `control:fx-handles:<clip>`) no cabeçalho do clipe de vinheta no inspetor, ação de nó
  `handles` (alterna): `./scripts/ui invoke scene.fx.clip:<id> handles`. É o mesmo `scene.fx.select`. O rótulo da ação é
  estático ("Alças no monitor"): ligado ou não é o `state.handles` do nó (e o botão aceso), não um segundo texto.
  Desabilitado com o `disabled_reason` do core (`NoHandles::text`): "Só a vinheta tem alças no monitor" para `fade`/`filmGrain` e
  "As alças só existem para efeitos na saída; este efeito está num plano" para uma vinheta num plano (F062). O código fica em
  `workstation/scene_fx_handles.rs`; `scene_fx.rs` só chama `handles_action` e o botão.
- Estado: nó do clipe `state.handles`; nó `monitor.gizmo` `state.target` (`node`|`fx`) e `state.fx_clip`.
- O desenho e o mouse são da página do runtime (`runtime/src/overlay.js`): elipses recortadas ao quadro (a de `softness`
  tracejada), handles com halo, destaque no hover/arrasto, apagados quando têm chaves, nenhum fora do quadro. A página só
  desenha e posta `overlay.pointer` com `{u,v}` e o carimbo; o core decide. O adapter `scene_gizmo.rs` compara o carimbo
  (acrescenta `epoch` e `selection`), conta mensagens obsoletas (`overlay.stale`) e traduz para `scene.pointer.*`.
- Escape, perda de foco, diálogo (a WebView some), seleção de outro clipe, troca de cena, undo/redo e recarga da página
  cancelam o arrasto sem commit. O Escape vale também para o arrasto que o host mantém sem press na página (aberto por
  `scene.pointer.down` de um script): a página o enxerga em `fx.active` do overlay e posta `cancel` (`reason: escape`);
  sem arrasto aberto o Escape não posta nada. Um diálogo novo continua marcando `Workflow::has_dialog`.

## Evidências e status

| Superfície | Prova | Resultado |
|---|---|---|
| Biblioteca | `crates/core/tests/scene_fx_handles.rs` + `scene_fx_target.rs` (alvo, obsolescência e consulta = clique; ajudantes em `scene_fx_handles_support/`) + testes de unidade de `handles.rs`: posição dos handles contra `vignette_shade` (0 em `radius`, 1 em `radius+softness`) em 16:9, 1:1 e 9:16 com `roundness` 0/0,6/1; linhas de nível das elipses; ida e volta; arrasto ≡ `scene.fx.edit` bit a bit (radius, softness, centro); uma entrada, no-op, undo/redo, cancel; clique sem arrasto não escreve (`not_moved`, também fora da grade e fora do centro da alça); clamp; chaves; fora do quadro; clipe fora do intervalo; só vinheta; troca de alvo nos dois sentidos sem arrasto-fantasma; undo sob arrasto (e `active` da consulta = o do overlay com o gesto obsoleto); clipe removido/cena desanexada; leitura/probe; overlay/carimbo/resumo; capacidades | passam (21 + 10 testes de integração, 11 unidades: 10 de `handles.rs` e a de equivalência do predicado de alças em `scene_interaction/fx_handles.rs`; em F062: a vinheta num plano é recusada por toda porta, o alvo cai na edição e no `redo`, e o processo `tessel-headless` real a recusa igual) |
| Headless real | o mesmo arquivo (processo `tessel-headless` real) e `scripts/scene-fx-handles-proof.py --surfaces headless` | passam (18 verificações, oráculo escrito no script; `final_evidence: true` em árvore limpa) |
| Janela | GPUI `crates/desktop/src/workstation/scene_fx_handles_tests.rs` (8, o novo é a vinheta num plano: ação e botão desabilitados com o motivo do core); `scripts/scene-fx-handles-proof.py --surfaces native` (26 verificações): ação do painel = `scene.fx.select`, `monitor.gizmo target: fx`, página desenhou handles e elipses (pixels do canvas; o `drawn` é opt-in, `TESSEL_VIEWPORT_OVERLAY_ACK=1`, que a prova liga), handle ativo, imagem do monitor segue o candidato e volta com o undo, diálogo cancela o arrasto | passam |
| Página/JS | `runtime/test/fx-handles-overlay.test.mjs` (9, Chrome real, mouse real): parse, desenho, hit, prioridade, arrasto, Escape (do press da página e do arrasto do host em `fx.active`), captura perdida, resize, handle apagado | passam |
| Imagem/export | `crates/render/tests/scene_fx_handles.rs` (2, runtime e FFmpeg reais): perfil de escurecimento lido nos pontos dos handles que o core informa, contra `apply_pass_to_srgb`; export com o clipe alvo, o overlay descrito e um arrasto aberto = export do documento (hashes), só o release muda o export | passam |
| Mutações | `docs/migration/g41-handles/mutate.py` (43 mutantes: 33 Rust, 10 JS; M30–M33 são a recusa de um clipe de plano), relatório `mutation-report.json`; o executor recusa id desconhecido (saída 2) e reconstrói e confere o `dist` do runtime (`restored: false` e saída 2 se falha; também numa interrupção) | todos mortos, arquivos restaurados com hash conferido; depois do corredor, reconstruir `tessel-editor` e `tessel-headless` antes da prova de duas superfícies (BL082.63) |

Gates, hashes e mutações do head final estão no PR e em `docs/backlog.md` (BL082.35 e filhas).

### Limites (não provado)

- **Ponteiro físico na WKWebView** (BL015.2/BL043/BL044 seguem pendentes): as provas de janela usam `scene.pointer.*` e
  os testes JS usam o mouse do Chrome headless, não um mouse nativo na WKWebView.
- **Desempenho** do overlay com alvo de efeito (custo por `fx_overlay`): não medido; nenhum lock de GPU foi usado.
- **Clipes de plano (`shot`)**: sem alças; `scope_not_supported` em todas as portas (BL082.64, concluído). Desenhar as alças no
  quadro de um plano é a pendência BL082.41.
- **Windows**: não executado.
- **Chaves pelo handle**: o handle de um parâmetro com chaves é desabilitado; gravar chave por arrasto é a pendência
  BL082.37.
- **Gesto de efeito aberto pelo inspetor** (slider ou campo de chave): a imagem mostra o candidato, mas as elipses e os
  handles ficam nos valores do documento, então durante o arrasto do slider o contorno não coincide com o escurecimento.
  Só o arrasto de um handle desenha o candidato. Pendência **BL082.61**.
- **Frame da cena no adaptador da janela**: `scene_gizmo.rs` usa o playhead da **sequência** como frame da cena (5 pontos), o
  monitor converte com `scene_frame_at_playhead`; com taxas diferentes entre sequência e cena, overlay e imagem divergem. É
  anterior a esta entrega e vale também para o gizmo de nó (já em `main`, F055). Pendência **BL082.66**.
- **`monitor.gizmo` e a cena exibida**: `state.target`/`state.fx_clip` dizem o alvo da sessão, que pode ser de uma cena que
  o monitor não está exibindo (o core não conhece «cena exibida»). Pendência **BL082.67**.
- **Sem compensação de ponto de agarre**: o primeiro movimento depois de um press até ~0,025 de altura fora do handle
  salta o valor (o gizmo 3D usa âncora; aqui o valor é a posição crua do ponteiro, como o contrato acima descreve). Um
  clique sem deslocamento não salta nada: não escreve (regra «Clique não é arrasto»).
