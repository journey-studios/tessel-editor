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
  compartilhamento usa o ledger persistente do workspace entre processos do editor (BL111).
  Um arquivo reescrito no lugar por outro programa também fica, mas a prova é por conteúdo: uma reescrita com os **mesmos bytes** é indistinguível do arquivo
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
{"op":"scene.resources.claims"}
{"op":"scene.resources.recover"}
{"op":"project.import_scenes","source":"/abs/p.tessel.json","scenes":["street"],"scene_bases":{"street":"/abs/folder"},"dry_run":true}
{"op":"project.package_preflight","path":"/abs/out.tesselpack","scene_bases":{"street":"/abs/folder"}}
{"op":"project.package","path":"/abs/out.tesselpack","scene_bases":{"street":"/abs/folder"},"allow_partial":false}
```

- `base` ausente → base gerenciada. `reason` ∈ `reload`, `context_loss`, `asset_swap`,
  `requested`, `project_switch`.
- `resolve` só instala se o projeto e a revisão do bundle ainda forem os da partida; senão
  `stale_result` e nada muda. `preflight`, `adopt` e `import_scenes dry_run` não instalam nada.
- Uma falha não instala o resultado na sessão nem altera cenas do projeto. O rollback tenta
  remover somente publicações próprias sob prova; falhas de prova/remoção podem manter arquivos
  e pastas. Em `claims_unavailable`, consulte `details.kept` e `details.cleanup`: um arquivo sem
  claim mantido não está ao alcance de `recover`. Cancelar tenta a mesma limpeza (BL080).
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
- `Provisional` é o **dono** enquanto o resultado não foi aceito: remove ao cair; `accept()`
  (falível, BL111.2) entrega ao projeto; `discard()` remove e devolve o `CleanupReport`.
- **Nenhum `Clone`** em `PreparedSceneImport`, `Adoption`, `PendingAdoption`, `Created` nem
  `Provisional`: uma cópia cairia depois e apagaria a pasta do consumidor que ficou com o original.
- A remoção toma o mesmo `RootLease` (cooperativo) da publicação, com espera limitada. Raiz
  ocupada → os arquivos ficam e o relatório diz (`busy_roots`, `kept`). Um arquivo trocado por
  outro programa (identidade ou conteúdo diferente) ou uma pasta ainda com conteúdo também ficam,
  nunca `remove_dir_all`. Isto **não** é defesa contra escritor hostil do mesmo usuário (BL079).
- `tracked_under(root)` (pública, para testes e diagnóstico) lista as reivindicações vivas sob
  uma pasta; um teste de vazamento exige que fique vazia ao fim de cada cenário.

### Arquivos compartilhados (BL096)

Cada registro é um *titular* no registro de reivindicações do workspace (`scene_resources/claims/`, em disco desde o BL111, ver abaixo),
com chave por **caminho canônico** (`ws/../ws` e `ws` são o mesmo dono). Regras:

- `adopt` registra o que **criou** (arquivo/pasta) e o que **reaproveitou** (`Outcome::Reused`), o
  segundo ainda sob o `RootLease`, antes de soltá-lo: ninguém pode descartar nesse intervalo.
- Descartar um registro remove um arquivo só se ele é o **último** titular de algo que um
  resultado pendente criou. Se outro titular ainda existe, o arquivo fica e o relatório lista em
  `shared` (também em `to_json`); o último a sair leva tudo, inclusive as pastas.
- **Aceitar** é `Provisional::accept`, **explícito e falível** (BL111.2): as entradas somem do
  ledger em disco e só então o registro se esvazia; os arquivos passam a ser do projeto e nenhum
  descarte, em nenhuma ordem, os toca. `Ok` quer dizer "gravado". Em `Err`
  (`claims_unavailable`, `details.phase = "accept"`) o resultado **não** foi aceito: continua
  claimado e do titular. Um `Created` avulso solto continua sendo aceito, só que na melhor
  tentativa (destrutor não relata): nenhum fluxo que altera projeto depende disso.
- Uma raiz que já existia antes (sem entrada) nunca é removida por ninguém. `absorb` (import de
  várias cenas) transfere as reivindicações ao registro que absorve, **gravando a transferência
  antes de mover as listas** (falha devolve os dois registros intactos).
- Ordem de travas: lease da raiz → trava do registro. `create_folders` só toma a trava do registro.
- Entre processos: ver "Ownership entre processos (BL111)".

### Ownership entre processos (BL111)

O registro deixou de viver só na memória: é um **ledger por workspace** em
`<workspace>/.tessel-claims/` (`scene_resources/claims/ledger.rs`), compartilhado por qualquer
editor, CLI ou job que abra o mesmo workspace.

```text
.tessel-claims/
  ledger.lock     flock exclusivo durante uma leitura-modificação-escrita
  claims.json     versão 1: owners + entries, trocado por rename atômico
  claims.journal  aceites ainda não dobrados em claims.json (BL112.7); só existe num ledger grande
  live/<token>    um por processo dono, com flock mantido enquanto o processo vive
```

- **Claim** = recurso (caminho canônico) + identidade (tamanho, dispositivo/inode e SHA-256 do
  arquivo publicado) + **titular** (token aleatório do processo + sequência do resultado). O
  token é sorteado a cada início de processo; o PID é só texto de mensagem.
- **Lease**: o processo dono mantém `flock` exclusivo em `live/<token>` enquanto tem titulares
  no ledger. O kernel solta a trava quando o processo termina, inclusive `kill -9`; vivacidade é
  "a trava está tomada", **nunca** o PID, então PID reaproveitado não faz um morto parecer vivo
  nem o contrário. Se não dá para saber (erro que não seja "ocupado"), o titular conta como vivo.
- Quem abre o ledger primeiro faz o **reap** dos titulares cujo lease sumiu. Uma entrada sem
  titular é **órfã**: um resultado pendente que não existe mais. Ela fica até alguém decidir:
  um novo resultado que reaproveita o arquivo entra como titular (e decide se fica ou sai) ou
  `recover` remove após a mesma prova de conteúdo de qualquer cleanup (BL097).
- Um processo nunca remove arquivo aceito (sem entrada), nem entrada que outro titular vivo usa,
  nem arquivo que não é mais o publicado (identidade/hash diferente → `kept` com motivo).
- Falha fechada, e **quando** falhou decide o que se pode afirmar (`details.phase`):
  - `before_changes`: ledger ilegível, de versão mais nova, ou caminho do workspace que não é
    UTF-8 → `adopt` recusa com `claims_unavailable` **antes de copiar**; "nada foi alterado"
    é verdadeiro.
  - `after_changes`: o ledger abriu e passou na verificação inicial, mas uma gravação falhou
    **depois** de arquivos já copiados (disco, permissão, `claims.json.tmp` inutilizável). A
    garantia inicial não cobre isso. `adopt` não retorna sucesso: o arquivo (e as pastas ainda
    vazias feitas para ele) cuja claim não foi gravada tem rollback tentado na hora, sob o lease da raiz,
    condicionado à prova de identidade e SHA-256; o resultado já registrado é descartado; o erro lista
    `details.takenBack`, `details.kept` e `details.cleanup`. Nunca diz "nada foi alterado".
  - O sucesso de `create_file`/`create_dir` só vale com a claim **gravada**; o ledger nunca
    lista um arquivo que não foi mantido. Se a prova ou remoção do rollback também falhar,
    o arquivo pode permanecer sem claim em `details.kept`, fora do alcance de `recover`;
    esse caminho exige conferência manual, não uma promessa de recuperação pelo ledger.
  - Descarte cuja saída não pôde ser gravada (`Leave::Unverifiable`) mantém o arquivo e a
    claim (titular vivo) e informa em `kept`; o `recover` leva o órfão depois que o dono sai.
    Arquivo de titular vivo ou aceito nunca é removido por esses caminhos.
  - `after_changes` por **juntar-se** a um arquivo/pasta de outro resultado, ou por **fundir**
    (`absorb`): o resultado é recusado e descartado onde o ledger permite; a transferência é
    gravada antes de mover qualquer lista, então ninguém perde o dono.
  - `accept` (BL111.2): a gravação da aceitação falhou. `import::apply` chama `accept` como
    **último passo dentro de `state.transaction`**: o erro reverte o projeto (cenas, histórico e
    revisão como estavam) e o resultado é descartado sob prova (o que o ledger não deixa
    registrar saindo fica, claimado, e vai em `details.cleanup.kept`). `PendingAdoption::apply`
    faz o mesmo sem projeto a reverter. Se a gravação falha mas chegou ao disco, o descarte acha
    a entrada já sem titular e **mantém** o arquivo: o pior resultado é arquivo que fica, nunca
    arquivo apagado sob um dono.
  - **A lease sobrevive a uma gravação que falhou**: o processo só solta `live/<token>` depois de
    gravar o ledger sem ele. Soltar antes faria o próximo leitor reapá-lo como morto enquanto o
    disco ainda lista suas claims, e `recover` removeria arquivo de processo vivo.
  - **`claims.json` sem `ledger.lock` é dano, não ledger vazio**: sem lock e sem estado é "sem
    ledger"; com estado e sem lock (visto em 5 olhadas seguidas, para não confundir as corridas
    de uma olhada de criar/esvaziar o ledger) é `fault` e tudo recusa: `recover` não remove nada,
    descarte mantém, `claims` diz `available:false`. O caminho que cria (nova adoção) toma um
    lock novo e segue lendo o estado: nunca trata as claims como inexistentes.
  - Limite aberto (BL111.3): a cópia em curso não pode ser interrompida pelo observador
    `on_published`; os arquivos seguintes ainda podem ser copiados; o rollback de cada um é tentado sob prova,
    podendo manter arquivos sem claim em `details.kept`, fora do alcance de `recover`
    e sem remoção sem prova. Seguro, só desperdiça trabalho.
- Sem entradas nem owners o ledger inteiro é removido: workspace sem resultado pendente fica
  idêntico a um que nunca teve ledger.
- `recover(workspace, token)` (lib) devolve `Recovery { reaped_holders, orphans, report }`:
  remove só órfãos, sob o `RootLease` de cada raiz, arquivos primeiro e pastas vazias depois,
  com hash fora da trava do ledger e cancelamento (`cancelled`). `claims_status(workspace)`
  devolve o JSON `scene.resources.claims` (owners, reaped, orphans, entradas) sem alterar nada além do reap.
- Ordem de travas: lease da raiz → `ledger.lock`; nada espera lease de raiz com o ledger travado.

Superfícies: **lib** (`adopt`, `Provisional`, `recover`, `recover_with`, `claims_status`,
`tracked_under`), **JSONL** (`scene.resources.claims`, `scene.resources.recover`, BL111.1) e a
**janela** pelo mesmo canal de operações (`ui op`), ver abaixo.

#### `scene.resources.claims` e `scene.resources.recover` (BL111.1)

Mesmo `MediaHost`/`Extension` de todo `scene.resources.*` (`tessel-media`, `tessel-render`,
`./scripts/ui op`); nenhuma regra mora no desktop. Escopo: o **workspace do projeto aberto**
(`project.workspace`); sem workspace, `no_workspace` antes de qualquer job. Não aceitam caminho
arbitrário: não há como apontar o `recover` para uma pasta que o projeto não abriu. Ambos são
**jobs** (`wait:false`/host interativo devolvem o ID; o desktop nunca espera) e não mudam dados do projeto
(sem `Pending`, sem `job.commit`).

| Operação | Tipo do job | Efeito | Resposta / erros |
|---|---|---|---|
| `scene.resources.claims` | `preflight` | **Não é só leitura**: abrir o ledger faz o *reap* dos titulares cujo lease sumiu e remove um ledger que ficou vazio. Nunca remove nem lê o conteúdo de arquivo de recurso | `{action, readOnly:false, effects, workspace, available, fault, owners[], reaped[], orphans, entries[]}`. Ledger ilegível **não falha**: `available:false` com `fault` |
| `scene.resources.recover` | `hash` | Remove só **órfãos** (sem titular vivo), após a mesma prova de conteúdo de todo cleanup (BL097), sob o lease de cada raiz; faz o hash fora da trava do ledger | `{action, workspace, recovery:{reapedHolders, orphans, cleanup:{removedFiles, removedFolders, kept, shared, busyRoots}}}`. Cada arquivo mantido vira `warnings` do job. Erros (no job: `error.code`; esperando com `wait:true` chegam como `job_failed` com `details.job_code`): `claims_unavailable` (pode ocorrer após remoções; `details` contém o relatório parcial), `cancelled` (`details` leva o relatório parcial; o que já saiu continua removido e sem claim); `no_workspace` é recusa imediata, sem job |

Progresso do `recover`: etapa `recover`, `total` = arquivos órfãos, `done` = arquivos decididos.
Cancelamento (`job.cancel`) vale **entre arquivos e entre blocos do hash** do arquivo em curso
(BL111.2): o hash lê em blocos de 1 MiB e consulta o token antes de cada um; cancelar no meio
deixa aquele arquivo intacto e claimado e o erro (`details`) traz o relatório parcial e
`undecidedFiles` (ele e os não alcançados). Descartes comuns (destrutor) continuam sem cancelamento. Os
tipos de job existentes foram reaproveitados de propósito (`preflight`/`hash`) para não tocar o enum
de `core::jobs`, que a frente S1 também altera; o rótulo (`Scene resources claims`,
`Recover scene resources`) e a resposta dizem o que o job faz.

Superfícies: lib `recover_with` + testes unitários; JSONL real (`scene_claims_jsonl.rs`, processos
`tessel-media`); janela (`workflow_tests.rs` + `scripts/scene-claims-proof.py`). Sem controle novo
na janela (nenhum botão, nó de árvore ou comando de parser): o acesso é `ui op`, e o job aparece
no nó `jobs`, no diálogo de Tarefas e em `job.get`/`job.cancel`. Apresentação de um botão
"Recuperar" é **N/A nesta entrega** (decisão de escopo, não ausência de domínio): o domínio completo
está acessível por lib, JSONL e `ui op`.

Achado na prova: no macOS um `open` que corre contra o `unlink` do `ledger.lock` pelo processo que esvaziou o ledger responde `EINVAL` (visto em 2 de 5 execuções do teste de ciclos concorrentes antes da correção); a abertura é refeita (30/30 execuções seguidas passaram depois).

Limites: o lease é `flock` advisory de **filesystem local** (NFS/SMB podem não oferecer);
escritor hostil do mesmo usuário está fora de escopo (BL079); queda do SO não é sincronizada
(rename atômico, sem `fsync`); processo que morre depois de o projeto aceitar mas antes de soltar
os claims deixa um órfão que o `recover` verifica e pode remover; Windows não foi provado.

### BL111.2: fechamento de aceitar e fundir

Estado: **implementado e provado localmente** (a validar na revisão). Proveniência: `f8ae1c8`
(PR #9) → merge `dbde018` → fix `7a24a7b` (falha de claim após a cópia) → docs `a23bb1e` → merge
de `main` `e296b36` (S3 `ResourceSnapshot` e S4 `asset.identity`) no head integrado; as linhas de
backlog e matriz foram mescladas linha a linha (BL109 e BL102/BL102b ficam com o estado da main).

| Subcaso | Antes | Agora | Prova |
|---|---|---|---|
| (b) aceitar | `Drop for Created` chamava `Tracker::accept`, que ignora `fault` | `Tracker::accept -> Result`, `Created::accept`, `Provisional::accept -> Result<(), JobError>`; `Provisional::release` foi removido; `import::apply` e `PendingAdoption::apply` aceitam como último passo; erro reverte o projeto | `scene_claims_integrity.rs`: import e adoção com `claims.json.tmp` bloqueado → `claims_unavailable/accept`, projeto idêntico (resumo, revisão, `can_undo`), arquivos mantidos e claimados, `recover` não toca titular vivo; `scene_claims_processes.rs`: aceite recusado + `SIGKILL` → órfão recuperável; aceite gravado + `SIGKILL` → `recover` não remove nada |
| (c) fundir | `absorb` consumia `other` e ignorava o `transfer` | `Created::absorb(&mut other) -> Result`: transferência gravada **antes** de mover listas; em falha `other` fica intacto e `import.rs` o descarta com relatório | `a_merge_the_ledger_cannot_write_fails_and_nobody_loses_ownership`: `prepare` falha com `after_changes`, as claims continuam com o titular original (1 titular, não órfã) |
| (d) `claims.json` sem `ledger.lock` | `acquire(create=false)` → `Ok(None)` ("vazio") | `look_for_damage`: estado sem lock em 5 olhadas seguidas → `fault` | unidade (`a_state_file_without_its_lock...`, corrida `emptying_and_refilling...` com 3 escritores e 1 leitor: nunca dano), `scene_claims_integrity` e dois processos (`recover` recusa mesmo depois do `SIGKILL`, `claims.json` preservado) |
| lease x gravação falha (achado novo) | `sync_lease` soltava `live/<token>` **antes** de gravar | solta depois da gravação; falha mantém a lease | unidade (`a_failed_write_never_gives_up_the_lease...`) e dois processos (`a_process_refused_its_acceptance_stays_alive...`) |
| hash de recover | cancelamento só entre arquivos | `remove_file_unless_cancelled` passa o token ao hash (lê em blocos de 1 MiB e consulta antes de cada um); cancelar no meio deixa o arquivo intacto e claimado e o erro traz o relatório parcial + `undecidedFiles` | `cancelling_during_the_hash_of_a_large_file...` (arquivo esparso de 2 GiB cancelado após 40 ms) |

Mutação (cada regra antiga reintroduzida, teste correspondente falha; resultados em
`docs/migration/s2-integrity-mutations.md`): aceitar ignorando a gravação, fundir movendo antes e
ignorando o `transfer`, soltar a lease antes de gravar, estado sem lock como ledger vazio, hash do
`recover` sem o token.

Direção segura: se a aceitação chegou ao disco e o erro ainda assim foi devolvido, o descarte
seguinte encontra a entrada já removida e mantém o arquivo (vaza em vez de apagar). Nunca o
contrário; não depende de retry em memória, PID ou marca em memória.

#### Junção com a S1 (PR #6, head `008af90` + `main` `ac04dca`), **incorporada**

Os conflitos textuais foram `adopt.rs`, `import.rs`, `docs/backlog.md` e `docs/features/README.md`; a junção é semântica:
(1) `settle` e `try_apply` da S1 trocaram `drop(self.owned.release())` por `self.owned.accept()` fallível; um `Err` volta como
`StaleAdoption{refusal: Some(..)}` / `RejectedImport` com o resultado **ainda dono** dos arquivos e das claims; (2) o `accept`
do import é o último passo de `state.transaction` em `try_apply`; (3) `Provisional::absorb(&mut)` segue como em S2;
(4) o host entrega a recusa ao job `discard` mantendo `claims_unavailable`/`phase=accept` e `details.discard_job`
(`apply`/`job.commit` bloqueantes descartam com `details.cleanup`). `Provisional::discard`/`Created::remove` seguem síncronos:
são o trabalho do job. Provas e mutações: [F025](jobs.md#junção-com-claims-persistentes-e-aceite-fallível-bl0943-merge-de-main-ac04dca-s2)
e `crates/media/tests/job_discard_claims.rs`.

#### `before_changes` x `after_changes`

| `details.phase` | Quando | O que se pode afirmar | `why` | `takenBack` / `kept` | `cleanup` |
|---|---|---|---|---|---|
| `before_changes` | ledger ilegível, de versão mais nova, caminho não UTF-8 | nada foi alterado | sim | ausentes | ausente |
| `after_changes` (claim de arquivo/pasta publicada não gravada) | gravação falhou depois de copiar | o rollback tenta desfazer sob prova; o que não pode remover fica em `kept`, possivelmente sem claim e fora de `recover`; nunca "nada foi alterado" | sim | sim: o que foi desfeito e o que a prova ou o ledger não deixou remover | relatório do descarte do restante do resultado |
| `accept` | a gravação da aceitação falhou (último passo de `import::apply`/`PendingAdoption::apply`) | o resultado **não** foi aceito e o projeto está como estava; o descarte seguinte pode remover arquivos se a falha for transitória, ou mantê-los se não puder provar a remoção | sim | ausentes | consultar `details.cleanup` para o resultado real do descarte |
| `after_changes` (juntar-se a arquivo/pasta de outro resultado) | o ledger falhou ao registrar a dependência | o resultado foi recusado e descartado onde o ledger permite | sim | ausentes | relatório do descarte |

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
release o mesmo descarte levou 0,66 s (medido em BL112, no job de descarte). Descartar um
resultado grande num destrutor da thread de UI bloquearia; por isso a janela usa `job.discard`
(BL094/BL112, [F025](jobs.md)): o tick de `poll()` e o `Drop` do host interativo entregam o resultado a um
job `discard`, que mantém o dono dos arquivos até terminar, e `job.discard {job}` descarta sob pedido.

### Host (`MediaHost`)

`scene_adopt` captura o `AdoptionTarget` (projeto, workspace, cena, revisão do bundle) **antes**
de iniciar o job; o resultado vira `Pending::SceneAdopt(Box<PendingAdoption>)`. `apply` (via
`poll` ou `job.commit`) aceita só se tudo continua igual; senão responde `stale_result` com o
motivo (projeto, workspace, cena ou revisão) e remove: na lib e no host sem janela, na hora
(`PendingAdoption::apply`); no tick da janela por um job `discard` (`PendingAdoption::settle` devolve o
resultado ainda dono dos arquivos; o erro traz `details.discard_job`). Host descartado = `Drop` remove (host
interativo: por job de descarte). A API
`SceneExport` da R1 em `host.rs` não foi tocada.

Fora do que se faz: revisão do projeto que muda por edições não relacionadas não invalida um
import (ele só acrescenta cenas); `undo` do import desfaz o projeto, não os arquivos (a pasta fica
para um novo anexo); um `job` finalizado e nunca confirmado exige `job.commit` ou `job.discard`
(F025), ou o descarte do host.

Evidência por superfície do descarte em job (integração S1 + S3 + S4): lib/host (`job_discard`, `scene_adopt_host`,
`scene_import_cleanup`, `scene_cleanup_ownership`), processo JSONL real e janela própria com a latência dos pedidos durante a
remoção medida (detalhes e limites em [F025](jobs.md#evidência-por-superfície-integração-s1--s3--s4-a-validar-até-a-revisão)).
**Costura S1 x S2 (já incorporada, ver "Junção com a S1" acima e BL094.3):** `PendingAdoption::settle` devolve a adoção
recusada ainda dona de arquivos e claims; `apply` é `settle` + remoção inline e só a remoção (`discard`) lê/hasheia
**recursos**. O aceite é `Ledger::accept` (um só caminho para o tick e para quem pode bloquear): no tick da janela
(`settle`, `import::try_apply`) espera o lock só **20 ms** (um lock ainda ocupado depois disso, por outro processo ou por
outro job deste, recusa o aceite com `claims_unavailable`, BL112.3/N1); em quem pode bloquear (`apply`, `job.commit` da CLI,
jobs) a espera é limitada a 1,5 s. **O custo do aceite não depende do tamanho do ledger (BL112.7, ver "Diário de aceites"
abaixo).** Um dono persistente de claims precisa manter essas propriedades (testes `settle_*`/`apply_is_settle_*`).

### Diário de aceites (BL112.7)

**Problema.** Aceitar um resultado é o último passo da transação do projeto e roda na thread da janela. Reescrever
`claims.json` para isso custa `O(entradas do ledger inteiro)` (ler, parsear, montar, serializar): 304–310 ms com 10 mil
entradas em build debug, contra o teto de 100 ms do tick (BL112.3). O critério de 10 mil entradas continua e não foi
relaxado; também não foi "resolvido" com ajuste de constante, build release, ignorar o teste ou só mover o flush para depois
do `apply` (o aceite continuaria sendo `O(ledger)` na UI e o projeto passaria a apontar para arquivos antes de o ledger
dizer que são dele).

**Alternativas.** (1) *Aceite assíncrono / duas fases:* o tick só marcaria "aceito" e um worker gravaria depois. Descartado:
até o worker terminar, o projeto referencia arquivos que o ledger ainda lista como órfãos em potencial, e `recover` de outro
processo os removeria; reverter o projeto depois de aplicado exige uma segunda máquina de estados no `poll` (a mesma que o
BL112.5 deixou de lado). (2) *Reserva/transição reversível* (mover as entradas para um estado "aceitando" e confirmar
depois): ainda é uma reescrita `O(ledger)` ou um segundo arquivo com as mesmas regras de leitura única; é o diário com mais
estados. (3) **Diário por acréscimo (escolhido):** uma gravação `O(chaves do resultado)` sob o mesmo `ledger.lock`, lida por
todo leitor antes de qualquer outra coisa. É o menor que mantém as quatro regras abaixo sem mudar a máquina de estados dos
jobs, o contrato do `poll` nem os códigos da CLI.

**Formato.** Um ledger cujo `claims.json` + `claims.journal` passa de `INLINE_LIMIT` (64 KiB, ~500 entradas) é aceito
**acrescentando uma linha** a `claims.journal`, sem ler nem parsear `claims.json`:

```text
{"accept":["/ws/scenes/a/assets/x.png","/ws/scenes/a/assets"]}\n
```

Até o limite o aceite continua inline (reescreve o estado, poucos milissegundos), e um workspace sem nada pendente continua
parecendo nunca ter tido ledger.

**Invariantes.**

1. **Um só leitor.** O ledger efetivo é `claims.json` **menos** as chaves de todo registro completo do diário. Todo leitor
   (`recover`, descarte, `claims_status`, a próxima adoção, o `Tracker` de qualquer job) aplica o diário ao abrir; ninguém lê
   `claims.json` sozinho. Por isso nenhum projeto referencia um arquivo que `recover` poderia remover: o aceite gravado vale
   para o primeiro leitor que o veja, inclusive o de um processo que acabou de morrer (`SIGKILL` logo depois do tick).
   **Contra queda do processo, não contra perda de energia:** como o estado, o diário não tem `fsync`; o que o `write_all`
   entregou ao sistema operacional sobrevive a um `SIGKILL`, mas um corte de energia pode perder o registro (e então o
   projeto, que também não foi sincronizado, não o referencia mais).
2. **Compactação antes de qualquer coisa.** O `Tracker` que encontra um diário reescreve `claims.json` sem essas chaves
   (rename atômico) e só então remove o diário, **antes de responder a quem o abriu**. Assim ninguém reivindica um caminho
   de novo enquanto um registro antigo dele ainda existe (o registro o aceitaria por engano), e uma queda entre o rename e a
   remoção deixa um diário cujas chaves já não existem: reaplicar não muda nada (idempotente, sem campo de geração). A
   compactação é feita em jobs/CLI/`recover`/status, **nunca no tick**. Por que a idempotência basta: uma reivindicação nova
   só é gravada **depois** de a compactação do `Tracker` que a faz ter terminado (`with_lock` dobra o diário antes de
   responder a quem abriu) ou, se o diário não pôde ser removido, o ledger é `fault` e nada é reivindicado; logo um registro
   antigo nunca é aplicado depois de uma reivindicação nova do mesmo caminho, e uma queda entre o rename e a remoção só pode
   repetir uma dobra que já foi feita.
3. **Gravado antes de acreditado, e reversível.** O tick só responde `Ok` depois de o registro ser escrito inteiro sob o lock.
   Uma escrita que falha é desfeita (`set_len` até onde o arquivo estava) e responde `Err`: o resultado continua dono e
   claimado, o projeto, o undo e a revisão voltam ao que eram (o aceite é o último passo da transação) e o resultado vai
   para um job `discard` (`claims_unavailable`, `details.phase="accept"`). Um registro cujo escritor morreu no meio não tem
   `\n` final: nunca foi um aceite (o projeto a que pertencia também não foi salvo), não é lido e o próximo escritor o corta.
   Uma linha **completa** que não parseia é dano: o ledger é `fault` e tudo recusa (`recover` não remove nada), porque ignorar
   um aceite deixaria `recover` remover arquivo que o projeto referencia.
4. **Dono vivo e claim até um destino terminal; descarte nunca apaga o aceito.** O aceite só remove as chaves **do próprio
   resultado**; um resultado recusado, obsoleto (`stale_result`) ou descartado nunca grava registro, então o descarte segue
   sob prova (hash) e `Leave` como antes e as claims de terceiros nunca são tocadas. A lease do processo continua listada
   até o próximo `Tracker` compactar.

**Concorrência e falha.** O registro é escrito sob `ledger.lock` (o mesmo `flock` de qualquer escrita), com a espera de 20 ms
no tick; um holder longo recusa o aceite como antes (BL112.3/BL112.5). Aceites concorrentes e compactações nunca perdem nem
ressuscitam uma entrada (teste de threads em `claims/tests.rs`; processo real em `scene_claims_journal.rs`).

**Limites.** O tick não valida nem faz reap (isso é do próximo `Tracker`, fora da UI); o diário não tem `fsync`, como o
estado; o limite de 64 KiB é uma decisão de custo, não de correção (os dois caminhos têm as mesmas regras); um leitor que
compacta um diário grande paga `O(ledger)` fora da UI (jobs, CLI, `recover`, `claims`). O ganho só aparece com cerca de 500
entradas pendentes ou mais (64 KiB): um ledger desse tamanho é um cenário de estresse, não o uso normal (um projeto típico
tem poucas dezenas). **Pré-condição de qualquer mudança de formato (BL112.9):** o caminho grande de `Ledger::accept` só
acrescenta e não confere a versão de `claims.json`; com um `version: 2` um app antigo acrescentaria um diário que o novo pode
não conhecer, então a versão do estado precisa ser conferida nesse caminho antes de o formato mudar.

**Provas do diário no fechamento da revisão independente (F2–F4).** `claims/tests.rs`: dois titulares da mesma chave, o
aceite pelo diário e a saída do outro em qualquer ordem (`Untracked`, nada removido); `transfer` seguido de aceite (e
`transfer` depois do aceite, sem trazer a entrada de volta); `Created::absorb` e um registro que só se apoiava na pasta
(`Created::remove` mantém a pasta aceita) num ledger acima do limite; descarte, saída e aceite vazio nunca acrescentam; a
queda entre o rename do estado e a remoção do diário, reaplicada duas vezes e seguida de uma reivindicação do mesmo caminho,
e a mesma janela provocada **de verdade** (a remoção do diário falha por `chflags uchg`, só em macOS: o `Tracker` devolve
falha, o estado já está compactado e o próximo leitor completa). `scene_claims_journal.rs`: um **import** (não só uma adoção)
cujo diário não grava volta o projeto ao que era (sem cena, sem passo de undo, mesma revisão), mantém as claims pelo descarte
que não consegue provar, e num processo-janela real o `recover` deixa os arquivos enquanto a janela vive e os recolhe depois
do `SIGKILL`; a repetição do teste de `SIGKILL` só repete a rodada quando `details.why` diz que o lock foi segurado além do
orçamento do tick (`is_contention`): qualquer outra recusa falha na hora, e um teste com eventos injetados fixa a diferença
(falha de escrita, diretório no lugar, diário danificado e `claims_unavailable` sem motivo não são contenção).

**Descarte e claims (BL094.4, F1):** o último titular de um arquivo **não** sai do ledger antes do arquivo: `Tracker::begin_leave`
responde `Last` com a entrada (e a lease deste processo) ainda gravada; o job prova (SHA-256), remove e só então chama `leave`
para soltar a entrada. `SIGKILL` no meio do hash de um arquivo grande deixa uma claim órfã que `recover` recolhe, nunca um
arquivo que o ledger não lista. Se o ledger não consegue registrar a saída depois do arquivo tratado, a claim fica (dono vivo,
`kept` no relatório) e `recover` a resolve quando o processo terminar. `discarded` em `result_state` quer dizer "o descarte
terminou", não "tudo foi removido": o que ficou está em `result.cleanup.kept`.

## UI e CLI da janela

Sem controle novo (apresentação N/A). As operações acima, incluindo `scene.resources.claims` e
`scene.resources.recover` (BL111.1), funcionam via `./scripts/ui op` na janela; o job aparece em `jobs`.

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
| lib (unidade) | `scene_resources::claims` (21) | ledger em disco: sobrevive a trackers e some vazio; lease preso mantém titular vivo mesmo com PID inventado; lease solto remove titular mesmo com PID vivo; último a sair; transferência; órfão reaproveitado; ledger ilegível ou de versão nova recusa; aceitar/fundir/esquecer recusam ledger com falha e devolvem erro quando a gravação falha, deixando entrada e titular como estavam; a lease não é solta quando a gravação falha; estado sem lock é dano (e pasta sem estado é ledger vazio); esvaziar e recriar em paralelo nunca é dano |
| **dois processos reais (BL111)** | `crates/media/tests/scene_claims_processes.rs` (15 + papel filho) | o binário de teste é reexecutado como segundo processo (`TESSEL_CLAIMS_ROLE`) e usa `adopt` de verdade: arquivo compartilhado sobrevive ao descarte de qualquer um, nos dois sentidos; último titular leva tudo; aceito nunca é removido pelo outro; ledger inconfiável recusa antes de copiar; ciclos de adopt/discard concorrentes deixam o workspace como estava; `SIGKILL` do titular vira órfão sem remover nada; reaproveitar o órfão (aceitar/descartar); `recover` remove só o órfão; arquivo reescrito depois da morte fica; titular vivo não é recuperado; aceite recusado: processo vivo segue dono (lease mantida) e não é recuperado, `SIGKILL` depois → órfão recuperável; aceite gravado + `SIGKILL` → `recover` não remove; ledger sem lock recusa mesmo com o titular morto |
| **aceitar/fundir (BL111.2)** | `crates/media/tests/scene_claims_integrity.rs` (7) | `import::apply` e `PendingAdoption::apply` com a gravação bloqueada: erro `claims_unavailable/accept`, projeto idêntico, arquivos mantidos e claimados; aceite gravado continua anexando; fusão com gravação bloqueada falha sem perder o dono; ledger sem lock recusa tudo |
| lib (unidade, BL111.1) | `scene_resources::recover` (4) | progresso em arquivos órfãos e ledger removido no fim; cancelar entre arquivos e no meio do hash de um arquivo grande (2 GiB esparso) mantém o resto claimado, devolve o relatório parcial e uma nova execução termina; ledger ilegível recusa (`claims_unavailable`), nada removido, `claims` informa `available:false` |
| **JSONL real (BL111.1)** | `crates/media/tests/scene_claims_jsonl.rs` (6) | processos `tessel-media`: titular morto por `SIGKILL` → `claims` lista órfãos sem remover arquivo e declara `readOnly:false`; `recover` como job (`wait:false`, `done==total`) remove só os órfãos e restaura o workspace; titular vivo não é recuperado (e `job.commit` mantém); arquivo reescrito fica e o job avisa; ledger ilegível → `available:false` e `claims_unavailable` sem remover; sem workspace → `no_workspace`; fim ordenado da sessão não deixa claims |
| janela (BL111.1) | `workflow_tests.rs` (1) e `scripts/scene-claims-proof.py` | recusa sem workspace sem iniciar job; as duas operações voltam com job e o tick reporta o resultado; ledger ilegível falha com `claims_unavailable` na janela. A prova nativa dirige a janela própria por `op`/`get`/`invoke` com um segundo `tessel-media` real morto por `SIGKILL`; 23 checks, inclusive o ledger sem `ledger.lock` (`available:false`, `recover` recusa e nada é removido) |
| lib (unidade) | `scene_resources::*` e `directed_resources::lease` | bases, colisão de caixa, sessão/orçamento, lease |
| regressão | `scene_project_guards`, `directed_resources*`, `package`, `interchange`, `import` | as guardas de pacote incompleto continuam valendo para o que não é transportado |

Sem prova (não executado): Windows, escritor hostil concorrente, filesystem de rede, disco cheio real, cancelamento do `on_published` (BL111.3), botão de recuperação na janela, pixels/renderer, FFmpeg real além da suíte existente do
crate, o custo do hash em build de release e a limpeza com raiz ocupada além de 200 ms.

## Consumidor integrado e prova de restauração

**BL101 (S5): manifesto sem arquivos.** `scene.resources.preflight|resolve` não abrem raiz quando o plano de transporte não tem arquivo (assets declarados mas não usados, ou usados só por seções preservadas): o mapa é instalado com `resources: 0`, `complete: false` e `base: null`, sem exigir `scenes/<id>` nem workspace (`ResourceBase::unbound()`, `needs_files`). Qualquer cena que nomeie um arquivo mantém o preflight completo (`root_unavailable`, `resources_invalid`, contenção, hash) e, sem workspace, `no_workspace`; uma base explícita relativa continua `base_not_absolute`. `scene.resources.adopt` não mudou. Testes: `crates/media/tests/scene_attach_job.rs`; contrato do attach por job em [F043](scene-import-ui.md).

F041 consome `SceneResourceSession` por `ResourceResolver`; não resolve caminhos nem confia só no assetId. O harness integrado usa o projeto restaurado para avaliar e renderizar sem a origem, comparando sondas de pixel independentes. Comandos de carga continuam jobs na janela; job concluído precisa estar aplicado antes do monitor. Sem diálogo de anexar/carregar dedicado ainda (BL086).

## Import descartado após troca de projeto (revisão PR #3; ampliado por BL089)

PreparedSceneImport captura o ID de destino. A função compartilhada `scene_resources::import::apply` recusa outro projeto com stale_result antes da transação e descarta somente as pastas criadas pela preparação. MediaHost usa essa mesma função em poll/job.commit, portanto a guarda vale também pela biblioteca direta. Uma regressão conclui o job, troca o projeto e tenta commit, conferindo cena/histórico intactos e limpeza dos arquivos próprios.

### Correções da revisão incremental da S2

`job.commit` preserva o erro e o relatório parcial de uma recuperação cancelada. Se o ledger
falhar após remoções, `claims_unavailable` também inclui o relatório parcial; não promete
reverter arquivos já removidos. Uma falha na segunda cena de import agrega os relatórios de
cleanup de todas as cenas, preservando as entradas anteriores em `kept`/`shared`.
Testes: `a_cancelled_recovery_keeps_its_partial_report_through_job_commit`,
`a_ledger_failure_after_removal_reports_what_was_already_removed` e
`a_second_scene_merge_failure_reports_files_kept_from_both_scenes`.

Validação do coordenador após a revisão: fmt/check/clippy workspace e build desktop verdes;
suíte media completa e workflow desktop (8) verdes. O gate workspace passou core/desktop/media,
mas recusou dois testes de render porque o runtime ainda não estava instalado. Após
`./scripts/runtime setup`, o crate render inteiro passou com `TESSEL_REQUIRE_RUNTIME=1`
sob lock de GPU. Logs locais: `/tmp/tessel-s2-review-workspace.log` e
`/tmp/tessel-s2-review-render.log`; a falha inicial foi preservada, não contada como PASS.

A falha de `Tracker::forget` em arquivos ou pastas interrompe a recuperação com
`claims_unavailable` e relatório parcial, em vez de sucesso com claims antigas. Regressão:
`a_failed_forget_after_removals_is_an_error_with_the_partial_report` (duas claims, escrita
bloqueada depois de remover ambos os arquivos). Os testes de janela de import e claims usam
uma guarda de diretório com `Drop`, inclusive quando uma asserção falha.

Revalidação após o último finding: recover8, integrity8, processos16 (inclui papel filho),
JSONL6 e workflow desktop8 passaram; clippy workspace/build verdes. A regressão de forget
falha ao reintroduzir a regra antiga (sucesso indevido) e passa com a correção restaurada.
A prova nativa foi repetida no código atualizado: **23/23**, instância própria e lock GPU,
piso12GiB, relatório `/tmp/tessel-s2-review-native-final.json`. Há também regressão específica
para falha de forget de pasta após remoção parcial de diretórios.

### Documentação dos contratos de ownership (BL111.5)

Após a revisão, os contratos de 46 funções receberam docstrings: identidade de holders,
validação do ledger, leases, publicação/cleanup, import e consumo de resultados pelo host.
A alteração é documental; o percentual de cobertura depende da nova medição do CodeRabbit.
