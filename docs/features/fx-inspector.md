# F057 — Inspector neutro de efeitos e o segundo look (`fade`)

## Objetivo e limites

O inspector de efeitos é um **modelo do core**, derivado do catálogo e do estado, que descreve cada clipe de
efeito como uma lista de campos tipados. A biblioteca, a consulta JSONL `scene.fx.inspect` e a seção "Efeitos"
da janela (GPUI) consomem o mesmo dado; a UI não tem regra de domínio: lista o que o modelo lista, escreve o
que o modelo manda e recusa o que o core recusa. Esta entrega também move a vinheta para o modelo comum e
acrescenta o segundo look, o `fade` (escopo de saída), sem seção própria nem código especial na janela.

Não é o catálogo inteiro: **2 dos 59** efeitos do piloto estão implementados (`vignette`, `fade`). Não há
placeholder habilitado para os outros 57 (ver [Oferta](#oferta-o-que-existe-e-o-que-não)). Sem chaves
(`keys`, P3), sem escopo `shot`/`node` e sem transições (P4). O contrato de edição, gesto, plano e runtime é o
de [F016](effects.md); este documento só acrescenta o que é novo.

## Contrato do core / biblioteca

Módulo `tessel_editor_core::scene_edit` (`fx_inspector.rs`), sem GPUI:

| Item | O que é |
|---|---|
| `InspectorField` | Um campo escalar: `id` (`intensity`, `color_r`, `center_x`, `fade_in_frames`…), grupo (`param` ou `timing`), rótulos pt-br/en/es, valor, `min`/`max`/`step`, inteiro?, unidade, **padrão** (ou `None`), `at_default()`, aplicabilidade e motivo, destino (`Target`) |
| `InspectorField::op(clip, valor)` | O `FxOp` que escreve o campo (um componente de ponto/cor mantém os irmãos como o clipe os tem; fração em campo de quadro é recusada). A faixa e as regras da camada continuam sendo do core (`scene.fx.edit`) |
| `InspectorField::quantize` | A grade do `step` ancorada no mínimo (é `quantize_control`): ponteiro, teclado e script concordam |
| `InspectorField::reset_op` | O op que devolve o campo ao padrão; `None` se já está nele ou se não tem padrão |
| `ClipInspector::of` / `layer_inspector` | Os campos de um clipe / de toda a camada: `intensity` primeiro, depois os demais parâmetros na ordem do catálogo, depois o tempo (início, duração, fade in, fade out) |
| `reset_ops(campos, clipe)` / `ClipInspector::reset_ops` | O reset do **visual**: um op por parâmetro fora do padrão. Os componentes de um ponto/cor (`color_r/_g/_b`, `center_x/_y`) viram **um** `set_param` com todos os padrões (um op por componente seria montado a partir do clipe como está e o último apagaria os outros; foi um defeito real, achado pela prova da janela e coberto por teste). O tempo nunca entra no reset |
| `offers()` / `scopes()` | O que o catálogo oferece e o que cada escopo diz (`output` aplicável; `shot`/`node` com motivo `FX_SCOPE_NOT_YET`) |

Rótulos: `Labels { pt_br, en, es }` e `Locale` (`pt-br`, `en`, `es`; `pt` é aceito como pt-br) em
`directed_scene::fx::catalog`. Cada efeito traz `label` e `summary`, cada parâmetro um `label`, e o
inspector acrescenta os rótulos de tempo e os sufixos de componente (`Cor R`, `Centro X`).
`every_offered_effect_has_every_text_in_three_languages` (unidade) e
`every_offered_effect_is_inspected_with_every_field_and_text_in_three_languages` (JSONL) percorrem **os IDs
oferecidos pelo catálogo**: um efeito novo sem texto em algum idioma reprova.

### Oferta: o que existe e o que não

`scene.fx.inspect.offers` lista só efeitos reais (`vignette`, `fade`, ambos `scope: output`,
`applicable: true`). `scopes` diz `output` aplicável e `shot`/`node` não aplicáveis com o código
`FX_SCOPE_NOT_YET`. Os 57 efeitos restantes **não aparecem**: nenhum controle habilitado, nenhum nome sem
implementação. `FX_TRANSITION_NOT_YET` continua sendo a recusa de `transitions`.

## CLI headless

`scene.fx.inspect {scene, clip?, gesture?, locale?, probe?}` — consulta de leitura, sem janela, pelo mesmo
`headless::execute` da lib e do `ui op`:

```json
{"op":"scene.fx.inspect","scene":"street","clip":"f1","locale":"en",
 "probe":{"clip":"f1","field":"amount","value":4.0}}
```

Resposta: `action`, `scene`, `locale`, `catalog_version`, `source` (`document` ou `gesture`),
`edit_duration_frames`, `scopes`, `offers`, `clip_count`, `clips[]`, `budget`, `warnings`.
Cada clipe traz `clip`, `track`, `effect`, `effect_label(s)`, `hidden`, `reset_ops[]` (os ops de
`scene.fx.edit` que fazem o mesmo reset da janela) e `fields[]` com `id`, `group`, `label`/`labels`, `type`,
`value`, `min`, `max`, `step`, `integer`, `unit`, `default`, `at_default`, `applicable`, `reason` e
`reset {to, needed}`.

- `gesture` lê o **candidato** do gesto aberto (o valor legal mais recente) e traz `gesture {id, clip, refusal}`
  com a última recusa tipada; um gesto fechado deixa de ser legível.
- `probe {clip, field, value, quantize?}` julga um valor **sem gravá-lo**: monta o op do campo e roda o
  `dry_run_fx` do core. `ok: true` traz `would_change` (gravar um padrão que o clipe omite **é** mudança do
  documento) e `changes`; `ok: false` traz o erro tipado (`param_out_of_range`, `invalid_value`,
  `unknown_field`, `unknown_element`). Com `quantize: true`, o valor passa pela grade do campo.
- Idioma desconhecido e clipe desconhecido são erros tipados; nada é escrito.
- Reset por script: enviar `clips[].reset_ops` a `scene.fx.edit` (uma entrada de undo).

`tessel-headless` (processo real) responde igual à biblioteca: `the_real_headless_process_inspects_and_edits_a_fade_like_the_library`.

## UI e CLI da janela

A seção "Efeitos" (`workstation/scene_fx.rs`) pinta o modelo do core com Kit (`Slider`, `Switch`, botões de
ferramenta) e tokens do tema; nenhuma cor solta, nenhum diálogo novo (`has_dialog` não é tocado), um único
monitor. O que mudou:

- Um botão de **adicionar por efeito do catálogo** (`control:fx-add:<efeito>`); a seção `scene.fx` lista as
  ações `add:vignette` e `add:fade`, as duas habilitadas porque executam a operação real.
- Cada clipe é desenhado a partir de `controls_of` (o modelo): campos fora do padrão ganham a ação
  `reset:<campo>`, o clipe ganha `reset` (o visual) e um botão de reset (`control:fx-reset:<clipe>`). Um reset
  sem o que restaurar é listado **desabilitado com motivo** (`Já está no padrão`), nunca um callback vazio.
  Um campo cuja faixa tem um só valor sai do painel e fica no nó desabilitado com o motivo.
- Nós `scene.fx.clip:<id>`: campos com `default`, `at_default`, `group`, `enabled`, `disabled_reason`; ações
  `remove`, `toggle_hidden`, `reset`, `reset:<campo>`. Todo gesto passa por `SceneCommand::FxEdit`/gesto do core:
  um reset é **uma** entrada de undo.
- Equivalente semântico: `./scripts/ui invoke scene.fx add:fade`, `ui set scene.fx.clip:fx-1 amount 0.4`,
  `ui invoke scene.fx.clip:fx-1 reset`, `ui op '{"op":"scene.fx.inspect","scene":"…"}'`.
- A janela imprime em pt-br (`LOCALE`); os outros idiomas são do modelo e das consultas.

## O `fade` (look, escopo de saída)

Fonte: pilot `d40ac3f7c0`, `fx/catalog/fade.ts` e `runtime/fx/shaders/fade.ts` (lidos em modo somente leitura;
a origem nunca foi instalada, construída nem executada).

| Parâmetro | Tipo | Faixa | Padrão |
|---|---|---|---|
| `color` | cor `{r,g,b}` em luz linear | 0–1 | preto |
| `amount` | número (razão) | 0–1, passo 0,01 | 1 |
| `intensity` | número | 0–1, passo 0,01 | 1 |

`typical_duration_frames` = 24. A saída de cada pixel é `mix(origem, color, amount × intensity × weight)` em
**luz linear** (curvas IEC 61966-2-1), uniforme no quadro. `intensity × weight` é o `amount` do passe (o validador
do plano o recalcula); o `amount` do parâmetro é o fator do shader e é o que as chaves animarão em P3 (sem chaves
agora). `intensity` entra por último na ordem do catálogo, como no piloto; a janela o mostra primeiro.

Diferenças em relação ao piloto, declaradas:

- O piloto trabalha em luz linear até o fim e codifica com dithering; o editor escreve 8 bits como a cena e
  **não tem dithering** (limite herdado de F016). Quando `intensity × weight × amount` é 0, o Rust e o runtime
  devolvem os bytes de entrada **exatamente** (sem ida e volta por luz linear): é a intenção do comentário do
  piloto ("exactly the identity at 0"). O corpus fixa os bytes (`ZERO_WEIGHT_IS_BYTE_IDENTITY` no manifesto).
- Sem chaves (`amount` não é animável ainda), sem escopo `shot`.

Plano e runtime: o `ScenePlan.fx` carrega o passe `fade` na ordem das trilhas, o validador recalcula o
`amount`, e `runtime/src/output-fx.js` desenha só os valores resolvidos pelo core (um material retido por efeito,
tabela `EFFECT_DRAWS`, ping-pong de dois alvos). O orçamento de 8 passes avisa (nunca recusa) contando vinheta e
fade juntos. O catálogo Rust e o do runtime são fixados por `runtime/test/fixtures/fx-catalog.json`
(regravado com `TESSEL_UPDATE_FX_CONTRACT=1`).

## Evidências e status

Estado **neste branch** (`t3/fx-inspector-fade-p2`), por superfície, no head de código. V = executado e passou
neste head em macOS/SwiftShader; o aceite humano e a integração com `main` seguem pendentes. Comandos reais e
números estão em [F016](effects.md#p2-inspector-neutro-e-segundo-look-fade) e no relatório de entrega.

| Superfície | Estado | Evidência |
|---|---|---|
| Lib | V (macOS, SwiftShader) | `scene_edit::fx_inspector` (9 unidades), `reference.rs` (9), `scene_fx_fade.rs` (15), `pilot_fade_corpus.rs` (4), `scene_fx.rs` (catálogo) |
| CLI headless (processo real) | V (macOS, SwiftShader) | `scene_fx_fade.rs::the_real_headless_process_inspects_and_edits_a_fade_like_the_library` |
| UI GPUI (entrada injetada) | V (macOS, SwiftShader) | `scene_fx_tests.rs` (38 testes `scene_fx`, 4 novos) |
| Janela real, só comandos semânticos | V (macOS, SwiftShader) | `scripts/scene-fx-ui-proof.py` (153 checks no head de código) |
| Pixels no runtime/monitor/export | V (macOS, SwiftShader) | `crates/render/tests/scene_fx.rs`, `scene_fx_export.rs`, `runtime/test/directed-scene-fade-browser.test.mjs` |

Não provado: ponteiro e teclado físicos (BL043), GPU real, áudio, **Windows** (a prova é macOS/SwiftShader),
equivalência com o golden da origem (`ORIGIN_GOLDEN_NOT_COMPARABLE`) e saídas reais da origem
(`ORIGIN_NOT_EXECUTED`). Paridade integral com a origem **não** é afirmada: 2 de 59 efeitos.
