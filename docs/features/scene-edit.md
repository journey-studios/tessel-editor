# F034 — Catálogo de propriedades e edição atômica da cena dirigida (BB04 mínimo)

## Status

Lib **Parcial**: implementada e validada pelo coordenador via Cargo, com 29 testes públicos
e 1 doctest. O lote usa importação e os gates estruturais/temporais compartilhados. CLI headless **P** (nenhum comando
`scene.*`), UI **P** (nenhum controle, nenhum nó em `tree.rs`), ligação com `EditorState::transaction` /
`document.rs` **P**. O que existe é o **núcleo puro** sobre
[`DirectedScene`](directed-scene.md): catálogo, operações por ID, lote atômico, dry-run.

## Objetivo e limites

Dar a BB07/BB08 (inspector, overlays, CLI/agentes) uma única porta de edição para a cena dirigida, sem que
cada adaptador reimplemente tipos, limites ou regras de integridade (BB04 de
[phased-roadmap](../migration/phased-roadmap.md)).

Escopo desta fatia (decidido na rodada 2, frente E):

- transformação de nó: `position`, `rotation`, `scale`;
- medidas de plano de imagem: `width`, `height`;
- parâmetros básicos de câmera que o modelo já tem: pose inicial do clipe (`start/position`, `start/target`,
  `start/fov`).

Fora, de propósito: luz, clima, efeitos, rig, caminhos, keys/segmentos de câmera (BB03), `renderOrder`,
criar/remover/renomear entidades, setter genérico de JSON. O catálogo cresce acrescentando entradas.

O candidato passa pelo **importador** (subconjunto BB01/BB02), pelo gate estrutural
`edit_join_issues()` e pelo temporal `temporal_issues()` (BB03), dentro de `apply`/`dry_run`.
Os adapters não precisam repetir esses gates. Isso não certifica render nem domínios adiados
como luz, clima e performance de nós. Uma entrada importável com junção inválida não produz
lote aceito, nem vazio, enquanto o candidato não reparar os problemas. Não há histórico novo: o candidato é a unidade de edição para a
integração futura com `EditorState::transaction`.

## Contrato do core / biblioteca

```rust
use tessel_editor_core::directed_scene::{DirectedScene, Vec3};
use tessel_editor_core::scene_edit::{
    EditOp, NodeProperty, CameraProperty, PropertyRef, PropertyValue, apply, dry_run,
};

let scene = DirectedScene::import_str(text)?;
let ops = [
    EditOp::set(
        PropertyRef::node("street", "hero-plane", NodeProperty::Position),
        PropertyValue::Vec3(Vec3 { x: 4.0, y: 1.0, z: 0.0 }),
    ),
    EditOp::set(
        PropertyRef::camera("wide", CameraProperty::StartFov),
        PropertyValue::Number(40.0),
    ),
];
let preview = dry_run(&scene, &ops)?;   // só o resumo; nada é criado
let outcome = apply(&scene, &ops)?;     // candidato + resumo; `scene` nunca muda
assert_eq!(preview, outcome.summary);
```

| Item | Contrato |
|---|---|
| `catalog()` | As 8 entradas (`PropertySpec`) em ordem estável: `id`, `name`, `kind`, `unit`, `range`, `default`, `applies_to`, `document_field`, `origin` |
| `PropertyRef` | `Node { scene_id, node_id, property }` ou `Camera { clip_id, property }`; `path()` canônico, `parse(&str)` |
| `PropertyValue` | `Number(f64)`, `Vec3`, `Quaternion` (tipos de `directed_scene`); `from_json(kind, &Value)` estrito, `to_json()` |
| `EditOp` | `Set { property, value }`, `Reset { property }`; `EditOp::from_json` (`{"set","value"}` / `{"reset"}`), `parse_batch` |
| `apply(&scene, &ops)` / `apply_with(.., &Limits)` | `Result<EditOutcome { candidate, summary }, EditError>` |
| `dry_run(&scene, &ops)` / `dry_run_with` | `Result<EditSummary, EditError>`; mesma validação, sucede **sse** `apply` sucede, mesmo resumo |
| `EditSummary` | `changes: Vec<PropertyChange { index, property, before, after }>`, `modified`, `changed_count()` |
| `properties_of(&scene)` / `read_property` | Lista o que se aplica, com valor atual, sem seleção; leitura por ID com os mesmos erros de ID/aplicabilidade |
| `EditError` | `code`, `message`, `index`, `path`, `id`, `issues()` (achados do importador), `cause()`; `From<EditError> for CommandError` |

### Catálogo

| Path | Tipo / unidade | Default (`reset`) | Faixa | Aplica a | Campo no bundle | Relação com a origem |
|---|---|---|---|---|---|---|
| `scene/<s>/node/<n>/position` | vec3 / unidades de cena | `(0,0,0)` | — | todo nó | `localPosition` | só editor |
| `scene/<s>/node/<n>/rotation` | quaternion `{x,y,z,w}` / sem unidade | identidade | — (comprimento ≈ 1 é regra do importador) | todo nó | `localRotation` | só editor |
| `scene/<s>/node/<n>/scale` | vec3 / multiplicador | `(1,1,1)` | — | todo nó | `localScale` | só editor |
| `scene/<s>/node/<n>/width` | número / unidades de cena | nenhum | `> 0` | `imagePlane` | `widthSceneUnits` | só editor |
| `scene/<s>/node/<n>/height` | número / unidades de cena | nenhum | `> 0` | `imagePlane` | `heightSceneUnits` | só editor |
| `camera/<c>/start/position` | vec3 / unidades de cena | nenhum | — | clipe | `initialPosition` | **mesmo path** da origem, escrita mais estreita |
| `camera/<c>/start/target` | vec3 / unidades de cena | nenhum | — | clipe | `initialTarget` | comando `setStart.target` da origem; path é deste catálogo |
| `camera/<c>/start/fov` | número / graus | nenhum | `(0, 180)` | clipe | `initialFovDeg` | comando `setStart.fovDeg` da origem; path é deste catálogo |

**Vocabulário da origem.** A gramática é a de `properties/property-paths.ts`
(`scene/<id>/...`, `camera/<id>/...`, IDs com `encodeURIComponent`) e os códigos de erro reutilizam os de
`PropertyErrorCode` (`UNKNOWN_PATH`, `UNKNOWN_SCENE`, `UNKNOWN_CLIP`, `INVALID_VALUE`, `NOT_APPLICABLE`).
A origem **não** tem paths de nó (ela só edita nós reescrevendo o bundle), então os de nó são deste editor,
escritos na mesma gramática. IDs de nó são únicos só dentro da cena: o path sempre carrega a cena.

**Divergência declarada em `camera/<c>/start/position`.** A origem primeiro converte o primeiro movimento do
clipe em keyframes para o resto da trajetória não se mover junto; aqui só `initialPosition` muda, e movimentos
de receita (relativos à pose inicial) acompanham. Preservar a trajetória exige avaliação temporal (BB03).
`PropertySpec::origin` (`EditorOnly` / `CommandOnly` / `PathNarrowerWrite`) torna isso consultável.

**Defaults.** São o valor neutro que `reset` escreve, não um default do schema (o schema não tem). Onde não existe
valor neutro (tamanho de plano, pose de câmera) `reset` é recusado com `NO_DEFAULT`, nunca adivinhado.

### Semântica do lote

1. Para cada op, em ordem: o path/IDs resolvem (`UNKNOWN_SCENE`, `UNKNOWN_NODE`, `UNKNOWN_CLIP`), a propriedade
   se aplica ao nó (`NOT_APPLICABLE`), e o valor tem o tipo do catálogo (`INVALID_TYPE`), é finito e está na
   faixa (`INVALID_VALUE`); `reset` precisa de default (`NO_DEFAULT`). A primeira falha encerra o lote com
   `index` (posição da op), `path` e, quando é o caso, `id`. Ops posteriores enxergam o que as anteriores
   escreveram (`before` do resumo).
2. Depois da última op, o documento candidato inteiro volta por `DirectedScene::import_value_with`. As regras de
   integridade **não são duplicadas aqui**: quaternion de comprimento ≈ 1, matriz mundial finita, regras de
   câmera (`CAMERA_DIRECTION_DEGENERATE`) etc. continuam do importador. Os achados voltam como
   `CANDIDATE_REJECTED` com cada `Issue` (JSON Pointer) e `ops`: as ops que alteraram o nó/clipe dentro do qual o
   ponteiro está. `ops` vazio = consequência fora do que foi tocado (ex.: escalar um pai a ponto de estourar a
   matriz dos filhos) e `index` fica `None`.
   Depois da importação, `edit_join_issues()` e `temporal_issues()` validam o candidato inteiro.
   Achados estruturais/temporais também geram `CANDIDATE_REJECTED`; os `ops` podem estar vazios
   quando a consequência aponta uma junção em vez do clipe alterado.
3. Atomicidade: a entrada é `&DirectedScene`; qualquer falha deixa a cena do chamador intacta e não existe
   candidato. Uma op que escreve o valor atual não toca o documento (preserva a grafia dos números);
   componentes inalterados de um vetor e chaves desconhecidas dentro dele sobrevivem.
4. `modified` é semântico (alguma propriedade terminou diferente de como começou); "alterar e restaurar" dá
   `false`, embora o JSON do candidato possa grafar um número restaurado diferente (`1` × `1.0`).
5. Números inteiros são gravados como inteiros (`16`, não `16.0`), como `JSON.stringify` na origem.

### Erros

| Código | Quando |
|---|---|
| `UNKNOWN_PATH` | path fora da gramática/catálogo (a mensagem lista os nomes conhecidos) |
| `UNKNOWN_SCENE` / `UNKNOWN_NODE` / `UNKNOWN_CLIP` | ID inexistente (`id` no erro) |
| `NOT_APPLICABLE` | `width`/`height` em nó que não é `imagePlane` |
| `INVALID_TYPE` | valor com forma errada (JSON estrito: chaves exatas, sem extras) ou `PropertyValue` de outro tipo |
| `INVALID_VALUE` | não finito ou fora da faixa |
| `NO_DEFAULT` | `reset` sem default no catálogo |
| `INVALID_OP` | op JSON que não é `{set,value}` nem `{reset}` |
| `CANDIDATE_REJECTED` | ops válidas, mas importador ou gates estruturais/temporais recusam o candidato (`issues()`, `cause()`) |
| `DOCUMENT_MISMATCH` | visão tipada e documento discordam da posição de uma entidade (não ocorre em cena importada) |

`CommandError`: `unknown_path`, `unknown_scene`, `unknown_node`, `unknown_clip`, `not_applicable`, `no_default`,
`invalid_op`, `invalid_type`, `invalid_value`, `document_mismatch`; um candidato recusado usa o código do
importador (`invalid_bundle`). `target` = ID ausente; `details` = `index`, `path`, `issues[].ops`.

## CLI headless

**P.** Nenhum comando ainda. Forma pretendida: `scene.edit` / `scene.describe` sobre `parse_batch`, com o mesmo
`EditError`→`CommandError`; sem socket de janela. Não declarar paridade antes disso.

## UI e CLI da janela

**P.** Inspector (BB08) deve listar `properties_of` e editar via `EditOp`; gesto de arrastar vira `Set` por ID
(BB07). Nada disso existe. Apresentação (foco, docking) é N/A para este módulo.

## Evidências e status

| Superfície | Estado |
|---|---|
| Lib, gate oficial | **Validado pelo coordenador:** `./scripts/cargo test -p tessel-editor-core --locked --test scene_edit` (29/29), com os gates da D ligados; clippy e formatação passam. Ver relatório de integração para workspace/build. |
| Lib, verificação estática desta frente | Fora do Cargo, sobre uma cópia em `/tmp` (HEAD `d7b405d` do core + o módulo): `clippy-driver -W clippy::all -D warnings` limpo (lib e testes); `rustfmt --check` limpo; 26/26 testes de `crates/core/tests/scene_edit.rs` e o doctest passaram em build debug direto com `rustc`. Não é o gate: não usa `Cargo.lock`, não roda o workspace e foi contra o `directed_scene` do HEAD, não contra a árvore em movimento da frente D |
| CLI headless / UI | **P** |

Cobertura de `crates/core/tests/scene_edit.rs` (API pública, fixture sintética `layered_scene.json`
adaptada com corte explícito na junção para formar baseline temporal válido): catálogo
(tipo/unidade/default/faixa/aplicabilidade/origem; defaults válidos); paths (gramática, round-trip, IDs com `/`
e espaço, literal × `%XX`, escapes inválidos, nomes desconhecidos); `properties_of`/`read_property` sem seleção;
lote válido (candidato, resumo antes/depois, matriz mundial já refletindo a edição, entrada intacta); dry-run
igual ao `apply` e imutável; falha tardia com rollback e primeira falha vence; ID ausente por tipo;
tipo errado e faixa (typed e JSON) com índice e path; `NOT_APPLICABLE`; `reset` com e sem default; achados do
importador atribuídos à op (quaternion, câmera) e consequência sem op (matriz que estoura); referências,
campos desconhecidos e restante do documento preservados (candidato == original + exatamente os campos
editados); op sem efeito deixa o documento idêntico; ops posteriores veem as anteriores e restaurar não é
modificar; lote vazio; round-trip por texto; `Limits`; concordância das faixas do catálogo com o importador
nas bordas (fov `0`/`180`, largura `0`); conversão para `CommandError`.

Linha da matriz e relatório: [foundation-e-report](../migration/foundation-e-report.md).

Três regressões adicionais falharam antes da integração do gate e passaram depois:
lote sobre entrada temporalmente quebrada, entrada com junção estrutural inválida e edição
que deixa o FOV composto fora do limite. `dry_run` e `apply` rejeitam igualmente e preservam
a entrada. A fixture da A não foi modificada; a adaptação de corte é local ao helper de testes.
