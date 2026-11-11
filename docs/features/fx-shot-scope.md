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

### Composição em etapas (`Fx::resolve(quadro, planos)`)

`Fx::resolve` recebe os intervalos dos planos (função pura; sem relógio nem ordem de pedidos) e devolve os passes em
**duas etapas**: `FxStage::Deck` (passes de plano, a imagem do próprio plano) primeiro, depois `FxStage::Output`, cada
uma na ordem das trilhas e dos clipes. Um passe de plano só existe nos quadros desse plano. A ordem **não** é só a das
trilhas: uma vinheta de saída em uma trilha anterior à de um fade de plano é desenhada **depois** dele. Uma futura
transição (P4b) entrará entre as duas etapas; o runtime já tem o ponto de encaixe (laço do deck separado do da saída).

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
| `add_clip { …, scope? }` | `scope` opcional (`output` por padrão), texto ou objeto. Um clipe que não cabe no plano é recusado **antes** de escrever (a recusa nomeia o plano e o intervalo legal); não há corte |
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

`scene_edit::fx_inspector::inspector_choices(clipe, planos)` devolve `InspectorChoice` — a primeira configuração do
modelo que é uma de poucas opções nomeadas, não um número. JSON de cada escolha (também o campo `scope` do nó da
janela): `{ id: "scope", group: "scope", type: "choice", label, labels {pt-br,en,es}, value, default: "output",
at_default, options: [...] }`; cada opção: `{ value, kind, shot, label, labels, intent, start_frame, end_frame_exclusive,
selected, applicable, reason, op }` — `applicable: false` com o **código** que a edição recusaria (`FX_SCOPE_MISMATCH`/
`UNKNOWN_SHOT`) e `op` (o `set_scope` pronto) só quando a opção pode ser escolhida e não é a atual. Rótulos: `Escopo/
Scope/Alcance`; opções `Saída/Output/Salida` e `Plano {id}/Shot {id}/Plano {id}`.

## Runtime (`runtime/src/output-fx.js`, `directed-scene.js`, `compositor.js`)

- `parseOutputFx(raw, requires, frame)` aceita `scope: {kind:'shot', shotId}` **só** com `fxShotScope` em `requires` e
  quando o `shotId` é o do `frame` (`/frame/shotId`); um passe de outro plano é `unsupported`
  (`{unsupported:'fx_scope', reason:'shot_not_in_frame'}`), `node` ou escopo desconhecido é `unsupported`
  (`reason:'scope_kind'`), nunca pulado. As etapas têm de vir em ordem.
- `OutputFxPipeline.draw` → `drawDeck` (passes de plano) e `drawOutput` (passes de saída): dois laços sobre o **mesmo**
  ping-pong de dois alvos (`targetOf(frame.drawn % 2)`), então o resultado do deck é a entrada da saída sem cópia. O
  ponto entre `drawDeck` e `drawOutput` é onde P4b compõe o segundo deck.
- `SCENE_CAPABILITIES` tem as 13 capacidades; telemetria: `frame.sources.directed_scene_fx_shot` conta os passes de plano
  (junto de `directed_scene_fx`).

## Janela (`workstation/scene_fx_scope.rs`)

A seção "Efeitos" **não tem estado próprio**: o seletor e as ações são pintados do mesmo modelo (`inspector_choices`) e
cada gesto é um `scene.fx.edit` por `fx_run` — o mesmo handler do `set`/`invoke` do nó.

- **Seletor**: cada clipe tem uma linha "Escopo" com um `Select` do Kit (opções: `Saída`, `Plano shot-1 · 0–47`, …).
  Escolher é um `set_scope` (uma entrada de undo); escolher o escopo que já vale não é edição; uma recusa
  (`fx_scope_mismatch`, `unknown_shot`, `fx_scope_not_yet`) é o **aviso do painel**, nada é escrito e o seletor volta
  ao escopo do documento no quadro seguinte. O escopo também está no título do clipe (`Vinheta · fx-1 · Plano shot-1 (FX 1)`).
- **Adicionar no plano exibido**: a linha "No plano exibido (shot-1)" tem um botão `+` por efeito. Cria um clipe no quadro
  que o monitor mostra, com o escopo do plano que o contém e a duração típica **cortada no fim do plano** (o corte é
  escolha da janela; o core nunca corta). Fora da edição o botão fica **desabilitado com motivo** e a ação recusa com
  `disabled_element` sem escrever.
- **Nós semânticos** (`./scripts/ui get|set|invoke`; sem `Command` novo: são ops de `SceneCommand::FxEdit`, cujo parser
  `parse_fx_batch` ganhou `set_scope` e `add_clip.scope`):
  - `scene.fx.clip:<id>`: campo `scope` (`type: "choice"`, valor `output`/`shot:<id>`, opções com `applicable`/`reason`),
    uma ação `set_scope:<escopo>` por opção (desabilitada com o motivo quando já é o escopo atual ou o clipe não cabe) e
    `state.scope`/`scope_label`/`shot`. `set` do campo `scope` aceita o **texto** do escopo.
  - `scene.fx`: ações `add_in_shot:<efeito>` (habilitada só com um quadro da edição exibido) e `state.scope`
    (`["output","shot"]`), `state.shots`, `state.shot_here`.
- Nenhum diálogo novo (nenhuma marca em `Workflow::has_dialog`).
- Limite: a dica de `unknown_action` de um clipe lista só as ações-base; as `set_scope:*` estão no `actions` do nó.

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
| Corte da duração | a janela corta ao fim do plano; o core recusa o que não cabe | "o core nunca corta" mantém a recusa tipada observável |

## Evidências

Cada linha diz o que foi executado, em que head e com que resultado. O que **não** foi executado está dito como tal;
nenhuma linha "a validar" é um sucesso. Ambiente: macOS, Chrome headless com SwiftShader (sem GPU real).

| # | Prova | Superfície | Resultado | Onde |
|---|---|---|---|---|
| 1 | os 7 casos do piloto (valores à mão, proveniência acima), a regra única, as bordas dos dois corpora (`valid-minimal-continuous` `[0,48)/[48,96)` e `valid-cut-instance-offsets` `[0,40)/[40,100)`), lote atômico, retemporização, composição em etapas, plano/capacidade/orçamento, leituras, inspector, salvar e reabrir | Lib | **V** — `scene_fx_shot_scope` 31/31 (head `e336118`) | `crates/core/tests/scene_fx_shot_scope.rs` |
| 2 | processo `tessel-headless` real, sessão JSONL: `add_clip.scope`, `set_scope`, as três recusas, leituras, plano | CLI headless | **V** — `the_headless_executable_scopes_a_clip_like_the_library_and_refuses_the_same_way` (mesmo arquivo, mesmo head): sobe o executável e conversa por JSONL | idem |
| 3 | runtime sem GPU: `fxShotScope` é a 13ª capacidade, passe de plano aceito só com ela e no quadro do plano, `shot_not_in_frame`, etapas em ordem, deck antes da saída, mesmo ping-pong, laços separados | Runtime (JS puro) | **V** — 266/266 (head `55dcc38`) | `runtime/test/output-fx.test.mjs`, `directed-scene.test.mjs` |
| 4 | imagens com referência independente (Python/`scene.render`): o efeito de plano só escurece o próprio plano e é igual à saída no plano; a ordem das etapas **não** comuta; o export tem o hash de `scene.render` um quadro de cada lado do corte | Render (Chrome/SwiftShader) | **V** — 8/8 (head `0448d02`): pior delta para a referência **0** nos dois corpora; mudança mais forte contra o baseline **220** (minimal) e **250** (corte); as duas ordens diferem **218**; export: 2 quadros dentro e 2 fora do plano, todos os hashes iguais a `scene.render` | `crates/render/tests/scene_fx_shot_scope.rs` |
| 5 | janela: nós, seletor Kit, `+` no plano exibido, recusas tipadas no painel, undo/redo, botões desabilitados com motivo | UI/CLI da janela (GPUI) | **A validar** — 7 testes escritos e revisados estaticamente contra as APIs do Kit e da janela, **não compilados** (commit `5a19273`): `REFUSED_DISK` (BL082.46) | `workstation/scene_fx_scope_tests.rs` |
| 6 | janela própria por `./scripts/ui` (só `op/get/set/invoke`): pixels do monitor contra Python, export de 8 quadros em volta do corte | UI/CLI da janela | **A validar** — `scripts/scene-fx-shot-scope-proof.py` escrito, a parte pura (`.spec.py`) 13/13; **não executado** (`REFUSED_DISK`) | `scripts/scene-fx-shot-scope-proof.py` |
| 7 | mutações, um mutante por vez (`docs/migration/p4a-shot-scope/mutate.py`, 35 mutantes aplicáveis conferidos com `--check`) | todas | JS puro **6/6 MORTOS** (cada um com o teste que falhou, em `mutation-report.json`); core (22), imagem (3) e janela (4): **A validar** (`REFUSED_DISK`) | `docs/migration/p4a-shot-scope/` |
| 8 | gates | — | **fmt limpo** e `cargo tree -p tessel-editor-core` sem gpui/wry no head final. Antes da fatia da janela, no código do commit do core (`e336118`): `cargo check --workspace --locked` e `cargo clippy --workspace --all-targets --locked -- -D warnings` passaram e `cargo test -p tessel-editor-core --locked` (53 binários) deu **1336 passou, 0 falhou, 4 ignorados** (inclui os goldens do plano: `scene_plan_performance_golden` e os corpora, bytes iguais). **Não** foram executados no head final: `check`/`clippy`/`test --workspace`/`build -p tessel-editor` (a janela é código novo ainda não compilado; `REFUSED_DISK`) | — |
| 9 | desempenho (passe de plano, duas etapas) | Runtime/export | **A validar** — não medido (BL082.43) | — |

`REFUSED_DISK`: `./scripts/disk-guard.sh` recusou todo lote pesado depois da fatia de imagem (4 a 7 GiB livres; o piso é 12 GiB e
o passo mínimo é 1 GiB). Nada foi limpo para contornar a recusa. O registro é BL082.46; quem tiver ≥ 14 GiB livres roda os comandos
dali e atualiza esta tabela.

## Limites e o que **não** foi provado

- Escopo `node`, transições e o segundo deck (P4b/BL082.4): recusados ou não existentes; a emenda do deck está no
  runtime mas é só o ponto de encaixe.
- Ponteiro/teclado físicos sobre o `Select` do Kit e a pintura da WKWebView nativa: a janela é provada por GPUI
  (entrada injetada) e por `./scripts/ui` (só `op`/`get`/`set`/`invoke`); BL043 segue aberto.
- Desempenho (custo de um passe de plano, quadros mais lentos): não medido; fica "A validar" no backlog.
- Windows, GPU nativa e a execução do motor original (`ORIGIN_NOT_EXECUTED`): fora do que foi provado.
- Os frames de "handle"/costura entre planos (cena com corte) usam o plano do quadro; não há fusão entre planos.
