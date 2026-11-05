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
| `ready.capabilities.directed_scene` | `{ layer_kind, plan_kind: "tessel-scene-plan", plan_versions: [1], capabilities: [8 nomes de Capability::as_str], resource_identity: ["generation","asset_id","revision_id","content_hash"], resource_kinds: ["still"] }`. Os 8 nomes são exatamente `Capability::ALL` (teste) |
| `asset.load` → `asset.scene` | `{ asset_id, revision_id, content_hash, generation }`. Só para `kind: "still"`; vincula o **slot** (o `asset.id` do runtime) à identidade |
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
| `resource_mismatch` | revisão/hash difere do plano, tamanho difere, ou não é `still` |
| `resource_stale` | geração mais velha que a em uso, ou slot de outra identidade/geração |
| `resource_budget` | slots/bytes além do orçamento (host: 32 slots, 512 MiB RGBA; runtime: 64 e 768 MiB) |
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
- **Só imagens estáticas** (`still`/PNG nos testes); vídeo, splat, luz, clima, rigs, efeitos continuam recusados
  como domínio não avaliado.
- **O hash do conteúdo não é verificado aqui.** O runtime compara o que o mapa declara com o que o plano exige;
  provar que os bytes têm aquele hash é do adaptador que materializa/verifica (N/`media`).
- **Perda de contexto**: provada no runtime (testes Node, que forçam `WEBGL_lose_context`); pelo bridge não há
  mensagem para provocá-la, então do lado Rust só a lógica de reenvio foi provada, com transporte roteirizado.
- **Troca de recurso no meio do frame** é coberta pela checagem existente (`used`/`isCurrent`), sem teste
  determinístico próprio.
- **Concorrência**: `node --test` com concorrência padrão falhou **uma** vez o teste de splat (`splat.test.mjs`);
  ele passa sozinho e a suíte inteira passa serializada (`--test-concurrency=1`). Não investiguei a causa.
- **Não medido**: custo por frame, tamanho do plano e memória GPU (BL064).
- Pixels contra o runtime da origem (BL018/BL071) e UI desktop: não provados.

## Como reproduzir

```text
pnpm --dir runtime run build
cd runtime && python3 /tmp/m-gpu-lock.py node --test --test-concurrency=1 test/*.test.mjs   # lock fcntl em /tmp/tessel-scene-gpu.lock
cd .. && python3 ~/.cache/tessel-editor/cargo-serial.py test -p tessel-editor-render --test scene_seam --locked
TESSEL_REQUIRE_RUNTIME=1 python3 /tmp/m-gpu-lock.py python3 ~/.cache/tessel-editor/cargo-serial.py \
  test -p tessel-editor-render --test scene_frames --locked -- --test-threads=1
```

`/tmp/m-gpu-lock.py` só aplica `fcntl.flock` em `/tmp/tessel-scene-gpu.lock` ao comando inteiro.

## Integração M/N/O/P

`scene.plan {scene,frame|time,gesture?,partial?}` usa o parser/core compartilhado, aplica os gates e retorna `{plan,complete}`. `partial` é diagnóstico; os renderers recusam plano incompleto.
`scene.render {scene,frame,width?,height?,out?,probes?}` existe na sessão JSONL de `tessel-render` e no socket. Dimensões e probes reutilizam a validação de saída do core.

- Headless: `SceneHost` reúne MediaHost e SceneRenderer. `resources_root` opcional pede verificação de arquivos pelo MediaHost antes do frame. Sem raiz explícita, exige mapa previamente carregado.
- Desktop: carregar por `scene.resources.resolve {scene,base}` (job), aguardar/aplicar o resultado. `scene.render` e o monitor só usam o mapa aprovado, sem hash/IO pesado no thread gráfico.
- `scene_operation::approved_map` converte ResourceResolver para SceneResourceMap. O plano confere revisão/hash; o media verifica os bytes.
- Monitor: `SceneSlots` e mensagens idênticas ao headless, IDs separados, confirmação de assets, seek interativo coalescido, resposta explícita após frame.done, cancelamento no reset e invalidação na perda de contexto. A apresentação da cena é local à janela; não altera histórico/documento.
- Voltar a Sequência/Fonte encerra o preview dirigido. Projeto substituído cancela pedidos; reload reconstrói os slots.

Provas finais e limites em [round-mnop-report.md](../migration/round-mnop-report.md). Export dirigido BL008 ainda não implementado.
