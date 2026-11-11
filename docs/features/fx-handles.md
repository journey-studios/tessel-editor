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
handle** (BL082.37); `fade` e `filmGrain`; escopo shot/node; luzes spot/direcional; câmera espacial; snapping, seleção
múltipla e grupos; ponteiro físico na WKWebView; desempenho integrado; Windows.

## Contrato do core / biblioteca

Geometria pura: `directed_scene::fx::{VignetteShape, VignetteParams, VignetteHandle, HandleValue, hit_handle,
on_picture, vignette_params_at}` (`handles.rs`). Visão por clipe e quadro: `scene_interaction::{FxTarget,
FxHandlesView, FxOverlay}`; cola com o projeto: `scene_project/fx_handles.rs`.

Duas operações novas (`SceneCommand::FxSelect` / `FxHandles`, `crates/core/src/ops.rs`):

- `scene.fx.select {scene, clip?}` — **estado de sessão**, não edição (nenhuma entrada de histórico, `document_changed:
  false`). Define o clipe-alvo; esvazia a seleção de entidades (`selection_cleared`) e cancela um arrasto vivo
  (`cancelled_drag`). `clip` nulo/omitido limpa. Um efeito sem handles (`fade`, `filmGrain`) é recusado como
  `fx_has_no_handles`; clipe inexistente, `unknown_element`.
- `scene.fx.handles {scene, clip?, frame|time, probe?{handle, point{u,v}}}` — leitura. Responde `selected`, `visible`,
  `reason` (`clip_not_in_frame`, `degenerate_output`), `handles[{id,param,kind,at}]`, `disabled_handles[{id,reason,at?}]`,
  `ellipses[{id,center,rx,ry}]`, `values`, `active`, `metrics{hit_radius,unit}`. Com `probe` faz um *dry run* do mesmo
  `set_param` que o arrasto enviaria (`probe.ok/value/would_change`).

O arrasto **reaproveita** `scene.fx.gesture.*`: o press num handle abre o gesto (`fx_gesture_begin`), cada `move` é UM
`set_param` candidato (preview ao vivo no monitor, o documento não muda), o `up` faz o commit (UMA entrada "Edit
effects", nenhuma se nada mudou). O documento final é **bit a bit** o de `scene.fx.edit` com o mesmo valor. Um valor que a
camada recusa mantém o último candidato legal (`applied:false`, `candidate_kept:true`); nenhuma posição do ponteiro
produz essa recusa por construção (o valor é quantizado e preso à faixa), então o ramo só é coberto pelos testes do
gesto de F016 — dito aqui sem eufemismo.

Regras:

- **Alvo único.** `scene.select` (qualquer modo) larga o clipe (`fx_target_dropped`, `cancelled_drag`); `scene.fx.select`
  esvazia a seleção. Um press que não cai num handle oferecido segue o picking de sempre (selecionar larga o alvo; press
  no vazio limpa a seleção e larga o alvo). Trocar de alvo cancela o arrasto: não sobra arrasto-fantasma.
- **Parâmetro com chaves** (`animated_param`): o handle fica desabilitado, aparece apagado e o press nele é respondido
  `refused: "animated_param"` sem abrir gesto nem escrever nada; não grava chaves nem usa `FX_STATIC_UNDER_KEYS`. Os outros
  handles continuam.
- **Fora do quadro** (`handle_off_picture`): não é desenhado, não captura o ponteiro e é listado em `disabled_handles`.
  Um press precisa cair no quadro (`u`, `v` em 0..1), então um press além da borda é `invalid_value` do ponteiro.
- **Gesto obsoleto**: cena trocada/desanexada, projeto substituído, undo, revisão nova, clipe removido ou o quadro fora do
  clipe matam o arrasto (e o alvo, quando o clipe deixa de existir). O release de um arrasto morto não escreve.
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
  `handles` (alterna): `./scripts/ui invoke scene.fx.clip:<id> handles`. É o mesmo `scene.fx.select`. Desabilitado para
  `fade`/`filmGrain` com `disabled_reason` "Só a vinheta tem alças no monitor".
- Estado: nó do clipe `state.handles`; nó `monitor.gizmo` `state.target` (`node`|`fx`) e `state.fx_clip`.
- O desenho e o mouse são da página do runtime (`runtime/src/overlay.js`): elipses recortadas ao quadro (a de `softness`
  tracejada), handles com halo, destaque no hover/arrasto, apagados quando têm chaves, nenhum fora do quadro. A página só
  desenha e posta `overlay.pointer` com `{u,v}` e o carimbo; o core decide. O adapter `scene_gizmo.rs` compara o carimbo
  (acrescenta `epoch` e `selection`), conta mensagens obsoletas (`overlay.stale`) e traduz para `scene.pointer.*`.
- Escape, perda de foco, diálogo (a WebView some), seleção de outro clipe, troca de cena, undo/redo e recarga da página
  cancelam o arrasto sem commit. Um diálogo novo continua marcando `Workflow::has_dialog`.

## Evidências e status

| Superfície | Prova | Resultado |
|---|---|---|
| Biblioteca | `crates/core/tests/scene_fx_handles.rs` (19) + testes de unidade de `handles.rs` e `fx_handles.rs`: posição dos handles contra `vignette_shade` (0 em `radius`, 1 em `radius+softness`) em 16:9, 1:1 e 9:16 com `roundness` 0/0,6/1; linhas de nível das elipses; ida e volta; arrasto ≡ `scene.fx.edit` bit a bit (radius, softness, centro); uma entrada, no-op, undo/redo, cancel; clamp; chaves; fora do quadro; clipe fora do intervalo; só vinheta; troca de alvo nos dois sentidos sem arrasto-fantasma; undo sob arrasto; clipe removido/cena desanexada; leitura/probe; overlay/carimbo/resumo; capacidades | passam |
| Headless real | o mesmo arquivo (processo `tessel-headless` real) e `scripts/scene-fx-handles-proof.py --surfaces headless` | passam (18 verificações, oráculo escrito no script) |
| Janela | GPUI `crates/desktop/src/workstation/scene_fx_handles_tests.rs` (7); `scripts/scene-fx-handles-proof.py --surfaces native` (26 verificações): ação do painel = `scene.fx.select`, `monitor.gizmo target: fx`, página desenhou handles e elipses (pixels do canvas), handle ativo, imagem do monitor segue o candidato e volta com o undo, diálogo cancela o arrasto | passam |
| Página/JS | `runtime/test/fx-handles-overlay.test.mjs` (8, Chrome real, mouse real): parse, desenho, hit, prioridade, arrasto, Escape, captura perdida, resize, handle apagado | passam |
| Imagem/export | `crates/render/tests/scene_fx_handles.rs` (2, runtime e FFmpeg reais): perfil de escurecimento lido nos pontos dos handles que o core informa, contra `apply_pass_to_srgb`; export com o clipe alvo, o overlay descrito e um arrasto aberto = export do documento (hashes), só o release muda o export | passam |
| Mutações | `docs/migration/g41-handles/mutate.py` | ver o relatório e o resumo no PR |

Gates, hashes e mutações do head final estão no PR e em `docs/backlog.md` (BL082.35 e filhas).

### Limites (não provado)

- **Ponteiro físico na WKWebView** (BL015.2/BL043/BL044 seguem pendentes): as provas de janela usam `scene.pointer.*` e
  os testes JS usam o mouse do Chrome headless, não um mouse nativo na WKWebView.
- **Desempenho** do overlay com alvo de efeito (custo por `fx_overlay`): não medido; nenhum lock de GPU foi usado.
- **Windows**: não executado.
- **Chaves pelo handle**: o handle de um parâmetro com chaves é desabilitado; gravar chave por arrasto é a pendência
  BL082.37.
- A recusa da camada durante o arrasto (`applied:false`, `candidate_kept:true`) não é alcançada por nenhuma posição de
  ponteiro; o ramo é coberto apenas pelos testes do gesto de F016.
