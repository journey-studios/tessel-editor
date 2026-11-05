# F037 — Plano avaliado de um frame de cena dirigida (BB05, primeira fatia)

## Status

Lib **V no subconjunto documentado** (29 testes públicos e o doctest passaram via Cargo;
clippy lib limpo na integração do coordenador). CLI headless **P** (nenhum comando). UI **P** (nenhum controle).
BB05 continua **parcial**: esta fatia entrega o **contrato e o plano**; não existe renderer que o
consuma (BL007) e nenhum recurso é resolvido em disco (BL009/BL010). Nada aqui é render real.

## Objetivo e limites

Dado um `DirectedScene` importado ([F031](directed-scene.md)) e um frame inteiro da edição, o core
devolve um **plano puro, tipado e serializável** com tudo que um runtime precisa para desenhar o
frame: câmera, nós com matrizes locais e de mundo, ordem de pintura das camadas, recursos
requeridos por `assetId`, versão do plano e capacidades exigidas. O runtime só executa estado
explícito (D4: regra no core, execução visual no runtime).

O plano **não reimplementa** câmera, animação nem matrizes: usa `Bundle::evaluate_frame`
([F033](scene-temporal.md), [F035](scene-animation.md)), o grafo (`SceneGraph`) e
`Bundle::paint_order`. O teste de corpus compara o plano com a avaliação bit a bit.

Fora do escopo desta entrega (listado no plano como **domínio não avaliado**, nunca omitido):
luz, clima, efeitos, rigs, caminhos, `reveal`, mapas de dados dos nós (`surface`, `normalMap`,
`sceneDepth`, `relief`) e nós `splat`. Também fora: IO, GPUI, resolução/leitura de arquivos,
áudio/narração (BL016), os gates `temporal_issues()`/`edit_join_issues()` (o adapter que habilitar
cenas deve chamá-los antes; o plano não os roda).

## Contrato do core / biblioteca

```rust
use tessel_editor_core::directed_scene::DirectedScene;
use tessel_editor_core::scene_plan::{PlanOptions, ScenePlan};

let scene = DirectedScene::import_str(text)?;
let plan = ScenePlan::build(&scene, 12)?;            // completo, ou erro tipado
let plan = ScenePlan::build_with(&scene, 12, PlanOptions { allow_partial: true })?; // parcial explícito
plan.required_asset_ids();                            // lista estável de recursos
plan.require_capabilities(&runtime_supports)?;        // o runtime tem o que o plano pede?
let wire = plan.to_json_string();                     // JSON canônico
let same = ScenePlan::from_json_str(&wire)?;          // valida; == plan
```

| Item | Contrato |
|---|---|
| `ScenePlan::build(scene, edit_frame)` | Plano **completo**. `edit_frame: i64` como em `evaluate_frame`. A cena só é lida: erro não altera nada |
| `ScenePlan::build_with(.., PlanOptions)` | `allow_partial: true` devolve o que **é** avaliado, com `unsupported` preenchido e `is_complete() == false`. Decisão explícita do chamador (diagnóstico); um runtime que o desenhe como se fosse o quadro mostra a cena sem luz/clima |
| `ScenePlan::validate()` | Invariantes (abaixo). `build` valida o que devolve; `from_json*` valida o que lê |
| `to_json` / `to_json_string` / `to_json_pretty` / `from_json` / `from_json_str` | Forma de transporte. Escrita à mão: o core usa `serde` sem `derive` (não se altera `Cargo.toml`) |
| `required_asset_ids()`, `resource(id)`, `node(id)` | Consultas |
| `missing_capabilities(supported)` / `require_capabilities(supported)` | `Err(PlanError::CapabilityUnavailable { missing })` |
| `scene_plan::capabilities()` | O que esta versão do plano carrega (`supported`) e recusa (`unsupported`: todos os `Domain`) |
| `PLAN_KIND` = `"tessel-scene-plan"`, `PLAN_VERSION` = 1, `OUTPUT_COLOR_SPACE` = `"srgb-display"` | Constantes |

### Campos do plano (`ScenePlan`)

| Campo | Conteúdo |
|---|---|
| `version` | `PLAN_VERSION`. Outra versão é recusada **antes** de ler qualquer outro campo (`UnsupportedVersion`) |
| `source` | `bundle_id`, `revision_id`, `scene_definition_id`, `scene_instance_id` — identidade, nunca caminho |
| `output` | `width`, `height`, `fps` (como declarado, ex. 24000/1001), `alpha_mode` (`straight`/`opaque`; `premultiplied` já é recusado na importação). Cor sempre `srgb-display` |
| `frame` | frame de edição, shot, frame da instância e do clipe de câmera; tempos como `Ratio` exato (12 frames a 24000/1001 = 1001/2000 s) |
| `camera` | `CameraState` do avaliador (posição, alvo, FOV, velocidades) |
| `animation` | `NodeAnimation`: `Static` ou `Animated { track_ids, channels }`. As matrizes **já** incluem os canais |
| `nodes` | Todos os nós da definição, em ordem de documento: tipo, pai (nó ou socket), posição/rotação (Euler°)/escala locais, `local_matrix` e `world_matrix` (column-major, 16 números), `renderOrder` próprio, `asset_id`, tamanho em unidades de cena |
| `layers` | Planos de imagem **de trás para frente**: nó, `asset_id`, `alpha` do asset, `render_order` próprio, `pass_order` (passe de composição) |
| `resources` | Texturas requeridas, **ordenadas por `assetId` (bytes), sem repetição**: `revision_id`, `content_hash`, `relative_path`, formato, dimensões, alpha, espaço de cor, `used_by` (nós, em ordem de documento) |
| `requires` | `Capability`s que o conteúdo exige: `camera`, `nodeTransforms`, `sockets`, `performanceChannels`, `imagePlanes`, `straightAlpha`, `opaqueAlpha`, `compositionPassOrder`. Ascendente. Um plano sem camada opaca não pede `opaqueAlpha` |
| `unsupported` | `UnsupportedDomain { domain, subject }` ordenados; vazio no plano completo |
| `uninterpreted` | Campos do bundle que o importador preservou e o plano não interpreta (JSON Pointer + motivo), ordenados por ponteiro |

### Políticas

| Tema | Política |
|---|---|
| Identidade de asset | **`assetId`.** `relative_path` é metadado (declarado, relativo à pasta do bundle, nunca absoluto — o plano recusa `..`, absoluto e drive); `revision_id`/`content_hash` servem a quem resolve e verifica o arquivo. O plano **não verifica** se o arquivo existe (BL009) |
| Domínios não avaliados | Declarados e não avaliados ⇒ `PlanError::Unsupported { domains }` (ordenado por `Domain` e nó). Escopo: `weather`, `weatherKeys`, `lighting`, `effects`, `weatherGround`, `rigs`, `paths` (lista do próprio avaliador, `unevaluated_sections`); `rigBind`, `reveal`, `surface`, `normalMap`, `sceneDepth`, `relief` por nó; `splat` por nó. `null` conta como não declarado, como no importador. Mapas de dados não variam no tempo (por isso o avaliador temporal não os lista), mas mudam o aspecto do plano: um plano que os ignorasse não seria o quadro |
| Splat | **Não presumido.** Um nó `splat` tem transformação e asset, mas luz/clima/efeitos sobre splat são BL025. No plano parcial o nó continua em `nodes` (filhos dependem da matriz), mas não é camada nem recurso e consta em `unsupported` |
| Campos não interpretados | Chave desconhecida (`UnknownField`) e seção declarada e mantida crua (`narration`, `outline`, `metadata`, `contactPlaneId`, `frameAudit`: `DeferredSection`) vão para `uninterpreted`, sem bloquear. **Escopo:** chaves do topo + definição de cena, instância, shot, clipe, trilhas, passes e assets que o frame usa; o plano de um frame não muda porque **outra** cena do bundle ganhou um campo |
| Ordem de pintura | A de `Bundle::paint_order` (passe de composição > `renderOrder` > 0; empate = ordem de documento). `layers` copia as entradas para que a ordem seja conferível só pelo plano. Limite herdado: o desempate por profundidade da câmera do Three.js **não** é modelado |
| Transparência | Do schema: `alpha` do asset da camada (`straight`/`opaque`) e `alphaMode` do `renderProfile`. Não há opacidade por nó no schema; nenhuma foi inventada |
| Determinismo | Função pura do bundle e do frame inteiro: a ordem das perguntas não importa; o mesmo texto de entrada dá o mesmo JSON. Chaves JSON saem na ordem do mapa do `serde_json` |
| Erros do frame | O erro do avaliador (`PlanError::Frame(TemporalError)`, mesmo `code`) vem **antes** dos domínios, na ordem de verificações da origem |

### Invariantes (`validate`)

Ids de nós únicos e resolvidos (pai ≠ si mesmo, sem ciclo); nó `imagePlane` com asset e tamanho
positivo, `group` sem asset, `splat` com asset; uma camada por plano de imagem, de trás para frente
(ordem efetiva ascendente, empate pela ordem dos nós) e coerente com o nó e com o recurso;
recursos ordenados, sem repetição, usados, `png`, caminho relativo seguro, `used_by` exato;
`requires` igual ao derivado do conteúdo; `unsupported` ordenado, com nós existentes, e todo nó
`splat` listado; `uninterpreted` ordenado; números finitos; saída com dimensões positivas e taxa
representável. Na leitura: `kind` e versão primeiro, chave desconhecida ou ausente recusada (um
plano que carregasse dados ignorados não faria round-trip), tempos só reduzidos, `complete` igual a
`unsupported` vazio.

### Erros (`PlanError`, `code()`; `From<PlanError> for CommandError`)

| Variante | `code` | Quando |
|---|---|---|
| `Frame(TemporalError)` | o do avaliador (`frame_out_of_range`, `invalid_camera`, `invalid_animation`, `invalid_time`) | Frame fora da edição, sem shot, canais em conflito, nó não finito... |
| `Unsupported { domains }` | `unsupported` (+ `details.domains`) | Domínio declarado e não avaliado, plano completo pedido |
| `CapabilityUnavailable { missing }` | `unsupported` (+ `details.missing`) | O runtime não tem o que o plano exige |
| `UnsupportedVersion { found, supported }` | `unsupported_plan_version` | Plano de outra versão |
| `Invalid { pointer, message }` | `invalid_plan` | Plano (JSON ou montado à mão) que viola o contrato |
| `Unresolved`, `MalformedDocument` | `invalid_plan` | Não ocorrem após uma importação; a API fica total em vez de entrar em pânico |

## Contrato com a frente I (recursos, BL009–BL011)

- O plano lista os recursos por `assetId`, na ordem estável acima; `required_asset_ids()` é a lista
  que o manifesto deve conseguir responder. **Nenhuma dependência** do módulo `scene_resources`: o
  plano não o importa e o manifesto não precisa importar o plano.
- O manifesto de I continua dono de resolver caminhos, hashes e ausentes. Dados que o importador
  preserva e o plano não usa **não** viram recurso: aparecem em `uninterpreted`, ou em `unsupported`
  (mapas de dados e splats, cujos assets o plano **não** lista como requeridos).
- Teste de integração sugerido depois do merge das duas frentes (BL062): para cada frame, todo
  `required_asset_ids()` é conhecido do manifesto do mesmo bundle.

## CLI headless

**P.** Nenhum comando ainda. Proposta (BL062): `scene.plan { scene, frame, partial? }` →
`ScenePlan::to_json`, com o mesmo parser compartilhado (`ops::parse_op`) e o mesmo `PlanError` →
`CommandError`. Não implementado nesta fatia: ela não toca `ops.rs`, `scene_project` nem `headless`.

## UI e CLI da janela

**P / N/A.** Nenhum controle. A UI não precisa do plano: o consumidor é o runtime/renderer
(BL007), que ainda não existe.

## Evidências e status

| Superfície | Estado |
|---|---|
| Lib | **I** — 29 testes públicos escritos em `crates/core/tests/scene_plan.rs`; **não executados** (sem Cargo nesta rodada) |
| Qualidade | `rustfmt` aplicado só aos arquivos da frente. Cargo/clippy/doctest **não rodados** |
| Contra a origem | Nenhuma comparação nova. O plano é comparado com `evaluate_frame` (que as suítes F033/F035 comparam com a origem); a ordem de pintura vem de `paint_order` (BB01/BB02) |
| CLI headless / UI | **P** |

Limites: o plano não é um render; não há consumidor; recursos não são resolvidos nem verificados em
disco; domínios listados acima são recusados; o custo por frame refaz a linha do tempo de nós
(`NodeTimeline::new`) a cada chamada, o que importa para exportar muitos frames (BL063);
cada nó leva duas matrizes de 16 números no JSON.
