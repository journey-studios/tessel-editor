# F034 — Catálogo de propriedades e edição atômica da cena dirigida (BB04 mínimo)

## Status

Lib **Parcial**, com a fatia S6 (BL004/BL005) **a validar** até o coordenador conferir o head da PR.
O núcleo puro sobre [`DirectedScene`](directed-scene.md) (catálogo, operações por ID, lote atômico, dry-run)
foi ampliado em S6 com componentes `X/Y/Z`, `renderOrder` e a escrita de pose inicial de câmera equivalente à
origem. O lote usa importação e os gates estruturais/temporais compartilhados. CLI headless e janela usam os
comandos `scene.*` de [scene-project](scene-project.md) e o inspector de [scene-inspector](scene-inspector.md); esta
página descreve só o contrato do catálogo/lote. Fica de fora um editor geral de keyframes.

## Objetivo e limites

Dar a BB07/BB08 (inspector, overlays, CLI/agentes) uma única porta de edição para a cena dirigida, sem que
cada adaptador reimplemente tipos, limites ou regras de integridade (BB04 de
[phased-roadmap](../migration/phased-roadmap.md)).

Escopo (primeira fatia + S6):

- transformação de nó: `position`, `rotation`, `scale`, e cada eixo de `position` e `scale` isoladamente
  (`position/x` ... `scale/z`);
- plano de imagem: `width`, `height` e `renderOrder` (inteiro, opcional no bundle);
- pose inicial do clipe de câmera: `start/position`, `start/target`, `start/fov`, e cada eixo de posição e alvo
  (`start/position/x` ... `start/target/z`).

Fora, de propósito: luz, clima, efeitos, rig, caminhos, **editor geral de keys/segmentos** (a única escrita que toca
movimentos é a conversão do primeiro movimento descrita abaixo), criar/remover/renomear entidades, setter genérico
de JSON. O catálogo cresce acrescentando entradas, sempre ao final (as oito primeiras e sua ordem não mudam).

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
| `catalog()` | As 21 entradas (`PropertySpec`) em ordem estável (as 8 originais, `renderOrder`, depois os 12 componentes): `id`, `name`, `kind`, `unit`, `range`, `default`, `applies_to`, `document_field`, `origin`, `component`, `optional`, `converts_first_movement` |
| `PropertyRef` | `Node { scene_id, node_id, property }` ou `Camera { clip_id, property }`; `path()` canônico, `parse(&str)` |
| `PropertyValue` | `Number(f64)`, `Integer(i64)`, `Vec3`, `Quaternion` (tipos de `directed_scene`); `from_json(kind, &Value)` estrito (`Integer` só aceita número inteiro: `1.5` é `INVALID_TYPE`, nunca arredondado), `to_json()` |
| `EditOp` | `Set { property, value }`, `Reset { property }`; `EditOp::from_json` (`{"set","value"}` / `{"reset"}`), `parse_batch` |
| `apply(&scene, &ops)` / `apply_with(.., &Limits)` | `Result<EditOutcome { candidate, summary }, EditError>` |
| `dry_run(&scene, &ops)` / `dry_run_with` | `Result<EditSummary, EditError>`; mesma validação, sucede **sse** `apply` sucede, mesmo resumo |
| `EditSummary` | `changes: Vec<PropertyChange { index, property, before, after }>`, `modified`, `conversions: Vec<MovementConversion { index, clip, segment, recipe, keys }>`, `changed_count()` |
| `properties_of(&scene)` / `properties_with_components` / `read_property` | Lista o que se aplica, com valor atual, sem seleção; `properties_of` é a listagem **padrão**: sem componentes, mas **com** `renderOrder` dos planos de imagem (não é mais a lista da primeira fatia); `properties_with_components` acrescenta os componentes depois das propriedades do mesmo nó/clipe; leitura por ID com os mesmos erros de ID/aplicabilidade |
| `EditError` | `code`, `message`, `index`, `path`, `id`, `issues()` (achados do importador), `cause()`; `From<EditError> for CommandError` |

### Catálogo

| Path | Tipo / unidade | Default (`reset`) | Faixa | Aplica a | Campo no bundle | Relação com a origem |
|---|---|---|---|---|---|---|
| `scene/<s>/node/<n>/position` | vec3 / unidades de cena | `(0,0,0)` | — | todo nó | `localPosition` | só editor |
| `scene/<s>/node/<n>/rotation` | quaternion `{x,y,z,w}` / sem unidade | identidade | — (comprimento ≈ 1 é regra do importador) | todo nó | `localRotation` | só editor |
| `scene/<s>/node/<n>/scale` | vec3 / multiplicador | `(1,1,1)` | — | todo nó | `localScale` | só editor |
| `scene/<s>/node/<n>/width` | número / unidades de cena | nenhum | `> 0` | `imagePlane` | `widthSceneUnits` | só editor |
| `scene/<s>/node/<n>/height` | número / unidades de cena | nenhum | `> 0` | `imagePlane` | `heightSceneUnits` | só editor |
| `camera/<c>/start/position` | vec3 / unidades de cena | nenhum | — | clipe | `initialPosition` | **mesmo path e mesma escrita** da origem (converte o 1º movimento) |
| `camera/<c>/start/target` | vec3 / unidades de cena | nenhum | — | clipe | `initialTarget` | comando `setStart.target` da origem; path é deste catálogo |
| `camera/<c>/start/fov` | número / graus | nenhum | `(0, 180)` | clipe | `initialFovDeg` | comando `setStart.fovDeg` da origem; path é deste catálogo |
| `scene/<s>/node/<n>/renderOrder` | **inteiro** / sem unidade | `0` | `[-(2^53-1), 2^53-1]` | `imagePlane` | `renderOrder` (opcional: ausente lê `0`) | só editor (a origem tem o campo no bundle) |
| `scene/<s>/node/<n>/position/{x,y,z}` | número / unidades de cena | `0` | — | todo nó | `localPosition/{x,y,z}` | só editor |
| `scene/<s>/node/<n>/scale/{x,y,z}` | número / multiplicador | `1` | — | todo nó | `localScale/{x,y,z}` | só editor |
| `camera/<c>/start/position/{x,y,z}` | número / unidades de cena | nenhum | — | clipe | `initialPosition/{x,y,z}` | **`parent-write`**: a origem não tem caminho por eixo; é a escrita de `start/position` com um eixo trocado (mesma conversão) |
| `camera/<c>/start/target/{x,y,z}` | número / unidades de cena | nenhum | — | clipe | `initialTarget/{x,y,z}` | **`parent-write`**: `setStart.target` com um eixo trocado |

**Componentes.** Escrever `position/x` é escrever o pai com aquele eixo trocado: mesma aplicabilidade, mesmas
checagens, mesmos efeitos (um eixo de `start/position` converte o primeiro movimento exatamente como o vetor
inteiro). Existem para que um slider, um script ou um eixo de gizmo (S8) enderece um número sem enviar os outros
dois. A faixa do componente é a do pai (nenhuma para vetores); `rotation` não tem componentes (quaternion não
tem eixo independente: normalizado, o importador recusa o resto). A gramática é a mesma da origem com mais um
segmento (`start/position/x`); um id de nó com `/` continua endereçado pelo escape (`%2F`).

**`renderOrder`.** Inteiro que o bundle guarda ou omite (omitido = `0`). O catálogo lê/escreve o campo do próprio
nó; uma passada de composição que cubra o nó continua sobrescrevendo a ordem de desenho
(`Bundle::paint_order`), então editar o campo não muda o que a composição já decide. Gravar o default sobre um campo
ausente é no-op e não acrescenta o campo; `reset` sobre um valor explícito grava `0`. `1.5` ou `"1"` são
`INVALID_TYPE`; fora de `±(2^53-1)` é `INVALID_VALUE`; nó que não é `imagePlane` é `NOT_APPLICABLE`.

**Vocabulário da origem.** A gramática é a de `properties/property-paths.ts`
(`scene/<id>/...`, `camera/<id>/...`, IDs com `encodeURIComponent`) e os códigos de erro reutilizam os de
`PropertyErrorCode` (`UNKNOWN_PATH`, `UNKNOWN_SCENE`, `UNKNOWN_CLIP`, `INVALID_VALUE`, `NOT_APPLICABLE`).
A origem **não** tem paths de nó (ela só edita nós reescrevendo o bundle), então os de nó são deste editor,
escritos na mesma gramática. IDs de nó são únicos só dentro da cena: o path sempre carrega a cena.

**Pose inicial equivalente à origem (BL005).** `setProperties` da origem, ao escrever
`camera/<c>/start/position`, antes de mover o início converte o **primeiro movimento** do clipe em keyframes
(`keysFromRecipe`/`toKeyframes`), para que o resto da trajetória não se mova junto. Aqui a mesma regra roda
dentro do lote (`scene_edit::camera_keys`), com a aritmética de pose do avaliador temporal
(`movement_pose_at`/`movement_end_pose`), sem segunda cópia:

| Movimento 0 do clipe | Depois da escrita |
|---|---|
| `hold` | uma key linear no último frame |
| `dolly`/`truck`/`zoom` | uma key no último frame, na pose final, mantendo o easing do movimento |
| `orbit` | uma key linear a cada 2 frames sobre o arco + a pose final (cordas). Erro medido (`scene_edit_oracle.rs`, caso `restore-orbit`: 40° com easing em 20 frames, raio 9,2195): **0,0976 unidades = 1,06% do raio**, e cresce com a velocidade da órbita; a origem tem as mesmas cordas |
| já são keys | nada muda |

Chaves que o schema não declara no movimento (`note`, por exemplo) **são preservadas** na conversão: o importador as mantém e reporta (F031) e o documento é o dado canônico. A origem as descarta, mas porque o `zod` as remove no parse, antes de qualquer comando; aqui nada as removeu (decisão, com teste `a_conversion_keeps_the_fields_the_schema_does_not_declare`). Um `yawDeg` em `dollyIn` ou um `easing` em `hold` também é dado desconhecido e permanece. Só saem os campos da própria receita (`easing`, `distanceSceneUnits`, `fovDeltaDeg`, `yawDeg`, `pitchDeg`).

Regras do lote: a conversão parte da pose que o clipe **tem agora** (inclusive de ops anteriores do mesmo lote) e só
ocorre quando o valor muda de fato — escrever o valor atual não converte nem toca o documento; só a primeira escrita
que muda o início converte (as seguintes veem keys). `start/target` e `start/fov` escrevem no lugar e **não**
convertem (a origem só os tem como comando `setStart`, sem conversão). Se a conversão não pode ser calculada (câmera sobre o próprio alvo), a
op falha com `CAMERA_CONVERSION_FAILED` (`invalid_camera`) no índice dela e o lote inteiro volta (rollback); um
movimento de um frame converte, mas a key resultante é recusada pelo importador como a origem a recusa
(`CANDIDATE_REJECTED`: `CAMERA_SEGMENT_TOO_SHORT`, `CAMERA_KEY_FRAME_ORDER`), também com rollback. As
conversões feitas voltam em `EditSummary::conversions` e, nas respostas `scene.edit` e `scene.gesture.update`, no
campo aditivo `conversions` (ausente quando nada converteu **ou quando o lote é um no-op líquido**, ver abaixo). `PropertySpec::origin` de `start/position` é
`path-equivalent-write` (`PathNarrowerWrite`, a escrita mais estreita, deixou de existir); os eixos de `start/*` são `parent-write`
(a origem não tem caminho nem comando por eixo) e os de nó seguem `editor-only`.

**Divergências da origem e limites do lote** (comparações do corpus em `scene_edit_oracle.rs`; dependência de ordem em `scene_edit_properties.rs`; observações apenas por leitura são identificadas abaixo):

- *A → B → A.* Após a conversão e a validação do candidato serem bem-sucedidas, a origem mantém keys da primeira escrita; o editor julga o efeito líquido, vê nenhum, devolve a cena de entrada (sem keys, sem `conversions`, `noop: true`). Para movimentos retos as poses coincidem; para órbita as cordas da origem diferem do arco que o editor mantém (1,06% do raio no caso medido). Se o lote tem **outra** edição real, a conversão da primeira escrita permanece, como na origem, e é listada.
- *Escrita intermediária inválida (H1, confirmada contra a origem real).* A origem valida cada escrita no bundle inteiro: `[start/position → alvo, start/target → outro]` é recusado já na primeira (`CAMERA_REFUSED`). O importador e os gates validam o candidato final, então esse caso é aceito (corpus `intermediate-degenerate-then-target`). Porém, cada conversão usa a pose intermediária no índice que a pede: a ordem das ops pode causar `CAMERA_CONVERSION_FAILED` antes da validação final. Por leitura de `batch.rs` (sem teste específico de A→B→A com movimento de um frame), mesmo um lote A→B→A é recusado se gerar keys inválidas, pois a validação precede a devolução da cena de entrada no no-op.
- *Eixos (H3).* Nenhum caso compara um caminho por eixo com a origem, porque ela não o tem. Os casos `axis` do corpus **compõem o vetor inteiro** (pose corrente com um eixo trocado) e o escrevem pela escrita da origem; provam eixo == pai com um eixo trocado, e só isso.
- *Zoom no primeiro movimento com camada de FOV (H2).* Coberto por `zoom-first-with-fov-layer`: igual à origem frame a frame.

**Oráculo fixo.** `docs/migration/fixtures/scene-edit/` (`cases.json`, `expected.json`, `manifest.json`) guarda 31
casos sintéticos (15 conversões, 4 erros, 3 lotes, 2 comandos `setStart`, 3 no-ops, 2 por eixo composto, 1 órbita restaurada, 1 divergência de validação intermediária; os 24 originais não mudaram byte a byte) e, por caso, o clipe resultante e a
pose de cada frame **como a própria `setProperties`/`resolveCameraAtClipFrame` da origem respondeu** (fontes reais
empacotadas sem alteração, commit e hashes no manifesto; nada copiado para o repositório). Gerador:
`node scripts/scene-edit-oracle/oracle.mjs generate`; conferência com regeneração byte a byte:
`node scripts/scene-edit-oracle/oracle.mjs verify` (sai 2 se a origem não estiver disponível; `--fixtures-only` só
confere integridade e **não** prova equivalência). O consumidor Rust é `crates/core/tests/scene_edit_oracle.rs`.
Tolerância: `abs <= 1e-9 + 1e-12·max(|a|,|b|)` por componente de pose, fixada antes da comparação, com controle
negativo (a escrita sem conversão é reprovada pelo mesmo comparador). Para órbitas a comparação é frame a frame
contra a própria origem, então o erro de corda é o mesmo nos dois lados.

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
4. `modified` é semântico e por **dado**, não por path: para cada propriedade que o lote escreveu (um componente conta como o
   vetor pai: `position/x` e `position` são o mesmo dado) compara-se o valor final do candidato com o da cena de entrada.
   "Alterar e restaurar" dá `false`, por qualquer combinação de pai e eixo (`position/x → 9` e depois `position →` original é
   `false`; `position → (2,y,z)`, `position/x → 5`, `position →` original, `position/x → 2` é `true`). Quando `false`, o
   candidato **é a cena de entrada** (mesmo documento e grafia, sem keys de conversão) e `conversions` vem vazio.
5. Números inteiros são gravados como inteiros (`16`, não `16.0`), como `JSON.stringify` na origem.
6. **Reset**: escreve o default do catálogo (`position/x` → `0`, `scale/x` → `1`, `renderOrder` → `0`); sem default
   (`width`, `start/*`) é `NO_DEFAULT` e o lote inteiro falha. Reset de um valor que já é o default é no-op.
7. **No-op**: um lote cujo efeito líquido é nulo (`modified == false`) não substitui a cena, não suja o projeto e não
   deixa entrada de undo (`noop: true`). Um lote que muda algo é **uma** entrada de undo, inclusive as conversões.
8. **Atomicidade**: vale também para a conversão: se o candidato final é recusado, nenhuma conversão sobra.

### Erros

| Código | Quando |
|---|---|
| `UNKNOWN_PATH` | path fora da gramática/catálogo (a mensagem lista os nomes conhecidos) |
| `UNKNOWN_SCENE` / `UNKNOWN_NODE` / `UNKNOWN_CLIP` | ID inexistente (`id` no erro) |
| `NOT_APPLICABLE` | `width`/`height`/`renderOrder` em nó que não é `imagePlane` |
| `INVALID_TYPE` | valor com forma errada (JSON estrito: chaves exatas, sem extras; `renderOrder` não inteiro, como `1.5`) ou `PropertyValue` de outro tipo |
| `INVALID_VALUE` | não finito ou fora da faixa; inclui um `renderOrder` inteiro além de ±(2^53−1) (`1e16`, inteiro acima de `i64::MAX`) |
| `NO_DEFAULT` | `reset` sem default no catálogo |
| `INVALID_OP` | op JSON que não é `{set,value}` nem `{reset}` |
| `CANDIDATE_REJECTED` | ops válidas, mas importador ou gates estruturais/temporais recusam o candidato (`issues()`, `cause()`) |
| `CAMERA_CONVERSION_FAILED` | a conversão do primeiro movimento não pôde ser calculada (câmera sobre o próprio alvo); índice da op que a pediu |
| `DOCUMENT_MISMATCH` | visão tipada e documento discordam da posição de uma entidade (não ocorre em cena importada) |

`CommandError`: `unknown_path`, `unknown_scene`, `unknown_node`, `unknown_clip`, `not_applicable`, `no_default`,
`invalid_op`, `invalid_type`, `invalid_value`, `document_mismatch`, `invalid_camera`; um candidato recusado usa o código do
importador (`invalid_bundle`). `target` = ID ausente; `details` = `index`, `path`, `issues[].ops`.

## CLI headless

`./scripts/headless` (ou `tessel-headless`) executa `scene.edit` (`dry_run?`), `scene.properties` (`components?`),
`scene.get`, `scene.catalog` e `scene.capabilities` sobre as mesmas funções de lib, sem janela:

```bash
printf '%s\n' \
  '{"op":"scene.attach","id":"street","path":"<bundle.json>"}' \
  '{"op":"scene.edit","scene":"street","ops":[{"set":"scene/street/node/hero/position/y","value":3.5},{"set":"scene/street/node/hero/renderOrder","value":7}]}' \
  '{"op":"scene.edit","scene":"street","dry_run":true,"ops":[{"set":"camera/wide/start/position/x","value":1.5}]}' \
  '{"op":"scene.properties","scene":"street","components":true}' \
  '{"op":"history.undo"}' | ./scripts/headless
```

Contrato **aditivo e negociado**: `scene.capabilities` ganha `edit` (`catalog_version: 2`, `components`,
`render_order`, `camera_start_conversion: "origin-keyframes"`, `batch: "atomic"`, `properties_option_components`);
quem não lê o bloco recebe a listagem padrão (sem componentes, mas **com** as linhas de `renderOrder` dos planos de imagem),
as oito primeiras entradas de `scene.catalog` na mesma ordem e respostas de `scene.edit` sem `conversions` quando nada converte. `scene.catalog` acrescenta por entrada
`component_of` (`{parent, index}` ou `null`), `optional` e `converts_first_movement`. `scene.tree` lista em `editable`
também os caminhos de componente e `renderOrder` (S8 pode endereçar um eixo sem a janela).

## UI e CLI da janela

O inspector (BB08, [scene-inspector](scene-inspector.md)) mostra `renderOrder` como propriedade de plano de imagem:
um slider de passo 1, com valor digitado arredondado para inteiro antes de virar `EditOp`, mesmo reset/gesto/undo das
demais. Os componentes **não** viram controles novos: o inspector já tem um slider por eixo, e listá-los repetiria
cada número; ficam endereçáveis por `scene.edit`/`ui op` (e pelo `set` semântico do nó da propriedade, que já edita um
eixo). Mover um eixo de `start/position` pelo slider passa pelo mesmo lote e converte o primeiro movimento.
Apresentação (foco, docking) é N/A para este módulo.

## Evidências e status

| Superfície | Estado |
|---|---|
| Lib | **A validar** (S6). `scene_edit.rs` (29), `scene_edit_properties.rs` (32: componentes, `renderOrder`, conversão, rollback, reset, no-op, ordem de ops, efeito líquido por dado, A→B→A, chaves desconhecidas, `INVALID_VALUE` de `renderOrder`) e `scene_edit_oracle.rs` (7: 31 casos contra o oráculo fixo, controle negativo, divergências medidas) passaram via `cargo-serial.py test -p tessel-editor-core --locked` no head da correção dos findings da revisão Opus. Doctest e golden JSON ordenado da PR4 preservados; **a suíte completa do core não foi reexecutada depois dessa correção** (disco abaixo do piso) |
| CLI headless | **A validar** (S6). `crates/core/tests/scene_edit_session.rs` (8) e `scene_interaction.rs` (22, `scene.gesture.update`/`commit` com `conversions`): capabilities/listagem/catálogo aditivos (`origin` dos eixos), lote de componentes + `renderOrder` como uma entrada de undo, lote recusado sem mutação, no-op sem entrada, conversão com dry-run igual ao apply, o executável `tessel-headless` responde linha a linha como a lib, o lote com eixo escondido entre escritas do vetor é edição real com uma entrada de undo, restauração líquida é no-op, `renderOrder` fora da faixa é `invalid_value` |
| UI (desktop) | **A validar** (S6). Testes de `scene_model.rs` (lista de propriedades sem componentes, `renderOrder` inteiro: slider, arredondamento, janela) e `scene_tests.rs` (`set` semântico de `renderOrder`, undo, componente sem controle duplicado; **arrastar o slider de `start/position`** converte o primeiro movimento ao soltar, uma entrada de undo, desfazer restaura a receita; `set` semântico do eixo informa `conversions`) em GPUI headless de teste (`scene_`: 91 passaram). **Sem** captura nativa nem ponteiro físico nesta fatia |
| Oráculo | `node scripts/scene-edit-oracle/oracle.mjs verify` (regeneração byte a byte contra a origem, somente leitura) saiu 0 com os 31 casos; os 24 originais não mudaram |

Limites: um editor geral de keyframes não existe; só o primeiro movimento é convertido; uma conversão de órbita usa
cordas a cada 2 frames (igual à origem); `renderOrder` escrito no nó não vence uma passada de composição que o cubra.

## Evidências herdadas da primeira fatia

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

Integração com main6f7a944 (S2/S5): conflito restrito ao backlog, resolvido preservando
as linhas da base e a entrega S6. O auto-merge de ops/attach foi validado com o core completo
(595 passaram,3 ignorados) e desktop completo (203 passaram), além de fmt, clippy workspace
all-targets com -Dwarnings e build desktop. Nenhuma nova prova nativa de edição foi declarada.

Revisões R2–R4: a 33ª regressão de `scene_edit_properties` e a correção de produção de preservação por receita em `0d5304a` estão escritas, ainda sem compilação ou execução. Os resultados acima de 32 testes pertencem ao head executado pelo Cursor, não ao head final; gates completos e prova nativa continuam pendentes.
