# F016 — Adicionar e processar efeitos (vinheta, fade e filmGrain; escopos de saída e de plano)

## Status

**Catálogo parcial; cobertura da vinheta por superfície na tabela P0+P1 abaixo.** Uma fatia completa de ponta a ponta: o efeito **vinheta** com escopo **saída** (a imagem
final da cena dirigida), criado, editado e removido pela biblioteca, pelo JSONL sem janela e pela janela;
avaliado por quadro no core; persistido; desfeito em uma entrada; desenhado pelo monitor e pelo export.
Não é a migração do motor de efeitos: o catálogo tem **três** efeitos (vinheta, `fade` desde P2 e `filmGrain` desde P3) e o piloto tem 59 shaders. O restante
(catálogo, transições, edição de shots) continua pendente em BL028/BL036/BL082 e **não** há placeholder
habilitado para ele.

### Histórico anterior à entrega P0+P1

A tabela a seguir registra os heads anteriores e a correção R1 ainda sem build naquela ocasião. Não descreve o estado atual; veja a atualização P0+P1 e os registros por SHA abaixo.

| Superfície | Estado | Evidência (ver "Evidências": o que foi executado e o que **não** foi) |
|---|---|---|
| Lib | A validar | `crates/core/tests/scene_fx.rs` (16 do head anterior + 9 da correção R1), `reference.rs` (5): **a correção R1 foi escrita sem compilar** (disco) |
| CLI headless (JSONL) | A validar | `scene.fx.*` pelo `tessel-headless` real igual à lib (`scene_fx.rs`); export headless com FFmpeg real (`crates/render/tests/scene_fx_export.rs`); orçamento/avisos novos só na resposta (lib/JSONL) |
| UI / CLI da janela | V (semântico, head anterior) · A validar (R1) · Parcial (ponteiro físico) | 7 testes GPUI do head anterior + 6 novos da R1 (gesto/cena, quantização) **não compilados**; `scripts/scene-fx-ui-proof.py` 70/70 no head anterior, **não reexecutada** depois do endurecimento (A2); captura nativa só de observação |
| Pixels no monitor | V no corpus (head anterior) | monitor == `scene.render` (hash) e sondas == vinheta independente em Python (pior 0,5/255) |
| Pixels no runtime após a correção do ping-pong | V (SwiftShader) | `directed-scene-fx-browser.test.mjs` 7/7: ordem com cores distintas, 5 e 9 passes, resize, erro e perda de contexto; igual à referência nas duas ordens |
| Pixels no export | V no corpus (head anterior) | MP4 decodificado == referência sobre o baseline decodificado (pior 7/255 pela lib, 7 pela janela), fora do clipe == baseline |

**Atualização P0+P1 (branch `t3/fx-candidate-preview`, BL036/BL018/BL019/BL082.6/.16/.17).** O código da correção R1
foi **compilado e executado** neste head (antes: "não compilado"); o gesto de efeito, o aviso de orçamento e a
inserção no quadro exibido são do **core** e passam pelos mesmos handlers em lib, JSONL e janela; o corpus pequeno da
vinheta tem proveniência fixada. Estado por superfície depois desta entrega (números reexecutados no head `b5b077e`; ver "Âncoras de SHA"):

| Superfície | Estado | Evidência executada neste branch |
|---|---|---|
| Lib (gesto, aviso, tempo→quadro, corpus) | V | `crates/core/tests/scene_fx_gesture.rs` (16), `pilot_fx_corpus.rs` (5) |
| CLI headless (JSONL, processo real, sem janela) | V | `scene_fx_gesture.rs::the_headless_executable_runs_the_gesture_without_a_window…` (processo `tessel-headless`), mesmos `scene.fx.gesture.*` |
| UI GPUI (ponteiro injetado, teclado do adaptador, **monitor**) | V (entrada injetada; **não** é ponteiro/teclado físico) | `workstation/scene_fx_tests.rs` (25 em `b5b077e`, incluindo os 3 do monitor e o `set` semântico de ponto/cor) |
| Janela real, só comandos semânticos | V | `scripts/scene-fx-ui-proof.py` **115/115 em `b5b077e`** (`op`/`get`/`set`/`invoke`, instância privada, lock de GPU; 113/113 no `88d1715` anterior) |
| Pixels (runtime/Chrome SwiftShader) | V | `crates/render/tests/scene_fx_gesture_frames.rs` (candidato antes do commit, commit == prévia, cancelar, undo/redo) |


## Decisão de domínio (antes de implementar)

O efeito vive **uma vez**, em `fx`, chave de primeiro nível do **documento do bundle dirigido**
(`DirectedScene`), não na timeline de sequências do editor e não numa cópia no plano:

- o bundle já é o dado canônico e sem perdas (`document()`); o undo, a persistência (o documento da cena
  viaja no projeto), o pacote e o intercâmbio já o carregam sem código novo;
- o `ScenePlan`/`PlanTimeline` já são a única avaliação por quadro que monitor e export consomem, então
  um `fx` resolvido por quadro dentro do plano dá **a mesma avaliação offline, de preview e de export**;
- duplicar na sequência criaria dois modelos a reconciliar. A fatia **não** põe nada na timeline do projeto.

Não é um bundle "do piloto": a chave `fx` é uma extensão **do editor** ao documento (campos desconhecidos
continuam preservados e avisados, `UNKNOWN_FIELD`; `fx` é lido, validado e normalizado). O piloto é uma
especificação a reabrir, não algo que se afirme portado.

### Formato (`catalogVersion` 1)

```json
"fx": {
  "catalogVersion": 1,
  "tracks": [{
    "id": "fx-track-1", "label": "FX 1", "hidden": false,
    "clips": [{
      "id": "fx-1", "effect": "vignette",
      "startFrame": 0, "durationFrames": 48,
      "params": { "intensity": 0.7, "radius": 0.75, "softness": 0.45, "roundness": 0.6,
                  "center": { "x": 0.5, "y": 0.5 }, "color": { "r": 0, "g": 0, "b": 0 } },
      "fadeInFrames": 8, "fadeOutFrames": 8, "hidden": false, "scope": { "kind": "output" }
    }]
  }],
  "transitions": []
}
```

- Frames de edição (inteiros); o fim é exclusivo. `params` é escrito como o catálogo tipa (omitir usa o
  padrão). Fades de 0, `hidden: false` e `scope` de saída são grafia canônica (omitidos); uma camada sem
  clipes sai do documento, então adicionar e remover devolve o documento byte a byte.
- Sem `fx` (ou vazia) o plano **não tem a chave `fx`** e os bytes do plano de antes não mudam (golden).
- Recusado com código tipado, nunca ignorado: escopo `node` (`FX_SCOPE_NOT_YET`; o escopo `shot` é lido desde
  F062, ver [fx-shot-scope.md](fx-shot-scope.md)), qualquer entrada em `transitions` (`FX_TRANSITION_NOT_YET`), `catalogVersion > 1`
  (`CATALOG_TOO_NEW`), efeito fora do catálogo (`UNKNOWN_EFFECT`), parâmetro desconhecido, de outro tipo ou
  fora da faixa, sobreposição de clipes na mesma trilha (`FX_CLIP_OVERLAP`), clipe vazio ou fora da edição,
  fades que não cabem (`ENVELOPE_TOO_LONG`), IDs duplicados/reservados e limites (ver abaixo).

**IDs** (trilhas e clipes) seguem a regra do piloto `^[a-z0-9][a-z0-9_-]{0,63}$` **sem diferenciar maiúsculas**:
letras ASCII de qualquer caixa, dígitos, `-` e `_`; começa por letra ou dígito (não há `.`, nem `-`/`_` inicial);
até 64 bytes. As palavras reservadas `track`, `fx` e `transition` valem em **qualquer caixa** (`FX`, `Track`,
`TRANSITION` são recusados, comparação em minúsculas como no piloto). Caixa mista é aceita e preservada como escrita
(`Intro-1`). A recusa vem do importador (`INVALID_VALUE`/`RESERVED_ID`) e, num `scene.fx.edit`, é atômica
(`invalid_value`/`reserved_id`, índice da op, nada muda).

**Limites** (64 trilhas, 1024 clipes por trilha) são problemas do **importador** com o código `FX_LIMIT`.
Numa edição que estoura o limite o candidato é recusado pelo importador: o código do comando é
`invalid_bundle` e o issue interno é `FX_LIMIT` (`details.issues[].code`); **não existe** comando `fx_limit`.

### Catálogo (`directed_scene::fx::catalog`)

A tabela está na **ordem do catálogo** (a ordem dos parâmetros é a de `params` do clipe e a do contrato com o
runtime); a seção da janela mostra **intensidade primeiro** por escolha de apresentação (`controls_of`
ordena `intensity` na frente), não por causa do catálogo.

| Efeito | Parâmetro | Tipo | Faixa | Padrão |
|---|---|---|---|---|
| `vignette` | `radius` | número | 0–1,5 | 0,75 |
| | `softness` | número | 0,05–1 | 0,45 |
| | `intensity` | número | 0–1 | 0,7 |
| | `color` | cor `{r,g,b}` em luz linear | 0–1 | preto |
| | `roundness` | número | 0–1 | 0,6 |
| | `center` | ponto (`x` à direita, `y` **para baixo**) | 0–1 | (0,5; 0,5) |
| `fade` | `color` | cor `{r,g,b}` em luz linear | 0–1 | preto |
| | `amount` | número | 0–1 | 1 |
| | `intensity` | número | 0–1 | 1 |

As faixas, os tipos e a ordem são **testados automaticamente** contra o runtime: `runtime/test/fixtures/fx-catalog.json`
é a projeção do catálogo Rust (`the_runtime_contract_fixture_is_the_catalog` compara e, com
`TESSEL_UPDATE_FX_CONTRACT=1`, regrava) e `runtime/test/output-fx.test.mjs` compara o `OUTPUT_FX_EFFECTS` do runtime
com o mesmo arquivo (nomes, tipo, ordem e faixas): uma faixa mudada só de um lado reprova.

`typical_duration_frames` = 96 (duração padrão de um clipe adicionado sem duração, cortada no fim da edição).

### Avaliação determinística por quadro

`Fx::resolve(frame)` devolve um `FxPass` por clipe visível que cobre o quadro, **por etapa e depois por trilha**:
primeiro os clipes de escopo `shot` (o *deck*; só cobrem o quadro quando ele é do seu plano, porque o importador os
mantém inteiros dentro dele), depois os de `output`, cada etapa na ordem das trilhas e dos clipes (ver
[fx-shot-scope.md](fx-shot-scope.md)). O peso do envelope é amostrado no **meio** do quadro:
`min((f + 0,5)/fadeIn, (dur − f − 0,5)/fadeOut)`; `amount = clamp(intensity × weight)`. `amount ≤ 0`
é identidade exata (sem ida e volta sRGB, então intensidade 0 não move um pixel).

Vinheta (especificação do shader do piloto, transcrita em `reference.rs` e **independentemente** em Python na
prova da janela): distância `|(p − centro)·2·forma| / max(|forma|,1)` com `forma = (mix(1, w/h, roundness), 1)`,
`shade = smoothstep(radius, radius+softness, distância)`, mistura com `color` em **luz linear** (curvas
IEC 61966-2-1 exatas) por `shade × amount`.

### Plano e runtime

`ScenePlan.fx` (chave aditiva, só quando há passes): `trackId`, `clipId`, `effect`, `frameInClip`, `weight`,
`amount`, `params`, `scope: {kind: "output"}` (ou, desde F062, `{kind: "shot", shotId}`: ver
[fx-shot-scope.md](fx-shot-scope.md)). `Capability::OutputFx` (`outputFx`, a 10ª) entra em `requires`
quando há passe; um runtime sem ela recusa o plano pelo nome. (Um passe de plano exige também `fxShotScope`, a 13ª.) O validador recalcula `amount` e recusa um plano
adulterado. `PlanTimeline::at_frame` re-resolve `fx` e `requires` por quadro (igual a `ScenePlan::build` em
todos os quadros, testado). O runtime só desenha (`runtime/src/output-fx.js`, `OutputFxPipeline`):

- **Pipeline retido** por compositor: a cena, a câmera, a geometria de tela cheia e **um `ShaderMaterial` por efeito**
  são criados uma vez e reaproveitados em todos os quadros e passes (os uniformes são trocados por passe, nada é
  recompilado). Descarte: `dispose()` libera geometria e materiais; perda de contexto (`dropRetained`), erro de
  um passe (o pipeline é descartado, `fx_pipeline_discarded`, e recriado no quadro seguinte) e `retain: false`
  (descartado a cada quadro) o fazem; resize só troca os alvos. Contadores: `fx_pipeline_created/discarded`,
  `fx_program_created/reused`; `retained()` informa `fx_pipeline_entries` e `fx_material_entries`.
- **Ping-pong de verdade**: o passe `i` lê o alvo `(i-1) % 2` e escreve o `i % 2`: **dois** alvos sem MSAA e **sem
  depth** (`depthBuffer: false`), qualquer que seja o número de passes (antes: um alvo com depth por passe). A
  ordem dos passes é a do plano; o último passe vai ao alvo do shot/saída que o monitor e o export leem.
- Efeito ou escopo desconhecido é `unsupported` tipado, nunca pulado; a busca do efeito e do parâmetro é por
  **propriedade própria** (`Object.hasOwn`), então `constructor`, `toString`, `__proto__`, `hasOwnProperty` e
  nomes não-string dão `unsupported`/parâmetro desconhecido tipados, não `TypeError`.
- `frame.sources.directed_scene_fx` conta os passes; `directed_scene_fx_shot` (F062) conta os que são de um plano.

**Medição pareada** (`runtime/scripts/measure-fx-cost.mjs`, mesmo cenário antes/depois: parede plana de 1280×720,
mediana de 30 quadros, 0/1/2/8 passes, pelo bridge real em Chrome headless). **Só SwiftShader (GL por software)**:
compara dois builds do mesmo runtime na mesma máquina e **não** é a velocidade de uma GPU nativa nem deve ser
citada como tal.

| Passes | Antes (ms/quadro) | Depois (ms/quadro) | Alvos vivos antes → depois |
|---|---|---|---|
| 0 | 54,3 | 55,6 (ruído; sem passes não há mudança) | 1 → 1 |
| 1 | 89,5 | 70,9 | 2 → 3 |
| 2 | 94,1 | 68,9 | 3 → 3 |
| 8 | 121,5 | 91,8 | 9 → 3 |

### Orçamento de passes (política do core)

`FX_PASS_BUDGET = 8` (o piloto avisa no mesmo número). **Política: avisar, nunca recusar** — o runtime não tem regra
de domínio e uma recusa tornaria impossível abrir um bundle que o piloto aceita. `Fx::pass_budget(duração)` varre
os clipes visíveis (clipes e trilhas ocultos não contam; o fim é exclusivo) e devolve o pico, o primeiro quadro
acima e quantos quadros ficam acima. `scene.fx.list` e toda resposta de `scene.fx.edit` (dry-run, no-op e real)
trazem `budget {max_passes, policy:"warn", peak_passes, over_budget, first_frame_over, frames_over}` e
`warnings[]` (`{code:"FX_PASS_BUDGET", ...}` só quando há quadros acima); `scene.fx.frame` traz
`max_passes`, `over_budget` e `warnings`; `scene.fx.catalog` declara a política (`scope.pass_budget`). O plano **mantém
todos** os passes na ordem das trilhas e o runtime desenha todos (dois alvos qualquer que seja a contagem).
A janela mostra o aviso (BL082.16, ver "Gesto de efeito").

## Entrada compartilhada (lib · JSONL · janela)

Core: `scene_edit::fx` (`FxOp`, `apply_fx`/`apply_fx_with`/`dry_run_fx`) e `scene_project::fx`. O lote é
**atômico** (clona o JSON, valida cada op, reimporta o candidato com as regras do importador e dos gates
estrutural/temporal; recusa ⇒ nada muda e a op é nomeada). Um comando = uma entrada de undo ("Edit effects");
no-op e dry-run não gravam nada.

| Op JSONL | Efeito |
|---|---|
| `scene.fx.catalog` | catálogo, faixas, escopo e recusas como dado |
| `scene.fx.list {scene}` | trilhas e clipes |
| `scene.fx.frame {scene, frame\|time}` | passes resolvidos de um quadro |
| `scene.fx.edit {scene, ops[], dry_run?}` | lote atômico de `add_clip {effect, start_frame, duration_frames?, track?, id?, params?}`, `remove_clip {clip}`, `set_param {clip, param, value}`, `set_field {clip, field: start_frame\|duration_frames\|fade_in_frames\|fade_out_frames, value}`, `set_hidden {clip, hidden}` |

Sem trilha, `add_clip` usa a primeira trilha **visível** em que cabe (uma trilha oculta não desenharia o clipe) ou
cria `fx-track-N`; a trilha oculta só recebe um clipe quando é **nomeada** em `track`. O ID do clipe é reservado
antes do da trilha (um clipe `id: "fx-track-1"` que precisa de trilha nova não colide: a trilha vira
`fx-track-2`). O rótulo da trilha nova é `FX n` com o **menor n livre** (remover uma trilha e criar outra não repete
rótulo); sem ID de clipe, `fx-N`; remover o último clipe de uma trilha remove a trilha. Um lote que não muda nada
(`ops: []` ou só no-ops) deixa o documento, a cena e o histórico como estão — inclusive uma camada `fx` vazia
escrita pelo autor (só um lote que muda algo e termina vazio devolve o documento sem `fx`).

**Reduzir um clipe abaixo dos fades é recusado**, não ajustado: o piloto apara os fades à nova duração; este editor
recusa com `invalid_bundle`/`ENVELOPE_TOO_LONG` e não altera nenhum valor que o autor não nomeou. Os dois caminhos
são explícitos: encurtar os fades no mesmo lote ou antes. Códigos de erro do comando (`command_code`):
`unknown_effect`, `unknown_param`, `param_type`, `param_out_of_range`, `unknown_fx_track`, `unknown_fx_clip`,
`reserved_id`, `duplicate_id`, `fx_clip_overlap`, `fx_clip_empty`, `fx_clip_out_of_range`, `envelope_too_long`,
`catalog_too_new`, `fx_scope_not_yet`, `fx_transition_not_yet`, e desde F062 `fx_scope_mismatch` e `unknown_shot`
(ver [fx-shot-scope.md](fx-shot-scope.md)); um candidato recusado pelo
importador mantém `invalid_bundle` com os issues em `details.issues[]` (os limites chegam como `FX_LIMIT`).

### Janela

Seção **Efeitos** no inspetor de cena (`workstation/scene_fx.rs`): botão `+` (adicionar vinheta) e, por clipe,
título, interruptor de visibilidade, remover e sliders do Kit (intensidade primeiro, raio, suavidade, cor R/G/B,
arredondamento, centro X/Y, início, duração, fade in/out). Cada gesto é **um** `scene.fx.edit` por
`Workstation::execute` (um slider guarda o valor no arrasto e envia **uma** edição ao soltar; Escape descarta;
perda de foco confirma). Recusas do core aparecem como o aviso do painel (`scene.panel.state.notice`).

- **O gesto pertence à cena em que começou.** O buffer do arrasto é chaveado por cena, clipe e controle; trocar a cena
  do painel (seletor ou `set scene.picker`) o **descarta**; mesmo assim o commit confere que a cena do gesto é a que o
  painel mostra (`stale_gesture`) e nunca edita o `fx-1` de outra cena. Um clipe removido durante o gesto
  mostra `unknown_element` no painel (não é engolido).
- **O slider grava o passo, não ruído de f32.** O Kit entrega `f32` (`0.4` chega como `0.4000000059604645`): o valor
  é alinhado à grade do passo (ancorada no mínimo), limitado à faixa e escrito nas casas do passo antes do
  `SetParam`; um valor igual ao do documento não envia comando nem entrada de undo. Só o gesto do ponteiro quantiza:
  um `set` por nó/CLI é o valor do autor e é validado, não arredondado.

Nós semânticos: `scene.fx` (ação `add:vignette`; `state.clip_count`, `effects`, `scope`) e
`scene.fx.clip:<id>` (campos numéricos `intensity`, `radius`, `softness`, `roundness`, `center_x/_y`, `color_r/_g/_b`,
`start_frame`, `duration_frames`, `fade_in_frames`, `fade_out_frames` e `hidden`; ações `remove`, `toggle_hidden`).
`set` de um componente de ponto/cor mantém os irmãos como o documento os tem; quadros exigem inteiros.

### Gesto de efeito (`scene.fx.gesture.*`, BL082.6)

O candidato de um arrasto/teclado é do **core** (`scene_project/fx_gesture.rs`), no mesmo *slot* único dos gestos
de cena (`scene_interaction::GestureSlot`); a janela só traduz entrada em comandos. Ciclo JSONL (os mesmos
`SceneCommand::FxGesture` da janela): `scene.fx.gesture.begin {scene, clip, label?}` → `update {gesture, ops[]}`
(quantas vezes) → `commit {gesture}` | `cancel {gesture}`; `status`. `scene.fx.list/frame` e `scene.plan` aceitam
`gesture` e então mostram/desenham o **candidato**; o monitor e a seção Efeitos fazem o mesmo.

- **Amarra** projeto, cena, clipe e a revisão-base (`Arc::ptr_eq` da cena + revisão do projeto). Cada `update` deriva
  da **base** do `begin`, nunca do candidato anterior (um valor volta ao original se o `update` o restaura).
- **A prévia não escreve** documento, arquivo, histórico nem revisão. Um `update` inválido devolve o erro tipado
  (`param_out_of_range`, `invalid_op`, `gesture_op_not_allowed`, …) e **mantém o último candidato legal**;
  a recusa fica em `gesture.refusal`.
- **Cancelar, trocar de cena/projeto, undo/edição externa e revisão velha nunca confirmam valores antigos**:
  o gesto vira `gesture_invalidated` (só pode ser cancelado), outro projeto é `unknown_gesture`, outra sessão
  `no_gesture`. Perda de foco da janela é um `commit` do que está aberto (definido, sem *commit* duplo: o gesto
  some do slot); Escape é `cancel`.
- **Plano e export.** `scene.plan`/`scene.evaluate`/`scene.fx.list|frame` com `gesture` consomem o **candidato** (é
  assim que o monitor o desenha). O export de cena e o plano **sem** `gesture` usam só a cena **confirmada**
  (`scene_export/snapshot.rs`): um gesto aberto nunca entra num vídeo. Um gesto que o core já não aplica
  (`gesture_invalidated`) não é mais pedido pelo monitor, que volta a mostrar o documento.
  *Revisão independente do head `c834ff7`:* esta regra do monitor e a quantização por casas decimais do passo
  (`0.25` não vira `0.3`; teste novo em `control.rs`) foram **escritas sem compilar** (disco abaixo do piso); só
  `cargo fmt --check` foi executado. **Executadas depois em `b5b077e`** (ver "Âncoras de SHA"): `check`, `clippy`,
  testes do workspace e prova da janela passaram, incluindo o teste novo de `control.rs`.
  *BL082.20 (teste de janela do **monitor**, executado em `b5b077e`):* o gesto invalidado por **undo** ou por
  **edição externa** no meio do arrasto continua no slot do core, mas o monitor **não o pede** (o plano que ele
  guarda é o da cena confirmada, sem `gesture_invalidated` e sem falha na tela), o painel mostra o mesmo valor, o
  `commit` velho é recusado com `gesture_invalidated` sem entrada de undo (e o redo continua disponível), e
  trocar de cena, Escape e um polegar em repouso deixam o monitor no documento. Os testes leem o pedido que o
  monitor guarda (`ViewportHost::directed_desired_plan`, só em teste: uma janela de teste não tem WebView), não
  pixels. Corrigido junto: cancelar o gesto ao trocar de cena e ao desistir de um `commit` recusado agora **sincroniza
  o monitor na hora** (antes ele só voltava ao documento no próximo comando alheio).
  O `set` semântico de uma componente de ponto/cor (`center_y`, `color_g`...) agora parte do **documento**, não do
  candidato de um gesto aberto no mesmo clipe: o irmão não confirmado de um arrasto não vaza para o documento
  (`a_semantic_set_never_writes_the_candidate_of_an_open_gesture_into_the_document`, executado em `b5b077e`).
- **Um commit que muda algo é UMA entrada de undo ("Edit effects")**; commit sem mudança e cancelar são **zero**.
  Undo/redo restauram valores **e pixels** (prova nos pixels em `scene_fx_gesture_frames.rs`).
- **Inserção no quadro exibido**: o botão `+` usa `EditorState::scene_frame_at_playhead` (o tempo do playhead da
  sequência convertido pelo relógio do **bundle**; 30 fps → 24 fps: playhead 15 = quadro 12), a duração cabe na
  edição e um quadro fora da edição é recusado atomicamente (`fx_clip_out_of_range`). `add_clip` aceita exatamente
  um entre `start_frame` e `start_time` (inteiro em segundos ou `{num,den}`).
- **Teclado/faixa/quantização** são uma só regra (`directed_scene::fx::quantize_control`) usada pelo ponteiro e pelo
  teclado do adaptador; confirmar e cancelar são os mesmos comandos. Isto prova o **adaptador**, não um teclado físico.
- **Aviso de orçamento (BL082.16)**: a seção Efeitos mostra a linha `fx-budget-warning` e o nó `scene.fx.budget`
  (`visible`, `state.over_budget/peak_passes/first_frame_over/frames_over/message/warnings[FX_PASS_BUDGET]`,
  `state.source: document|gesture`) a partir da **mesma** visão; atualiza ao editar, ocultar, remover, no candidato
  de um gesto e na troca de cena. **Só avisa**: nunca trunca, recusa, reduz qualidade ou esconde (o plano mantém
  os nove passes).

| Código | Quando |
|---|---|
| `gesture_active` / `gesture_kind_mismatch` | já há um gesto aberto / outro tipo de gesto (também em `scene.fx.list/inspect/frame` com o ID de um gesto `properties`) |
| `gesture_target_mismatch` / `gesture_scene_mismatch` / `gesture_op_not_allowed` | op fora do clipe ou cena do gesto |
| `gesture_invalidated` / `unknown_gesture` / `no_gesture` | base velha, outro projeto, outra sessão |
| `param_out_of_range`, `invalid_op`, `unknown_element`, `invalid_bundle` | refusas do `apply_fx` no candidato |

### Corpus pequeno e proveniência (P0, BL036/BL018/BL019)

`docs/migration/fixtures/pilot-fx/{cases.json,manifest.json}`, gerados e verificados por
`scripts/pilot-fx-corpus.py generate|verify [--origin PATH]`. As expectativas vêm das **fórmulas do piloto** (GLSL,
cor, envelope de fade, catálogo) lidas **somente por `git show`** do snapshot `d40ac3f7c0` da origem (nada
instalado, executado nem alterado) e calculadas por uma implementação **independente em Python**; o manifesto grava
git blob e sha256 de cada fonte, o hash do corpus e os bloqueios. Casos: janela de quadros (defaults, fades, intensidade
0, oculto, ordem de trilhas, últimos quadros), pixels de uma imagem 8×4 (centro, centro deslocado com cor, `amount`
0, sem efeito; tolerância 1/255), comportamento do gesto (prévia sem registro, um undo por arrasto, recusa mantém o
último legal, cancelar, no-op) e recusas por faixa. `crates/core/tests/pilot_fx_corpus.rs` confere o core contra
eles, com controle negativo (expectativa adulterada é pega).

- **Divergência declarada** (`release-on-refusal`): ao soltar depois de uma recusa, o piloto **volta à camada do
  pressionar**; aqui o `commit` confirma o **último candidato legal** (uma entrada de undo) e a recusa fica visível
  no gesto. **O plano autorizado** ([pilot-parity-execution-plan.md](../migration/pilot-parity-execution-plan.md), item 2 de P1)
  manda manter o **último candidato legal** e dar erro tipado: a camada do pressionar do piloto **não** é a regra
  daqui, é só a divergência registrada; o caso grava `origin` e `rust` e o teste prova o comportamento atual.
- **Bloqueios que ficam abertos** (`manifest.json`): `ORIGIN_GOLDEN_NOT_COMPARABLE` (o PNG dourado da origem exige o
  harness dela, não executado) e `ORIGIN_NOT_EXECUTED` (não se executou a origem; as expectativas são fórmulas
  transcritas, não saídas do piloto). Não há paridade pixel-a-pixel com o piloto alegada.

## Evidências

**Âncoras de SHA da matriz (a conferir antes de confiar em qualquer número abaixo).** O código de produção da PR37
é o de `85ba335` (cabeça remota inicial desta rodada); o **único** commit de código acrescentado depois é `b5b077e`
(`test(fx): Approve the scene's resources before the monitor tests ask for its plan`: só `scene_fx_tests.rs`). Os
gates completos e a prova da janela desta rodada rodaram em **`b5b077e`** (um commit só de documentação vem depois e
não altera código). Antes disso, o último head com gates completos era `88d1715` e a janela real 113/113 usou o binário
de `96e3b64`; `3cf7178`, `53ac7b8`, o merge `9c53744` e `85ba335` estavam **sem compilar**. Isso foi corrigido:

| Gate em `b5b077e` (`TESSEL_REQUIRE_RUNTIME=1`, `./scripts/cargo`, runtime `dist/runtime.js` sha256 `7b6b4338…2a98`) | Resultado |
|---|---|
| `cargo fmt --all -- --check` | exit 0 |
| `check --workspace --locked` | exit 0 |
| `clippy --workspace --all-targets --locked -- -D warnings` | exit 0 |
| `build -p tessel-editor --locked` (`tessel-editor` sha256 `3707c7ed…7324`) | exit 0 |
| `cargo tree -p tessel-editor-core` filtrado por `gpui\|wry` | vazio (grep exit 1 = nenhuma ocorrência) |
| `test --workspace --locked --no-fail-fast` | exit 0: 106 suítes, **1874 passaram, 0 falharam, 12 ignoradas** |
| `python3 scripts/pilot-fx-corpus.py verify` | exit 0 (10 casos, 6 de gesto, 11 fontes; origem não relida) |
| `scene-fx-ui-proof.spec.py` normal e `-O` | 24/24 nos dois |
| `scene-fx-ui-proof.py` (janela real, instância própria, lock de GPU, só `op`/`get`/`set`/`invoke`) | **115/115**, 0 FAIL, 0 SKIP; monitor == vinheta independente em Python (pior 7/255 no export); SwiftShader |

**Defeito achado e corrigido nesta rodada (só de teste).** Na primeira execução do head `85ba335` (1871 passaram, 3
falharam) os três testes de janela do **monitor** de BL082.20 falharam porque a janela de teste não tem pasta para
`scene.resources.resolve`: o plano era recusado com `scene_resources_not_loaded` e o monitor nunca guardava o
pedido. Nenhuma lógica de produção estava errada. O teste agora instala o mapa aprovado que o bundle declara
(`load_resources`) antes de o monitor seguir a cena. Discriminação provada por mutação: sem o filtro de invalidação em
`viewport_sync`, falham o teste do undo e o da edição externa; trocando a sincronização do monitor ao cancelar por um
simples repaint, falha o de troca de cena/Escape/polegar em repouso (25/25 de `scene_fx_tests` no código real).

As lacunas do P0 seguem explícitas e **não** foram fechadas: `ORIGIN_NOT_EXECUTED`
e `ORIGIN_GOLDEN_NOT_COMPARABLE` (a origem `d40ac3f7c0` só foi lida por `git show`; não se executou nem se instalou).
O ponteiro/teclado físicos, a GPU real e a paridade integral com o piloto **não** estão provados; o monitor foi
provado pelo pedido que guarda (testes) e por pixels do SwiftShader (prova da janela e `scene_fx_gesture_frames.rs`).

**Reconciliação por head (P0+P1).** Os parágrafos abaixo guardam o histórico e os limites de cada entrega
(32860ec, R1, R2); onde dizem "não compilado/não reexecutado", isso era verdade **naquele head**. No branch
`t3/fx-candidate-preview` o Rust foi compilado e executado: `fmt`, `check`, `clippy -D warnings` e
`test --workspace --locked` (105 suítes, 1855 passaram, 0 falharam, 12 ignoradas, `TESSEL_REQUIRE_RUNTIME=1`, nenhum
SKIPPED), `cargo tree -p tessel-editor-core` sem gpui/wry, e a prova da janela 113/113. (Contagens e `build` do head
final: tabela "Âncoras de SHA" acima, em `b5b077e`.)

Duas camadas, para não misturar o que rodou com o que só foi escrito. **Executado neste head (correção R1)**:
runtime (`node --test` dos arquivos de efeito: 11 puros + 7 em Chrome/SwiftShader), medição pareada, e a
`scene-fx-ui-proof.spec.py` (23 testes, sem janela). **Escrito neste head e não executado** (o disco da máquina ficou
abaixo do piso de 12 GiB durante a entrega, então nenhuma compilação Rust foi iniciada; só `cargo fmt --all --check`
passou): os testes de core (`scene_fx.rs`, 9 novos), os 6 novos da janela GPUI e todo o código Rust da R1. Os números
abaixo marcados "head anterior" são os da entrega 32860ec e **não** foram repetidos.

- **Lib/JSONL** — `cargo test -p tessel-editor-core --test scene_fx`: 16 testes (catálogo; plano sem `fx` idêntico ao
  anterior; criar/editar/remover com undo/redo; no-op e dry-run; trilhas automáticas; 13 recusas atômicas;
  sobreposição; recusas de import; envelope; salvar/reabrir; plano com passes e capacidade; igualdade
  `PlanTimeline` em todos os quadros; plano adulterado recusado; pixels de referência — baseline, intensidade 0,
  1, centro movido; `cargo tree` sem gpui/wry; processo `tessel-headless` real igual à lib, com a recusa
  `param_out_of_range` e saída 1).
- **Runtime (executado, R1)** — `runtime/test/output-fx.test.mjs` (11, puro: nomes herdados como efeito e como
  parâmetro, contrato com `fixtures/fx-catalog.json` (faixas, tipos, ordem), ping-pong e material retido com renderer
  falso, passe que falha restaura o alvo) e `directed-scene-fx-browser.test.mjs` (7, Chrome/SwiftShader: parede cor
  `[200,160,80]`; amount 0 == baseline; amount 1 == transcrição JS ±2; centro (1,1) discrimina; dois passes
  empilhados; efeito desconhecido ⇒ `frame.failed`/`unsupported`; **ordem com cores distintas** — vermelho então azul
  difere de azul então vermelho e o runtime iguala a referência nas **duas** ordens; 5 e 9 passes em dois alvos sem
  religar programas (`fx_program_created === 0`, `target_entries === 3`); resize; erro no 2º passe ⇒ pipeline
  descartado e quadro seguinte igual à referência; perda de contexto e `retain: false`).
- **Renderer (lib + processo Node/Chrome)** — `crates/render/tests/scene_fx.rs` (3): a cena do corpus recebe a vinheta
  pela edição do core; 54 sondas por quadro comparadas à referência do core (pior diferença 0); intensidade 0 ==
  baseline (sondas idênticas); fora do clipe == baseline; centro movido discrimina; mesmo quadro ⇒ mesmo hash depois de
  um seek para longe; fade (3º quadro de um fade-in de 10: peso 0,25); runtime sem `outputFx` recusado pela lib.
- **Export (FFmpeg real)** — `crates/render/tests/scene_fx_export.rs`: 1080p, quadros 44..52 (o clipe termina em 48):
  hash dentro ≠ baseline, fora == baseline, hash de cada quadro == `scene.render`; MP4 decodificado == referência
  sobre o baseline decodificado (pior 7/255, tolerância 16, maior mudança 69 — discrimina); `history.undo`
  devolve os hashes do baseline; o export não grava undo nem clipe de timeline.
- **Janela (GPUI)** — `scene_fx_tests.rs` (7): adicionar é uma entrada; nós descrevem o clipe; `set` por nó é uma entrada
  e componentes mantêm irmãos; recusas tipadas (`param_out_of_range`, `invalid_value`, `unknown_field`,
  `unknown_element`) sem sobra e com aviso no painel; ocultar/alternar/remover e undo; arrasto do slider confirma
  **uma** vez ao soltar; Escape descarta; cliques nos botões de adicionar e remover.
- **Prova da janela, endurecimento A2 (executado: só os testes puros)** — `scripts/scene-fx-ui-proof.py` não
  deixa mais nada passar sem olhar: export que passa de 600 s (`EXPORT_TIMEOUT_S`) é FAIL e o job é cancelado; job
  falho, arquivo ausente/vazio, lista de hashes ausente/curta/vazia e contagem de quadros errada são FAIL; fecha com a
  cobertura (`N de N quadros comparados`); undo/redo andam com limite de passos e prazo e falham se o histórico não
  anda; o relatório JSON é gravado (atômico) em qualquer saída e registra as superfícies exercidas (janela) e as não
  cobertas (headless/lib, ponteiro) com o motivo; o editor lançado e todo o grupo de processos da sua sessão são
  encerrados e conferidos (`stop_lab`). `scripts/scene-fx-ui-proof.spec.py` (23 testes, relógio falso — um export de
  600 s não roda) fixa cada FAIL. **A prova nativa não foi reexecutada depois do endurecimento.**
- **Janela real (head anterior, 32860ec)** — `scripts/scene-fx-ui-proof.py` (instância privada, binário direto, `HOME` curto, lock de GPU, só
  `op`/`get`/`set`/`invoke`): 70/70 — árvore viva, baseline monitor == render, adicionar, intensidade 1 (54 sondas ==
  vinheta em Python, pior 0,5), intensidade 0 == baseline, centro movido, `param_out_of_range` no painel sem mudar valor,
  histórico nem imagem, undo/redo, ocultar, **salvar/reabrir o projeto** com os parâmetros, e dois exports pelo diálogo
  (`flow.export_scene`) decodificados contra o modelo Python. A captura nativa
  (`--shot`, `screencapture -l`) é só observação: mostra as bordas do monitor escurecidas.

## P2: inspector neutro e segundo look (`fade`)

Branch `t3/fx-inspector-fade-p2` (BL004, BL014, BL029, BL040, BL082.1; plano em
[pilot-parity-execution-plan.md](../migration/pilot-parity-execution-plan.md)). O contrato do inspector, do
`scene.fx.inspect` e do `fade` está em [F057](fx-inspector.md); aqui ficam o que muda em F016 e as provas.

- **Catálogo**: `CATALOG = [vignette, fade]`; cada efeito e parâmetro tem rótulos pt-br/en/es. O JSON de
  `scene.fx.catalog` traz `label`/`summary` por idioma.
- **Fade** (`look`, saída): `mix(origem, color, amount × intensity × weight)` em luz linear, `amount` 0 e
  `intensity` 0 são a identidade exata (bytes iguais), determinístico. A janela e o JSONL o editam pelos mesmos
  ops e gestos da vinheta; o orçamento de 8 passes conta os dois; a composição segue a ordem das trilhas e é
  discriminante (fade→vinheta ≠ vinheta→fade nos pixels).
- **Janela**: a seção "Efeitos" é pintada do modelo do core; nenhuma lista de campos, faixas, passos ou padrões
  vive na UI. A vinheta foi migrada para ele preservando IDs semânticos (`intensity`, `center_x`…), gesto,
  preview, orçamento, inserção por tempo, teclado, undo e quantização.
- **Proveniência do fade**: `scripts/pilot-fade-corpus.py` (independente de `pilot-fx-corpus.py` e do
  `reference.rs`) gera `docs/migration/fixtures/pilot-fx/fade-cases.json` e `fade-manifest.json` (10 casos:
  janelas/envelope/ocultos/ordem e 7 de pixels, 2 deles identidade, 2 de composição em ordens opostas). Os
  bloqueios `ORIGIN_GOLDEN_NOT_COMPARABLE` e `ORIGIN_NOT_EXECUTED` permanecem; a diferença documentada
  `ZERO_WEIGHT_IS_BYTE_IDENTITY` está no manifesto. `verify --origin` conferiu os blobs.
- **Recusas** (sem mudança de contrato): `amount`/`intensity` fora de 0–1 e cor fora de 0–1 são
  `param_out_of_range` atômicos; efeito fora do catálogo continua `unknown_effect`.

### Provas do P2 (macOS, SwiftShader, sem GPU real)

| O quê | Onde |
|---|---|
| Lib: ciclo de vida, recusa atômica, plano, orçamento (9 passes com fade), gesto, persistência, pixels contra fórmula transcrita no teste, ordem, preview == commit, cancelar/undo = baseline | `crates/core/tests/scene_fx_fade.rs` (15) |
| Inspector: cobertura dos IDs oferecidos em 3 idiomas, padrão/`at_default`, escopos, `probe`, candidato do gesto e recusa, `reset_ops` | `scene_fx_fade.rs`, `scene_edit::fx_inspector` (9 unidades) |
| Processo `tessel-headless` real | `scene_fx_fade.rs::the_real_headless_process_…` |
| Corpus do fade contra o core | `crates/core/tests/pilot_fade_corpus.rs` (4, com controle negativo) |
| Referência de pixels | `directed_scene::fx::reference` (9 unidades, fade incluso) |
| GPUI: nós, sliders reais, reset, botão, ordem | `workstation/scene_fx_tests.rs` (38 `scene_fx`) |
| Runtime/Chrome: fade, identidade, ordem, retenção, ping-pong | `runtime/test/directed-scene-fade-browser.test.mjs` (4), `directed-scene-fx-browser.test.mjs`, `output-fx.test.mjs` |
| Pixels no monitor/runtime: fade contra fórmula local, composição nas duas ordens, preview == commit | `crates/render/tests/scene_fx.rs` (5) |
| Export decodificado: fade == fórmula do piloto sobre o baseline decodificado, fora do clipe == baseline, undo | `crates/render/tests/scene_fx_export.rs` (2) |
| Janela real (só `op`/`get`/`set`/`invoke`) | `scripts/scene-fx-ui-proof.py` 153/153; `fx-history-smoke-proof.py` 42/42 |

A prova da janela achou um defeito real no reset (um op por componente de cor/ponto, o último apagava os
outros); corrigido em `reset_ops` e coberto por teste de unidade e de GPUI.

## Limites e gaps (todos em [backlog](../backlog.md) BL028/BL036/BL082)

- Histórico: a correção R1 (BL082.7–BL082.15) ficou sem compilar por falta de disco; **foi compilada e testada no branch P0+P1** (gates no backlog BL082.17).
- A medição de custo é SwiftShader (comparação antes/depois); a GPU nativa não foi medida.

- Só `vignette`, `fade` e `filmGrain` (3 de 59). Os outros efeitos do piloto **não** foram portados; cada efeito novo é uma entrada no
  catálogo, um shader no runtime e um teste de pixel.
- Escopos `output` e, desde F062 (P4a), `shot` ([fx-shot-scope.md](fx-shot-scope.md)); `node` é recusado
  (`FX_SCOPE_NOT_YET`). Chaves e `seed` valem desde P3 ([F058](fx-keys.md)).
- Sem transições: `transitions` é recusado se não vazio. **Contrato de extensão** (sem controle habilitado): uma
  `crossDissolve` será `transitions[]` com `{id, type, fromShot, toShot, durationFrames}` resolvida no plano com
  **dois** quadros de origem (precisa que o plano e o runtime desenhem dois shots no mesmo quadro: capacidade nova
  `transitionBlend`), e o ripple/roll de shots será um comando de edição estrutural do core sobre `shots[]` que
  mantém o join/temporal gates; nenhum dos dois tem nó, comando nem botão ainda.
- A prévia ao vivo, o teclado do adaptador e a inserção no quadro exibido existem (seção "Gesto de efeito").
  **Lacuna:** o gizmo de cena (F055) ainda usa o número do playhead como quadro do bundle; só o monitor e a
  inserção de efeito convertem o tempo (registrado em BL082.6).
- Ponteiro físico real continua não provado (BL043); a prova da janela é semântica e o GPUI injeta entrada.
- Handles 2D da vinheta no monitor (G4.1, [F061](fx-handles.md), BL082.35): centro livre, `radius` e `softness` arrastáveis
  sobre as elipses que o `vignette_shade` desenha, pelo gesto de efeito já existente (documento ≡ `scene.fx.edit`). Só a
  vinheta tem handles, e só a de um clipe da saída: a de um clipe de plano (F062) é recusada com `scope_not_supported`
  (BL082.64; desenhá-las num plano é BL082.41); gravar chaves pelo handle é BL082.37.
- Não há dithering (o runtime escreve 8 bits como a cena); gradientes suaves podem mostrar bandas de 1/255.
- A janela pinta a seção só com uma cena anexada; sem cena, a edição é por `scene.fx.edit` (headless).

## Correções da conferência de merge (R2)

Corrigidos os resíduos de revisão do head07dde5a: limpeza da camada só quando o documento líquido mudou (add/remove sobre camada vazia autorada preserva documento e undo); fixture numérica usa floats para campos f64 do catálogo; orçamento espera 10 quadros acima do limite (0..5 e10..15); inicialização falha do runtime também limpa temporários; interrupção do proof registra FAIL e o mesmo código no processo/JSON. O teste de lote neutro agora confere documento e profundidade de undo.

Executado pelo coordenador: Python24/24 (inclui interrupção), Node puro output-fx11/11 (inclui contrato da fixture), fmt --check, node --check do medidor e git diff --check. **Rust continua não compilado/testado e prova nativa não repetida**: disco3,9GiB abaixo do piso12/reserva16. Não promover as evidências do head32860ec ao código novo. Aceite/merge permanecem pendentes em BL082.17.

## Correções após aviso de check da PR37 (2026-11-08)

`scene.capabilities` passa a anunciar os cinco verbos de gesto FX. A prova de troca de
cena exige begin/update bem-sucedidos e verifica a chave real `active`. Quantização não
limpa para inteiro passos menores que 1e-9: sem grade decimal representável em até nove
casas conserva o valor já encaixado. Regressões Rust adicionadas, **não executadas** por
disco abaixo do piso. Check CodeRabbit NEUTRAL não é aprovação; build/prova final pendentes.

## Revisão residual após gates de b5b077e

Escolher uma cena já exibida por fallback preserva o gesto de FX; entrar na cena dona do
gesto também não o cancela. A comparação usa scene_active_id, não apenas chosen.
Gestos de propriedades agora registram a última recusa em status, preservam candidato
e documento, e limpam a recusa no próximo update válido, como os gestos FX.
Regressão core scene_interaction: 23/23 nesta correção (24/24 com a prova abaixo).

### Validação do residual em `7deb23c` (código; só documentação vem depois)

`5694e0e` (correção) + `7deb23c` (só testes e prova: nenhuma linha de produção mudou). Main `30ac65b` já era
ancestral: nada a integrar. Runtime reconstruído nesta árvore (`./scripts/runtime setup`, `dist/runtime.js`
sha256 `7b6b4338…2a98`, igual ao de `b5b077e`); `tessel-editor` sha256 `3aa5a83c…622b`.

| Gate em `7deb23c` (`TESSEL_REQUIRE_RUNTIME=1`, `./scripts/cargo`) | Resultado |
|---|---|
| `cargo fmt --all -- --check` | exit 0 |
| `check --workspace --locked` / `clippy --workspace --all-targets --locked -- -D warnings` | exit 0 / exit 0 |
| `test --workspace --locked --no-fail-fast` | exit 0: 106 suítes, **1880 passaram, 0 falharam, 12 ignoradas**, nenhum SKIPPED |
| `build -p tessel-editor --locked`; `cargo tree -p tessel-editor-core` filtrado por `gpui\|wry` | exit 0; vazio |
| `scene_fx_tests` | 29/29 (a regressão do fallback + 3 novos) |
| `scene-fx-ui-proof.spec.py` e `directed-scene-proof`/`proof-surface`/`scene-lifecycle` specs, normal e `-O` | exit 0 nos dois modos (24, 4+15, 29, 7) |
| `pilot-fx-corpus.py verify` | exit 0 (10 casos, 6 de gesto, 11 fontes; origem não relida) |
| `scene-fx-ui-proof.py` (janela real, instância própria, lock de GPU, só `op`/`get`/`set`/`invoke`) | **116/116**, 0 FAIL, 0 SKIP (o novo: o seletor na cena já exibida mantém o gesto aberto); pior pixel do export 4/255 |

Casos validados do residual: o gesto fica na cena realmente exibida quando a escolha é nula, inexistente
(`detached`) ou igual; trocar de cena (inclusive a partir do fallback) cancela e não confirma nada; entrar na
cena dona do gesto o preserva; uma recusa de propriedade fica no `status`, mantém o último candidato legal e o
documento, e um update válido a limpa; uma recusa depois de gesto invalidado ou de verbo do outro tipo **não**
grava recusa (`scene_interaction.rs`, `scene_fx_gesture.rs`). Mutações: voltar `scene_follow` à comparação com
`chosen` derruba 3 testes GPUI; remover só a exceção da cena dona derruba 1; gravar a recusa antes de
`ensure_live` derruba o teste de core. JSONL em processo real (`tessel-headless`): begin, update, recusa
(`gesture_target_mismatch`), `status.refusal.code` igual, update válido zera, recusa de novo e commit do último
candidato legal (x=3, +1 undo), undo restaura 0.5, e update de gesto invalidado dá `gesture_invalidated` sem
recusa registrada. Limites inalterados: `ORIGIN_NOT_EXECUTED`, `ORIGIN_GOLDEN_NOT_COMPARABLE`; sem ponteiro
físico, GPU real nem paridade integral.

### Correção residual — gesto de uma terceira cena

Código validado `f5f6d1deb6d69a7d925bf5d747a76ccf0a6b6a59`: mudar o painel de Y para Z cancela somente o gesto de Y; um gesto de X permanece aberto. A regressão GPUI com três cenas falhou antes da correção e passou depois. Executado neste código: fmt/check/clippy workspace com -D warnings, workspace com TESSEL_REQUIRE_RUNTIME=1 (1881/0/12), build desktop, FX GPUI30/30, specs da prova normal/-O e janela116/116 sem falha/skip. Core tree sem gpui/wry. Logs locais em /tmp/pr37-thirdscene-final. Alterações posteriores deste registro são somente documentais. Sem ponteiro físico ou GPU real; gaps P0 preservados.

## Resíduos da revisão após o merge da PR37

Código `1ad715901df6edb14503ea081ae407c18bd0e94d` validado: budget só para FX live e cancelamento de Change inválido respeita id substituto. Core19/19, GPUI32/32, workspace1885/0/12 e janela116/116; fmt/check/clippy/build limpos. [Relatório e comentários tratados](../migration/pr37-review-followup-results.md). Os gates anteriores permanecem históricos; BL043 e gaps P0 mantidos.

### P2 — revisão independente e correções do inspector

BL082.21–.23: probe consulta a camada inteira mesmo com filtro de resposta; informa o valor quantizado julgado e o solicitado. JSONL recebe reset.op por campo (cor/ponto preservam irmãos, tempo usa set_field), equivalente à ação da janela. Oferta e predicado de reset vêm do core; o resumo do fade não promete keys ainda ausentes. Reforçada a conferência textual do corpus (BL082.25), sem mudar os casos/pixels nem os blockers P0. Os limites cruzados dos ranges continuam explícitos (BL082.24). Evidências atuais na atualização de [F057](fx-inspector.md); não herdar automaticamente os PASS de f8c4a7e para o código novo.

Fechamento P2: código5eb577b validado com workspace2060/0/12 e janela153/153. Resíduo posterior altera somente details.actions da recusa resetsemdefault, validado por GPUI1/1, fmt/check/clippy/build; não reatribuir a suíte/prova anterior a uma nova execução. [Detalhes e limites](fx-inspector.md#fechamento-da-validação-da-correção-p2).

## P3: chaves, easing, seed e o terceiro look (`filmGrain`)

P3 acrescenta a animação dos parâmetros (`keys`, easing da chave de chegada, hold, booleano, seed `uint32`) e o
`filmGrain`. Contratos: [F058](fx-keys.md) (formato, amostragem, operações, gesto, inspector, divergências) e
[F059](film-grain.md) (catálogo, hash inteiro, pixels, `Capability::FxNoise`).

- O documento ganha `keys` (frames locais ao clipe) e `seed`; sem eles os bytes e os planos de vinheta/fade não mudam.
  `FX_KEYS_NOT_YET` foi removido; `FX_SCOPE_NOT_YET` e `FX_TRANSITION_NOT_YET` continuam.
- O plano carrega os valores **já amostrados** e, nos efeitos de ruído, a `seed` do quadro. `Capability::FxNoise`
  (`"fxNoise"`, 11ª capacidade do runtime) só é pedida por passes com seed; um runtime P2 é recusado na negociação.
- `scene.fx.inspect` aceita `frame`/`time` e devolve por clipe o bloco `animation`; `scene.fx.edit` ganhou as ops de
  chave e `set_seed`; o gesto `scene.fx.gesture.*` também move/desloca/escala chaves (último candidato legal, um undo).
- Evidências, comandos e limites: [F058](fx-keys.md#evidências-e-status) e [F059](film-grain.md#evidências-e-status).
  Limites que permanecem: GPU nativa, Windows, ponteiro físico (BL043), execução do motor original
  (`ORIGIN_NOT_EXECUTED`, `ORIGIN_GOLDEN_NOT_COMPARABLE`), 56 efeitos, escopos `shot`/`node` e transições (P4), timeline (P6).

### Correção P3 após a revisão da PR 42 (BL082.30–.34)

- **UI completa das chaves (BL082.30):** quadro e valor tipados por chave, mover para qualquer quadro, deslocar/escalar
  com parâmetros, navegar anterior/próxima/ir para a chave por `scene.fx.seek` (regra do core), seed arbitrária,
  `base_revision` e editores com Enter/Blur de um único commit; base desabilitada sob chaves (`base_under_keys`) com
  o valor amostrado à parte; `set_param` direto continua com `FX_STATIC_UNDER_KEYS`, sem autokey.
- **Aritmética (BL082.31):** `scale_keys` com âncora `i64::MIN` ou `num = i64::MAX` é recusa tipada, e o arredondamento
  é meio para cima exato nos dois lados da âncora.
- **Custo (BL082.32):** 1024 ops por lote (`FX_LIMIT` antes de copiar), limites por op/clipe/camada antes de crescer,
  edição no lugar e contadores determinísticos; o custo é por lote (revalidação do importador), medido em par.
- **Intenção do gesto e proofs (BL082.33):** gesto liga verbo e alvo (`gesture_*_mismatch`) sem tocar candidato nem
  revisão; `scene_frame_at_playhead`/captura/navegação provados com início de FX ≠ 0 e taxas NTSC; velocidade/offset
  são N/A (não existem no clipe de cena deste caminho).
- **No-op tipado (BL082.34):** `key_param` compara valores tipados (`1` e `1.0`, ordem de chaves, cor/ponto).
Detalhes e evidências: [F058](fx-keys.md).

## P4a: escopo `shot` (BL082.3, F062)

Um clipe pode ficar no **plano** em vez da saída: `scope: { "kind": "shot", "shotId": "<id>" }` no documento (a saída
continua sendo a grafia canônica, **sem** a chave) e o clipe cabe **inteiro** em `[início, fim)` do plano. O importador
recusa o que não cabe com `FX_SCOPE_MISMATCH` (com os limites legais em `details`) e o plano que não existe com
`UNKNOWN_SHOT`; a edição (`add_clip.scope`, `set_scope`) usa a mesma regra e recusa com `fx_scope_mismatch`/`unknown_shot`,
sem cortar nada. A composição passa a ter **duas etapas** (os passes dos planos, depois os da saída, cada uma na ordem
das trilhas), o plano ganha `scope` por passe e a capacidade `fxShotScope` (a 13ª), pedida só quando algum passe é de
plano (planos de cenas sem clipe de plano mantêm os bytes). O runtime desenha a lista do plano, que traz os passes do plano
antes dos da saída, e recusa o plano que um runtime sem a capacidade não sabe desenhar. A seção Efeitos ganha o seletor de escopo por clipe e a ação de
adicionar no plano exibido. `node` e as transições seguem recusados (`FX_SCOPE_NOT_YET`, `FX_TRANSITION_NOT_YET`).
Contrato, decisões, evidências por superfície e limites: [F062](fx-shot-scope.md).
