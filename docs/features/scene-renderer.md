# F041 — Renderer de cena dirigida (BL007 / BL063)

## Status

| Superfície | Estado | Evidência |
|---|---|---|
| Runtime Three.js (consumidor do `ScenePlan`) | **V** | 8 testes Node puros + 10 testes em Chrome real (SwiftShader); suíte completa do runtime 79/79 (ver "Limites": um teste de splat falhou uma vez sob concorrência padrão) |
| Lib Rust (`tessel_editor_render`: mapa de recursos, livro de slots, mensagens, `SceneRenderer`) | **V** | 22 testes sem navegador (`scene_seam`) |
| CLI headless (`tessel-scene-render`) | **V** | 1 teste de ponta a ponta em Chrome real (`scene_frames`), incluindo falha por frame, código de saída e uso inválido |
| Frames reais contra o oráculo independente | **V** | 6 testes em Chrome real (`scene_frames`); 12 frames fora de ordem, 12 sondas por frame, diferença máxima de canal **0** (tolerância 6) |
| Socket do app | **V no corpus** | `scene.plan` e `scene.render`; 40 checks de pixels no WKWebView, com recursos verificados por F040 |
| UI / monitor desktop | **Parcial** | Botão Ver no monitor e `monitor.show_scene`; WebView provada por pixels. Inspeção visual e ponteiro físico pendentes |
| Ciclo de vida do monitor dirigido no WKWebView (BL092) | **V no corpus** | 13 testes de host (`scene_lifecycle_tests`) + prova nativa `scripts/scene-lifecycle-proof.py`: cenários S0–S10 (13 blocos), 218 checks, 0 falhas em 2 execuções seguidas (Apple GPU). Ver "Ciclo de vida no WKWebView" |
| Exportação dirigida (BL008) | **P** | **não** é alegada. O caminho de export original segue compatível (`export` 2, `frames` 7, `video` 5 testes passam) |

Nada aqui prova pixels contra o **runtime da origem** (BL018/BL071); o oráculo é o modelo
independente do corpus `scripts/directed-scene-proof` (cálculo próprio em Python, validado pela frente P
contra a origem).

## O que é

O core decide o frame (`ScenePlan::build`, [F037](scene-plan.md)); um adaptador decide qual arquivo local
aprovado guarda cada recurso; o runtime **só executa** o que está no plano: câmera, matrizes de mundo,
planos de imagem, alpha e ordem. O JS não recalcula nada de domínio (nenhuma animação, hierarquia, easing
ou ordenação por profundidade).

```
ScenePlan (core) ─┐
                  ├─ SceneSlots::prepare ─► asset.release* · asset.load* ─► (asset.state ready) ─► frame.render
SceneResourceMap ─┘                         [layer directed_scene]                                 └► frame.done
```

O mesmo conjunto de mensagens serve ao headless (`ProcessTransport` → worker Node → Chrome) e ao monitor
desktop (WebView): muda só o transporte.

## Esquema (aditivo, bridge v1 inalterada)

Nenhuma mensagem nova e nenhuma mudança de sentido: `decode_event` do core continua recusando tipos
desconhecidos, então o transporte reaproveita `asset.load`, `asset.release`, `frame.render`, `frame.cancel`.

| Onde | Acréscimo |
|---|---|
| `ready.capabilities.features` | `"directed_scene"`. Um runtime que não lista não recebe o layer (o host recusa antes: `backend_unavailable` com o comando de rebuild) |
| `ready.capabilities.directed_scene` | `{ layer_kind, plan_kind: "tessel-scene-plan", plan_versions: [1], capabilities: [9 nomes de Capability::as_str], resource_identity: ["generation","asset_id","revision_id","content_hash"], resource_kinds: ["still","splat"] }`. Os 9 nomes (com `splats`, F048) são exatamente `Capability::ALL` (teste) |
| `asset.load` → `asset.scene` | `{ asset_id, revision_id, content_hash, generation }`. Para `kind: "still"` (PNG) e `kind: "splat"` (`meta.format: "spz"`, F048); vincula o **slot** (o `asset.id` do runtime) à identidade |
| `frame.render` → `plan.layers[]` | `kind: "directed_scene"`, `scene_plan` (o `ScenePlan::to_json()` inteiro) e `resources: [{ asset_id, slot, revision_id, content_hash, generation }]` |
| `ready.capabilities.layers` | **Não** muda: lista os tipos do plano comum; o gate é `features` |
| Telemetria | `frame.done.source.directed_scene_layers`; `cache`/`debug.retention`: `scene_resources`, `scene_generation` |

A camada é composta como as demais (alvo offscreen com MSAA 4, `flipV: true`, `fit: false`), então o canvas do
monitor mostra o que o headless escreve em PNG. Forma de transporte da câmera: `{x,y,z}` (`Vec3::to_json`); as
únicas matrizes são arrays de 16 (coluna-maior). Um plano com vetor em array é recusado com
`invalid_plan` e o ponteiro (`/camera/position`).

### Identidade do recurso

Identidade = `(generation, assetId, revisionId, contentHash)`. **Nunca** `assetId` sozinho, nem como chave de
cache.

- `generation` é a geração de projeto do host: cresce quando um projeto é aberto, trocado ou religado.
  Um recurso de geração menor que a já usada é `resource_stale`; ao subir a geração, o runtime libera tudo
  da anterior.
- O mesmo `assetId` com outra revisão ou hash é **outro recurso, em outro slot** (convive até o orçamento obrigar a
  trocar). Provado: `the_same_asset_id_with_another_revision_gets_another_slot` e o teste de troca em Chrome.
- Os slots são `scene-res-0…`: conjunto pequeno e fixo. WHY: o `Id` do core é internado e nunca liberado; ids
  vindos de bundle (não confiáveis) crescendo por projeto/revisão vazariam memória. Teste: 200 gerações usam
  ≤ 32 ids distintos.

### Erros tipados

Todos com `code` estável, sem desenho parcial:

| Código | Quando |
|---|---|
| `invalid_plan` | layer sem plano/`kind` errado/forma inválida (`details.pointer`, ex.: `/camera/fovDeg`, `/nodes/0/worldMatrix`, `/output/width`) |
| `unsupported_plan_version` | `planVersion` ≠ 1 (checado antes de ler outros campos) |
| `unsupported` | plano parcial (`complete: false`; `details.domains`) ou `requires` que o runtime não anuncia (`details.missing`) |
| `resource_unresolved` | sem entrada no mapa, slot não carregado, arquivo ausente ou raiz ilegível |
| `resource_mismatch` | revisão/hash difere do plano, tamanho difere, ou o tipo não é o do plano (`still` para PNG; `splat` `.spz` para splat — o host recusa antes um arquivo que não é SPZ) |
| `resource_stale` | geração mais velha que a em uso, ou slot de outra identidade/geração |
| `resource_budget` | slots/bytes além do orçamento (host: 32 slots, 512 MiB; runtime: 64 e 768 MiB). Um splat conta os bytes do arquivo mais 16 bytes por splat, uma vez por recurso, nos dois lados. Além disso o quadro soma os splats de cada nó `splat` (arquivo compartilhado conta por nó) e é recusado acima de 8 Mi (`details.frame_splats`; host `DEFAULT_FRAME_SPLATS`, runtime `LIMITS.frameSplats`), BL125 |
| `resource_path_invalid` | caminho relativo no mapa ou link que sai da raiz aprovada |
| `bundle_invalid` | importação ou gates (`edit_join_issues` + `temporal_issues`) do bundle (`details.issues`) |
| `cancelled` | frame substituído por outro mais novo (último ganha) ou `frame.cancel` |
| `context_lost` | o contexto gráfico caiu no meio; o host reenvia (até 2×) quando volta |
| `backend_unavailable`, `timeout` | runtime ausente/morto/sem a feature; sem resposta no prazo |

## Contrato da biblioteca (`tessel_editor_render`)

| API | Contrato |
|---|---|
| `SceneResourceMap::new(generation)` · `insert` · `with` | Mapa `assetId → ResolvedResource { revision_id, content_hash, path }`. `path` absoluto (senão `resource_path_invalid`) |
| `SceneResourceMap::from_root(generation, &plan, root)` | Resolve cada `relativePath` do plano sob uma **raiz aprovada**; recusa link que sai dela. Revisão/hash vêm do plano: **a raiz é confiável**, o arquivo não é re-hasheado aqui |
| `SceneResourceMap::from_json` | `{ generation, resources: { id: { revisionId, contentHash, path } } }`. Este é o **seam para N**: um mapa explícito de recursos locais aprovados com geração, sem esperar adaptador |
| `scene::import_gated(text)` | `DirectedScene::import_str` + `edit_join_issues` + `temporal_issues` (o `finalizeBundle` da origem). Um renderer não desenha bundle que o core recusaria |
| `SceneSlots::prepare(&plan, &map) -> Prepared` | Decide `releases`, `loads`, `waiting` e monta o layer. **Recusa não altera o livro** (tudo é validado antes). LRU entre slots que o frame não precisa; orçamento de contagem e bytes |
| `SceneSlots::mark_ready / mark_failed / release_all / reset` | Confirmação do runtime, carga recusada (esquecida: a próxima requisição recarrega), projeto fechado, runtime morto (a geração é mantida, então mapa velho segue recusado) |
| `scene::frame_render_message(&SceneFrameMessage)` | O `frame.render` com **um** layer `directed_scene`. `inspect: false` é o caminho barato do monitor |
| `SceneRenderer::submit / wait / cancel / render / release_all / shutdown` | Anfitrião síncrono sobre `BridgeBackend`. `render` reenvia o frame em `context_lost`; `cancelled` e demais falhas voltam como são |
| `FrameOptions { size, png, probes, inspect }` | Tamanho padrão = saída do plano. `png` só é escrito pelo worker Node |

### CLI headless

```text
tessel-scene-render --bundle bundle.json [--root DIR | --resources map.json] [--generation N]
    [--frame N]... [--size WxH] [--probe X,Y]... [--out DIR] [--gl swiftshader|gpu]
```

Renderiza os frames **na ordem dada** (seek não é sequência). Uma linha JSON por frame:
`{"frame","ok":true,"hash","mean","probes","png","width","height","render_ms","total_ms","cache","source"}` ou
`{"frame","ok":false,"error":{code,message,details?}}`. Saída 1 se algo falhou, 2 em uso inválido. Frame fora
dos shots falha sozinho; os demais renderizam.

## Seam para o monitor desktop (O / coordenador)

O monitor não precisa de `SceneRenderer` (ele é síncrono); usa as mesmas peças puras, com o seu transporte:

1. no seek, `SceneSlots::prepare(&plan, &map)`; enviar `prepared.releases` e depois `prepared.loads[].message`
   (`asset.load`) pela WebView;
2. ao receber `asset.state` (`ready`/`failed`) → `mark_ready(id)` / `mark_failed(id)`;
3. quando todos os `prepared.pending_slots()` estiverem `ready`, enviar
   `frame_render_message(&SceneFrameMessage { inspect: false, png: None, .. })` com
   `revision` crescente: o runtime mostra só o frame mais novo (os outros respondem `cancelled`);
4. em `context.lost` não é preciso recarregar nada: o runtime mantém as imagens decodificadas e reconstrói a GPU,
   o host só reenvia o frame após `context.restored`;
5. ao fechar/trocar projeto: `release_all()`, ou subir a `generation` (o runtime libera a anterior); se a WebView
   recarregar, `reset()`.

`ProcessTransport` (headless) e a WebView (`host.rs`, que registra os caminhos de `asset.load` para servir
arquivos) carregam as mesmas mensagens. A integração posterior foi exercitada no WKWebView pelo harness `socket_render_pixels` (40/40).

## Ciclo de vida no WKWebView (BL092)

O monitor dirigido (`DirectedViewport`, em `scene_monitor_host.rs`) responde a cada pedido explícito
(`scene.render`) com **um** resultado tipado, e o monitor mostra sempre o frame atual com o hash do seu render original.

| Evento | O que acontece | Resposta ao pedido explícito |
|---|---|---|
| Monitor oculto | só os frames do próprio monitor ficam parados; o pedido explícito desenha mesmo oculto | frame normal |
| Ocultar com frame em voo / reabrir | o frame termina; ao reabrir, um seek parado desenha mesmo com a mesma chave | frame normal |
| Runtime ainda subindo / indisponível | recusa na hora (não estaciona) | `backend_unavailable` |
| Perda de contexto | cancela o frame em voo, **mantém o livro de slots** (o runtime guarda os bitmaps, só as texturas morrem), suspende até `context.restored` e redesenha o monitor sem recarregar nada | `context_lost`; pedido novo durante a perda também recebe `context_lost` |
| Respostas velhas (`frame.done`/`frame.failed` de id que não é o ativo) | recusadas e contadas em `directed.refused.stale_frames` | nenhuma (já respondido) |
| `monitor.reload` (página some) | responde o pedido em voo, esquece o livro (`reset`) e descarta toda mensagem da página velha até `page.ready` | `cancelled` ("The viewport is reloading") |
| Timeout / erro de cena (a página continua viva) | responde o pedido, **libera** cada slot na página (`asset.release`) e redesenha | `timeout` / erro do core |
| Troca de projeto / fechar o monitor | `release_all` dos slots, `loads`/`shown` zerados; frame em voo respondido | `cancelled` |
| Troca de recurso com frame em voo | o frame é refeito com a identidade nova; um `asset.state failed cancelled` de load substituído, ou `resource_stale` de outra geração/recurso, é ignorado e contado em `refused.stale_asset_states`; uma falha real continua falhando o frame | frame ou erro tipado |

**Por que liberar numa e esquecer na outra:** os nomes dos slots (`scene-res-N`) saem do LRU. Esquecer o livro numa
página viva deixa as imagens no runtime e o próximo pedido usa outros nomes: a prova nativa mostrou o runtime
chegando a 21 recursos contra 3 no livro após seis perdas de contexto, até o limite de 64 recusar tudo. A correção separa
os três casos em `reset_directed_runtime_for` (libera), `forget_directed_runtime` (página sumiu) e `suspend_directed`
(contexto perdido, livro mantido).

`directed` no relatório do viewport (com `TESSEL_VIEWPORT_INSPECT=1`; a chave `directed_scene` segue igual):
`scene`, `suspended`, `active{id,sent,waiting,explicit}`, `queued{explicit}`, `slots{occupied,generation,tracked_loads}`,
`refused{stale_frames,stale_asset_states}`, `context_losses`, `shown{request_id,edit_frame}`.

### Evidência

- Host (sem WebView): `cargo test -p tessel-editor scene_lifecycle`, 13 testes com página roteirizada. Os testes de
  vazamento (`repeated_context_losses_...`, `a_timeout_releases_...`) e o do restore falham com o comportamento antigo
  (mutação verificada: 3 falham).
- Nativo: `python3 scripts/directed-scene-proof/with-gpu-lock.py python3 scripts/scene-lifecycle-proof.py --binary <editor> --out <json>`.
  Uma instância própria (HOME/socket curtos), só operações semânticas do socket, página de teste que **não altera o
  `runtime.js`**: embrulha `TesselRuntime.receive` para `debug.loseContext()`/`restoreContext()` num frame escolhido e atrasa
  `fetch('/asset?p=...')` de cópias do corpus (`slow-a/`, `slow-b/`) para manter cargas em voo. Cenários S0–S10: perda no
  meio do frame, monitor oculto com pedido explícito, ocultar/reabrir com pedido pendente, `monitor.reload` com pedido
  esperando assets lentos, troca de recurso (sem frame e com frame em voo), troca de projeto com loads substituídos,
  perda seguida de troca de projeto, perda nunca restaurada, perda seguida de reload e seis perdas seguidas. Cada
  cenário confere o frame atual contra o modelo independente e contra o **hash do render explícito original**, que toda
  resposta pendente chega tipada (nunca pendurada) e que o livro, o runtime (`scene_resources`) e o orçamento (≤ 32 slots) concordam.

### Limites do BL092

- A perda de contexto é **forçada** (`WEBGL_lose_context`); reset real de driver não foi provado.
- A janela de um `asset.state ready` atrasado de um load substituído **para o mesmo slot** foi fechada por BL102/BL102b
  (seção abaixo; janela e headless) para runtimes que anunciam `asset.identity`; com um runtime antigo continua valendo a heurística por slot
  e por detalhes do erro, sem garantia contra esse `ready`.
- Medido só em macOS / Apple GPU com o corpus de 3 recursos; orçamento com muitos recursos não foi exercitado nativamente.

## Identidade de carga em `asset.load`/`asset.state` (BL102)

Antes, o host só sabia o **nome do slot** (`scene-res-N`) de um `asset.state`. O nome é reaproveitado (LRU, troca de
projeto, timeout) e a página responde as cargas na sua ordem: um `ready` velho que chegasse depois de o slot ser dado a
uma carga nova era indistinguível do `ready` novo e destravava o frame antes da hora (desenhava com a imagem errada ou
sem ela). Agora a identidade do pedido viaja e volta. Bridge continua v1; tudo é aditivo e negociado.

| Peça | Contrato |
|---|---|
| Capability | `ready.capabilities.features` lista `asset.identity`; `capabilities.asset_identity = { echo: ["request","scene"], scene_fields: [...] }` |
| Pedido | `asset.load.asset.request`: inteiro não negativo opcional, um por mensagem enviada, nunca reutilizado pelo host (nem após reload da página). O host só o envia se o runtime anunciou `asset.identity` |
| Eco | todo `asset.state` (`loading`, `ready`, `failed`, inclusive o `cancelled` de uma carga que um release ultrapassou) repete `request` e, se o descritor tinha `scene` completo, `scene{asset_id,revision_id,content_hash,generation}`. Só esses campos; nunca o caminho. Descritor sem identidade recebe as mensagens de sempre |
| `request` inválido | `asset.state failed` com `error.code = invalid_request`, sem eco; a carga não acontece |
| Regra do host | `bridge::LoadIdentity::answered_by` (core, puro): só vale a resposta cujo `request` é o da carga **atual** do slot e cuja `scene` volta inteira e igual. Sem eco, eco parcial ou de outro pedido: recusado e contado em `directed.refused.stale_asset_states`, sem tocar o livro. Com o eco certo, uma falha é real (inclusive `cancelled`) e falha o frame |
| Runtime antigo / sem a feature | nenhum campo novo é enviado; vale a heurística anterior (`superseded_load`) |

O mesmo `request` não se repete nem para a mesma imagem no mesmo slot: um `ready` do primeiro load de `red` no slot não
confirma o segundo load de `red` (teste `the_same_picture_loaded_again_into_a_slot_is_a_different_request`).
`render.status`/`monitor.viewport` ganham `directed.asset_identity` (negociado) e `directed.slots.identified_loads`.

Superfícies: **lib** (`stamp_asset_load`, `LoadIdentity`, `FEATURE_ASSET_IDENTITY` em `tessel_editor_core::bridge`, 6 testes
puros) · **runtime** (`runtime/src/asset-identity.js`, 6 testes Node + 2 testes em Chrome) · **host dirigido/janela**
(`scene_monitor_host.rs`, 4 testes novos de `scene_lifecycle_tests.rs`, mutação verificada: sem a comparação 2 falham; prova
nativa S0/S6). Evidência executada nesta rodada: `cargo test -p tessel-editor scene_lifecycle` 17/17; `cargo test -p tessel-editor-core
--lib bridge` 11/11; `node --test` do runtime 93/93 em Chrome real (SwiftShader); prova nativa
`scene-lifecycle-proof.py` completa S0–S10 com **221/221 checks** num WKWebView real (o host negocia `asset.identity`, 3/3 cargas
carimbadas, `refused.stale_asset_states` ≥ 1 na troca de projeto). **Limite da prova nativa:** a página real não
injeta um `ready` velho de propósito; a corrida do mesmo slot é provada no host com página roteirizada e no runtime
real em Chrome (ordem de eco), não reproduzida deterministicamente no WKWebView.

### Headless (`SceneRenderer`, BL102b)

`BackendEvent::Asset` ganhou o campo aditivo `echo: Option<Value>` (só `request` e/ou `scene`, como recebidos; `None` de
runtime sem eco; nunca caminho). `bridge::decode_event` o preenche; a regra de julgamento continua uma só,
`LoadIdentity::answered_by`. O `SceneRenderer` (lib, `tessel-scene-render`, export dirigido) agora faz o mesmo que o host da janela:

| Peça | Contrato |
|---|---|
| Negociação | `ensure_ready` lê `features` do `ready`: `asset.identity` presente liga `asset_identity()`; ausente (runtime antigo) não envia campo novo e vale a heurística por slot, sem contar recusas |
| Carimbo | cada `asset.load` recebe `asset.request` (1, 2, 3…), contador do renderer **nunca reiniciado**, nem em `shutdown`/reinício do runtime: um eco tardio de um processo anterior nunca casa com carga nova |
| Regra | `ready`/`failed` só altera o livro de slots se repetir a identidade da carga que **detém o slot agora**; sem eco, eco parcial, de outro `request` ou de outra imagem: recusado e contado em `refused_asset_states()`, sem destravar nem falhar o frame |
| Falha real | só com o eco certo (inclusive `cancelled`), e então a carga é esquecida para ser tentada de novo |
| Escopo do estado | `loads` limpo por `release`, `release_all`, `shutdown`, `Closed`; `identified` reavaliado no próximo handshake |

Efeito observável: um runtime que negociou e não ecoa (ou ecoa outro pedido) não destrava o frame: a espera termina em `timeout`
(`the pictures`) com `refused_asset_states() > 0`, em vez de desenhar com a imagem errada. Superfícies: **lib** (6 testes novos em
`crates/render/tests/scene_seam.rs` com transporte roteirizado, mais 2 de `decode_event` no core; mutação verificada: sem a conferência 3
falham, incluindo o ready velho no mesmo slot, com `SceneSlots::new(3, ..)` para que o slot seja realmente reaproveitado) · **CLI JSONL**
(`tessel-scene-render` usa o mesmo `SceneRenderer`: prova real em `scene_frames.rs`, que exige `asset_identity()` negociado com o runtime
do repositório) · **UI**: N/A, a janela usa o host dirigido (acima). Limite: com runtime antigo continua sem garantia contra o ready velho.

## Retenção e invalidação (sem regressão do preview comum)

Reaproveita o que o runtime já tem: texturas do `AssetStore`, materiais/alvos no `LruCache`, a checagem
`used`/`isCurrent` de troca de recurso no meio do frame, `dropGpu`/`dropRetained` na perda de contexto,
cancelamento do último-ganha. Provado em Chrome real: frame repetido não cria textura (`texture_created: 0`);
hash idêntico com retenção ligada e desligada; um runtime frio desenha o mesmo hash de um aquecido; seek
fora de ordem repete o mesmo hash; troca de geração libera as texturas antigas; perda de contexto no meio do
frame e entre frames volta ao mesmo hash. Os 79 testes do runtime (preview, splat, proxy, ciclo de vida) passam
em execução serializada.

## Limites declarados

- **Sem mipmaps**: texturas lineares sem mipmap; reduzir muito a imagem pode ficar mais serrilhado que na origem.
- **Fundo opaco preto**: o alpha de saída não é carregado (`output.alphaMode` é registrado no plano, mas o frame
  sai opaco). Alpha de saída é trabalho futuro de BL008.
- **Sem desempate por profundidade**: a ordem do plano é a ordem de pintura, de propósito.
- **Sem cache de resultado de frame dirigido**: cada frame é redesenhado (medir em BL064 antes de otimizar).
- **Imagens estáticas e splats SPZ** (F048); vídeo, luz, clima, rigs, efeitos continuam recusados
  como domínio não avaliado.
- **O hash do conteúdo não é verificado aqui.** O runtime compara o que o mapa declara com o que o plano exige;
  provar que os bytes têm aquele hash é do adaptador que materializa/verifica (N/`media`).
- **Perda de contexto**: provada no runtime (testes Node, que forçam `WEBGL_lose_context`) e no WKWebView nativo
  (BL092, perda **forçada** por `debug.loseContext()` numa página de teste; ver "Ciclo de vida no WKWebView").
  Reset real de driver/GPU não foi provado; pelo bridge não há mensagem para provocá-lo.
- **Troca de recurso no meio do frame** é coberta pela checagem existente (`used`/`isCurrent`), sem teste
  determinístico próprio.
- **Concorrência**: `node --test` com concorrência padrão falhou **uma** vez o teste de splat (`splat.test.mjs`);
  ele passa sozinho e a suíte inteira passa serializada (`--test-concurrency=1`). Não investiguei a causa.
- **Não medido**: custo por frame, tamanho do plano e memória GPU (BL064).
- Pixels contra o runtime da origem (BL018/BL071) e UI desktop: não provados.

## Como reproduzir

```sh
pnpm --dir runtime run build
TESSEL_REQUIRE_RUNTIME=1 python3 scripts/directed-scene-proof/with-gpu-lock.py \
  node --test --test-concurrency=1 runtime/test/*.test.mjs
MBX_GC_MAX_TOTAL_SIZE=64GiB ./scripts/cargo test -p tessel-editor-render --test scene_seam --locked
TESSEL_REQUIRE_RUNTIME=1 MBX_GC_MAX_TOTAL_SIZE=64GiB \
  python3 scripts/directed-scene-proof/with-gpu-lock.py ./scripts/cargo \
  test -p tessel-editor-render --test scene_frames --locked -- --test-threads=1
```

O helper versionado aplica `fcntl.flock` ao comando inteiro; os comandos partem da raiz do repositório. Em sessões paralelas, coordene também o slot Cargo; o lock GPU não substitui essa coordenação.

## Integração M/N/O/P

`scene.plan {scene,frame|time,gesture?,partial?}` usa o parser/core compartilhado, aplica os gates e retorna `{plan,complete}`. `partial` é diagnóstico; os renderers recusam plano incompleto.
`scene.render {scene,frame,width?,height?,out?,probes?}` existe na sessão JSONL de `tessel-render` e no socket. Dimensões e probes reutilizam a validação de saída do core.

- Headless: `SceneHost` reúne MediaHost e SceneRenderer. `resources_root` opcional pede verificação de arquivos pelo MediaHost antes do frame. Sem raiz explícita, exige mapa previamente carregado.
- Desktop: carregar por `scene.resources.resolve {scene,base}` (job), aguardar/aplicar o resultado. `scene.render` e o monitor só usam o mapa aprovado, sem hash/IO pesado no thread gráfico.
- `scene_operation::approved_map` converte ResourceResolver para SceneResourceMap. O plano confere revisão/hash; o media verifica os bytes.
- Monitor: `SceneSlots` e mensagens idênticas ao headless, IDs separados, confirmação de assets, seek interativo coalescido, resposta explícita após frame.done, cancelamento no reset e, na perda de contexto, cancelamento do frame em voo mantendo o livro de slots (ver "Ciclo de vida no WKWebView"). A apresentação da cena é local à janela; não altera histórico/documento.
- Voltar a Sequência/Fonte encerra o preview dirigido. Projeto substituído cancela pedidos; reload reconstrói os slots.

Provas finais e limites em [round-mnop-report.md](../migration/round-mnop-report.md). Export dirigido BL008 ainda não implementado.

## Correções da revisão PR #3

`--bundle bundle.json` sem pai usa a pasta atual como raiz de recursos. O runtime revalida o número de recursos imediatamente antes do registro, além de geração e bytes; cargas concorrentes não ultrapassam sceneResourceEntries. `scene.plan` aplica admissão por referência, evitando cópia profunda do bundle por frame, com os mesmos gates.

Integração de S4 com main `2bc340d` (S3): conflitos apenas no backlog/matriz, preservando identidade negociada e snapshot. Coordenador reconferiu fmt, check/clippy workspace, core, media/export 11/11, desktop/lifecycle 17/17, render scene_frames/scene_seam/scene_export com runtime obrigatório e lock GPU, build do editor e testes JS puros de identidade 6/6. Log local `/tmp/tessel-s4-main-gates.log`; revisão CodeRabbit continua pendente.
