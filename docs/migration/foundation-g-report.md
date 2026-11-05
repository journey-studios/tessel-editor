# Relatório da frente G — documento, histórico e acesso programático das cenas dirigidas

2026-11-05. Rodada 2 da base de migração ([plano](parallel-implementation-plan.md), [roadmap](phased-roadmap.md)).
Checkout compartilhado `tessel-editor`, branch `feat/migration-foundation`, base A–E integrada em `52f3175` (HEAD `574f88e`).
Origem lida em somente leitura (`/Users/wallyssonnunes/www/journeystudios/tesselapp-image-coverage/packages/scene-engine @
d6adac15c61f4ef582a7ec259397b2e025d1f059`); nada foi instalado, compilado ou escrito nela, e nenhuma mídia privada foi copiada.
Esta frente **não fez** `git add`/commit/push/merge/PR nem tocou o PR draft #1, `Cargo.toml`/`Cargo.lock`, dependências, a matriz
`docs/features/README.md`, `docs/README.md`, `directed_scene/*` (F), `scene_edit/*`, runtime ou desktop. **Não rodou Cargo, rustc nem
clippy** (a frente F tem o slot de compilação), nem GPU/Chrome, nem `mbx clean/gc`, nem HOME alternativo. Contrato da feature:
[`docs/features/scene-project.md`](../features/scene-project.md) (F036).

## Entrega

Um consumidor real, ponta a ponta, sobre as peças que já existiam (sem repetir regra de nenhuma):

| Necessidade | Como |
|---|---|
| Criar projeto por lib/JSONL | `EditorState::empty` / `{"op":"project.create"}` (existentes) |
| Anexar bundle por ID | `scene.attach` (`bundle` · `text` · `path`), ID escolhido ou `scene-<n>`; `admit` roda `edit_join_issues` + `temporal_issues` |
| Listar / consultar catálogo, propriedades, câmera | `scene.list|describe|catalog|properties|get|camera|capabilities` — leem `scene_edit` e `Bundle::evaluate_camera`/`camera_at_clip_frame`/`frame_at_time` (nenhuma API nova da F) |
| Lote e dry-run | `scene.edit` → `scene_edit::apply`/`dry_run`; rejeição, dry-run e no-op não alteram estado nem histórico |
| Undo/redo, transação única | `Command::Scene` passa por `EditorState::execute`; um lote = uma entrada; `Snapshot` ganhou `scenes` (`Arc`); sem segunda pilha |
| Salvar/reabrir sem perda | bloco aditivo `scenes` em `.tessel.json` (`schema_version` continua 1); abre projetos antigos; preserva campos desconhecidos do bundle, do item e do bloco |
| Mesmo parser em todos os adaptadores | `ops::parse_op` → `Op::Run(Command::Scene(..))`: `tessel-headless`, `ui op` e a lib compartilham parser e executor |
| Package / import de projeto sem perda ou promessa falsa | guardas em `crates/media` (abaixo) |

### Arquivos

| Arquivo | Mudança |
|---|---|
| `crates/core/src/scene_project/{mod,command,run,query,store,scope}.rs` | **novo** módulo: `SceneBook`/`AttachedScene`, `SceneCommand`, `admit`, consultas, bloco do documento, `scope_json`/`deferred_in_use` |
| `crates/core/src/lib.rs` | `pub mod scene_project;` |
| `crates/core/src/state/mod.rs` | `EditorState.scenes`, `Command::Scene`, rótulo de histórico, `apply`, `summary()` (chave `scenes` só quando há) |
| `crates/core/src/state/history.rs` | `Snapshot` guarda/restaura `scenes` |
| `crates/core/src/document.rs` | `to_document` grava `scenes` só quando há; `from_document` lê e valida |
| `crates/core/src/ops.rs` | `scene.*` em `parse_op` (`scene_command`, `scene_source`, `camera_at`, `optional_bool`) |
| `crates/core/src/headless.rs` | **sem mudança** (`Op::Run` já executa) |
| `crates/media/src/package.rs` | `plan()` registra recursos de cena como `missing` (`not_packaged`); preflight/create bloqueiam com `unsupported` sem `allow_partial` |
| `crates/media/src/interchange.rs` | `prepare()` acrescenta aviso quando a origem tem cenas |
| `crates/core/tests/scene_project.rs` | 18 testes públicos (lib + JSONL em processo + subprocesso) |
| `crates/media/tests/scene_project_guards.rs` | 4 testes das guardas, sem FFmpeg |
| `docs/features/scene-project.md`, este relatório | docs |

## Decisões que merecem revisão

1. **Bloco aditivo, sem subir `schema_version`.** Projetos sem cenas gravam os mesmos bytes de antes. Custo aceito: um editor
   anterior abre o projeto com cenas, ignora o bloco e **o apaga no próximo salvar**. Subir a versão tornaria todo projeto
   ilegível para os antigos, pior falha. Se o coordenador preferir proteger as cenas, o ponto de mudança é `document::SCHEMA_VERSION`
   + `migrate`, só quando o bloco existir.
2. **Cenas pertencem ao projeto, não à sequência.** Importar sequências de outro projeto não leva cenas; o plano avisa em vez de falhar.
3. **Package incompleto, não "portátil".** Bundle que nomeia arquivos (`assets[].relativePath`, mix) → `missing` com `state:"not_packaged"`;
   sem `allow_partial` o bloqueio é `unsupported` tipado. Com `allow_partial` o documento e as cenas viajam e o pacote diz
   `complete:false`. Todo bundle válido exige assets: nenhum projeto com cenas anexadas empacota completo nesta etapa. Some quando BB05/BB06 empacotarem os recursos.
4. **Tetos.** `MAX_SCENES = 64` e `MAX_SCENE_BYTES = 128 MiB` somados: `persist::save` não confere o tamanho e `persist::load` recusa
   > 256 MiB, então sem o teto seria possível salvar um projeto que ninguém reabre. O teto vale na entrada (anexar/abrir), não em cada
   edição (editar reescreve números no lugar).
5. **Comando em `Command`, não em `Op`.** Uma variante nova de `Op` quebraria o `match` exaustivo de `Workstation::run_op` (desktop,
   não é desta frente). `Command::Scene` não exige alterar o desktop e dá o histórico de graça.
6. **IDs de cena são `String`**, validados com o alfabeto de `Id`; nunca internados (`Id` vive até o fim do processo).

## Validação

**Executado nesta frente:**

- `node scripts/migration-oracle/oracle.mjs verify`: `ok: 55 casos verificados contra a origem (hashes + regeneração byte a byte)`,
  antes de escrever qualquer expectativa. Os testes consomem o corpus pinado (`docs/migration/fixtures`, 4 bundles válidos e
  inválidos) e não geram expectativa nova.
- `rustfmt --check --edition 2024` (binário direto, **somente formatação/sintaxe**) em todos os `.rs` novos e editados: limpo.
  Isso **não** prova que compila nem que os testes passam.

**Escrito e NÃO executado** (o coordenador roda; nenhum resultado é alegado):

| Arquivo | Testes |
|---|---|
| `crates/core/tests/scene_project.rs` | `a_project_keeps_its_scenes_through_save_and_reopen`, `a_project_without_scenes_writes_what_it_always_wrote`, `the_stored_block_is_validated_like_an_attach_and_never_half_opens`, `unknown_fields_survive_attach_edit_save_and_reopen`, `catalog_properties_and_camera_are_queryable_by_id_alone`, `the_camera_of_every_edit_frame_matches_the_origin_oracle`, `the_camera_can_be_asked_by_frame_exact_time_or_clip_frame`, `undo_and_redo_move_a_whole_batch_as_one_step`, `several_scene_commands_inside_a_transaction_are_one_undo_entry`, `a_late_invalid_batch_changes_nothing_and_keeps_index_path_and_issue_codes`, `dry_run_and_no_op_leave_state_and_history_alone`, `attach_runs_the_importer_and_the_structural_gate_before_anything_changes`, `the_temporal_gates_run_on_attach_and_on_every_edit`, `bundles_come_from_an_object_a_text_or_a_file_and_only_one_at_a_time`, `scene_ids_are_caller_chosen_stable_and_bounded`, `a_project_holds_a_bounded_number_of_scenes`, `a_json_lines_session_runs_the_whole_flow_in_process`, `the_headless_executable_answers_exactly_like_the_library` |
| `crates/core/src/scene_project/{mod,scope,store}.rs` | 11 testes unitários (alfabeto de ID = `Id`, IDs gerados, round-trip do bloco, versão futura, entrada ruim com código do importador, blocos malformados, duplicata, famílias adiadas) |
| `crates/media/tests/scene_project_guards.rs` | `a_package_never_claims_to_be_portable_while_a_scene_names_files_it_does_not_carry`, `a_partial_package_keeps_the_scenes_in_its_document_and_reports_the_gap`, `a_project_without_scenes_packages_complete`, `importing_sequences_never_carries_scenes_and_the_plan_says_so` |

Comando sugerido:

```bash
./scripts/cargo test -p tessel-editor-core --lib scene_project
./scripts/cargo test -p tessel-editor-core --test scene_project --test headless --test project
./scripts/cargo test -p tessel-editor-media --test scene_project_guards --test package --test interchange
./scripts/cargo clippy --workspace --all-targets --locked -- -D warnings
```

**Riscos de compilação por eu não ter compilado** (olhe primeiro se algo falhar): o código foi conferido contra as assinaturas reais
(`scene_edit`, `directed_scene`, `CommandError`), mas é código às cegas; possíveis pontos: `large_enum_variant` do clippy em `Command`
(`SceneCommand::Attach` carrega um `Value`) e as asserções
`details.index == 1`/`QUATERNION_NOT_NORMALIZED` copiadas de `scene_edit.rs`. Os testes de pacote assumem que o bundle `valid-minimal-continuous`
(dois PNGs em `assets[]`) é o fixture com recursos, e o caso completo remove a cena do projeto; não existe bundle válido sem assets.

## Superfícies

| Superfície | Estado |
|---|---|
| Lib | Implementada; testes escritos, não executados |
| CLI headless | Implementada pelo mesmo parser/executor; testes (inclusive subprocesso) escritos, não executados |
| Socket / `ui op` da janela | **P**: roteia por `Op::Run`, sem alteração no desktop; **não exercitado**. "Transporte protegido": nenhum teste de socket (exige a janela); o que se provou, quando rodar, é o envelope JSONL (erros tipados, linha inválida, arquivo ilegível/grande demais, rejeições sem efeito) |
| UI visual | **P**: sem controle, gesto, árvore ou inspetor |

## Limites e dependências

- **Dependência de F (`directed_scene`)**: os testes de câmera usam `evaluate_camera`/`camera_at_clip_frame`/`frame_at_time`, que já
  existem; a F ainda está mudando `directed_scene/*` no mesmo checkout (animação de nós). Se ela mudar as chaves do JSON de câmera
  (`FrameEvaluation::to_json`), `scene.camera` deve seguir; hoje reproduz as mesmas chaves.
- **Dependência de BB05/BB06**: recursos por caminho relativo (imagens, mix) não são resolvidos, copiados nem empacotados; o import de
  sequência não leva cenas; render/export de cena dirigida não existe.
- Sem paginação em `scene.properties`; `scene.describe` lista todos os nós.
- A cena anexada **não** é `tessel-scene@1` e **não** habilita o runtime do bundle legado.
- Comentários e mensagens de erro do código estão em inglês, como o resto do core; a documentação está em português.

## Linha proposta para a matriz (não editei `docs/features/README.md`)

```markdown
| F036 | [Cenas dirigidas no projeto: documento, histórico e acesso programático](scene-project.md) | P | P | I¹³ | I¹³ | `scene.attach|detach|edit|list|describe|catalog|properties|get|camera|capabilities`; bloco `scenes` de `.tessel.json` |
```

```markdown
¹³ Anexar bundle por ID, consultar catálogo/propriedades/câmera, lote com dry-run, undo/redo em uma entrada e salvar/reabrir
preservando campos desconhecidos, pela lib e por `tessel-headless` com o mesmo parser. Testes escritos e **não executados** quando
esta linha foi proposta; só vira V depois do `cargo test` do coordenador. Socket/`ui op` e UI visual P. Package de projeto com cena que
nomeia arquivos fica incompleto (`unsupported` sem `allow_partial`) até BB05/BB06; o import de sequências não leva cenas. Bloco aditivo:
editor anterior ignora e apaga as cenas ao salvar. [Contrato](scene-project.md).
```

Colunas da matriz: UI **P**, CLI da janela **P** (nenhuma verificação de `ui op`), CLI headless **I**, Lib **I**.

## Revisão pelo coordenador

As limitações de versão e tamanho descritas acima foram corrigidas na integração: schema 2
somente com cenas; schema 1 continua para projetos comuns. Leitores antigos recusam 2.
O limite de cenas também vale nas edições, e persistência/pacote limitam a serialização
real, sem alocar/escrever acima do teto. Testes adicionais cobrem essas condições.
O aviso de cenas omitidas no intercâmbio passou ao core, evitando divergência entre
biblioteca e adapter. Cargo oficial foi executado pelo coordenador; status final no PR.

As 4 guardas de mídia passaram após corrigir a criação da pasta de conteúdo de pacote
parcial sem mídia. O teste que removia todos os assets do bundle foi corrigido: o schema
original exige ao menos um asset, então esse bundle deve ser recusado, não habilitado.
O caso de pacote completo sem cenas continua coberto.

Validação oficial final do coordenador: workspace com **522 testes passando,
zero falhas e 2 ignorados** (ZIP64 de 4,5 GiB); clippy com warnings negados e
formatação passaram. Os 19 testes públicos de cena incluem executável headless real;
as 4 guardas de mídia também passaram. UI visual/socket continuam sem prova específica.
