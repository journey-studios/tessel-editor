# R5 — Limpeza de arquivos de import/adopt de cenas (BL089)

Frente R5 da [próxima entrega](next-delivery-ten-agents.md). Contrato e evidências por superfície:
[F040](../features/scene-resource-integration.md) ("Limpeza de resultados cancelados, falhos ou
descartados"). Estado e atribuição ficam só em [backlog](../backlog.md); este relatório não é fila.

Estado: **implementado e testado pela biblioteca**; a integração do adopt no `MediaHost` está
**pronta como patch, não aplicada e não testada** (ownership de R1).

## O que foi auditado

A guarda `PreparedSceneImport.project` + `apply`/`discard` do PR #3 não foi reimplementada. Os
demais caminhos:

| # | Caminho | Achado | Resolução |
|---|---|---|---|
| 1 | `prepare` falha/cancela na 2ª cena | só raízes que o import "achava" ter criado (um `exists()` antes do `adopt`) eram removidas; arquivos copiados para uma pasta que já existia ficavam | dono por destrutor; propriedade vem do que o `adopt` criou |
| 2 | `prepare` cancelado depois do último arquivo | o token só era conferido antes de cada arquivo/cena: o job terminava `succeeded` e o resultado era aplicado apesar do cancelamento | `token.check()` ao fim de `prepare` (teste via `prepare_observed`) |
| 3 | job `succeeded` cujo slot nunca é aplicado (host descartado, cancel tardio, `panic` com unwinding) | pasta órfã (documentado como limitação em F040) | `Drop` do `PreparedSceneImport` |
| 4 | `apply` com mesmo ID de projeto em **outro workspace** (cópia da pasta) | anexava cenas cujos arquivos não estão na base derivada do workspace aberto | `PreparedSceneImport.workspace` + `stale_result` + limpeza |
| 5 | `apply` recusado no `transaction` | `remove_created` manual | destrutor (sem caminho a esquecer) |
| 6 | `scene.resources.adopt` cujo projeto/cena mudou durante o job | sem checagem; arquivos ficavam | `AdoptionTarget` + `PendingAdoption::apply` → `stale_result` e limpeza (integração: patch) |
| 7 | `adopt` direto, cancelado/falho | `Journal` já desfazia os links; pastas criadas saíam; subpastas e arquivos de rollback falho não eram conferidos | `Adoption::created` + remoção com identidade |
| 8 | `panic` **dentro** de `directed_resources::materialize` | `Journal` não tem `Drop` | fora do ownership de R5 (F038): ver "Limites" |

## APIs e seams entregues

`crates/media/src/scene_resources/`:

- `cleanup.rs` (novo): `Created` (registro: arquivos com identidade + pastas em ordem de criação),
  `Provisional` (dono; remove ao cair, `release()`, `discard()`), `CleanupReport`.
  Remoção: toma o `RootLease` cooperativo (espera ≤ 200 ms), arquivos do mais novo ao mais velho,
  só se a identidade (tamanho + dev/inode em Unix) ainda é a publicada; pastas com `remove_dir`
  (nunca `remove_dir_all`); o que ficou vai em `kept`.
- `adopt.rs`: `Adoption { base, report, created }` (perdeu `Clone`; ninguém clonava),
  `Adoption::provisional()`, `AdoptionTarget::of(state, scene)`, `PendingAdoption::{new, apply, discard}`.
  `created` vem do próprio `adopt`: o `create_dir` que teve sucesso e as subpastas que não existiam
  antes da publicação.
- `import.rs`: `PreparedSceneImport` perdeu `Clone` e passou a ter `workspace` e um `Provisional`;
  `discard(self) -> CleanupReport`, `owned()`, `prepare_observed(.., on_published)`; `prepare`
  delega a ele. `PreparedScene` perdeu `created_root` (continua `Clone`: é só dado).
- `mod.rs`: `pub mod cleanup` e re-exports.

Decisão sobre Clone/Drop: o dono não é `Clone` e nenhum tipo que o contém é. Um clone cairia depois
e apagaria os arquivos do consumidor que ficou com o original; o compilador impede, em vez de um
comentário. `Created` também não é `Clone`.

## Patch para o coordenador (R1 dono de `media/host.rs`)

Arquivo: [`next-r5-host.patch`](next-r5-host.patch) (contexto `-U1`). Verificado com
`git apply --check` contra o `host.rs` atual do checkout (com o WIP de R1) **e** contra
`914c830`. Não aplicado, **não compilado nem testado** no host (a biblioteca que ele chama está
testada).

```
git apply docs/migration/next-r5-host.patch
```

Efeito: `Pending::SceneAdopt(Box<PendingAdoption>)`; `scene_adopt` captura `AdoptionTarget::of(state,&id)`
antes do job e devolve o resultado como pendente; `apply` chama `pending.apply(state)` (aceita →
`Ok(None)`; senão `stale_result` e remoção). Nada muda para quem usa `wait:true` e não alterou o
projeto durante o job. Depois de aplicar, o coordenador deve rodar
`test -p tessel-editor-media --test scene_resource_integration --test scene_import_cleanup`;
um teste de host para o adopt (projeto trocado entre `job.wait` e `job.commit`) só pode ser
escrito depois do patch, porque hoje o host devolve `None` para o adopt.

`import_scenes` no host **não precisa de patch**: o slot já guarda o `PreparedSceneImport` e agora ele
se limpa sozinho em todos os finais.

## Testes

Executados (via `cargo-serial.py`, só `tessel-editor-media`; verde em 8 repetições do binário
de teste, sem sobras em `/tmp`):

- `test -p tessel-editor-media --locked --lib --test scene_import_cleanup --test scene_resource_integration --test scene_resource_publication --test scene_project_guards --test directed_resources --test directed_resources_transport --test interchange`
  → 31 + 21 + 19 + 4 + 19 + 20 + 2 passam (os 21 de `scene_import_cleanup` são novos).
- `clippy -p tessel-editor-media --all-targets --locked -- -D warnings`; `rustfmt --check --edition 2024`
  nos arquivos que toquei (o `fmt -p` do wrapper `mbx` não imprimiu diferença).
- Mutação manual (revertida): sem o `token.check()` final, sem a checagem de workspace e sem o
  `Drop`, 10 dos 17 testes de import/adopt da primeira versão falharam, incluindo os alvos de cada
  mutação.

`scene_import_cleanup.rs` (21): cancelar durante a cópia (adopt e import 2ª cena), cancelar
logo depois do último arquivo publicado, falha na 2ª cena, descarte sem `apply`, host descartado
sem commit, `apply` aceito + `undo` não toca arquivos, projeto trocado, workspace trocado
(biblioteca e `job.commit` depois de `project.open` de uma cópia), attach recusado tarde (cena do
usuário preservada), destino pré-existente com arquivo do usuário e arquivo reaproveitado,
arquivo trocado depois da cópia, raiz ocupada, `PendingAdoption` (aceito, projeto trocado,
cena destacada, não aplicado).

Não executados: o gate do workspace (coordenador); `desktop`/`render` (não toquei; só conferi por
busca que nada fora de `media` usa `Adoption` nem `PreparedSceneImport`); Windows; escritor hostil;
disco cheio real; UI/janela; o adopt pelo host (depende do patch). Em dois momentos
`tessel-editor-core` não compilou por WIP de outras frentes (`scene_interaction/gesture.rs`,
`temporal/frame.rs`); esperei e repeti, sem editar arquivo alheio.

## Limites

- Lease cooperativo (BL079): a remoção não defende de escritor hostil nem de `openat`.
- Sem contagem de referências entre operações: se dois jobs escolhem a mesma pasta de destino antes
  de qualquer um criá-la, o segundo reaproveita arquivos do primeiro e o primeiro descartado os
  remove. `plan` ignora pastas existentes; a janela é a corrida entre dois `plan`, documentada em F040.
- Identidade por inode só em Unix; fora dele só o tamanho (como o lease).
- `panic` dentro de `directed_resources::materialize` (F038): o `Journal` não tem `Drop`; arquivos
  já publicados naquela chamada ficariam. R5 só cobre o que a chamada devolveu. Sugestão ao dono de F038.
- Um job finalizado e nunca confirmado em host não interativo fica até o host cair (agora limpo
  pelo `Drop`); `JobRegistry::prune` de um job com resultado pendente deixaria o slot sem dono até
  lá. O host hoje não chama `prune`.
- Revisões não relacionadas do projeto não invalidam um import (ele só acrescenta cenas).

## Patches de backlog e matriz (coordenador integra)

Backlog:

- **BL089** → `Validar` (Lib: validado pelos 21 testes acima; host/adopt: aguarda patch; CLI headless:
  `project.import_scenes` e `scene.resources.adopt` usam as mesmas funções, sem teste de socket;
  UI: N/A, sem controle). Aceite fechado: troca de projeto/workspace, cancelamento (cópia, depois do
  último arquivo), job rejeitado/descartado, erro tardio e reuso, sem apagar o anterior.
  Evidência: `crates/media/tests/scene_import_cleanup.rs`; doc F040. Dependência nova: aplicar
  `docs/migration/next-r5-host.patch` depois de R1.
- **Novo (proposta BL094)** — "Descartar explicitamente job de import/adopt finalizado e não aplicado
  em host não interativo (`job.discard`) e impedir `prune` de job com resultado pendente";
  responsável: coordenador/R1 (host); aceite: `job.discard` remove arquivos próprios, `job.get` mostra
  `discarded`; teste de host.
- **Novo (proposta BL095)** — "`Journal` de `directed_resources::materialize` com `Drop`/guarda de
  panic"; responsável: dono de F038; aceite: teste que força `panic` no `on_published` e vê o
  root igual ao de antes.

Matriz F040: Lib `Parcial` (BL089 fechado na biblioteca; BL077/BL078/BL079 seguem), CLI headless
`Parcial`, UI `P`. Linha de evidência: `scene_import_cleanup` (21). Nenhuma linha nova na matriz.
