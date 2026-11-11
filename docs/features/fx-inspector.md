# F057 — Inspector neutro de efeitos e o segundo look (`fade`)

## Objetivo e limites

O inspector de efeitos é um **modelo do core**, derivado do catálogo e do estado, que descreve cada clipe de
efeito como uma lista de campos tipados. A biblioteca, a consulta JSONL `scene.fx.inspect` e a seção "Efeitos"
da janela (GPUI) consomem o mesmo dado; a UI não tem regra de domínio: lista o que o modelo lista, escreve o
que o modelo manda e recusa o que o core recusa. Esta entrega também move a vinheta para o modelo comum e
acrescenta o segundo look, o `fade` (escopo de saída), sem seção própria nem código especial na janela.

Não é o catálogo inteiro: **3 dos 59** efeitos do piloto estão implementados (`vignette`, `fade` e, desde P3, `filmGrain`). Não há
placeholder habilitado para os outros 56 (ver [Oferta](#oferta-o-que-existe-e-o-que-não)). Chaves, easing e seed estão em [F058](fx-keys.md) desde P3; sem escopo `shot`/`node` e sem transições (P4). O contrato de edição, gesto, plano e runtime é o
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

`scene.fx.inspect.offers` lista só efeitos reais (`vignette`, `fade`, `filmGrain`, todos `scope: output`,
`applicable: true`). `scopes` diz `output` aplicável e `shot`/`node` não aplicáveis com o código
`FX_SCOPE_NOT_YET`. Os 56 efeitos restantes **não aparecem**: nenhum controle habilitado, nenhum nome sem
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
`reset {to, needed, op}`. `op` é a operação JSONL executável para aquele campo (inclusive duração/fades); é `null` quando o campo já está no padrão. Campos sem padrão têm `reset: null`.

- `gesture` lê o **candidato** do gesto aberto (o valor legal mais recente) e traz `gesture {id, clip, refusal}`
  com a última recusa tipada; um gesto fechado deixa de ser legível. O ID precisa ser de um gesto de **efeito**: o de um gesto `properties` (arrasto de anel ou slider) é
  recusado com `gesture_kind_mismatch`, sem escrever nada e sem encerrar aquele gesto (F2 da revisão da PR #40; teste
  `an_effect_read_names_an_effect_gesture_and_refuses_the_id_of_a_ring_drag`).
- `probe {clip, field, value, quantize?}` julga um valor **sem gravá-lo**: monta o op do campo e roda o
  `dry_run_fx` do core. `ok: true` traz `would_change` (gravar um padrão que o clipe omite **é** mudança do
  documento) e `changes`; `ok: false` traz o erro tipado (`param_out_of_range`, `invalid_value`,
  `unknown_field`, `unknown_element`). Com `quantize: true`, o valor passa pela grade do campo.
- Idioma desconhecido e clipe desconhecido são erros tipados; nada é escrito.
- Reset por script: enviar `clips[].reset_ops` a `scene.fx.edit` (uma entrada de undo) para o visual inteiro, ou `[fields[].reset.op]` para um campo. O op de uma componente inclui os irmãos atuais; consulte novamente depois de outra edição. Não combine vários ops de componentes do mesmo parâmetro lidos da mesma base: para resetar o visual use o lote agregado, que funde componentes.
- Quando a consulta usa `gesture`, os ops descrevem o candidato: para resetá-lo sem confirmar use `scene.fx.gesture.update` e depois commit/cancel. A ação semântica de reset da janela opera no documento; para reproduzi-la consulte sem `gesture`.
- O filtro `clip` limita apenas os clipes descritos na resposta; `probe.clip` é consultado na camada completa, no mesmo documento ou candidato escolhido. O probe retorna `value` julgado após quantização e `requested_value` original, tanto no sucesso quanto na recusa. Não escreve estado.
- As faixas são escalares: duração, fades, início e overlap também dependem de outros campos. Um reset ou probe pode ser recusado atomicamente pelo domínio por essas restrições; a faixa não promete que toda combinação é válida (BL082.24).

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
- G4.1 ([F061](fx-handles.md)): o clipe de vinheta ganha a ação `handles` (botão `control:fx-handles:<clipe>`), que mostra ou
  esconde as alças dele no monitor pelo mesmo `scene.fx.select` de um script; desabilitada com motivo (`Só a vinheta tem alças
  no monitor`) para `fade` e `filmGrain`. O estado do nó do clipe ganha `handles`.

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
| Lib | V (macOS, SwiftShader) | `scene_edit::fx_inspector` (9 unidades), `reference.rs` (9), `scene_fx_fade.rs` (17 após a revisão; 15 inicial), `pilot_fade_corpus.rs` (4), `scene_fx.rs` (catálogo) |
| CLI headless (processo real) | V (macOS, SwiftShader) | `scene_fx_fade.rs::the_real_headless_process_inspects_and_edits_a_fade_like_the_library` |
| UI GPUI (entrada injetada) | V (macOS, SwiftShader) | `scene_fx_tests.rs` (39 testes `scene_fx` após a revisão; 38 inicial) |
| Janela real, só comandos semânticos | V (macOS, SwiftShader) | `scripts/scene-fx-ui-proof.py` (153 checks no head de código) |
| Pixels no runtime/monitor/export | V (macOS, SwiftShader) | `crates/render/tests/scene_fx.rs`, `scene_fx_export.rs`, `runtime/test/directed-scene-fade-browser.test.mjs` |

Não provado: ponteiro e teclado físicos (BL043), GPU real, áudio, **Windows** (a prova é macOS/SwiftShader),
equivalência com o golden da origem (`ORIGIN_GOLDEN_NOT_COMPARABLE`) e saídas reais da origem
(`ORIGIN_NOT_EXECUTED`). Paridade integral com a origem **não** é afirmada: 3 de 59 efeitos (o terceiro, `filmGrain`, entrou em P3).

## Revisão independente P2 e correções

A revisão independente de f8c4a7e corroborou reset multicomponente, leitura pura, gestos, fórmula, pixels (mutações sRGB e amount duplicado discriminantes), processo headless e janela153/153. As correções subsequentes acrescentam probe fora do filtro, valor quantizado na recusa e reset.op por campo/tempo; UI usa oferta e predicado de reset do core, e reset:start_frame (sem padrão) é unknown_action. Não são as mesmas evidências do head anterior: os gates da correção são registrados abaixo após execução.

O teste de render que reaplica a mesma edição agora se descreve como determinismo; o ciclo real de gesto/commit/cancel/undo é provado pelo core e pela janela. O corpus mantém os mesmos pixels/expectativas, mas acrescenta o blob do catálogo da vinheta e verifica padrões, forma do shader, ordem/visibilidade e clamp do envelope contra o texto fixado. São verificações de transcrição/proveniência, sem executar a origem ou transformar o corpus em golden comparável.

Gates da correção executados pelo coordenador em macOS/SwiftShader: fmt/check/clippy -Dwarnings/workspace/build e coretree sem gpui/wry passam; workspace **2060/0/12** (107suítes, TESSEL_REQUIRE_RUNTIME=1), corefade17/17 e GPUIFX39/39; specsPython28/28 normal/-O. Corpus offline e --origin verificam10casos/12fontes, cinco mutações de transcrição rejeitadas sem alterar os blobs reais nem os casos de pixels. Logs: /Users/wallyssonnunes/.cache/tessel-pr39-followup/logs. Runtime hash7dab67b7…fba3; desktop32c72841…1947. Prova nativa final ainda aguardando lock na publicação desta correção; 153/153 do headf8c4a7e continua histórico, não reatribuído ao novo binário.

### Fechamento da validação da correção P2

No código **5eb577b3fd01c7875c27a5c2a3fb2106b912e255**, a prova nativa final terminou: **153/153, exit0, zero falhas/skip**, sem cliques por coordenada. O JSON confirma árvore limpa em5eb577b e os hashes desktop32c72841…1947/runtime7dab67b7…fba3. A vinheta+fade coincide com a referência Python (pior0,6/255 na composição); export da vinheta teve pior7/255 e mudança discriminante69; arquivo/hashes/cobertura8/8 e cleanup foram verificados. Logs/JSON em /Users/wallyssonnunes/.cache/tessel-pr39-followup/logs/window-proof.*. Isto substitui a espera pelo lock descrita no registro anterior.

Depois disso, um comentário residual apontou que reset:start_frame recusava corretamente, mas details.actions era vazio. O ajuste subsequente muda **apenas os detalhes da recusa** para as ações realmente anunciadas pelo nó. Regressão GPUI específica passa1/1 (inclui igualdade com a recusa genérica e estado/histórico intactos); fmt/check/clippy workspace e build foram reexecutados e passam. A suíte2060/0/12 e a prova153/153 permanecem ancoradas em5eb577b, sem atribuí-las a uma nova execução após o ajuste de diagnóstico. Logs residual-*.log no mesmo diretório. Windows/GPU real/ponteiro e teclado físicos/áudio ouvido/golden da origem continuam não provados.

## P3: o bloco de animação (BL004/BL029/BL040)

O modelo ganhou `scene_edit::fx_animation` (neutro, sem GPUI), lido por `scene.fx.inspect {frame|time}` e pintado pela
janela: por parâmetro `base`, `sampled` (o valor do frame, do mesmo resolvedor do plano), `animated`, `keys` e as
`operations` com `enabled`/`reason`/`template`; para a seed, `explicit`, `base`, `at_frame`. O `value` estático de P2
mantém seu significado (é a base). Booleanos têm tipo próprio (`Switch` real do Kit; um número é `invalid_value`).
**Não há auto-chave:** os sliders editam a base (o core avisa `FX_STATIC_UNDER_KEYS` quando há curva); chaves só por
captura, ação ou op. Nós e ações da janela (`key:*`, `seed`, `remove_animation`) e o contrato completo estão em
[F058](fx-keys.md#ui-e-cli-da-janela); o terceiro look em [F059](film-grain.md). Rótulos pt-br/en/es de `filmGrain` e
de suas operações vêm do core (BL040 segue parcial: a política geral de i18n do editor continua aberta).

**Correção P3 (BL082.30/.31/.33):** o campo de um parâmetro animado é a **base** (`animated`, `base`,
`edit_with: "keys"`, `enabled: false`, motivo `base_under_keys`, `sampled` à parte); o inspector devolve a chave
anterior/seguinte ao quadro pedido e as operações `go_previous|go_next|go_first|go_last` (motivo `no_frame` sem quadro
de edição, `no_previous_key`/`no_next_key`, `no_keys`), executadas por `scene.fx.seek`. Quadro e valor de cada chave,
deslocar/escalar curvas e seed arbitrária têm editores tipados na seção "Efeitos" (Kit `Input`, `Switch`, botões de
ícone) e campos/ações equivalentes no nó; contrato em [F058](fx-keys.md#ui-e-cli-da-janela).
