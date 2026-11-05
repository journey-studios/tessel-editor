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
- BL089/BL096/BL097 (ver "Limpeza de resultados cancelados, falhos ou descartados"): import e
  adopt só removem o que eles próprios criaram, e dois resultados que compartilham arquivos
  (mesma pasta de cena) respeitam um ao outro, em qualquer ordem de aceitar e descartar. O
  compartilhamento é **por processo**: dois processos do editor sobre o mesmo workspace não se
  enxergam (item proposto no relatório de integração). Um arquivo reescrito no lugar por outro programa também fica, mas a
  prova é por conteúdo: uma reescrita com os **mesmos bytes** é indistinguível do arquivo
  publicado e sai.
- BL095: um `panic` dentro de `materialize` desfaz os links já publicados (`ArmedJournal`), sob o
  lease do chamador.
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
| `scene_resources::import::{plan, prepare, prepare_observed, apply}` | intercâmbio de cenas; `prepare_observed` também informa cada arquivo publicado (cancelamento determinístico) |
| `scene_resources::cleanup::{Created, Provisional, CleanupReport}` | BL089: registro do que uma operação criou, o dono que remove ao cair e o relatório do que ficou |
| `scene_resources::{AdoptionTarget, PendingAdoption}` | BL089: resultado de um adopt que só vale se projeto e cena continuam os mesmos |

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
- Um resultado que só chega depois (`job.commit`/`poll`) e já não serve (outro projeto, o mesmo
  projeto aberto de outro workspace, cena destacada ou com outra revisão do bundle) responde
  `stale_result` e remove os arquivos que a operação copiou (BL089).

## Limpeza de resultados cancelados, falhos ou descartados (BL089)

Auditoria dos caminhos em que arquivos copiados para `<workspace>/scenes/` ficam sem dono, e o
que cada um faz agora. Regra única: **remove-se só o que a própria operação criou**, nunca uma
pasta anterior, um arquivo reaproveitado, um arquivo do usuário nem um arquivo que já não é o
que foi publicado.

| Caminho | Antes | Agora |
|---|---|---|
| `adopt` cancelado/falho durante a cópia | `Journal` desfaz os links; pastas criadas saem | igual, e o registro `Created` confere os arquivos e as subpastas (teste: pasta e arquivo do usuário preservados) |
| `panic` dentro de `materialize` (BL095) | links já publicados ficavam (o `Journal` não tinha `Drop`) | `ArmedJournal` desfaz ao desenrolar a pilha, e o dono do `adopt` remove as pastas |
| duas preparações/adoções para o mesmo destino (BL096) | descartar a primeira apagava o arquivo que a segunda reaproveitou | registro de reivindicações: só o **último** a sair remove (ver "Arquivos compartilhados") |
| arquivo reescrito no lugar com o mesmo tamanho (BL097) | tamanho + dispositivo/inode igualavam e o arquivo do usuário era apagado | confere o SHA-256 e a metadata antes/depois; muda → `kept`; ilegível → `kept` |
| `prepare` falha/cancela na 2ª cena | `remove_created` removia as raízes criadas | o destrutor do resultado remove os arquivos copiados pelas cenas anteriores, inclusive em pasta que já existia |
| `prepare` cancelado depois do último arquivo | o job terminava `succeeded` e o resultado era aplicado | `prepare` confere o token ao fim: `cancelled` e limpeza |
| job `succeeded` cujo resultado nunca é aplicado (cancel tardio, host descartado, `panic`) | pasta órfã | `Drop` do `PreparedSceneImport` remove |
| `apply` com outro projeto | descartava as pastas (PR #3) | igual |
| `apply` com o mesmo projeto aberto de **outro workspace** (cópia da pasta, mesmo ID) | anexava cenas cujos arquivos não estão na base gerenciada derivada | `stale_result` e remoção |
| `apply` recusado (ID tomado, limite de cenas, orçamento) | `remove_created` | destrutor (mesmo efeito, sem caminho a esquecer) |
| `scene.resources.adopt` cujo projeto, workspace ou cena mudou durante o job | sem verificação; pasta ficava | `PendingAdoption::apply` → `stale_result` (com o motivo) e remoção; integrado em `host.rs` (`Pending::SceneAdopt`) |
| host descartado com um adopt sem `job.commit` | pasta órfã | o `Drop` do `PendingAdoption` remove |

Peças (`scene_resources::cleanup`):

- `Created` é o **registro**: arquivos publicados, cada um com a identidade (tamanho, hash
  SHA-256 e, em Unix, dispositivo/inode) que tinha ao publicar, mais as pastas criadas em ordem
  e o que ele só **reaproveita** de outro resultado. Vem de `Adoption::created`, calculado pelo
  próprio `adopt` (o `create_dir` que teve sucesso e as subpastas que não existiam antes), não de
  um `exists()` anterior que outra operação pode invalidar.
- `Provisional` é o **dono** enquanto o resultado não foi aceito: remove ao cair; `release()`
  entrega ao projeto; `discard()` remove e devolve o `CleanupReport`.
- **Nenhum `Clone`** em `PreparedSceneImport`, `Adoption`, `PendingAdoption`, `Created` nem
  `Provisional`: uma cópia cairia depois e apagaria a pasta do consumidor que ficou com o original.
- A remoção toma o mesmo `RootLease` (cooperativo) da publicação, com espera limitada. Raiz
  ocupada → os arquivos ficam e o relatório diz (`busy_roots`, `kept`). Um arquivo trocado por
  outro programa (identidade ou conteúdo diferente) ou uma pasta ainda com conteúdo também ficam,
  nunca `remove_dir_all`. Isto **não** é defesa contra escritor hostil do mesmo usuário (BL079).
- `tracked_under(root)` (pública, para testes e diagnóstico) lista as reivindicações vivas sob
  uma pasta; um teste de vazamento exige que fique vazia ao fim de cada cenário.

### Arquivos compartilhados (BL096)

Cada registro é um *titular* no registro de reivindicações do processo (`scene_resources/claims.rs`),
com chave por **caminho canônico** (`ws/../ws` e `ws` são o mesmo dono). Regras:

- `adopt` registra o que **criou** (arquivo/pasta) e o que **reaproveitou** (`Outcome::Reused`), o
  segundo ainda sob o `RootLease`, antes de soltá-lo: ninguém pode descartar nesse intervalo.
- Descartar um registro remove um arquivo só se ele é o **último** titular de algo que um
  resultado pendente criou. Se outro titular ainda existe, o arquivo fica e o relatório lista em
  `shared` (também em `to_json`); o último a sair leva tudo, inclusive as pastas.
- **Aceitar** é soltar o registro (`Provisional::release` seguido do `drop` do `Created`, ou o
  `drop` de um `Created` avulso): as entradas somem do registro e os arquivos passam a ser do
  projeto. Daí em diante nenhum descarte, em nenhuma ordem, os toca.
- Uma raiz que já existia antes (sem entrada) nunca é removida por ninguém. `absorb` (import de
  várias cenas) transfere as reivindicações ao registro que absorve.
- Ordem de travas: lease da raiz → mutex do registro. `create_folders` só toma o mutex.
- Fora da garantia: **outro processo**. O registro vive na memória; dois editores sobre o mesmo
  workspace só têm o `RootLease` (que serializa, não conta referências); item proposto em `docs/migration/integration-cleanup-report.md`.

### Identidade por conteúdo (BL097)

`Stamp` guarda tamanho, dispositivo/inode (Unix) e o **SHA-256** que `materialize` já verificou
ao publicar. Antes de apagar, `remove_file` (1) lê a metadata (inclui mtime/ctime), (2) confere
identidade, (3) calcula o SHA-256 do arquivo, (4) lê a metadata de novo e exige que seja a mesma
e (5) só então remove. Falha de leitura, metadata diferente ou hash diferente → `kept` (falha
fechada). Limites: reescrita com os mesmos bytes sai (nada do escritor se perde); entre o passo 4
e o 5 um escritor ainda ganharia (cooperativo, não é defesa contra escritor hostil; BL079).

Custo: uma leitura SHA-256 de cada arquivo que será de fato removido, nunca de resultado aceito.
Medição (`measure_discarding_a_result_with_a_large_file`, ignorado por padrão, build de **debug**,
este disco): descartar um resultado com um arquivo de 256 MiB levou 8,7 s (~29 MiB/s). Em
release o SHA-256 é bem mais rápido, mas **não foi medido**. Em consequência, descartar um
resultado grande num destrutor da thread de UI bloqueia; a saída é `job.discard` (BL094,
pendente).

### Host (`MediaHost`)

`scene_adopt` captura o `AdoptionTarget` (projeto, workspace, cena, revisão do bundle) **antes**
de iniciar o job; o resultado vira `Pending::SceneAdopt(Box<PendingAdoption>)`. `apply` (via
`poll` ou `job.commit`) aceita só se tudo continua igual; senão responde `stale_result` com o
motivo (projeto, workspace, cena ou revisão) e remove. Host descartado = `Drop` remove. A API
`SceneExport` da R1 em `host.rs` não foi tocada.

Fora do que se faz: revisão do projeto que muda por edições não relacionadas não invalida um
import (ele só acrescenta cenas); `undo` do import desfaz o projeto, não os arquivos (a pasta fica
para um novo anexo); um `job` finalizado e nunca confirmado continua exigindo `job.commit` ou o
descarte do host (`job.discard`, BL094, segue pendente).

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
| lib + headless (host) | `crates/media/tests/scene_resource_integration.rs` (19) | base relativa recusada; preflight só leitura; adopt/resolve com hash e geração; reuso; origem quebrada sem mutação; relink; cancel/reuso; mapa de geração velha não instala; invalidate/release/detach/troca de projeto; orçamento; arquivo alterado; salvar/reabrir/mover workspace; pacote Directory+Archive restaurado sem origem com hash do bundle; pacote a partir de base autorizada; arquivo ausente (e `allow_partial`); pacote adulterado recusado; import com colisão de ID e de caminho + desfazer; import quebrado sem mutação |
| lib + host (BL089/BL096) | `crates/media/tests/scene_import_cleanup.rs` (29) | adopt registra só o que criou; adopt em pasta existente e cancelado preservam o do usuário; dono remove ao cair e `release` mantém; import cancelado na 2ª cena, logo após o último arquivo, falho na 2ª cena e descartado sem apply deixam o workspace idêntico; apply mantém e `undo` não toca arquivos; projeto trocado e workspace trocado tornam stale e limpam; attach recusado tarde limpa e preserva a cena do usuário; destino pré-existente e arquivo reaproveitado ficam; arquivo trocado fica; raiz ocupada fica; host descartado sem commit limpa; `job.commit` após `project.open` de cópia é stale; `PendingAdoption` aceito/projeto trocado/workspace trocado/cena destacada/revisão trocada/não aplicado; import e adopt para a mesma cena nas duas ordens, dois imports (`street-2`) e nenhuma reivindicação vazada |
| lib (BL096/BL097/BL095) | `crates/media/tests/scene_cleanup_ownership.rs` (23 + 1 medição ignorada) | aceitar/descartar em ordens opostas, três titulares, raiz pré-existente, grafia `ws/../ws`, outra cena, relatório `shared`, arquivo acrescentado na pasta do primeiro, segundo cancelado, estresse com threads; reescrita no lugar de mesmo tamanho, mtime restaurado, 1 byte, acréscimo, mesmos bytes, arquivo ilegível; `panic` em `materialize` e em `adopt` |
| host (R5) | `crates/media/tests/scene_adopt_host.rs` (13) | adopt imediato como antes; `job.commit` no mesmo projeto; projeto trocado, cópia aberta de outro workspace, cena destacada e bundle trocado são stale e limpam; host descartado limpa; pasta existente preservada; dois jobs para a mesma cena; janela interativa aplica no `poll` e reporta falha |
| lib (unidade) | `scene_resources::claims` (4) | registro de reivindicações |
| lib (unidade) | `scene_resources::*` e `directed_resources::lease` | bases, colisão de caixa, sessão/orçamento, lease |
| regressão | `scene_project_guards`, `directed_resources*`, `package`, `interchange`, `import` | as guardas de pacote incompleto continuam valendo para o que não é transportado |

Sem prova (não executado): Windows, escritor hostil concorrente, **dois processos** sobre o mesmo
workspace, disco cheio real, janela/UI, pixels/renderer, FFmpeg real além da suíte existente do
crate, o custo do hash em build de release e a limpeza com raiz ocupada além de 200 ms.

## Consumidor integrado e prova de restauração

F041 consome `SceneResourceSession` por `ResourceResolver`; não resolve caminhos nem confia só no assetId. O harness integrado usa o projeto restaurado para avaliar e renderizar sem a origem, comparando sondas de pixel independentes. Comandos de carga continuam jobs na janela; job concluído precisa estar aplicado antes do monitor. Sem diálogo de anexar/carregar dedicado ainda (BL086).

## Import descartado após troca de projeto (revisão PR #3; ampliado por BL089)

PreparedSceneImport captura o ID de destino. A função compartilhada `scene_resources::import::apply` recusa outro projeto com stale_result antes da transação e descarta somente as pastas criadas pela preparação. MediaHost usa essa mesma função em poll/job.commit, portanto a guarda vale também pela biblioteca direta. Uma regressão conclui o job, troca o projeto e tenta commit, conferindo cena/histórico intactos e limpeza dos arquivos próprios.
