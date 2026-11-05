# F042 — Exportação de cena dirigida (BL008)

## Objetivo e limites

`scene.export` grava um trecho de uma cena dirigida como **MP4 H.264 real**: o mesmo `ScenePlan` completo e
original de [F037](scene-plan.md), os mesmos recursos verificados de [F040](scene-resource-integration.md) e o
mesmo renderer Three.js de [F041](scene-renderer.md) que `scene.render` usa, encadeados com FFmpeg. Nada é
simulado: cada frame do arquivo foi desenhado pelo runtime (Node + Chrome + Three.js) a partir de um plano
calculado pelo core.

- **Presets, tempo e rate:** reusa `Preset` (`fullhd`, `uhd4k`, vertical, os do projeto ou um objeto inline), a
  geometria `contain/cover` e `Rate`/`Ratio` racionais do core. O intervalo é `[from, from+frames)` em frames de
  edição da cena.
- **Job único:** o export é um job `video-export` do `JobRegistry` existente (progresso por frame, cancelar,
  staging, publicação por rename). Não há segunda fila.
- **Snapshot imutável:** a cena e o mapa de recursos são congelados na chamada. Editar, destacar a cena, gesto
  vivo (preview/candidato) ou invalidar a sessão de recursos depois **não** muda um export em andamento.
- **Sem efeito no projeto:** o resultado não altera cena, timeline, histórico nem cria clipe. Não há `job.commit`.
- **Áudio:** o mix WAV declarado no bundle é gravado no MP4 (AAC, mesma duração do vídeo) e tem contrato
  próprio em [F042A](scene-export-audio.md). Cena sem áudio exporta vídeo apenas (aviso `no_audio_sources`).
  `audio:false` (ou preset sem áudio) omite o declarado com o aviso `audio_omitted`. Cena que declara
  `narration` com áudio pedido é **recusada** (`unsupported`). Nunca um stream silencioso nem um mix parcial.
- **Domínios sem avaliação** (por exemplo `lighting`) continuam sendo recusados antes de qualquer job, pela
  mesma regra de `ScenePlan::build`.

Fora do escopo desta entrega: narração/takes (BL017), alpha de saída (BL090), equivalência visual contra o
runtime da origem (BL018/BL071). A UI está integrada pelo coordenador, com prova viva ainda pendente.

## Contrato do core / biblioteca

### Core (`tessel_editor_core::scene_export`, sem gpui)

| Tipo | Papel |
|---|---|
| `SceneExportSpec::from_json(&EditorState, &Value)` | Lê `{scene, preset?, from?, frames?, audio?}`; inteiros estritos (nada de float); preset por id, objeto ou o `export_preset` do projeto |
| `SceneExportSchedule` | `[from, from+frames)`, rate da cena → rate do preset, `sample_time(i)` exato (`Ratio`), `edit_frame(bundle, i)` (igual ao `Bundle::frame_at_time`, piso), geometria do preset |
| `AudioPlan` | `NoSources`, `Omitted { declared }` ou `Mix(DirectedAudioMix)` ([F042A](scene-export-audio.md)); recusa o resto |
| `SceneExportSnapshot::capture` | Cena **commitada** (nunca candidato de gesto), `preset.validate()`, admissão (`edit_join_issues` + `temporal_issues`), cronograma, política de áudio, assets exigidos (um `ScenePlan::build` por shot tocado), plano do último frame |
| `SceneExportSnapshot::plan(i)` | `ScenePlan::build` do frame de edição do frame de saída `i`: **o mesmo** de `scene.plan` |
| `SceneExportSnapshot::timeline()` + `plan_with(&timeline, i)` | O laço do export usa uma `PlanTimeline` (R6) por job: mesmo plano, mesmo intervalo e mesmos erros que `plan(i)` (teste de equivalência), sem refazer o preparo por frame |

Taxa diferente (`24 → 30`): o frame de saída `i` mostra o frame de edição `floor(i·scene_fps/out_fps)`; frames
são repetidos ou pulados, **nunca** misturados. O aviso `frame_rate_conversion` aparece no resultado.

### Mídia (`tessel_editor_media::scene_encoder`)

- `SceneFrameRenderer` / `SceneRendererFactory`: a fronteira do renderer. Cada export constrói o seu (Chrome
  próprio), na thread do job, e o libera ao fim (`finish`) mesmo em erro.
- `FrozenResources`: o `ResolvedResourceMap` aprovado, congelado. O renderer recebe **só** isto como
  `ResourceResolver`.
- `SceneExport::freeze(snapshot, &SceneResourceSession, destino, replace)` recusa `scene_resources_not_loaded`,
  `stale_generation`, `resources_stale` ou o erro de resolução de um asset exigido.
- `preflight(&SceneExport, &Tools, max_size) -> Value` e `run(...)`: ver abaixo.
- `MediaHost::with_scene_renderer(factory)`; sem fábrica, `scene.export` responde `backend_unavailable`.

### Operações (JSONL, socket e biblioteca usam o mesmo JSON)

`scene.export {scene, preset?, destination, from?, frames?, replace?, audio?, wait?, resources_root?}`

| Campo | Notas |
|---|---|
| `scene` | obrigatório; id desconhecido → `unknown_element` |
| `preset` | id (`fullhd`, `uhd4k`, …), objeto inline (validado como preset do projeto) ou omitido → `export_preset` do projeto |
| `destination` | `.mp4`; não pode existir a menos que `replace:true` |
| `from`, `frames` | inteiros; padrão `0` e "até o fim da edição"; fora do intervalo → `invalid_range` |
| `audio` | `false` omite o áudio declarado (aviso `audio_omitted`); `true`/omitido recusa se há áudio declarado |
| `wait` | `true` (padrão no headless) devolve o resultado; `false` (ou o host interativo) devolve `{job, state, info}` |
| `resources_root` | conveniência do headless/`SceneHost`: roda `scene.resources.resolve` com essa base antes (como `scene.render`) |

`scene.export.preflight` recebe o mesmo pedido e responde `{ok, blockers[], warnings[], summary, estimate,
free_bytes, encoder}` **sem** criar job nem tocar o destino (FFmpeg/libx264, `.mp4`, destino, pasta,
`render_too_large` quando o renderer informa, recursos presentes e do tamanho aprovado, espaço em disco).

### Fluxo Rust sem UI

```rust
let mut host = SceneHost::new(MediaHost::new(), RuntimeConfig::discover()?);
headless::execute_ext(&mut state, None, Some(&mut host),
    &json!({"op":"scene.attach","id":"street","path":"render-bundle.json"}))?;
headless::execute_ext(&mut state, None, Some(&mut host),
    &json!({"op":"scene.resources.resolve","scene":"street","base":"corpus"}))?;
let result = headless::execute_ext(&mut state, None, Some(&mut host), &json!({
    "op":"scene.export","scene":"street","preset":"fullhd","from":8,"frames":8,
    "destination":"/tmp/street.mp4"}))?;
// result["verification"]["ok"], result["frame_hashes"]["list"], result["resources"], …
```

### Resultado

`action, destination, scene, preset, range {from, frames, to}, bytes, frames, fps, duration_seconds,
render {w,h}, output {w,h}, audio {included, declared}, resources [{asset, sha256, revision, bytes}],
warnings, verification (ffprobe), frame_hashes {count, first, last, digest, list?}, metrics`.

`frame_hashes.list` existe em exports de até 64 frames e traz o `hash` de runtime de cada frame: é o `hash` que
`scene.render` responde para o mesmo frame de edição e tamanho. `digest` é o SHA-256 da sequência.

### Erros relevantes

| Código | Quando | Efeito |
|---|---|---|
| `invalid_request`, `invalid_range`, `unknown_element`, `invalid_value` | pedido, cena, intervalo ou preset inválido | recusado **antes** do job; nada muda |
| `unsupported` | narração declarada sem `audio:false`; mix em formato não suportado; domínio sem avaliação | idem |
| `scene_resources_not_loaded`, `stale_generation`, `resources_stale` | recursos não aprovados/obsoletos | idem |
| `backend_unavailable` | sem renderer (sem runtime) | idem |
| `destination_exists`, `preflight_failed`, `disk_full` | falha do preflight do job | job falha; destino intocado; sem staging |
| `render_failed` | um frame falhou (com `backend_code`) | job falha; sem arquivo |
| `cancelled` | `job.cancel` | sem arquivo parcial; sem staging |
| `encode_failed`, `verification_failed` | FFmpeg ou ffprobe | sem publicação |

Falha em qualquer ponto remove o staging (`.tessel-staging-scene-export-*`) e deixa um destino pré-existente intacto;
`replace:true` troca o arquivo em um único rename e só depois da verificação.

## CLI headless

```text
./scripts/render            # tessel-render, JSONL em stdin/stdout, sem janela
{"op":"scene.attach","id":"street","path":"corpus/bundle.json"}
{"op":"scene.export","scene":"street","preset":"fullhd","from":0,"frames":4,
 "destination":"/tmp/out.mp4","resources_root":"corpus"}
{"ok":true,"result":{"action":"scene.export","frames":4,"verification":{"ok":true},…}}
```

O segundo `scene.export` para o mesmo destino responde `destination_exists` e o processo sai com 1 (linha
falha). Sem Node/Chrome, `tessel-render` responde `backend_unavailable` para `scene.export`.
Não há flag nova: é só uma operação da sessão. Lacuna: `tessel-scene-render` (binário de um frame) não exporta.

## UI e CLI da janela

O coordenador integrou o diálogo `flow.export_scene`, o relatório `scene.export.preflight` e a fábrica `ProcessSceneRenderer` no host da janela. Na aba Cena do inspector, o botão Exportar do cabeçalho abre esse diálogo; o fluxo normal continua disponível na aba Clipe e em `flow.export`.

Campos: cena, preset, primeiro frame e quantidade opcionais, destino e áudio. O áudio só é omitido ao escolher explicitamente `no`; caso contrário o core mantém sua política de recusa para cenas com áudio. O socket usa `scene.export` e acompanha o job por `job.get`, sem bloquear a janela. Todo diálogo usa o mecanismo existente que esconde a WebView.

Compilação e 190 testes desktop passaram após a integração, incluindo dois testes do pedido e erro de campo do novo diálogo. A prova nativa/socket com export real permanece pendente da R10; testes desktop não a substituem. O patch original em `next-r1-ui.patch` é histórico e não deve ser reaplicado.

## Evidências e status

| Superfície | Estado | Evidência |
|---|---|---|
| Core (cronograma, snapshot, política de áudio, sem mutação, `plan_with`≡`plan`) | **V** | `crates/core/tests/scene_export.rs`: 15/15 após revisão do coordenador |
| Pipeline FFmpeg/job sem navegador (injeção de falha) | **V** | `crates/media/tests/scene_export.rs`: 7/7 + 1 unitário (`args`). **Não** prova pixels |
| Lib + host headless com Chrome real | **V no corpus** | `crates/render/tests/scene_export.rs`: 10/10 |
| CLI headless (`tessel-render`) | **V no corpus** | mesmo arquivo: sessão JSONL exporta 4 frames 1080p, 2º pedido recusa o destino |
| Socket do app / UI | **Implementado, prova viva pendente** | Integração do coordenador: desktop 190/190, dois testes novos do diálogo; R10 valida export real |
| Áudio dirigido | **Recusado de forma explícita** | `declared_audio_and_unevaluated_domains…` |

O que cada teste de Chrome prova (SwiftShader, rodado sob `with-gpu-lock.py`):

- **1080p**: 8 frames `[8,16)` → MP4 `h264 1920x1080 yuv420p`, `bt709`/`tv`, `24/1`, 8 frames, duração `8/24`
  (ffprobe), sem stream de áudio, verificação interna `ok`; 12 sondas do oráculo independente (`expected.json`,
  ×6) nos frames de edição 11 e 12 decodificados do MP4 com diferença máxima de canal **1** (tolerância 14);
  hash de runtime dos 8 frames **igual** ao de `scene.render` para o mesmo frame/tamanho; imagem decodificada vs
  PNG de `scene.render`: diferença média **0,47** (máx. 100 em bordas de alto contraste, esperado de H.264 4:2:0);
  `resources[].sha256` iguais aos do corpus. Sem clipe na timeline, histórico inalterado, sem staging.
- **UHD 4K**: 2 frames (11, 12) → `3840x2160`, 12 sondas ×12, delta máx. **1**, hashes iguais aos de `scene.render` a 3840×2160.
- **Conversão 24→30** (preset inline 640×360, `from=40`, 24 frames de edição cruzando o corte em 48): 30 frames
  de saída a `30/1`, hash de cada frame = `scene.render` do frame de edição `40+floor(i·24/30)`; o frame 0 repete.
- **Recursos congelados**: `scene.resources.invalidate` no meio do export não o altera nem o quebra (16 frames, `succeeded`).
- **Inválidos**: cena desconhecida, `from<0`, `from=96`, `frames=0`, `from+frames>96`, preset desconhecido, preset
  inline com largura ímpar, `frames=1.5` e recursos não aprovados recusam com código tipado, sem job
  (`job.list` igual), sem arquivo, sem staging e com a cena inalterada.
- **Destino existente / extensão errada**: o job falha (`destination_exists` / `preflight_failed`), o arquivo
  existente permanece byte a byte; `replace:true` troca por um MP4 verificado.
- **Recurso sumido**: um arquivo apagado depois da aprovação bloqueia no preflight (`resource_*`), sem arquivo.
- **Cancelamento**: `job.cancel` após ≥2 frames de 96 → `cancelled`, sem MP4 e sem staging; o progresso por frame é reportado.
- **Áudio**: cena com `audio` declarado (WAV real + hash) tem o mix no arquivo (F042A); com `audio:false` exporta
  sem stream de áudio e com `audio_omitted`; `lighting` é recusado.

Tempo medido (SwiftShader, máquina de desenvolvimento, único dado de uma execução): ~210–290 ms/frame a
1080p, ~950 ms/frame a UHD (dominado pelo renderer, não pelo FFmpeg).

### Limites declarados

- **Cancelamento por frame:** um frame em desenho não é interrompido; o cancelamento vale entre frames.
- **Recursos:** os bytes foram verificados por hash quando o mapa foi aprovado (`scene.resources.resolve`); o
  export só confere presença e tamanho no início (um arquivo trocado por outro do mesmo tamanho durante o
  export não é detectado).
- **Plano por frame:** o job usa `PlanTimeline` (BL064); o ganho no export real **não foi medido** (o custo do corpus é dominado pelo renderer e o plano vira JSON por frame).
- **Rate:** frames de edição são repetidos/pulados na conversão; sem mistura nem interpolação.
- **Cena sem corte longo / 4K longo:** só exports curtos foram exercitados; não há teste de resistência a
  milhares de frames nem de memória do renderer ao longo de um export longo.
- **Não é a origem:** nada aqui compara pixels com o runtime da origem (BL018/BL071); o oráculo é o corpus
  sintético independente. A aceitação independente é de R10 (`scripts/directed-scene-export-proof`).
- Existem duplicações pequenas de `export.rs` (`encode_args`, `png_size`, `kill`, leitura de stderr) em
  `scene_encoder`, porque o arquivo é do coordenador; patch de deduplicação no relatório da R1.

### Guarda de identidade da timeline (revisão de integração)

`SceneExportSnapshot::plan_with` recusa com `stale_scene` uma timeline preparada para outra instância de cena, mesmo com bundle de conteúdo idêntico. O teste público novo falhou antes da guarda; o caminho normal de export usa a timeline do próprio snapshot. O gate de intervalo continua sendo executado antes da identidade.

### Prova independente R10

A R10 reporta 23/23 fases em `scripts/directed-scene-export-proof/results.json` (exit 0), incluindo export CLI, socket e diálogo semântico, política de áudio e restauração sem origem. Fonte e binários têm proveniência no relatório `docs/migration/next-r10-report.md`. Como houve edições após o build, repetir com a árvore final antes do aceite integrado. Não prova node:16, preflight do diálogo ou ponteiro físico.

Gate final do coordenador: [R1–R10](../migration/round-r1-r10-report.md) e [prova final](../migration/round-r1-r10-results/export.json), 23/23 fases, 570 checks, binários atuais (`older_than_sources:false`), incluindo CLI, socket e diálogo. A prova anterior da R10 permanece histórica. Header node:16, preflight do diálogo e ponteiro físico continuam sem prova específica.
