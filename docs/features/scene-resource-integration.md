# F040 — Recursos da cena dirigida no editor: carga, geração, pacote, restauração e intercâmbio (BB06)

## Status

Lib **Parcial** (testada com arquivos sintéticos; ver "Evidências"). CLI headless **Parcial** (as
operações existem no `MediaHost`; `tessel-media` as expõe pelo mesmo `headless::execute_ext`).
UI **P** (sem controle; apresentação fora desta frente). **Nenhuma prova de pixels**: não há
renderer neste crate, então só se prova dependência e arquivo.

Sobre a [F038](scene-resources.md): nenhuma regra de contenção, hash, cabeçalho, limite, staging,
rollback ou cancelamento foi duplicada. `directed_resources::{preflight, materialize*}` continua
sendo a única implementação; esta feature a liga ao host, ao pacote e ao intercâmbio e implementa BL080 no subconjunto testado; BL079 permanece parcial (lease cooperativo, sem defesa contra escritor hostil).

## Objetivo e limites

1. **Base explícita e transportável.** Uma `ResourceBase` é sempre um caminho **absoluto**
   (`base_not_absolute` caso contrário; nunca o cwd). A base **gerenciada** de uma cena é
   `<workspace>/scenes/<sceneId>/`: derivada do workspace e do ID, portanto persiste ao salvar,
   reabrir, mover ou renomear o workspace inteiro, sem nenhum estado novo no documento. Uma base
   explícita (`base`, `source`, `scene_bases`) vale só para a operação que a recebeu e nunca é
   gravada.
2. **`assetId` identifica o recurso.** Carga/preflight entrega, por `assetId`, o caminho
   aprovado (canônico, sob a base), o `contentHash` do bundle (SHA-256 já conferido), o
   `revisionId`, a **geração** e uma `cache_key` com escopo (projeto, geração, cena, revisão do
   bundle, chave, hash). Duas cenas com o mesmo `relativePath` e conteúdos distintos nunca
   compartilham chave.
3. **Geração.** `SceneResourceSession` é dona do que foi aprovado. Troca de projeto, recarga,
   perda de contexto, troca de asset ou pedido explícito sobem a geração e esquecem tudo. O
   resultado de um job iniciado numa geração antiga **não instala** (nem parcialmente).
4. **Orçamento de bytes** (padrão 2 GiB, `with_budget`): uma cena que não cabe é recusada inteira.
5. **Pacote/restauração.** O pacote carrega `scenes/<id>/<relativePath>` para tudo que a F038
   transporta, verificado contra o hash do bundle. Só é `complete` quando nada que uma cena
   nomeia ficou de fora. A restauração prova, para cada arquivo, que o pacote o lista com o
   hash que o bundle declara; sem isso (ou sem o arquivo) o pacote "completo" é recusado.
6. **Intercâmbio.** `project.import_scenes` traz cenas de outro projeto/pacote: ID livre
   (comparado sem caixa), pasta própria por cena, bundle anexado byte a byte (as referências são
   internas ao bundle), um único passo de desfazer.

Limites declarados (nada abaixo é prometido):

- Domínios não transportados pela F038 (mapas, grades `f32`, profundidade/relevo, narração,
  seções preservadas: BL077) **permanecem `partial/unsupported`**: o pacote continua incompleto
  (`not_packaged`, razão `preserved_not_transported`) e só sai com `allow_partial`.
- BL079: o *lease* é **cooperativo** (`flock` advisory na raiz) mais checagem de contenção por
  arquivo antes e depois de publicar. **Não** fecha um escritor hostil do mesmo usuário trocando
  um pai por link entre duas operações de caminho; isso exige E/S ancorada em diretório
  (`openat`), que não existe na std e pediria dependência (`libc`/`rustix`) → patch de `Cargo`
  do coordenador.
- Identidade de arquivo por inode só existe em Unix; Windows e normalização Unicode NFC/NFD
  continuam em BL078. O PNG é validado só no cabeçalho (F038).
- Um import cujo resultado nunca é aplicado (job concluído mas projeto mudou) deixa
  `scenes/<id>/` órfão no workspace; não é apagado automaticamente (pode ser reaproveitado por
  `adopt`).
- `package --replace` remove o pacote antigo antes do `rename`; uma falha entre os dois deixa o
  destino ausente (comportamento anterior, fora do escopo do BL080 de cenas; observação).

## Contrato do core / biblioteca

Módulo `tessel_editor_media::scene_resources` (o core não mudou):

| API | Contrato |
|---|---|
| `ResourceBase::{explicit, managed}` / `root` / `kind` / `exists` | base absoluta; gerenciada = `<workspace>/scenes/<id>` |
| `check(scene, base, options, token, progress) -> PreflightReport` | só leitura; todos os problemas |
| `load(&LoadRequest{scene, scene_id, project_id, generation, base}, ...) -> ResolvedResourceMap` | erros `manifest_blocked`, `resources_invalid`, `root_unavailable`, `cancelled` |
| `adopt(scene, id, workspace, &source_base, options, token, progress, on_published)` | copia para a base gerenciada; refaz = relink; falha remove as pastas que criou |
| `SceneResourceSession` | `observe_project`, `invalidate(Invalidation)`, `install`, `release`, `get`, `status`, `with_budget` |
| `trait ResourceResolver` | `resolve(scene, &ResourceKey)` e `resolve_asset(scene, "assetId") -> Result<ApprovedResource, ResolveError>` |
| `ResolveError` | `scene_resources_not_loaded`, `resource_unknown`, `resource_not_carried`, `stale_generation`, `resource_budget_exceeded` |
| `package::{PackageInputs, preflight_with, create_with}` | aditivo: `PackageOptions`, `preflight` e `create` não mudaram de assinatura |
| `directed_resources::{RootLease, LEASE_FILE, materialize_observed, materialize_leased, Published}` | BL079/BL080 |
| `scene_resources::import::{plan, prepare, apply}` | intercâmbio de cenas |

### Costura para o renderer (frente M)

`tessel-editor-render` já depende de `tessel-editor-media`, então **não há dependência circular**:
o renderer guarda um `Arc<dyn ResourceResolver>` (ou um clone de `SceneResourceSession`, obtido
com `MediaHost::scene_resources()`) e nunca recebe caminhos soltos nem lê o cwd.

```rust
let session = host.scene_resources();                   // clone barato (Arc)
let approved = session.resolve_asset("street", "backdrop")?;
// approved.path          caminho canônico aprovado, sob a base da cena
// approved.content_hash  sha256 do bundle (já conferido no load)
// approved.revision_id   revisionId do bundle
// approved.generation    geração em que foi aprovado
// approved.cache_key     use como chave de textura/cache (escopo completo)
```

Regras para M: (1) `resolve` a cada upload e **descarte** o que tiver `generation` menor que
`session.generation()`; (2) `ResolveError::Stale`/`SceneNotLoaded` significa "peça
`scene.resources.resolve` e repita", nunca "use o último caminho"; (3) perda de contexto →
`scene.resources.invalidate {reason:"context_loss"}`; (4) ao liberar uma cena, `release`.
`scene.detach` já libera sozinho. O host chama `observe_project` a cada requisição e em `poll`.

## CLI headless

Todas síncronas por padrão (`wait:true`) ou jobs (`wait:false`/host interativo); erros de job
chegam como `job_failed` com o código original em `details.job_code`.

```json
{"op":"scene.resources.status"}
{"op":"scene.resources.preflight","scene":"street","base":"/abs/folder"}
{"op":"scene.resources.resolve","scene":"street"}
{"op":"scene.resources.adopt","scene":"street","source":"/abs/where/it/was/authored"}
{"op":"scene.resources.release","scene":"street"}
{"op":"scene.resources.invalidate","reason":"context_loss"}
{"op":"project.import_scenes","source":"/abs/p.tessel.json","scenes":["street"],"scene_bases":{"street":"/abs/folder"},"dry_run":true}
{"op":"project.package_preflight","path":"/abs/out.tesselpack","scene_bases":{"street":"/abs/folder"}}
{"op":"project.package","path":"/abs/out.tesselpack","scene_bases":{"street":"/abs/folder"},"allow_partial":false}
```

- `base` ausente → base gerenciada. `reason` ∈ `reload`, `context_loss`, `asset_swap`,
  `requested`, `project_switch`.
- `resolve` só instala se o projeto e a revisão do bundle ainda forem os da partida; senão
  `stale_result` e nada muda. `preflight`, `adopt` e `import_scenes dry_run` não instalam nada.
- Falha deixa tudo como estava: sem pasta nova, sem arquivo perdido, sessão intacta, cenas do
  projeto intactas. Cancelar publica no máximo os arquivos já visíveis e os remove (BL080).

## UI e CLI da janela

N/A nesta frente (sem controle novo). Equivalente semântico: as operações acima via
`./scripts/ui`/CLI quando a frente de interface as expuser.

## BL080 / BL079

- **BL080.** `publish` checa o token **antes de cada arquivo**; `on_published` informa cada
  arquivo já visível (não passa por `Progress`, para não alterar o contrato existente). Ao cancelar
  ou falhar, o `Journal` desfaz **somente os links criados por aquela execução** (um arquivo que o
  destino já tinha fica); as pastas criadas por `adopt` também saem. Teste determinístico: o
  cancelamento parte do próprio `on_published`, sem corrida de threads.
- **BL079.** `RootLease` toma a raiz exclusivamente (`.tessel-staging-lock`, removido ao soltar;
  nome reservado que `ResourcePath` já rejeita). Segunda operação → `root_busy` com
  `details.holder` (pid), sem escrever. Antes e depois de cada `hard_link` o pai/destino
  canônico é conferido contra a raiz (`destination_escaped`).

## Evidências e status

Comandos (todos pelo helper `cargo-serial.py`):
`test -p tessel-editor-media --locked`, `clippy -p tessel-editor-media --all-targets --locked -- -D warnings`,
`fmt -p tessel-editor-media -- --check`.

| Superfície | Arquivo | O que prova |
|---|---|---|
| lib (publicação) | `crates/media/tests/scene_resource_publication.rs` (4) | cancelar após o 1º arquivo desfaz só os próprios; raiz reutilizável; `root_busy` sem escrita; lease liberado após falha |
| lib + headless (host) | `crates/media/tests/scene_resource_integration.rs` (18) | base relativa recusada; preflight só leitura; adopt/resolve com hash e geração; reuso; origem quebrada sem mutação; relink; cancel/reuso; mapa de geração velha não instala; invalidate/release/detach/troca de projeto; orçamento; arquivo alterado; salvar/reabrir/mover workspace; pacote Directory+Archive restaurado sem origem com hash do bundle; pacote a partir de base autorizada; arquivo ausente (e `allow_partial`); pacote adulterado recusado; import com colisão de ID e de caminho + desfazer; import quebrado sem mutação |
| lib (unidade) | `scene_resources::*` e `directed_resources::lease` | bases, colisão de caixa, sessão/orçamento, lease |
| regressão | `scene_project_guards`, `directed_resources*`, `package`, `interchange`, `import` | as guardas de pacote incompleto continuam valendo para o que não é transportado |

Sem prova (não executado): Windows, escritor hostil concorrente, disco cheio real, janela/UI,
pixels/renderer, FFmpeg real além da suíte existente do crate.

## Consumidor integrado e prova de restauração

F041 consome `SceneResourceSession` por `ResourceResolver`; não resolve caminhos nem confia só no assetId. O harness integrado usa o projeto restaurado para avaliar e renderizar sem a origem, comparando sondas de pixel independentes. Comandos de carga continuam jobs na janela; job concluído precisa estar aplicado antes do monitor. Sem diálogo de anexar/carregar dedicado ainda (BL086).
