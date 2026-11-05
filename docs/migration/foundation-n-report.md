# Relatório da frente N — recursos de cena no host, carga, pacote e intercâmbio (BL010 / BL011 / BL079 / BL080)

2026-11-05. Checkout compartilhado `tessel-editor`, branch `feat/directed-scene-end-to-end`.
Contrato e limites: [`docs/features/scene-resource-integration.md`](../features/scene-resource-integration.md) (F040).
Sem Git de escrita, sem subagente, sem mudança em `tesselapp`, render, runtime, desktop,
`Cargo.*` nem `crates/core`. Cargo só pelo helper `cargo-serial.py`; sem GPU/Chrome/janela;
mídia sintética; temporários próprios (alguns MiB).

## Arquivos

Novos (`crates/media`): `src/directed_resources/lease.rs`; `src/scene_resources/{mod,base,map,session,load,adopt,pack,import}.rs`;
`tests/scene_resource_publication.rs`, `tests/scene_resource_integration.rs`.
Alterados: `src/lib.rs` (`pub mod scene_resources;`), `src/host.rs`, `src/package.rs`,
`src/directed_resources/{mod,materialize,destination,preflight}.rs`.
Docs: `docs/features/scene-resource-integration.md`, este relatório.

## O que entrou (resumo)

- **BL010.** `SceneResourceSession` (geração, orçamento, release, invalidação) + `ResourceResolver`
  (costura do renderer, sem ciclo: `render → media`). Operações `scene.resources.status|preflight|resolve|adopt|release|invalidate`
  no `MediaHost`; jobs fora da UI; `resolve` só instala se projeto, geração e revisão do bundle ainda forem os da partida.
  `scene.detach` libera; troca de projeto sobe a geração.
- **BL011.** Base gerenciada derivada `<workspace>/scenes/<id>/` (persiste, move e renomeia com o workspace, sem estado novo).
  `adopt` (= relink). Pacote: `PackageInputs{scene_bases}`, `preflight_with`, `create_with` (aditivos), arquivos em
  `scenes/<id>/…` verificados pelo hash do bundle, restauração que exige o hash do bundle no manifesto do pacote.
  Intercâmbio: `project.import_scenes` (ID livre sem caixa, pasta por cena, desfazer em um passo).
- **BL080.** `publish` checa o token antes de **cada** arquivo; `on_published` observa; rollback só dos links da execução.
- **BL079.** `RootLease` (flock advisory, `root_busy`) + contenção por arquivo (`destination_escaped`). Limite: não fecha troca hostil de pai.

## Guardas de pacote: o que caiu e o que ficou

Caiu **somente** onde houve prova real (teste `a_package_carries_the_scene_files_and_restores_without_the_originals`, formatos
Directory e Archive): cena só com domínios suportados + base disponível → pacote `complete`, restaurado depois de apagar origem **e**
workspace, com os arquivos no hash do bundle, e o projeto restaurado resolve a cena sem a origem. Ficou: sem base, base com
arquivo ausente/alterado, e seções não transportadas (mapas/f32/profundidade/narração, BL077) continuam `not_packaged`/`unsupported`
(`allow_partial` exigido); `scene_project_guards` segue verde sem alteração.

## Comandos e resultado

```text
cargo-serial.py test   -p tessel-editor-media --locked     # todas as suítes do crate: 0 falhas
cargo-serial.py clippy -p tessel-editor-media --all-targets --locked -- -D warnings   # limpo
cargo-serial.py fmt    -p tessel-editor-media -- --check   # limpo (só o crate de media foi formatado)
```

Suítes novas: `scene_resource_publication` 4, `scene_resource_integration` 18, mais testes de unidade em `scene_resources::*` e `lease`.
Não rodados: `cargo fmt --all` (há diffs de formatação em arquivos de outras frentes, não tocados), `test -p tessel-editor-core`
(core sem alteração), render/desktop, Windows, escritor hostil, disco cheio, UI, pixels.

## Patches propostos ao coordenador (arquivos compartilhados)

Nenhum é necessário para compilar ou testar o que foi entregue. Opcionais, em ordem de valor:

1. **`crates/core`: persistir uma base de referência** (hoje só a gerenciada é persistente). Guardar `resourceBase` como entrada
   do bloco `scenes` (usa o mecanismo `extra` que já preserva chaves desconhecidas) + `AttachedScene::resource_base()` e um
   `SceneCommand::SetResourceBase { id, base }` (desfazível). `ResourceBase::explicit` já aceita o valor. Só vale a pena se o
   produto quiser “apontar para uma pasta fora do workspace” sem copiar.
2. **`project.import` (core/interchange)**: hoje `project.import` não traz cenas (por desenho) e avisa; se for desejado, delegar
   para `scene_resources::import` pelo host em vez de mudar o core. Nada a fazer se `project.import_scenes` bastar.
3. **`Cargo.toml` (BL079 completo)**: adicionar `rustix` (com `fs`) ao crate de media para gravar com `openat`/`linkat` ancorados num
   diretório aberto; só então trocar a checagem de contenção por arquivo por operações ancoradas e declarar a defesa contra troca de pai.

## Linhas sugeridas

Backlog (`docs/backlog.md`):

- BL010 → **Parcial+**: “Sessão por geração (`SceneResourceSession`), jobs `scene.resources.*`, invalidação por troca de projeto/contexto/asset, release e orçamento no `MediaHost`; falta o consumo real pelo renderer (M) e prova de pixels”.
- BL011 → **Parcial+**: “Base gerenciada persistente, adopt/relink, pacote/restauração com hash do bundle (Directory/Archive), `project.import_scenes`; falta transporte de mapas/f32/narração (BL077) e UI”.
- BL079 → **Parcial**: “Lease cooperativo + contenção por arquivo; troca hostil de pai continua aberta e depende de `openat` (patch de Cargo acima)”.
- BL080 → **Pronto na biblioteca**: “Cancel por arquivo publicado, rollback só dos próprios, teste determinístico (`scene_resource_publication`)”.

Matriz/índice (`docs/features/README.md`): adicionar
`| F040 | [Recursos da cena no editor: carga, geração, pacote e intercâmbio (BB06)](scene-resource-integration.md) | P | Parcial | P | Parcial | MediaHost: scene.resources.*, project.import_scenes; PackageInputs; sem renderer/UI |`
e na linha F038 trocar “sem ligação ao pacote/CLI/UI” por “ligado a host/pacote/intercâmbio em F040”.

## Observações para o coordenador

- `package --replace` apaga o pacote anterior antes do `rename`: uma falha no intervalo deixa o destino ausente (comportamento
  anterior, não agravado). Candidato a item de backlog próprio.
- Um `project.import_scenes` cujo job conclui mas cujo resultado é descartado (projeto mudou) pode deixar `scenes/<id>/` órfão no
  workspace; é reaproveitável por `adopt`, mas não é limpo sozinho.
- O erro de job chega ao chamador como `job_failed` com o código real em `details.job_code`, convenção já existente do host.
