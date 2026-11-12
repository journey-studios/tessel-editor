# F062 — Efeitos com escopo de plano (`shot`) (BL082.3, P4a)

## Objetivo e limites

Um clipe de efeito (F016/F057/F058) pode ficar **no plano** (`shot`) em vez de na saída: o efeito é desenhado sobre a
imagem de **um** plano da edição, antes dos efeitos da saída, e só nos quadros desse plano. Tudo é regra do core: o
importador valida, o resolvedor puro compõe em etapas, o plano carrega o escopo de cada passe, o runtime só desenha.
A biblioteca, o JSONL sem janela e a seção "Efeitos" da janela editam o mesmo documento pelos mesmos comandos.

É a base das transições (P4b/BL082.4, que precisam de efeitos presos a um plano e de um segundo deck), não as
transições. Continuam recusados com código tipado: escopo `node` (`FX_SCOPE_NOT_YET`) e `transitions`
(`FX_TRANSITION_NOT_YET`). Não há ripple/roll de planos, nem nova família de efeitos (3 de 59).

Fonte (somente leitura; **a origem nunca foi executada**, `ORIGIN_NOT_EXECUTED`): `packages/scene-engine/src/fx/` do
snapshot `d40ac3f7c0` da branch `t3/scene-engine-pilot`, lida por `git show`. Os casos vindos dela estão escritos à
mão em `crates/core/tests/scene_fx_shot_scope.rs`, com o arquivo e o blob de cada um (tabela abaixo). Plano de trabalho:
[p4a-shot-scope-fx-plan.md](../migration/p4a-shot-scope-fx-plan.md).

## Contrato do core / biblioteca

### Formato do documento

```json
{ "id": "v1", "effect": "vignette", "startFrame": 0, "durationFrames": 40,
  "scope": { "kind": "shot", "shotId": "shot-1" } }
```

- **Saída** é a grafia canônica **sem a chave** `scope` (o piloto nunca a escreve e o `setScope` dele de volta para a
  saída remove a chave). Um `{ "kind": "output" }` escrito à mão é lido e, como o documento é a árvore sem perdas,
  **preservado como foi escrito** até uma op tocar o clipe. Voltar para a saída por `set_scope` **remove** a chave.
- **Plano**: `{ "kind": "shot", "shotId": "<id>" }`. `shotId` é obrigatório e não vazio.
- O escopo é **texto** nas ops e nas leituras: `output` ou `shot:<id>` (o ID é tudo que vem depois do **primeiro**
  `:`, então um ID com `:` vai e volta inteiro). O objeto do documento também é aceito numa op.
- Forma errada do `scope` (não objeto, `kind` ausente ou de outro tipo, `shotId` ausente, vazio ou de outro tipo,
  `kind` desconhecido): `INVALID_TYPE`/`MISSING_FIELD`/`INVALID_VALUE` do estágio 1 do importador, com o ponteiro do campo.
- `kind: "node"` segue `FX_SCOPE_NOT_YET`.

### A regra única (`scope_fault`) e as duas recusas do importador

Um clipe de plano cabe **inteiro** em `[início, fim)` do plano (o fim é exclusivo; um clipe que **preenche** o plano é
válido). `directed_scene::fx::scope_fault(escopo, clipe, início, duração, planos)` é a **única** implementação: o
importador a usa no estágio 2 (integridade, depois dos planos, só se o estágio 1 limpo) e as ops de edição a usam para
julgar o candidato, então os dois não divergem. Nunca corta nem ajusta o clipe.

| Código | Caminho | Quando | `details` |
|---|---|---|---|
| `UNKNOWN_SHOT` | `/fx/tracks/i/clips/j/scope/shotId` | o plano não existe na edição | `{ shot, shots: [ids] }` |
| `FX_SCOPE_MISMATCH` | `/fx/tracks/i/clips/j` | o clipe começa antes do plano ou termina depois dele | `{ shot, start_frame, end_frame_exclusive, clip, clip_start_frame, clip_end_frame_exclusive }` e a mensagem diz "keep it between frames {início} and {fim exclusivo}, or use scope output" |

`Issue` ganhou `details` (opcional); um candidato recusado pelo importador continua `invalid_bundle` com os issues em
`details.issues[]`, agora cada um com o seu `details`. A leitura dos quadros dos planos vem de `Bundle.shots`, na ordem
do documento; os planos ladrilham a edição, então um quadro da edição está em um plano.

### Composição em etapas (`Fx::resolve(quadro)`)

`Fx::resolve` é uma função pura do quadro (sem relógio, sem ordem de pedidos e **sem os planos**) e devolve os passes em
**duas etapas**: `FxStage::Deck` (passes de plano, a imagem do próprio plano) primeiro, depois `FxStage::Output`, cada
uma na ordem das trilhas e dos clipes (`sort_by_key` é estável). Não precisa dos planos porque o importador já garante,
pela regra única, que um clipe de plano cabe inteiro no seu plano: um passe de plano só existe nos quadros desse plano.
A ordem **não** é só a das trilhas: uma vinheta de saída em uma trilha anterior à de um fade de plano é desenhada
**depois** dele. Uma futura transição (P4b) entrará entre o último passe do plano e o primeiro da saída; a lista do plano
já vem nessa ordem e o parser do runtime a confere.

O orçamento de passes (`FX_PASS_BUDGET = 8`, só avisa) conta **as duas etapas** juntas; `scene.fx.frame` reporta
`stages { deck, output }`.

### Plano e capacidade

- `ScenePlan.fx[].scope` é sempre `{ "kind": "output" }` ou `{ "kind": "shot", "shotId" }`, na ordem das etapas.
- `Capability::FxShotScope` (`"fxShotScope"`, a **13ª**; `Capability::ALL` passa de 12 para 13) é exigida **só** quando
  existe um passe de plano no quadro; um plano sem ele mantém os bytes (e o `requires`) de antes: o golden
  `crates/core/tests/scene_plan_performance_golden.rs` e os corpora não mudaram.
- A validação do plano recusa: passe de plano com `shotId` vazio ou que não é o do `frame` (`/fx/i/scope/shotId`),
  etapas fora de ordem (passe de plano depois de passe de saída: `/fx/i/scope`) e um `requires` que não é o derivado do
  conteúdo (a igualdade é exata: `fxShotScope` falta quando há passe de plano, ou sobra quando não há).
  `PlanTimeline::at_frame` re-resolve `fx` e `requires` por quadro, igual a `ScenePlan::build` em todos os quadros.
- Um runtime sem a capacidade é recusado na negociação (`unsupported` + `missing` com `fxShotScope`), nunca desenha o
  efeito na saída inteira nem o omite.

### Edição (`scene.fx.edit`, lote atômico, uma entrada de undo)

| Op | Contrato |
|---|---|
| `add_clip { …, scope? }` | `scope` opcional (`output` por padrão), texto ou objeto. Sem `duration_frames`, o clipe dura a típica do efeito **cortada onde o escopo termina** (o fim do plano, ou o da edição; `fx::default_duration`, a mesma regra do reset do inspector). Uma duração **dada** nunca é cortada: é julgada como veio e, se não cabe no plano, recusada **antes** de escrever (a recusa nomeia o plano e o intervalo legal) |
| `set_scope { clip, scope }` | `scope` é **obrigatório** (ausente: `invalid_op`; `null` ou lixo: `invalid_value`, caminho `scope`). Mover para o escopo que o clipe já tem é no-op (sem entrada de undo); para `output` remove a chave; atômico |

Retemporizar (`set_field` de início/duração) um clipe de plano para fora do plano é julgado pelo **importador** do
candidato (`invalid_bundle` com o issue `FX_SCOPE_MISMATCH`): o clipe não é cortado nem o escopo trocado em silêncio. As
chaves (`keys`) e a `seed` de um clipe de plano são as de F058/F059, locais ao clipe.

Códigos de erro de **comando** (`scene_edit::fx::command_code`): `fx_scope_mismatch` (com o `details` acima, mais
`index` e `path: "scope"` da op), `unknown_shot` (`shot`, `shots`), `fx_scope_not_yet` (`node`) e `invalid_value`/
`invalid_op` para o texto malformado. Nenhum deles muda documento, histórico ou revisão.

### Leituras

| Resposta | Acréscimo (F062) |
|---|---|
| `scene.fx.catalog` | bloco `scope`: `output` e `shot` (texto, regra e recusas), composição em etapas, capacidade; `node` recusado com seu código |
| `scene.fx.list` | `shots`: `[{ id, start_frame, end_frame_exclusive, intent }]` e o `scope` (texto) de cada clipe |
| `scene.fx.frame` | `shot` (o plano do quadro), `stages { deck, output }`, e o `scope` de cada passe |
| `scene.fx.inspect` | `shots`, `shot` (do quadro), `scopes` e, por clipe, `choices` (abaixo) |

### Inspector: a primeira escolha

`scene_edit::fx_choice` (`scope_choice(clipe, planos)`, o único caminho; `ClipInspector::of` o põe em `choices`) devolve `InspectorChoice` — a primeira
configuração do modelo que é uma de poucas opções nomeadas, não um número. Uma opção cabe quando `fx::shot_holds` diz que o
plano contém o clipe inteiro (a metade de contenção da regra única, sem montar a recusa); o `set_scope` pronto de uma
opção (e o que o seletor da janela executa) é `InspectorChoice::set_scope_op`. JSON de cada escolha (também o campo `scope` do nó da
janela): `{ id: "scope", group: "scope", type: "choice", label, labels {pt-br,en,es}, value, default: "output",
at_default, options: [...] }`; cada opção: `{ value, kind, shot, label, labels, intent, start_frame, end_frame_exclusive,
selected, applicable, reason, op }` — `applicable: false` com o **código** que a edição recusaria (`FX_SCOPE_MISMATCH`/
`UNKNOWN_SHOT`) e `op` (o `set_scope` pronto) só quando a opção pode ser escolhida e não é a atual. Rótulos: `Escopo/
Scope/Alcance`; opções `Saída/Output/Salida` e `Plano {id}/Shot {id}/Plano {id}` (`scope_label`, o mesmo texto do título do clipe).

O **reset da duração** (`duration_frames` do inspector) restaura `fx::default_duration`: a típica cortada onde o escopo do clipe
termina, a mesma de um `add_clip` sem duração. Em um clipe de plano o padrão é, portanto, o que o plano comporta a partir do
seu início, e a op de reset sempre cabe (antes, o padrão ignorava o espaço e o reset era recusado perto do fim do plano: BL082.48).

Os campos de **início** e **duração** de um clipe de plano são limitados pelo plano, nunca pela edição: o início vai do começo do
plano até onde o clipe, como está, terminaria no fim dele (`fim − duração`); a duração vai de 1 até o fim do plano a partir do início
do clipe (`fim − início`), com `fx::scope_start`/`scope_end` e `scene_edit::fx_choice::timing_limits` como único cálculo. Um clipe
da saída segue com os limites da edição, que perto do fim ainda oferecem o que a regra recusa (`FX_CLIP_OUT_OF_RANGE`; BL082.69).
Cada extremo que o inspector anuncia é conferido contra `edit`: aceito, e um quadro além dele recusado. A janela e `scene.fx.inspect` leem os mesmos números: o controle para onde a edição seria
recusada com `FX_SCOPE_MISMATCH` (CodeRabbit da PR #48).

### Alças da vinheta (F061 × F062, BL082.64)

A vinheta de um clipe **de plano não tem alças no monitor**: a janela e o overlay das alças são lidos para um clipe da saída
(quadros de saída, sobre a imagem pronta) e nada diz ainda onde fica a imagem de um plano (BL082.41). A recusa mora no **predicado
único** de F061, `FxClip::handles_refusal` (`directed_scene/fx/handles.rs`; `has_handles` é só a ausência de recusa), que devolve
`NoHandles::NotVignette` (`fx_has_no_handles`) ou `NoHandles::ScopeNotSupported` (`scope_not_supported`) e as palavras de cada uma
nos três idiomas. Quem pergunta é sempre o mesmo predicado: `FxHandlesView::of` responde o erro tipado (alvo = o clipe, `details`
`{clip, scope}`), a poda do alvo (`fx_clip_has_handles`) larga o alvo **na própria edição** que põe o clipe num plano (sem leitura),
e a ação `handles` e o botão do inspetor ficam desabilitados com o texto do core. Por isso `scene.fx.select`, `scene.fx.handles`
(leitura e `probe`), `scene.gizmo.hit` e `scene.pointer.down` respondem pela mesma recusa ou pela ausência de handle de efeito.
Devolver o clipe à saída devolve as alças.

## Runtime (`runtime/src/output-fx.js`, `directed-scene.js`, `compositor.js`)

- `parseOutputFx(raw, requires, frame)` aceita `scope: {kind:'shot', shotId}` **só** com `fxShotScope` em `requires` e
  quando o `shotId` é o do `frame` (`/frame/shotId`); um passe de outro plano é `unsupported`
  (`{unsupported:'fx_scope', reason:'shot_not_in_frame'}`), `node` ou escopo desconhecido é `unsupported`
  (`reason:'scope_kind'`), nunca pulado. As etapas têm de vir em ordem.
- `OutputFxPipeline.draw` segue desenhando a lista como vem, um passe por vez, no ping-pong de dois alvos (`targetOf(i % 2)`):
  o `parseOutputFx` confere que os passes do plano vêm antes dos da saída, então o resultado do deck é a entrada da saída
  sem cópia e sem uma segunda partição. A P4b compõe o segundo deck no primeiro passe da saída, quando existir.
- `SCENE_CAPABILITIES` tem as 13 capacidades; telemetria: `frame.sources.directed_scene_fx_shot` conta os passes de plano
  (junto de `directed_scene_fx`) e some quando não há nenhum; o teste do compositor no navegador confere o **valor**.

## Janela (`workstation/scene_fx_scope.rs`)

A seção "Efeitos" **não tem estado próprio**: o seletor e as ações são pintados do mesmo modelo (`scope_choice`) e
cada gesto é um `scene.fx.edit` por `fx_run` — o mesmo handler do `set`/`invoke` do nó.

- **Seletor**: cada clipe tem uma linha "Escopo" com um `Select` do Kit (opções: `Saída`, `Plano shot-1 · 0–47`, …).
  O valor do seletor é o **texto do escopo** (`SelectItem` tipado: `output`, `shot:<id>`), não o rótulo. Escolher é um
  `set_scope` (uma entrada de undo); escolher o escopo que já vale não é edição; a confirmação de um seletor de **outra cena** ou
  de um clipe que já não existe não escreve nada nem avisa (BL082.49); uma recusa
  (`fx_scope_mismatch`, `unknown_shot`, `fx_scope_not_yet`) é o **aviso do painel**, nada é escrito e o seletor volta
  ao escopo do documento no quadro seguinte. O escopo também está no título do clipe (`Vinheta · fx-1 · Plano shot-1 (FX 1)`).
- **Adicionar no plano exibido**: a linha "No plano exibido (shot-1)" tem um botão `+` por efeito. Cria um clipe no quadro
  que o monitor mostra, com o escopo do plano que o contém e **sem duração**: o core dá a típica cortada onde o plano termina
  (a janela não repete a regra; uma duração dada nunca é cortada). Fora da edição o botão fica **desabilitado com motivo** e a ação recusa com
  `disabled_element` sem escrever.
- **Nós semânticos** (`./scripts/ui get|set|invoke`; sem `Command` novo: são ops de `SceneCommand::FxEdit`, cujo parser
  `parse_fx_batch` ganhou `set_scope` e `add_clip.scope`):
  - `scene.fx.clip:<id>`: campo `scope` (`type: "choice"`, valor `output`/`shot:<id>`, opções com `applicable`/`reason`),
    uma ação `set_scope:<escopo>` por opção (desabilitada com o motivo quando já é o escopo atual ou o clipe não cabe) e
    `state.scope`/`scope_label`/`shot`. `set` do campo `scope` aceita o **texto** do escopo.
  - `scene.fx`: ações `add_in_shot:<efeito>` (habilitada só com um quadro da edição exibido) e `state.scope`
    (`["output","shot"]`), `state.shots`, `state.shot_here`.
- A confirmação do seletor vale só para o **documento em que ele foi pintado**: cada seletor guarda a revisão do projeto do último
  quadro pintado, e uma confirmação que chega com outra revisão não escreve (o core dá a um clipe novo o menor id livre, então um
  `fx-1` removido e outro `fx-1` criado são clipes diferentes para a mesma chave `cena+id`; CodeRabbit da PR #48).
- A ação `handles` e o botão de alças do clipe de plano ficam desabilitados com o texto do core (seção "Alças da vinheta").
- Nenhum diálogo novo (nenhuma marca em `Workflow::has_dialog`).
- A dica de `unknown_action` lista os nomes das ações **do próprio nó** (`action_names` sobre o mesmo `clip_actions`/`section_actions` que o nó lista): em um clipe, as `set_scope:*` incluídas; na seção, as `add:*` e as `add_in_shot:*`. Um teste compara a dica com o `actions` de cada nó. Três testes antigos do painel (`scene_fx_tests.rs`) fixavam listas exatas e foram ajustados ao contrato novo: as ações `add:*` agora são filtradas por prefixo (mais as `add_in_shot:*`), a lista de campos do `fade` termina em `scope` e a dica inclui as `set_scope:*`.

## Proveniência dos casos do piloto (`ORIGIN_NOT_EXECUTED`)

Lidos por `git show d40ac3f7c0:<arquivo>` no repositório do piloto (somente leitura; **nada foi executado, instalado nem
editado lá**). Os valores esperados foram **escritos à mão** nos testes; cada caso aponta o arquivo e o blob de onde veio.
O pacote de teste do piloto (`fx-test-bundle.ts`, blob `ec7668445809912b1972df2b0d202c6f4eba1e7e`) tem os planos
`s1 [0,48)`, `s2 [48,96)`, `s3 [96,144)`; o corpus `valid-minimal-continuous` tem os dois primeiros como `shot-1`/`shot-2`.

| # | Caso do piloto | Arquivo (em `packages/scene-engine/src/fx/`) | Blob | Teste aqui (`crates/core/tests/scene_fx_shot_scope.rs`) |
|---|---|---|---|---|
| 1 | clipe 0..24: `setScope` para o plano e de volta para a saída **remove a chave** | `fx-commands-params.spec.ts` | `7094de47bb6c00054085d15aa03ada290e493c61` | `pilot_case_1_set_scope_to_a_shot_and_back_to_the_output_removes_the_key` |
| 2 | clipe em 40, `setScope` s1: `FX_SCOPE_MISMATCH` | `fx-commands-params.spec.ts` | `7094de47bb6c00054085d15aa03ada290e493c61` | `pilot_case_2_a_clip_that_leaves_the_shot_is_refused_with_the_legal_interval` |
| 3 | `addClip` guarda o escopo de plano (e a seed) e descarta o escopo de saída | `fx-commands-clips.spec.ts` | `0b6bb7ba22769bfbe6badc042a99c86a68e4c28e` | `pilot_case_3_add_clip_keeps_a_shot_scope_and_a_seed_and_drops_an_output_scope` |
| 4 | um clipe dentro do plano e um que o preenche são válidos | `fx-explain.spec.ts` | `0838ac544bb7692a28f378f3a475896ecaaab4f6` | `pilot_cases_4_and_6_a_clip_inside_its_shot_is_valid_and_one_that_crosses_it_is_not` |
| 5 | um plano que não existe: `UNKNOWN_SHOT`, a dica lista os planos | `fx-explain.spec.ts` | `0838ac544bb7692a28f378f3a475896ecaaab4f6` | `pilot_case_5_a_shot_that_does_not_exist_is_unknown_and_the_hint_lists_the_shots` |
| 6 | clipe em 40, escopo s1: `FX_SCOPE_MISMATCH` no import | `fx-codes.spec.ts` | `8813008ac3aa8214ebd86d267f34a237fccf7d56` | `pilot_cases_4_and_6_a_clip_inside_its_shot_is_valid_and_one_that_crosses_it_is_not` |
| 7 | escopo `node` segue recusado (`NODE_SCOPE_NOT_YET` no piloto, `FX_SCOPE_NOT_YET` aqui) | `fx-codes.spec.ts` | `8813008ac3aa8214ebd86d267f34a237fccf7d56` | `pilot_case_7_a_node_scope_is_still_refused_by_the_importer_and_by_the_edit` |

Divergências **de propósito** (ver a tabela de decisões): o código do escopo `node`, `set_seed` como op própria e a grafia
em texto `shot:<id>` (o piloto só lê o objeto).

## Decisões deixadas ao implementador (e por quê)

| Decisão | Escolha | Motivo |
|---|---|---|
| Onde fica o cruzamento fx × planos | estágio 2 do importador (`integrity`), pela regra única `scope_fault` | o estágio 1 (`read_bundle`) só tem forma; os planos já estão lidos no estágio 2 e a mesma função serve às ops |
| Grafia | `output` / `shot:<id>` em ops e leituras; `{kind,shotId}` no documento e no plano | texto curto para a linha de comando; o objeto do piloto no arquivo; ID = tudo após o primeiro `:` |
| Divergência do piloto | código `FX_SCOPE_NOT_YET` (o piloto chama `NODE_SCOPE_NOT_YET`); `set_seed` é op própria (o `addClip` do piloto leva a seed) | nomes já em uso aqui; não renomear contratos existentes |
| `FX_SCOPE_MISMATCH.details` | `shot`, intervalo do plano, `clip`, intervalo do clipe | o adaptador mostra o intervalo legal sem reparsear a mensagem |
| Códigos de comando | `fx_scope_mismatch`, `unknown_shot`, `fx_scope_not_yet` | minúsculas como os de F016 |
| Capacidade | `fxShotScope`, 13ª, só com passe de plano | planos sem plano preso não mudam bytes |
| Inspector | `InspectorChoice` (`type: "choice"`) | o modelo só tinha campos numéricos e booleanos; o escopo é a primeira opção nomeada |
| Nós da janela | `set_scope:<escopo>`, `add_in_shot:<efeito>`, campo `scope` | segue `add:<efeito>`/`reset:<campo>` de F016/F058 |
| Duração padrão | o core corta a duração **omitida** (a típica, onde o escopo termina: `fx::default_duration`) e recusa a **dada** que não cabe | uma duração dada é a intenção do autor e a recusa tipada fica observável; a omitida é o corte típico, e a janela e o reset do inspector usam a mesma regra em vez de repeti-la |
| `Fx::resolve` | sem os planos (`resolve(quadro)`) | o importador garante a contenção; o plano é lido só onde há regra (`Shot::holds_frame` é o único predicado de "o quadro está no plano") |

## Evidências

Cada linha diz o que foi executado, em que head e com que resultado. O que **não** foi executado está dito como tal;
nenhuma linha "a validar" é um sucesso. Ambiente: macOS, Chrome headless com SwiftShader (sem GPU real).

| # | Prova | Superfície | Resultado | Onde |
|---|---|---|---|---|
| 1 | os 7 casos do piloto (valores à mão, proveniência acima), a regra única, as bordas dos dois corpora (`valid-minimal-continuous` `[0,48)/[48,96)` e `valid-cut-instance-offsets` `[0,40)/[40,100)`), lote atômico, retemporização, a duração padrão e o `reset` da duração junto ao fim do plano, composição em etapas, plano/capacidade/orçamento, leituras, inspector, os limites de início e duração de um clipe de plano (do plano, não da edição, cada extremo conferido contra `edit`), salvar e reabrir | Lib | **V** — `scene_fx_shot_scope` **33/33** em `4820958` (32/32 em `9cd63f6`, 31/31 em `e336118` e `a80ed73`); no mesmo head, `--lib` 575, `scene_fx` 25, `scene_fx_gesture` 19, `scene_fx_handles` 21 e `scene_fx_target` 10, todos ok | `crates/core/tests/scene_fx_shot_scope.rs` (e os módulos `scene_fx_shot_scope/plan.rs` e `reads.rs`, o mesmo binário) |
| 2 | processo `tessel-headless` real, sessão JSONL: `add_clip.scope`, `set_scope`, as três recusas, leituras, plano e a recusa das alças de um clipe de plano (`scope_not_supported`, `the_headless_executable_refuses_the_handles_of_a_shot_clip`) | CLI headless | **V** — `the_headless_executable_scopes_a_clip_like_the_library_and_refuses_the_same_way` (mesmo arquivo, heads `e336118`, `a80ed73`, `1cbc3da` e `4820958`): sobe o executável e conversa por JSONL | idem |
| 3 | runtime sem GPU: `fxShotScope` é a 13ª capacidade, passe de plano aceito só com ela e no quadro do plano, `shot_not_in_frame`, etapas em ordem (o parser recusa o passe de saída antes do de plano), deck antes da saída, mesmo ping-pong **num só laço**, e a telemetria `directed_scene_fx_shot` contada por valor num navegador real | Runtime (JS puro) | **V** — **384/384** em `4820958` (os 373 da `main` e 11 de F062; eram 266/266 nos heads `55dcc38`, `a80ed73` e `621b991`: o runtime inteiro, `./scripts/runtime test`) | `runtime/test/output-fx.test.mjs`, `directed-scene.test.mjs` |
| 4 | imagens com referência independente (Python/`scene.render`): o efeito de plano só escurece o próprio plano e é igual à saída no plano; a ordem das etapas **não** comuta; o export tem o hash de `scene.render` um quadro de cada lado do corte | Render (Chrome/SwiftShader) | **V** — 8/8 (heads `0448d02`, `a80ed73`, `621b991` e `4820958`; nos três últimos o crate `render` inteiro: 120 ok em `a80ed73` e `621b991` e **131** em `4820958`, sempre 0 falhas e 3 ignorados; os deltas abaixo são os medidos em `a80ed73`, e em `621b991` e `4820958` os mesmos testes passam com os mesmos limites): pior delta para a referência **0** nos dois corpora; mudança mais forte contra o baseline **220** (minimal) e **250** (corte); as duas ordens diferem **218**; export: 2 quadros dentro e 2 fora do plano, todos os hashes iguais a `scene.render` | `crates/render/tests/scene_fx_shot_scope.rs` |
| 5 | janela: nós, seletor Kit, `+` no plano exibido, recusas tipadas no painel, undo/redo, botões desabilitados com motivo | UI/CLI da janela (GPUI) | **V** — os 12 testes de `scene_fx_scope_tests` (os 7 de antes, 4 da correção da revisão independente — a confirmação de outra cena, a de um clipe removido, a dica da seção e o `reset` da duração de um clipe no plano, que também afirma os `min`/`max` de início e duração que o nó publica — e 1 da revisão do CodeRabbit: `a_pick_of_a_clip_recreated_with_the_same_id_writes_nothing`, a confirmação de um seletor pintado para outra revisão não escreve), `a_vignette_on_a_shot_has_no_handles_and_the_target_leaves_it` (`scene_fx_handles_tests`: ação e botão das alças desabilitados com o motivo do core, `scene.fx.select` recusado, o alvo sai na edição) e a caixa `tessel-editor` inteira, **508/508**, 0 falhas (head `4820958`, `TESSEL_REQUIRE_RUNTIME=1`; eram 429/429 em `621b991` e 425/425 em `a80ed73`: a diferença são os testes da `main` nova, #46 e #41), incluindo os 3 testes antigos do painel ajustados e o de F057 que compara a dica com o nó. A primeira execução real (BL082.46) achou **duas falhas do próprio código novo**, ambas corrigidas sem mudar comportamento: a expectativa do teste de recusas (o alvo de `unknown_shot` é o plano ausente, não o nó: `From<EditError>` usa o id como alvo, como `unknown_clip`; `be3628b`) e `clippy::type_complexity` num auxiliar de teste (`a80ed73`). A entrada do seletor é injetada (confirmação do `Select`, clique nos botões): o ponteiro físico segue não provado | `workstation/scene_fx_scope_tests.rs` |
| 6 | janela própria por `./scripts/ui` (só `op/get/set/invoke`): pixels do monitor contra Python, export de 8 quadros em volta do corte | UI/CLI da janela | **V** — `scripts/scene-fx-shot-scope-proof.py` **180/180** (head `c138d6d`, árvore limpa, nenhum arquivo sujo; eram 168/168 em `4820958` e 154/154 em `a80ed73` e `621b991`; macOS, instância privada com `HOME` curto isolado; prova funcional, sem o lock da GPU: AGENTS.md). Monitor e `scene.render` iguais à transcrição Python da vinheta (pior delta **0,5**, tolerância 3) nos quadros 12 e 47, e o quadro 48/60 é o baseline exato; plano com **um** passe `shot-1` nos quadros 12 e 47 e nenhum em 48/60 (e a telemetria do quadro do monitor, `directed_scene_fx_shot`, vale 1 no quadro 12 e está ausente no 60, conferida por valor), `fxShotScope` só onde há passe; etapas: o plano desenha a vinheta do plano **antes** do fade da saída, contra a ordem das trilhas (a ordem invertida é outra imagem: 188 no monitor, 60 no export); as cinco recusas de escopo tipadas (`fx_scope_mismatch`, `unknown_shot`, `fx_scope_not_yet`, `invalid_value` duas vezes) por `set` e por `invoke`, com o aviso do painel e sem escrever nada (histórico, intervalo, escopo, imagem); undo/redo; `project.save` + `project.open`; export de 8 quadros (44..51) iguais à composição Python (pior delta **6,9**, tolerância do codec 16). As 14 verificações novas são a recusa das alças de um clipe de plano: a ação `handles` desabilitada com o motivo do core (a palavra «plano»), `invoke handles`, `scene.fx.select` e `scene.fx.handles` com `scope_not_supported` (alvo o clipe, escopo em `details`), o nó do gizmo mantido, nada escrito, e o mesmo por um processo `tessel-headless` real (as 6 linhas respondidas, a recusa, a saída 1), com as alças de volta quando o clipe volta à saída. As 12 verificações novas da rodada 3 são uma por render do script (`rendered`: o render respondeu uma sonda por ponto da grade). A parte pura (`.spec.py`) **21/21** (eram 13: o render sem sonda, sem resposta ou malformado, as expectativas estritas, a execução que termina vermelha com uma só falha e a leitura das recusas do `headless`) e a do F016 que ela reutiliza **28/28**; os **39** arquivos de spec de `scripts/`: **1264** testes, todos ok (1256 antes da rodada 3). O `.spec.py` da prova (21), o `test_mutate.py` (8) e o `mutate.py --check` (50 de 50) rodam em Python **3.9.6** (o do sistema do macOS, precedente BL145.1) e em **3.14**; a suíte inteira dos 39 arquivos, só em 3.14 | `scripts/scene-fx-shot-scope-proof.py` |
| 7 | mutações, um mutante por vez (`docs/migration/p4a-shot-scope/mutate.py`, 50 mutantes aplicáveis conferidos com `--check`; e o corredor da F061, `docs/migration/g41-handles/mutate.py`, 43) | todas | **50/50 MORTOS** (core 29, JS 6, imagem 3, navegador 1, janela 11) e **43/43** (F061: M01–M33 em Rust e J01–J10 na página), no código final `af2ace1`: os arquivos mutados são **idênticos** em `c2c6367` e no merge `4820958` (o sha256 dos 8 e dos 15 arquivos de `hashes` e de `hashes_before`/`hashes_after` dos dois relatórios foi conferido depois do merge com a `main` `f321ec0`). Em `621b991` eram 41 (core 25, JS 6, imagem 3, navegador 1, janela 6); desde então: C27–C30 (limites dos campos de tempo), W08–W09 (seleção presa à revisão), W10–W11 (ação e botão das alças de um clipe de plano), W12 (o limite que o nó publica) e, no corredor da F061, M30–M33 (a recusa). Cada runner teve a linha de base verde antes, cada mutante foi morto por pelo menos um teste nomeado (em `mutation-report.json`), os arquivos foram restaurados e conferidos por hash e `git status`, e `runtime/dist` foi reconstruído **sem** mutante depois de cada um dos de imagem. Uma primeira execução do corredor da #48 (01:00) parou com `baseline desktop exit 1` por erro do **executor** (dois nomes de teste antes do `--` no `cargo test`, que aceita um): nenhum mutante foi julgado, o executor foi corrigido e os dois corredores rodaram de novo inteiros, uma vez, no código final. O autoteste do executor, `python3 docs/migration/p4a-shot-scope/test_mutate.py` (hermético: um diretório temporário com alvo e mutantes sintéticos, sem tocar `runtime/src/output-fx.js`; a restauração que falha para o lote, a linha de base que não compila, a execução sã e a forma dos comandos do cargo), guarda essas regras; na rodada 3 ganhou o id de mutante desconhecido (recusado com saída 2 antes de construir, rodar ou gravar qualquer coisa), o sinal no meio da reconstrução que restaura o `dist` (a execução nunca conta como restaurada, na primeira e numa reconstrução posterior) e os ids conhecidos, que seguem selecionando só esses: **8** testes. `mutate.py` e `test_mutate.py` não estão entre os arquivos mutados (`hashes_before`: 15 e 8 arquivos conferidos por sha256 em `c138d6d`), por isso os relatórios dos corredores seguem válidos sem rodar de novo | `docs/migration/p4a-shot-scope/`, `docs/migration/g41-handles/` |
| 8 | gates | — | Head `4820958` (a `main` `f321ec0` integrada; código e testes em `af2ace1`, os relatórios dos corredores em `c2c6367`; os commits depois de `4820958` mudam docs e, nas rodadas 3 e 4, só Python (o executor do corredor, a prova da janela e os specs; nenhum `.rs`, então nenhuma porta de Rust se repetiu); macOS, E/S pesada no SSD externo, etapas pesadas atrás da trava `heavy-gate.sh`): `cargo fmt --all -- --check`, `./scripts/cargo check --workspace --locked` e `./scripts/cargo clippy --workspace --all-targets --locked -- -D warnings` **limpos**; `./scripts/cargo build -p tessel-editor --locked` e `tessel-headless` **ok**; `TESSEL_REQUIRE_RUNTIME=1 ./scripts/cargo test --workspace --locked --no-fail-fast`: **2555 ok, 1 falha, 16 ignorados** em 125 binários (desktop 508, audio 10, core 1406 + 4 de documentação, media 496, render 131; a `main` tem 2473 ok, 1 falha e 13 ignorados: a diferença são os testes de F062 e das correções, e os `scene_gizmo_perf_*` da #49, de que 3 medições de orçamento são `#[ignore]`); `./scripts/runtime test` **384/384** (a `main` tem 373); specs Python dos 39 arquivos: 1256 testes ok (1264 em `c138d6d`, com os 8 da rodada 3). A única falha é de **ambiente**: `a_full_disk_ends_the_export_with_disk_full_and_leaves_the_volume_clean` (`crates/media/tests/export.rs`) não monta a imagem de teste (`hdiutil: attach failed - Permissão negada` no sandbox); F062 não toca `crates/media` (BL082.47). `cargo tree -p tessel-editor-core` **sem** gpui/wry. As rodadas anteriores: head de código `621b991` (2397 ok, runtime 266/266) e `a80ed73` (2387 ok). | — |
| 9 | desempenho (passe de plano, duas etapas) | Runtime/export | **A validar** — não medido (BL082.43) | — |

**Histórico e ambiente.** A entrega original não compilou a janela porque `./scripts/disk-guard.sh` recusou todo lote pesado (`REFUSED_DISK`: 4 a 7 GiB livres, piso de 12 GiB) e nada foi limpo para contornar a recusa. A validação (BL082.46) rodou depois com toda a E/S pesada num SSD externo (cache do mbx, `TMPDIR` e a loja do pnpm no SSD; o piso de 12 GiB medido nele), sem apagar nem mudar nada do disco interno e sem tomar o lock da GPU. Desempenho (linha 9) é o único item que exige esse lock e **não** foi medido.

## Limites e o que **não** foi provado

- Alças da vinheta de um clipe de plano (BL082.41): recusadas com `scope_not_supported` (BL082.64, concluído); desenhá-las no quadro
  de um plano segue pendente.
- Limites de início e duração de um clipe **na saída**: ainda oferecem o que a regra recusa perto do fim da edição; o conserto
  muda o JSON de `scene.fx.inspect` da saída e não é desta entrega (BL082.69, com a mudança de lugar de `timing_limits`).
- Limites de início e duração de um clipe `shot`: têm evidência de biblioteca (`reads.rs`) e de janela (`scene_fx_scope_tests.rs`), não de um processo real
  do `tessel-headless`; é a mesma op `scene.fx.inspect` despachada pelo mesmo handler (BL082.74).
- Um escopo `{"kind":"output","shotId":…}` no documento ou numa op é aceito em silêncio como saída; o leitor do plano e o runtime o recusam (BL082.72).
- `scene.fx.inspect` com `clip` monta o inspetor de todos os clipes antes de filtrar: `O(clipes × planos)`, só perto dos limites declarados (BL082.70).
- Escopo `node`, transições e o segundo deck (P4b/BL082.4): recusados ou não existentes; o runtime desenha a lista como vem
  e o parser garante a ordem; a emenda do segundo deck nasce com a P4b.
- Ponteiro/teclado físicos sobre o `Select` do Kit e a pintura da WKWebView nativa: a janela é provada por GPUI
  (entrada injetada) e por `./scripts/ui` (só `op`/`get`/`set`/`invoke`); BL043 segue aberto.
- Desempenho (custo de um passe de plano, quadros mais lentos): não medido; fica "A validar" no backlog.
- Windows, GPU nativa e a execução do motor original (`ORIGIN_NOT_EXECUTED`): fora do que foi provado.
- Os frames de "handle"/costura entre planos (cena com corte) usam o plano do quadro; não há fusão entre planos.
