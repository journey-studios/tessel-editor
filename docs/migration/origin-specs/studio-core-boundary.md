# Studio ↔ core: onde cada decisão mora

**Regra.** `packages/scene-engine` (o core) decide tudo; `apps/scene-studio` é uma casca fina que o expõe.

- O que é legal, o que um gesto resulta, que modelo uma timeline desenha e o que um painel de detalhe mostra são
  **funções puras do core**, testadas lá e alcançáveis pela CLI `scene`.
- O Studio só: lê o ponteiro e as teclas (pixel → frame), desenha, traduz texto (i18n), liga o store (undo,
  salvar) e chama o core. Nada que a CLI não consiga fazer pode existir só no Studio.
- O guard `apps/scene-studio/src/studioCoreBoundary.spec.ts` trava a regra: o Studio não importa `zod`/`node:*`/
  `@tessel/scene-engine/node`; escrita de bundle (`/bundle-*`) só sai dos savers; os arquivos de lógica que
  migraram não voltam para `src/engine`.

Decisões registradas em `scene-fx-decisions.md` (#128 em diante).

## 1. O que foi movido para o core

| Arquivo (antes, no Studio) | O que decide | Mora agora em | Movido |
| --- | --- | --- | --- |
| `engine/fxClipDrag.ts` | mover/redimensionar clipe: arredonda, mínimo 1, vizinha, fantasma de erro | `gestures/fx-clip-drag.ts` (`planFxClipDrag`) | sim |
| `engine/fxTransitionDrag.ts` | borda arrastada → par (alinhamento, duração) mais próximo; limite por alinhamento | `gestures/fx-transition-drag.ts` (`planFxTransitionDrag`, que já devolve `command` + `target`) | sim |
| `engine/fxEdit.ts` | comando de remoção, comandos de inserção/troca vindos da galeria, `FxItemRef` | `gestures/fx-edit.ts` | sim |
| `engine/shotsDrag.ts` | arrasto de borda/corte de plano: comando, modo, pré-visualização | `gestures/shot-drag.ts` (`planShotDrag`, `judgeShotPlan`) | sim |
| `engine/shotsEdit.ts` | ordem e janelas dos planos, edição de duração | `gestures/shot-edit.ts`, `gestures/shot-order.ts` | sim |
| `engine/keyFrameDrag.ts` | arrasto de chave: frame legal, `scaledFrames` (Shift), mover tudo | `gestures/key-frame-drag.ts` | sim |
| (novo) mover/escalar chaves por comando | `move key` / `move all` / `scale` sobre uma trilha de chaves | `gestures/key-retime.ts`, `gestures/key-path-retime.ts` | novo no core |
| `engine/fxTimeline.ts` | barras do `fx` (clipes, trilhas, transições), cor por família, escondido por grupo | `timeline/fx-bars.ts` | sim |
| `engine/keyTracks.ts` | trilhas de chaves (luz, clima, parâmetros) derivadas do bundle | `timeline/key-tracks.ts` | sim |
| barras da câmera e dos planos (várias funções espalhadas) | barras da câmera por plano, barras de planos, janela de edição | `timeline/camera-bars.ts`, `timeline/shot-bars.ts` | sim |
| mapa de rig/follow da timeline | modelo do rig (ossos desenhados, faixas do follow) | `timeline/rig-view.ts` | sim |
| conversão quadro de edição ↔ quadro da instância de cena | `spansOfScene`, `instanceFrameOfEdit` | `gestures/scene-spans.ts` | sim |
| intervalo de frames (clamp/overlap) usado pelos arrastos | `frame-range` | `gestures/frame-range.ts` | sim |
| bundles de teste (`fxTestBundle`, `rigTestBundle`) | fixtures | `@tessel/scene-engine/testing` | sim |
| detalhe de um plano (nunca existiu) | id, intenção, início/fim/duração, transição de entrada | `timeline/shot-detail.ts` (`describeShotDetail`) | novo no core |
| editar início/fim/duração de um plano por campo | `moveCut` / `trimShot` ripple / `setShotDuration` | `gestures/shot-field-edit.ts` (`planShotFieldEdit`) | novo no core |
| detalhe de clipe/transição (nunca existiu) | cabeçalho + um campo por parâmetro, com controle, faixa, padrão e chaves | `properties/fx-inspector.ts` (`describeFxInspector`) | novo no core |
| rótulos dos parâmetros dos efeitos | nome de cada um dos 117 parâmetros em pt-br/en/es | `fx/catalog/fx-param-labels.ts` (`fxParamLabel`) | novo no core |
| ids que um parâmetro `ref` aceita (luzes, caminhos, camadas) | `fxRefIds`, `fxScopeDefinitions` | `fx/fx-ref-ids.ts` (extraído de `fx-explain-clips.ts`) | movido |

## 2. O que fica no Studio, de propósito

| Arquivo | O que faz | Por que não é decisão |
| --- | --- | --- |
| `engine/timelineScale.ts` (`frameAtX`) | pixel do ponteiro → frame da linha | só geometria da tela |
| `engine/rigView.ts`, `layerSpace.ts`, `pathView.ts`, `cameraPath.ts` | qual rig/caminho a seleção quer; mapa imagem ↔ mundo para desenhar alças | desenho; o motor tem as contas (`rigsOfScene`, `drawnBones`, `rigHandles`) |
| `engine/shotsTimeline.ts`, `shotsText.ts`, `fxTitles.ts`, `keyDragText.ts` | texto do que se mostra | i18n e apresentação |
| `engine/liveDrag.ts`, `dragPoint.ts`, `fxDeleteKey.ts` | vida de um arrasto, ponteiro, tecla Delete | eventos do navegador |
| `components/CameraTrack.tsx` (~268/303), `hooks/useCameraClip.ts` (~85) | clamps de pixel → frame ao arrastar a barra da câmera | arredondamento de ponteiro; **o motor julga** a edição (`explainCameraClip`) |
| `inspector/describe*.ts` | traduz o modelo do core para campos do Leva e liga os callbacks do store | adaptadores; o *quê* mostrar vem do core |

## 3. Gesto do Studio → comando do core → CLI

| Gesto no Studio | Comando do core | CLI (`scene …`, JSON, saída 0/1/2, nada gravado sem `--write`) |
| --- | --- | --- |
| mover/redimensionar clipe de efeito | `moveClip`, `resizeClip` | `fx <bundle> apply '[{"op":"moveClip",…}]'` |
| arrastar borda de transição | `resizeTransition` | `fx <bundle> apply '[{"op":"resizeTransition",…}]'` |
| soltar efeito da galeria sobre um corte | `setTransition` | `fx <bundle> transition set …` |
| Delete num clipe/transição | `removeClip`, `removeTransition` | `fx <bundle> apply …` |
| arrastar borda/corte de plano | `trimShot`, `moveCut`, `setShotDuration`, `setEditDuration` | `shots <bundle> trim|move-cut|duration|edit-duration` |
| campo de início/fim/duração do plano | os mesmos comandos, por `planShotFieldEdit` | `shots …` (iguais) |
| arrastar uma chave (frame) | `retimeKeysAtPath` (`mode: key`) | `keys <bundle> <path/keys> move-key <i> --frame N` |
| arrastar a barra entre as chaves | `retimeKeysAtPath` (`mode: move`) | `keys … move-all --shift=N` |
| Shift+arrastar a última chave | `retimeKeysAtPath` (`mode: scale`) | `keys … scale --last N` |
| mover/escalar chaves de progresso de um follow | `retimeFollowKeys`, `setFollowProgress` (rig) | `follow <bundle> <rigId> <followId> move-key|move-all|scale …` |
| abrir o painel de um clipe/transição/plano | `describeFxInspector`, `describeShotDetail` | `inspect <bundle> clip|transition|shot <id> [--frame N]` |
| botão "chave no playhead" / "restaurar" de um parâmetro | `addKeyEdit`/`removeKeyEdit`, `resetFxParamEdit` | `set <bundle> <path>/keys=[…]`, `fx … apply` com `setParam`/`unkeyParam` |

Cada operação do `fx apply` tem um exemplo executado na skill `scene-cli` (seção "Efeitos").

## 4. Auditoria das rotas de gravação (`studio-harness.mjs`)

Toda rota de gravação do harness passa pelo core; nenhuma escreve o bundle por conta própria.

| Rota | O que chama |
| --- | --- |
| `/bundle-edit` | `editBundle` |
| `/bundle-camera`, `/bundle-camera-commands` | `explainCameraClip` + `applyPropertyChanges`; `editBundle` |
| `/bundle-fx`, `/bundle-shots` | `applyPropertyChanges` (que termina em `finalizeBundle`) |
| salvamentos de cena (clima, chaves de clima, looks, chão, iluminação, rig, caminho) | `parseSceneSave` + `applyPropertyChanges` |
| takes e corte da mix (narração) | `makeTakes`, `cutTakesFromMix` |

No Studio, os savers (`engine/save*.ts`) são os únicos arquivos que citam `/bundle-*` (guard spec).

## 5. Painel de detalhe: camada → o que mostra → caminho no registro

Gerado do registro de propriedades e do catálogo; nenhum efeito tem código próprio.

| Camada (seleção) | O painel mostra | Caminho no registro / comando |
| --- | --- | --- |
| Clipe de efeito (`fxClip`) | nome, família (cor), resumo e "quando usar" no idioma; trilha, escopo, início, duração, fade in/out, semente (+ nova semente), escondido; **um bloco por parâmetro** | `fx/<clip>/{track,scope,start,duration,fadeIn,fadeOut,seed,hidden}` e `fx/<clip>/params/<param>` (+ `/keys`) |
| Transição (`fxTransition`) | idem; duração, alinhamento, handles, easing, semente, escondido; parâmetros | `transition/<id>/{duration,alignment,handles,easing,seed,hidden}` e `transition/<id>/params/<param>` |
| Barra de plano (`shot`) | id, "plano n de m", intenção, instância, câmera, transição de entrada, transições de efeito; início/fim/duração | `describeShotDetail` + `planShotFieldEdit` (decisão #14: **nenhuma propriedade** escreve o intervalo de um plano) |
| Chave de follow (`follow` + `keyIndex`) | valor, easing, **frame** (novo), remover | `follow/<rig>/<follow>/…` e `retimeFollowKeys` |
| Faixa de follow / caminho / rig | já tinham painel completo (`describeFollow`, `describePath`, `describeRig`) | `follow…`, `path…`, `rig…` |
| Câmera (barra) | `describeCamera` (movimentos, velocidade, easing) | `camera/<clip>/…` |
| Luz / clima (losango de chave) | `describeLight`, `describeWeather` (valor e easing da chave) | `scene/<id>/light/…/keys`, `scene/<id>/weather/…/keys` |
| Narração (clipe) | `describeNarrationLine` (texto, voz, tempo, take) | `editBundle` `narration` |
| Grupo do outline | `describeGroup` | `editBundle` `outline` |

### Controles por tipo de parâmetro (`FxParamDef.kind`)

| Tipo | Controle | Notas |
| --- | --- | --- |
| `number` | slider com `min`/`max`/`step` e unidade | sem `step`, `(max-min)/100` |
| `boolean` | interruptor | |
| `enum` | seleção | |
| `color` | seletor de cor | 0..1 no bundle, 0..255 no Leva (`colorChannels`) |
| `vec2` | ponto (x, y) | nota diz se é fração da tela ou velocidade |
| `ref` | seleção de luz/caminho/camada (`fxRefIds`) | desabilitada, com nota, quando a cena não tem nenhum |

Todo parâmetro tem **chave no playhead** (desabilitada fora do item: as chaves contam do início dele) e **restaurar**
(uma edição só, parâmetro e chaves, um passo de undo). Uma edição de campo faz o mesmo caminho do arrasto:
`setProperty` → `PropertyChange { domain: "fx" }` → `commitFx` (um passo de undo, prévia ao vivo com `applyFx`);
se o motor recusa, a linha de status do painel mostra o motivo e a camada volta ao último valor confirmado.

O spec `properties/fx-inspector.spec.ts` monta o modelo de **todos os efeitos de `FX_CATALOG`** e confere tipo de
controle, rótulo nos três idiomas e faixa válida; `inspector/describeFx.spec.ts` confere o painel de cada um nos
três idiomas, com a tradução real (chave faltando falha).
