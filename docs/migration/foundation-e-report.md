# Relatório da frente E — BB04 mínimo (catálogo de propriedades e edição atômica)

2026-11-05. Rodada 2 da base de migração ([plano](parallel-implementation-plan.md),
[roadmap](phased-roadmap.md)). Checkout `tessel-editor` em `d7b405d` (A/B integradas em `72fa715`, oráculo B
`f18b1cf`). Esta frente não fez `git add`/commit/push/switch/reset/clean/merge, não alterou `Cargo.toml`/`Cargo.lock`,
`directed_scene/*`, `ops`/`state`/`document`/`headless`/desktop/runtime, a matriz nem índices, e não modificou
`tesselapp` nem a origem. As alterações em curso das outras frentes (D em `directed_scene/`, C em `runtime/`)
foram preservadas.

## Entrega

Módulo `tessel_editor_core::scene_edit`: **catálogo concreto de propriedades** de nó/câmera e **lote atômico
por IDs** com dry-run sobre um `DirectedScene` imutável. Contrato, catálogo, semântica do lote e erros em
[`docs/features/scene-edit.md`](../features/scene-edit.md) (F033 proposta).

### Arquivos (todos dentro do ownership)

- `crates/core/src/lib.rs` — apenas `pub mod scene_edit;`
- `crates/core/src/scene_edit/`
  - `mod.rs` — API pública e reexports, doctest
  - `catalog.rs` — `PropertySpec` (path/nome, tipo, unidade, faixa, default, aplicabilidade, campo do bundle,
    relação com a origem), as 8 entradas, `catalog()`
  - `reference.rs` — `PropertyRef` (por IDs) e a gramática de paths (`encodeURIComponent`, como a origem)
  - `value.rs` — `PropertyValue` (`Number`/`Vec3`/`Quaternion`), decodificação JSON estrita, escrita que preserva
    chaves desconhecidas e a grafia de números inalterados
  - `op.rs` — `EditOp` (`Set`/`Reset`), `EditOp::from_json` (`{"set","value"}`/`{"reset"}`), `parse_batch`
  - `batch.rs` — `apply`/`apply_with`/`dry_run`/`dry_run_with`, resumo, `properties_of`/`read_property`
  - `error.rs` — `EditError`, `LocalizedIssue`, `From<EditError> for CommandError`
- `crates/core/tests/scene_edit.rs` — 26 testes públicos (usa a fixture sintética `directed_scene_fixtures/layered_scene.json`
  da frente A, só leitura)
- `docs/features/scene-edit.md`, `docs/migration/foundation-e-report.md`

### Decisões

- **Vocabulário da origem onde corresponde, declarado onde não.** Li `properties/property-paths.ts`,
  `camera-properties.ts`, `types.ts`, `define.ts` e `edit/edit-bundle.ts` (snapshot `d6adac15`, somente leitura).
  Gramática e códigos de erro (`UNKNOWN_PATH`, `UNKNOWN_SCENE`, `UNKNOWN_CLIP`, `INVALID_VALUE`, `NOT_APPLICABLE`)
  são os dela. A origem **não tem paths de nó**; os de nó são deste editor na mesma gramática. `camera/<c>/start/position`
  é o mesmo path da origem mas com escrita mais estreita (a origem converte o 1º movimento em keys antes); `start/target`
  e `start/fov` correspondem ao comando `setStart`, sem path na origem. `PropertySpec::origin` registra isso.
- **Sem setter livre.** Uma op nomeia uma entrada do catálogo; o JSON é decodificado pelo tipo dela (chaves exatas,
  sem extras). Não há caminho para gravar um campo fora do catálogo.
- **Sem regras de integridade duplicadas.** O candidato volta por `DirectedScene::import_value_with`; o catálogo só
  declara tipo/finitude/faixa (o que um slider precisa) e um teste fixa que as faixas coincidem com o importador nas
  bordas. Quaternion de comprimento ≈ 1, matrizes finitas, regras de câmera etc. continuam do importador e voltam
  como `CANDIDATE_REJECTED` com `Issue` (JSON Pointer) e as ops que tocaram a entidade.
- **Sem histórico paralelo.** O resultado é um `DirectedScene` candidato + resumo; a integração com
  `EditorState::transaction` (um lote = um passo de undo) é da etapa seguinte.
- **Escopo do catálogo** conforme a rodada: `position`/`rotation`/`scale`, `width`/`height` (só `imagePlane`),
  pose inicial da câmera. Rotação é **quaternion** (o que o bundle guarda), não Euler. `renderOrder`, componentes
  `position/x|y|z`, luz/clima/rig e keys de câmera ficaram de fora.
- Reuso: `directed_scene::{Vec3, Quat, NodeKind, Entity, Limits, ImportError, Issue}`, `CommandError`
  (`From<EditError>`), códigos `INVALID_TYPE`/`INVALID_VALUE` do importador. Nenhuma alteração em `directed_scene`.

## Verificação

**Gate oficial: não executado** (a frente D é dona da compilação nesta rodada; instrução de não rodar Cargo).
Comandos para o coordenador:

```sh
MBX_GC_MAX_TOTAL_SIZE=64GiB ./scripts/cargo test -p tessel-editor-core --locked --test scene_edit
MBX_GC_MAX_TOTAL_SIZE=64GiB ./scripts/cargo test -p tessel-editor-core --locked          # + doctest scene_edit
./scripts/cargo clippy -p tessel-editor-core --all-targets --locked -- -D warnings
cargo fmt --all -- --check
./scripts/cargo tree -p tessel-editor-core --locked                                       # sem gpui/wry; deps inalteradas
```

**Verificação estática feita por esta frente (não é o gate):** sem Cargo, sobre uma cópia temporária em `/tmp`
(`git archive HEAD crates/core` + o módulo e o teste atuais), usando os `.rlib` de dependências que já estavam em
`target/debug/deps` (somente leitura) e `RUSTUP_TOOLCHAIN=1.99.1`:

| Verificação | Resultado |
|---|---|
| `rustfmt --edition 2024` nos arquivos próprios (`mod.rs` segue os submódulos; teste) | formatado e `--check` limpo |
| `rustc --emit=metadata` da lib e `clippy-driver -W clippy::all -D warnings` da lib e do teste | limpo (a 1ª rodada achou `result_large_err` em `EditError` e `approx_constant` no teste; ambos corrigidos) |
| `rustc --test` direto (debug, `/tmp`) e execução de `scene_edit` | **26 passed, 0 failed** |
| `rustdoc --test` com filtro `scene_edit` | 1 doctest passou |

Limites desta evidência: build contra o `directed_scene` do **`d7b405d`** (o HEAD do checkout passou a `6de7a5d`
durante o trabalho; `lib.rs` tem só a minha linha sobre ele) (a árvore da D estava em movimento e não foi
usada); sem `Cargo.lock`/features do workspace; não rodou o resto da suíte do core. Por isso a coluna Lib fica
**Parcial** e não V. Temporários: ~33 MiB em `/tmp`, removidos ao fim; disco livre ~9,6 GiB (piso 6 GiB respeitado);
sem GPU/Chrome, sem `mbx clean/gc`, `HOME`/`RUSTUP_HOME`/`CARGO_HOME` inalterados.

## Contratos para integração (o que o coordenador/D/BB07 consomem)

- API pública: `scene_edit::{catalog, properties_of, read_property, apply, apply_with, dry_run, dry_run_with,
  parse_batch, EditOp, PropertyRef, PropertyValue, EditOutcome, EditSummary, PropertyChange, PropertyInfo,
  EditError, LocalizedIssue, NodeProperty, CameraProperty, PropertySpec, ...}`. Mudanças em `DirectedScene` que
  quebrem `bundle().scenes[i].nodes()` / `camera_clips` em ordem de documento são detectadas por
  `DOCUMENT_MISMATCH` em vez de editar o nó errado.
- **Gate temporal (D).** O candidato é um `DirectedScene`; basta rodar o gate de D em `outcome.candidate` (e falhar o
  lote se reprovar) antes de entregar ao histórico. Hoje só vale a importação; um teste confirma que
  `edit_join_issues()` é igual antes/depois de edições de nó, e nada afirma continuidade de pose. A fixture
  `layered_scene.json` reprova `finalizeBundle` por `CONTINUOUS_VELOCITY_JUMP` na origem: um candidato dela não é
  "válido para render".
- **Histórico (`EditorState::transaction`).** Um lote = um candidato = uma transação; o resumo traz `before`/`after`
  por path para o passo de undo. `modified == false` permite não registrar passo vazio.
- **Documento/CLI/UI.** Pendentes: comando `scene.*`, `./scripts/headless`, nó em `state/tree.rs`, inspector. Os
  adaptadores devem traduzir para `EditOp`/`parse_batch` e apresentar `EditError`/`CommandError`, sem regra própria.

## Lacunas conhecidas

- Nenhuma execução pelo gate oficial; nenhuma integração com `document.rs`, histórico, headless ou desktop.
- Catálogo mínimo: falta Euler/graus para rotação (UI), componentes escalares, `renderOrder`, luz/clima/rig, keys
  e segmentos de câmera, criação/remoção/renomeação de entidades, reparent.
- `camera/<c>/start/position` não preserva a trajetória como a origem (precisa de BB03); registrado em `origin`.
- Atribuição de achados às ops é por entidade (nó/clipe), não por campo, e não segue descendentes: consequência em
  filhos aparece com `ops` vazio e o ponteiro do achado.
- Sem limite de tamanho do lote além de `Limits` do importador sobre o candidato; chamador não confiável deve limitar
  `parse_batch`.
- `EditOutcome` clona o documento inteiro por lote (clareza sobre desempenho; documento grande + muitos lotes
  pequenos pode pedir edição incremental depois).
- A fixture usada é a da frente A (somente leitura); se A/D a alterarem, os testes dependem de IDs `street`/`wide`,
  nós `root, stage, bg-plane, hero-plane, prop-plane` e dos valores citados.
- Divergência de tipo `modified` × grafia do JSON ao restaurar um valor (`1` × `1.0`), documentada.

## Linha exata para a matriz (`docs/features/README.md`)

```
| F033 | [Catálogo de propriedades e edição atômica da cena dirigida (BB04 mínimo)](scene-edit.md) | P | P | P | Parcial¹¹ | `scene_edit::{catalog, apply, dry_run}` sobre `DirectedScene`; sem comando/headless/UI nem histórico ainda |
```

¹¹ Lib implementada e coberta por 26 testes públicos + 1 doctest, **executados só fora do Cargo** (cópia do HEAD
em `/tmp`, clippy/fmt limpos); gate oficial `./scripts/cargo test -p tessel-editor-core --locked` pendente do
coordenador (promover para V só após rodá-lo na árvore integrada com D). Valida tipo/finitude/faixa/ID e delega a
integridade ao importador; **não** é certificação temporal nem de render. O número `¹¹` é sugestão; o coordenador
renumera conforme as notas existentes (e o ID F033, se D usar outro).

## Integração e revisão pelo coordenador

O coordenador ligou o gate estrutural e `temporal_issues()` de D diretamente ao pipeline
de `apply_with`, reutilizado por `apply` e `dry_run`. Não há regra adicional em adapter.
Três regressões novas falharam antes e passaram depois: entrada com salto temporal,
junção estrutural quebrada e mudança válida isoladamente que quebra FOV composto.
As três comprovam rollback; `dry_run` rejeita pelo mesmo erro que `apply`.

Os testes usam uma adaptação local da fixture A com corte explícito para formar um
baseline temporal válido; a fixture A original permanece intacta. Cargo oficial:
`./scripts/cargo test -p tessel-editor-core --test scene_edit --locked`: **29/29**.
O ID consolidado é F034, nota ¹²; F033 ficou com avaliação temporal.
Histórico, documento e adapters permanecem pendentes, sem alegar paridade.

Validação conjunta final: `fmt`, `check --workspace`, `clippy --workspace --all-targets -D warnings`
e `test --workspace --locked` com runtime obrigatório passaram: **453 testes, 0 falhas,
2 ignorados** (ZIP64 4,5 GiB). Build release passou. O release recém-compilado passou
nas 147 checagens de lifecycle, com export de 12 quadros idêntico antes/depois.
Relatório consolidado: [foundation-integration-report](foundation-integration-report.md).
