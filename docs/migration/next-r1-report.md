# R1 — Export dirigido real (BL008 / F042)

Branch `feat/directed-scene-next-delivery`, base `914c830`. Sem Git write (nada commitado). Contrato frozen 3:
`scene.export {scene, preset, destination, from, frames, wait?}`, só aditivo. Contrato e evidências por
superfície: [`docs/features/scene-export.md`](../features/scene-export.md).

## O que entrou

| Camada | Arquivos (todos sob ownership de R1) | O que faz |
|---|---|---|
| Core (sem gpui; `cargo tree -p tessel-editor-core` sem `gpui`) | `crates/core/src/scene_export/{mod,spec,schedule,snapshot}.rs` + `pub mod scene_export;` em `lib.rs` | `SceneExportSpec`, `SceneExportSchedule` (rate racional, `[from, from+frames)`), `AudioPlan`, `SceneExportSnapshot` (cena commitada congelada, admissão, assets exigidos, `plan`/`plan_with`/`timeline`) |
| Mídia | `crates/media/src/scene_encoder/{mod,render,args,job}.rs` + `pub mod scene_encoder;` em `lib.rs`; `crates/media/src/host.rs` | `SceneFrameRenderer`/`SceneRendererFactory`, `FrozenResources`, `SceneExport::freeze`, `preflight`, `run` (FFmpeg `image2pipe` → libx264 BT.709, staging, `export::verify`, publicação por rename); `scene.export` e `scene.export.preflight` no `MediaHost`; `with_scene_renderer` |
| Render | `crates/render/src/scene_export/mod.rs` + `pub mod` em `lib.rs`; `crates/render/src/scene_host.rs`; `crates/render/src/bin/tessel-render.rs` (só doc) | `ProcessSceneRenderer` sobre o `SceneRenderer` de F041 (Chrome próprio por export, mesmo `inspect:true` do `scene.render`); `SceneHost` entrega a fábrica ao `MediaHost` e aceita `resources_root` |
| Testes | `crates/core/tests/scene_export.rs` (14), `crates/media/tests/scene_export.rs` (7), `crates/render/tests/scene_export.rs` (10), unit `scene_encoder::args` (1) | ver "Testes" |
| Docs | `docs/features/scene-export.md`, este relatório, `docs/migration/next-r1-ui.patch` | |

Nada em `export.rs`, `preset.rs`, `ops.rs`, `state`, `headless.rs`, `jobs.rs`, `scene_operation.rs`, matriz,
backlog ou índices foi editado. `JobKind::VideoExport` já existia e é reaproveitado.

### Seams / decisões

- **Mesmo plano, mesmos recursos, mesmo renderer.** Cada frame é `PlanTimeline::build` (R6, agora adotado, ver
  abaixo) sobre o frame de edição que o cronograma define; o renderer recebe um `ResourceResolver` que é o
  `ResolvedResourceMap` aprovado e congelado; o desenho passa pelo `SceneRenderer` de F041. Por isso o `hash`
  de runtime de cada frame do export é igual ao do `scene.render` equivalente (testado).
- **`scene.export` não passa por `ops::parse_op`.** Entra pela `Extension` (`MediaHost`/`SceneHost`), como
  `scene.render` e `scene.resources.*`. Consequência: o core `ops.rs` não conhece a operação (coerente com o
  contrato aditivo); quem monta o host precisa chamar `with_scene_renderer`.
- **Áudio:** recusa explícita (`unsupported`) ou omissão explícita (`audio:false` → `audio_omitted`). Nunca um
  stream silencioso. O corpus base (sem áudio) exporta sem aviso além de `no_audio_sources`.
- **Cancelamento:** por frame; staging é um `Drop`, então erro, cancelamento e pânico removem tudo.
- **Adoção de `PlanTimeline` (BL064, pós-R6):** `SceneExportSnapshot::timeline()` + `plan_with(&timeline, i)`;
  o laço do job cria uma timeline por job (empresta `snapshot.scene()`; não está dentro do snapshot) e
  `collect_required_assets` usa uma por snapshot. `plan(i)` continua existindo para pergunta única (e
  `scene.plan` **não** adota, como pede R6). Validação do frame (`edit_frame`, intervalo) continua só no
  snapshot. Teste: `a_prepared_timeline_plans_exactly_what_a_direct_build_does` (30 frames com conversão 24→30
  cruzando o corte, e o mesmo erro além do fim). **Ganho no export real não medido** (nenhum benchmark pesado
  foi rodado, a pedido): o custo por frame do export de corpus é dominado pelo renderer (~210–290 ms/frame a
  1080p em SwiftShader); como o plano é serializado em JSON por frame para o runtime, o ganho esperado é o da
  linha "plano + JSON" de R6 (~1,1–1,2×), não o do plano puro.

## Testes (executados no meu checkout compartilhado, sob `with-gpu-lock.py` quando há Chrome)

Executados, com resultado: ver "Resultado da última execução" ao fim. Comandos (sempre via `cargo-serial.py`):

```text
test -p tessel-editor-core  --locked --test scene_export
test -p tessel-editor-media --locked --test scene_export   (+ --lib scene_encoder)
TESSEL_REQUIRE_RUNTIME=1 with-gpu-lock.py cargo-serial.py test -p tessel-editor-render --locked --test scene_export -- --test-threads=1
clippy -p tessel-editor-{core,media,render} --lib --bins --test scene_export --locked -- -D warnings
tree -p tessel-editor-core --locked   # sem gpui
```

Cobertura pedida: export curto real **1080p** e **UHD 4K** com ffprobe e pixels (oráculo independente e hashes
de `scene.render`), conversão de rate, inválidos sem mutação, **cancel sem arquivo parcial**, recursos
congelados, destino existente, recurso sumido, áudio e domínio recusados, e a mesma operação pela sessão JSONL de
`tessel-render` (CLI headless).

**Não executados por R1:** gate global (`cargo test --workspace`, `pnpm test`), clippy/fmt do restante dos
crates, testes de `desktop` (sem alterações), socket/UI do app (a UI é patch), benchmark do export, export
longo/estresse, GPU real (tudo em SwiftShader), `scripts/directed-scene-export-proof` (R10) contra a
implementação final.

Observação de ambiente: durante a entrega o checkout compartilhado ficou sem compilar por WIP de outra frente
(`temporal/frame.rs`, lifetimes). Para não esperar, provei o meu código em uma **cópia somente-leitura**
(`/tmp/te-r1-ws`, com os arquivos de terceiros quebrados voltados para `HEAD`; removida ao fim) e,
quando o checkout voltou a compilar, **reexecutei no checkout real**. Um `rustfmt` direto formatou só os meus arquivos.

## Conflitos com `next-r5-host.patch` (não aplicado, não editado)

Nenhum hunk sobrepõe os meus. O patch toca (a) o `use crate::scene_resources::{…}` (linhas ~57–60), (b) o enum
`Pending` (+ `SceneAdopt`), (c) o corpo de `scene_adopt` e (d) o `match` de `apply`. Eu toquei, em
`media/src/host.rs`: uma linha de `use` separada (`scene_encoder`), uma linha de `use` do core, um campo +
`with_scene_renderer`, duas linhas no `match` de `handle` (`scene.export*`), `scene_export`,
`scene_export_preflight`, `freeze_scene_export` e a lista `ours` de `Extension::execute`. O contexto de
cada hunk de R5 permanece intacto (o `use crate::scene_resources::{` e as linhas vizinhas não foram alterados).
Interação semântica: `scene.export` só **lê** o `SceneResourceSession` (congela o mapa aprovado) e não adota,
copia nem remove arquivos; portanto o limite BL096 (dois jobs de adoção no mesmo destino) não o afeta, e
`PendingAdoption` não participa de export. Reconciliar com `git apply --3way` deve ser mecânico.

## Patches e pendências para o coordenador

1. **UI (R2/coordenador):** `docs/migration/next-r1-ui.patch` (diálogo `flow.export_scene`, relatório via
   `scene.export.preflight`, fábrica do renderer no host da janela). Sem isso o app responde `backend_unavailable`.
2. **Deduplicação com `export.rs`** (dono: coordenador): hoje `scene_encoder` duplica `encode_args`, `encode_error`,
   `png_size`, `kill`, `as_disk_full` e `read_tail`. Tornar esses itens `pub(crate)` em `export.rs` e trocar as
   cópias por chamadas (sem mudar comportamento; o teste unitário de `args` e os 7 testes de mídia cobrem).
3. **`scene_operation::approved_map`** generalizado para `&dyn ResourceResolver` (hoje aceita o tipo concreto),
   para o monitor/exportador compartilharem a montagem do mapa de recursos do runtime (`render::scene_export::resource_map`
   é a cópia de R1).
4. **`ops.rs`**: nada obrigatório. Se o coordenador quiser `scene.export` no parser compartilhado (para `describe`
   de operações), é preciso um `Command` que o core não executa; não recomendado (o export é por definição um
   efeito de host).
5. **Matriz** (`docs/features/README.md`): adicionar
   `| F042 | [Exportação de cena dirigida](scene-export.md) | V no corpus | V no corpus | V no corpus (lib+CLI) | P | scene.export[.preflight], scene_export, scene_encoder; UI e socket pendentes (next-r1-ui.patch); áudio dirigido recusado |`
   e, na linha F041, trocar "export dirigido BL008 pendente" por link a F042. A UI é **P** até integrar o patch.
6. **Backlog:** BL008 → "A validar" (lib e CLI com evidência real; faltam UI/socket e a aceitação independente de R10).
   BL019: acrescentar os 10+7+14 testes de F042; ainda **sem** prova de áudio dirigido. BL064: R1 adotou
   `PlanTimeline` (ganho no export real **não medido**). Novos itens sugeridos: (a) áudio dirigido no export
   (depende de BL016), (b) re-hash dos recursos no início do export se a política de integridade pedir, (c) export
   longo/estresse e memória do renderer, (d) GPU real.
7. **`scripts/directed-scene-export-proof/adapters.json`** (R10): nomes confirmados: `scene.export` com
   `destination, frames, fps {num,den,fps}, duration_seconds, output.width/height, audio, warnings, verification,
   resources`. `fps` é um objeto (`Rate::to_json`), não um número; ajustar o adaptador se ele comparar escalares.
   Erros usados: `destination_exists`, `invalid_range`, `unknown_element`, `invalid_value`/`invalid_request`,
   `unsupported` (áudio), aviso `audio_omitted` (não está na lista `audio_not_mixed_warning_codes` do adaptador).

## Limitações

- Cancelamento só entre frames; um frame de UHD em SwiftShader leva ~1 s.
- Integridade dos recursos: hash verificado na aprovação; no export só presença e tamanho (troca por arquivo do
  mesmo tamanho durante o job não é detectada).
- Áudio dirigido: recusado ou omitido, nunca mixado (depende de BL016).
- Conversão de rate repete/pula frames de edição; sem interpolação.
- Alpha de saída e mipmaps (BL090) seguem como declarados em F041 (fundo opaco).
- Provas só no corpus sintético, em SwiftShader; nada contra o runtime da origem.
- Cena sem corte curto em UHD: só 2 frames de 4K foram exportados nos testes (tempo).

## Resultado da última execução (checkout real, depois da adoção de `PlanTimeline`)

| Comando | Resultado |
|---|---|
| core `--test scene_export` | 14/14 |
| media `--test scene_export` | 7/7 (e `--lib scene_encoder` 1/1 na execução anterior à adoção; código de `args` não mudou) |
| render `--test scene_export` (Chrome real, `with-gpu-lock.py`, `TESSEL_REQUIRE_RUNTIME=1`) | 10/10 em ~52 s |
| clippy core+media+render (`--lib --bins --test scene_export -D warnings`) | limpo |
| `cargo tree -p tessel-editor-core` | 0 ocorrências de `gpui` |

Medidas desta execução (SwiftShader, uma amostra, não é benchmark): 1080p 8 frames em 3,8 s (224 ms/frame,
30 115 bytes); UHD 4K 2 frames em 3,8 s (904 ms/frame, 25 117 bytes); delta máximo de canal contra o oráculo = 1;
frames do MP4 vs PNG de `scene.render`: diferença média 0,47.
