# F037 — Plano avaliado de um frame de cena dirigida (BB05, primeira fatia)

## Status

Lib **V no subconjunto documentado**; CLI headless e socket **V no corpus** por `scene.plan`.
A UI consome o plano pelo renderer [F041](scene-renderer.md). A biblioteca de plano é pura: resolução de arquivos pertence a [F040](scene-resource-integration.md), e execução GPU ao runtime. BB05 continua parcial nos domínios recusados e no export dirigido BL008.

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
`sceneDepth`, `relief`). Nós `splat` entram desde [F048](scene-splat.md) (matriz e recurso aprovado; sem luz, clima ou rig). Também fora: IO, GPUI, resolução/leitura de arquivos,
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

### Muitos frames: `PlanTimeline` (BL064, aditivo)

```rust
use tessel_editor_core::scene_plan::{PlanOptions, PlanTimeline};

let timeline = PlanTimeline::new(&scene);          // empresta a cena; não calcula nada ainda
let plan = timeline.build(frame)?;                 // == ScenePlan::build(&scene, frame)
let plan = timeline.build_with(frame, PlanOptions { allow_partial: true })?;
```

| Item | Contrato |
|---|---|
| Resposta | **A mesma** de `ScenePlan::build_with(&scene, frame, options)`: mesmo `ScenePlan` (valor e JSON canônico, byte a byte), mesmos erros na mesma ordem (erro de frame antes de domínio/documento; recusa de `unsupported` antes dos erros estáticos) e mesmas capacidades. Nenhum domínio recusado passa a ser aceito |
| Ordem | Qualquer: para frente, para trás, embaralhada, repetida. A resposta depende só do frame, nunca da ordem das perguntas |
| Unidade de reuso | O **plano**: instância, clipe de câmera e plano da cena que um *shot* fixa são preparados uma vez (sob demanda, `OnceLock`) e reutilizados; por frame só se reavalia o que depende do frame (câmera, matrizes locais/de mundo dos nós, `frame`). O molde é validado uma vez; `at_frame` escreve o que varia e erra tipado (`PlanError::Unresolved`) se a lista de nós não alinhar |
| Memória | Limitada pelo bundle, não pelos frames perguntados: `prepared_layouts()` nunca passa de `instâncias + clipes + shots` (teste). Nenhum plano é guardado |
| Geração | A linha do tempo **empresta** a cena: nova geração da cena = nova `PlanTimeline` (o empréstimo impede usar a antiga). Cena de outra geração nunca responde por esta |
| Threads | `Sync`; vários chamadores perguntam frames diferentes ao mesmo tempo e recebem as respostas sequenciais (teste) |
| Falha de preparo | Uma instância/clipe que não prepara falha igual para todo frame que o precisa, com o mesmo erro (guardado como resultado) |
| Consumidor atual | `ScenePlan::build`/`build_with` seguem como estão (mesma assinatura, refatorados sobre o mesmo núcleo `Prepared`). A adoção na exportação/monitor é um patch do coordenador (ver [relatório R6](../migration/next-r6-report.md)) |

Evidência de custo e tamanho (medida, não suposta): ver "BL064 — medição" em Evidências.

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
| `requires` | `Capability`s que o conteúdo exige: `camera`, `nodeTransforms`, `sockets`, `performanceChannels`, `imagePlanes`, `straightAlpha`, `opaqueAlpha`, `compositionPassOrder`, `splats` (só com nó `splat`; F048). Ascendente. Um plano sem camada opaca não pede `opaqueAlpha` |
| `unsupported` | `UnsupportedDomain { domain, subject }` ordenados; vazio no plano completo |
| `uninterpreted` | Campos do bundle que o importador preservou e o plano não interpreta (JSON Pointer + motivo), ordenados por ponteiro |

### Políticas

| Tema | Política |
|---|---|
| Identidade de asset | **`assetId`.** `relative_path` é metadado (declarado, relativo à pasta do bundle, nunca absoluto — o plano recusa `..`, absoluto e drive); `revision_id`/`content_hash` servem a quem resolve e verifica o arquivo. O plano **não verifica** se o arquivo existe (BL009) |
| Domínios não avaliados | Declarados e não avaliados ⇒ `PlanError::Unsupported { domains }` (ordenado por `Domain` e nó). Escopo: `weather`, `weatherKeys`, `lighting`, `effects`, `weatherGround`, `rigs`, `paths` (lista do próprio avaliador, `unevaluated_sections`); `rigBind`, `reveal`, `surface`, `normalMap`, `sceneDepth`, `relief` por nó (também num nó `splat`: nomeados, nunca ignorados). `null` conta como não declarado, como no importador. Mapas de dados não variam no tempo (por isso o avaliador temporal não os lista), mas mudam o aspecto do plano: um plano que os ignorasse não seria o quadro |
| Splat | **Desde F048.** Um nó `splat` com asset `spz` vira uma entrada de `splats` (`node_id`, `asset_id`, `after_layers`; ordem do documento), o recurso entra em `resources` (`PlanResource::is_splat`) e o plano exige `Capability::Splats`. A matriz de mundo e a câmera são as do avaliador. `after_layers` (recalculado pelo validador) diz quantos planos são pintados antes do splat: o primeiro plano estritamente mais perto da câmera que a origem do nó o cobre; a ordem entre splats da mesma fatia é a do Spark por câmera. Luz, clima, rigs e mapas de dados continuam recusados por `unsupported`. A chave `splats` só existe no JSON quando há splat: planos sem splat mantêm os bytes (golden), e um runtime sem a capacidade recusa o plano pelo nome |
| Campos não interpretados | Chave desconhecida (`UnknownField`) e seção declarada e mantida crua (`narration`, `outline`, `metadata`, `contactPlaneId`, `frameAudit`: `DeferredSection`) vão para `uninterpreted`, sem bloquear. **Escopo:** chaves do topo + definição de cena, instância, shot, clipe, trilhas, passes e assets que o frame usa; o plano de um frame não muda porque **outra** cena do bundle ganhou um campo |
| Ordem de pintura | A de `Bundle::paint_order` (passe de composição > `renderOrder` > 0; empate = ordem de documento). `layers` copia as entradas para que a ordem seja conferível só pelo plano. Limite herdado: o desempate por profundidade da câmera do Three.js **não** é modelado |
| Transparência | Do schema: `alpha` do asset da camada (`straight`/`opaque`) e `alphaMode` do `renderProfile`. Não há opacidade por nó no schema; nenhuma foi inventada |
| Determinismo | Função pura do bundle e do frame inteiro: a ordem das perguntas não importa; o mesmo texto de entrada dá o mesmo JSON. Chaves JSON saem na ordem do mapa do `serde_json` |
| Erros do frame | O erro do avaliador (`PlanError::Frame(TemporalError)`, mesmo `code`) vem **antes** dos domínios, na ordem de verificações da origem |

### Invariantes (`validate`)

Ids de nós únicos e resolvidos (pai ≠ si mesmo, sem ciclo); nó `imagePlane` com asset e tamanho
positivo, `group` sem asset, `splat` com asset; `splats` igual aos nós `splat` na ordem do documento, cada um com o recurso `spz` do próprio asset (`/splats`, `/splats/N/assetId`); uma camada por plano de imagem, de trás para frente
(ordem efetiva ascendente, empate pela ordem dos nós) e coerente com o nó e com o recurso;
recursos ordenados, sem repetição, usados, `png` (planos) ou `spz` (splats), caminho relativo seguro, `used_by` exato;
`requires` igual ao derivado do conteúdo; `unsupported` ordenado, com nós existentes; `uninterpreted` ordenado; números finitos; saída com dimensões positivas e taxa
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
  (mapas de dados, cujos assets o plano **não** lista como requeridos). Os `spz` dos splats são listados (F048).
- Teste de integração `scene_plan_resources.rs` executado (BL063): em 4 bundles e 4 frames por bundle, todo
  `required_asset_ids()` é conhecido do manifesto do mesmo bundle.

## CLI headless, janela e UI

`scene.plan {scene,frame|time,partial?,gesture?}` retorna `{plan,complete}` pelo parser compartilhado e aplica os gates estruturais/temporais. É uma consulta sem histórico. O candidato do gesto deve estar vivo e pertencer à mesma cena. F041 consome o resultado no monitor e no headless.

## Evidências e status

| Superfície | Estado |
|---|---|
| Lib | **V** no subconjunto documentado — 29 testes públicos em `crates/core/tests/scene_plan.rs` e o doctest passaram (`./scripts/cargo test -p tessel-editor-core --locked --test scene_plan`; `--doc scene_plan`) |
| Qualidade | fmt e clippy da lib limpos com `-D warnings`; gate workspace local passou (ver [relatório](../migration/round-hijkl-report.md)) |
| Contra a origem | Nenhuma comparação nova. O plano é comparado com `evaluate_frame` (que as suítes F033/F035 comparam com a origem); a ordem de pintura vem de `paint_order` (BB01/BB02) |
| CLI headless / socket / UI | **V no corpus** para consulta; monitor **Parcial** em F041 (aparência/ponteiro pendentes) |

Limites: a biblioteca de plano não é um render nem faz I/O (consumidor F041, recursos F040); domínios listados acima são recusados; cada nó leva duas matrizes de 16 números no JSON (o tamanho por frame é o custo dominante da exportação, ver abaixo).

### BL064 — medição (R6)

Release (`cargo test --release`), 3 repetições do processo × 5 passadas × 120 frames amostrados por cena, mediana; árvore pristina (main `914c830`) contra a árvore com `PlanTimeline`; sintéticas sem mídia privada (`crates/core/tests/scene_plan_performance_support`). Reproduzir: `python3 scripts/scene-plan-performance/run.py --tree . --base <árvore pristina>` (ver o [relatório](../migration/next-r6-report.md) para a tabela completa e as faixas).

| Cena (nós) | `build` (µs) pristino → direto depois | `PlanTimeline` quente (µs) | Ganho do plano | `to_json_string` (µs) | JSON/frame | Laço de exportação (plano + JSON) |
|---|---|---|---|---|---|---|
| small (11) | 18,1 → 19,2 | 5,4 | 3,6× | 86 | 8,3 KiB | 1,14× |
| medium (111) | 147,6 → 148,2 | 42,5 | 3,5× | 754 | 65,4 KiB | 1,17× |
| large (1051) | 1394 → 1502 | 374 | 4,0× | 6892 | 598 KiB | 1,09× |
| channel-heavy (25) | 43,7 → 45,3 | 15,0 | 3,0× | 193 | 20,7 KiB | 1,24× |
| wide-flat (4001) | 5561 → 5626 | 1108 | 5,1× | 27647 | 2223 KiB | 1,14× |
| camera-heavy (6; 10 000 movimentos) | 100,7 → 104,0 | 3,7 | 28× | 49 | 4,9 KiB | 2,7× |

Leitura honesta:

- **O ganho no plano é real e medido** (3–5× em cenas com nós; 28× com muita câmera), porque o custo de `build` é montagem independente do frame + `validate`, e isso agora é feito uma vez por *shot*. Refazer só `NodeTimeline::new`/`CameraTimeline::new` por frame (a hipótese original) vale ~3% do `build`, exceto em cena com milhares de movimentos de câmera (~90%).
- **No laço de exportação o ganho é pequeno (1,1–1,2×)**, exceto em cena de câmera pesada (2,7×): `to_json_string` custa ~4–5× o `build` e domina. O próximo ganho vem do escritor JSON, não do plano (proposta de BL à parte; escritor direto, byte a byte igual, não implementado aqui).
- O caminho direto `ScenePlan::build_with` não regrediu: diferenças dentro do ruído da máquina (faixas sobrepostas; ±10%, com um ponto isolado de +40% em `medium` que não se repete numa segunda medição).
- Tamanho: as matrizes dos nós (`nodes.matrices`) são ~19% (small) a ~27% (medium, large, wide-flat) do JSON; uma edição inteira de `large` são ~4,1 GiB (4205 MiB) de JSON. Reduzir exige mudar o formato (`PLAN_VERSION`): fora desta entrega.
- Memória: 20 000 frames seguidos, direto e pela linha do tempo: RSS de pico igual dentro de 1 MiB (4,8–7,1 MiB); sem crescimento por frame.
- Descartado: um índice mais leve em `NodeFrame` (~10% do caminho quente, cirurgia em struct pública).

Testes (R6): `crates/core/tests/scene_plan_performance.rs` (equivalência com o corpus 55+47 em 4 ordens × 2 modos, linha do tempo nova por frame, sintéticas, trocas de geração, instância quebrada, ordem de erro, threads, memória limitada), `scene_plan_performance_golden.rs` (saída do planejador fixada na main pristina por perfil `debug`/`release`: a main sem alteração já difere 1 ulp entre perfis), unit test em `scene_plan/timeline.rs`, e `scripts/scene-plan-performance/run.spec.py`.

## Adapter integrado de consulta

`scene.plan {scene,frame|time,partial?,gesture?}` executa SceneCommand::Plan pelo mesmo parser da lib, JSONL e socket. Aplica admissão estrutural/temporal antes do plano. `partial:false` é o padrão. `gesture` usa candidato vivo do mesmo projeto/cena, com erro tipado em invalidação, sem histórico. O teste de operação em `scene_flow` deixou de ser ignorado; 60 checks do plano passaram no processo headless. Consumo real: [F041](scene-renderer.md).

### Gate integrado e ordem de chaves JSON

A execução do workspace unifica `serde_json/preserve_order` vindo de GPUI. O golden da R6 originalmente passava isolado e falhava no workspace apenas pela ordem das chaves. O comparador agora ordena objetos recursivamente antes do digest, mantendo valores, números, arrays e erros exatos. Os digests de referência não foram regenerados nem as tolerâncias afrouxadas.
