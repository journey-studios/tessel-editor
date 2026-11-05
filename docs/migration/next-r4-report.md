# R4 — replace de pacote atômico (BL088)

Relatório da frente R4 da [próxima entrega](next-delivery-ten-agents.md). Estado de execução vive só em
[`docs/backlog.md`](../backlog.md); aqui ficam diagnóstico, contrato e evidências. Contrato da feature: [F026](../features/package.md).

## 1. Diagnóstico (antes do patch)

Código lido: `crates/media/src/package.rs`, `create_with`, trecho entre `verify_package(&staged)` e a publicação.
O `replace:true` fazia, nesta ordem: (1) `remove_dir_all`/`remove_file` do destino; (2) `fs::rename` do staging
sobre o destino. Qualquer erro em (2), ou um erro parcial em (1), deixa o pacote anterior destruído; a promessa
de F026 ("destino anterior preservado") só valia para falhas **antes** de (1).

Reprodução, sobre o código original com apenas o ponto de injeção `PublishFs` adicionado (sem mudança de
comportamento), `./scripts/cargo test -p tessel-editor-media --test package_atomic` por `cargo-serial.py`:

| Falha | Resultado observado no código original |
|---|---|
| `rename` do staging falha (Directory) | destino `None`: pacote anterior apagado |
| `rename` do staging falha (Archive) | destino `None`: arquivo anterior apagado |
| pacote anterior com subpasta não removível (0555), **falha real de FS, sem injeção, pela operação pública `project.package`** | `permission_denied`; pacote anterior meio apagado (`verify` → `invalid_package`) |
| pai sem permissão de escrita (controle) | `permission_denied` já no staging; anterior intacto (não era bug) |
| cancelamento antes/ao chegar em `verify` (controle) | anterior intacto (não era bug) |
| `replace` com destino inexistente / sem `replace` (controle) | igual ao comportamento esperado |

Causa: remover antes de publicar. Não é problema de staging nem de lease.

## 2. Correção

`package::publish` (novo, privado) substitui o apagar-depois-renomear. Tabela de passos, falhas e garantias em
[F026 § Replace atômico](../features/package.md). Resumo: o pacote anterior só é apagado **depois** que o novo
(completo, verificado) está no destino; archive sobre archive é um único `rename`; pasta ou troca de tipo usa
(1) mover o anterior para `.tessel-previous-<pid>-<n>`, (2) publicar, (3) apagar o anterior, com (1) desfeito se (2) falhar.
Reaproveitado: `Staging`, `io_error` (códigos `permission_denied`/`disk_full`/`io_error`), `verify_package`, `ctx.check()`.
Não mexi em `fsx.rs`, `scene_resources/pack.rs`, `host.rs`.

### APIs/seams

- Inalterados: `PackageOptions`, `PackageInputs`, `preflight`, `preflight_with`, `create`, `create_with`, `restore`,
  `verify_package`, formato do resultado de `project.package` (só `warnings` pode ganhar uma linha).
- Aditivo `#[doc(hidden)]`: `trait PublishFs { fn rename }`, `struct StdFs`, `create_with_fs(.., &dyn PublishFs)`.
  `create_with` chama `create_with_fs(.., &StdFs)`. É o único syscall do passo de publicação que não dá para falhar sob
  demanda com FS real; staging, hash e verificação rodam de verdade.
- Detalhes novos de erro (aditivos): `details.previous_package`, `previous_restored`, `restore_error`.
- Consumidores R1/R5 (`create_with`, `PackageInputs`, `pack::*`) não precisam de mudança.

## 3. Testes

Executados (por `cargo-serial.py`, 2026-11-05):

- `cargo test -p tessel-editor-media --test package_atomic`: **10/10** (antes do patch: 6 falhas e 4 controles verdes).
  Falha de FS real: pai 0555, subpasta 0555 no pacote anterior, cancelamento pré-job e em corrida (8 rodadas por formato),
  pela lib e pela operação pública `project.package`/`project.verify` do `MediaHost` (JSON, `headless::execute_ext`).
  Falha injetada só do `rename` de publicação/colocar-de-lado/desfazer (Directory e Archive; `permission_denied`, `disk_full`, `io_error`).
- Relacionados: `package` 4/4, `scene_project_guards` 4/4, `scene_resource_integration` 19/19, `scene_resource_publication` 4/4,
  `interchange` 2/2, `flow` 2/2 (88 s, FFmpeg real), `--lib` 31 passaram/1 ignorado (ZIP64 4,5 GiB, não executado).
- `clippy -p tessel-editor-media --all-targets --locked -- -D warnings`: limpo.
- Processo real `target/debug/tessel-media` (JSONL, sem janela): replace repetido OK; sem `replace` → `destination_exists`;
  pai 0555 → `permission_denied` e `project.verify` do pacote anterior OK; subpasta 0555 no anterior → sucesso com
  `warnings` apontando `.tessel-previous-…` e novo pacote verificado.

Não executados: teste ZIP64 de 4,5 GiB (proibido pela tarefa); core e desktop (sem mudança); Windows/Linux (BL054);
UI nativa (o diálogo de empacotar não envia `replace` e não foi alterado; `ui op` aceita o mesmo JSON, sem prova própria);
workspace completo (gate do coordenador).

## 4. Limitações

- Entre os dois `rename` de pasta (ou troca de tipo) o destino não existe; não é atômico contra escritor hostil/concorrente
  nem à prova de queda do processo (o anterior fica em `.tessel-previous-…`, recuperação manual). Só archive→archive troca em um passo.
- Resto de pacote antigo não apagável fica parcial em `.tessel-previous-…` (reportado em `warnings`); nada o remove depois.
- Cancelamento não é observado entre os `rename` finais (por desenho).
- Sem teste em Windows (semântica de `rename` sobre pasta/arquivo aberto é outra); nenhum limite foi alterado.
- O sucesso do `replace` ainda exige espaço para novo + anterior ao mesmo tempo (o anterior continua no disco durante o staging, como antes).
- Rodei `cargo fmt -p tessel-editor-media` uma vez por engano (formata o crate todo): pode ter só reformatado arquivos de R1/R5
  sob `crates/media` (sem mudança semântica; não tenho baseline para confirmar). Depois usei só `rustfmt --check` nos meus arquivos.
- A compilação do core falhou duas vezes por WIP alheio (R3 `gesture.rs` E0521; depois R6 `NodeTimeline` E0477). Só repeti meu
  próprio comando até passar, sem editar esses arquivos.

## 5. Patches propostos ao coordenador

**backlog** (linha BL088 existente, sem ID novo): Estado **A validar**, Responsável R4. Aceite/evidência: falha injetada e
real mantêm o anterior legível e verificável (Directory/Archive); novo publicado completo; resto não apagável é avisado;
`package_atomic` 10/10, testes relacionados verdes, CLI `tessel-media` real; ver [next-r4-report](next-r4-report.md).
Windows e queda do processo entre os dois `rename` ficam fora da garantia (reusar BL054 e BL079, sem criar ID).
Marcar **Concluído** só após o gate global (a frente não rodou o workspace).

**matriz** (`docs/features/README.md`, F026): lib V, CLI headless/`scripts/media` V (processo real, `project.package`),
UI inalterada (replace não exposto no diálogo; sem prova nova). Nota: "replace preserva o anterior (BL088)".

**Arquivos da frente alterados**: `crates/media/src/package.rs`, `crates/media/tests/package_atomic.rs` (novo),
`docs/features/package.md`, `docs/migration/next-r4-report.md`.
