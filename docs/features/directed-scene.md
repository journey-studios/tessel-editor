# F031 — Cena dirigida: importar, validar e regravar `render-bundle` 1.0 (BB01/BB02)

## Status

Lib **Parcial**: subconjunto BB01/BB02 implementado e validado por 29 testes públicos próprios
e 3 testes de integração com o oráculo; validação completa da origem ainda pendente. CLI headless **P** (nenhum comando ainda). UI **P** (nenhum controle ainda). O que existe é o
**núcleo puro**: importa, valida, consulta o grafo mínimo e regrava. Adaptadores (comando `scene.*`, headless,
diálogo desktop, ponte com `document.rs`/runtime) são a próxima etapa e estão em "Pendências".

## Objetivo e limites

Decisões aplicadas ([migration/README.md](../migration/README.md)):

- **D2** — o `render-bundle.json` v1.0 é formato de **intercâmbio**, lido e regravado sem substituir o
  `.tessel.json`. Este módulo não toca `document.rs`, comandos, runtime, headless nem desktop.
- **D3** — cena em camadas 2.5D é um **tipo novo** (`DirectedScene`), não uma simulação com clipes 2D. Cobre
  planos de imagem em camadas, hierarquia/sockets, shots e a câmera segmentada na estrutura necessária;
  luz, clima, rigs, caminhos, narração e `frameAudit` ficam **preservados, não interpretados** (ver
  `DEFERRED_FIELDS`/`DEFERRED_RULES`).

Fonte lida (somente leitura, snapshot `d6adac15…`): `packages/scene-engine` (schema zod do bundle,
`validateBundleIntegrity`, `validateSceneGraphStructure`, `validateEditJoins`, `computeWorldMatrices`). Nada foi
instalado, compilado ou executado na origem por esta frente; as regras foram **lidas** e reimplementadas.

## Contrato do core / biblioteca

```rust
use tessel_editor_core::directed_scene::DirectedScene;

let scene = DirectedScene::import_str(text)?;            // Err(ImportError) deixa tudo como estava
let bundle = scene.bundle();                              // visão tipada, somente leitura
let hero = bundle.scene("street").unwrap();        // SceneGraph
let world = hero.world_matrix("prop-plane").unwrap();    // Mat4 column-major, igual ao Three.js
let users = bundle.referrers_of(&Entity::new(EntityKind::Asset, "hero"));
let order = bundle.paint_order("street");           // ordem de pintura dos planos
let out = scene.to_json_string();                         // regrava (campos desconhecidos incluídos)
```

| Item | Contrato |
|---|---|
| `DirectedScene::import_str[_with]` / `import_value[_with]` | Valida em 3 estágios (abaixo). Retorna a cena ou `ImportError`. `Limits` opcional |
| `validate_str` | Só o veredito do subconjunto de importação; junções estruturais são consulta separada |
| `bundle()` | `Bundle`: `asset`, `scene`, `instance`, `camera_clip`, `shot`, `scene_of_instance`, `shot_at_frame`, `edit_duration`, `edit_time_of_frame`, `references`, `referrers_of`, `paint_order` |
| `SceneGraph` | `nodes`, `node`, `sockets`, `socket`, `parent` (`Parent::Node` ou `Parent::Socket`), `children`, `ancestors`, `local_matrix`, `world_matrix`, `image_planes` |
| `notices()` | `UNKNOWN_FIELD` por chave desconhecida (com JSON Pointer). Não bloqueia |
| `document()` / `into_document()` / `to_json_string()` / `to_json_pretty()` | O documento canônico (`serde_json::Value`) |
| `edit_join_issues()` | Regras estruturais de junção entre shots (códigos da origem) |

**Tipos reutilizados:** `time::Rate` valida o FPS (≤ 1000 fps, racional) e `time::Ratio` serve ao tempo da
edição; `CommandError` ganha `From<ImportError>` com os códigos `invalid_json`, `bundle_too_large` e
`invalid_bundle`. **`Id` não é usado** para IDs do bundle: ele é internado e vaza memória por valor distinto, o
que não serve para entrada não confiável; IDs do bundle são `String`.

### Estágios e erros

1. **Forma e vocabulário** — tipos, campos obrigatórios, números (inteiro/finito/intervalo), enums, caminhos
   relativos seguros.
2. **Integridade** — IDs duplicados, referências (asset, nó, shot, instância, clipe de câmera, canal), shots que
   cobrem a edição sem buracos, câmera (receitas, keys, easings, camadas ≤ 3), áudio, composição.
3. **Grafo** — pai/socket efetivo, ciclo (`PARENT_CYCLE`, `EFFECTIVE_GRAPH_CYCLE`), órfãos, matrizes finitas
   (`NON_FINITE_TRANSFORM` se a hierarquia estoura).

Os problemas de forma escondem os de integridade (uma referência para um campo que não leu só geraria ruído). Todo
`Issue` traz `code`, `path` (JSON Pointer) e `message`; a lista nunca é vazia. Códigos da origem são mantidos
(`DUPLICATE_NODE_ID`, `NODE_PARENT_MISSING`, `SHOT_*`, `CAMERA_*`, `FPS_MISMATCH`...); os próprios do importador
são `INVALID_TYPE`, `MISSING_FIELD`, `INVALID_VALUE`, `UNKNOWN_FIELD` (aviso), `UNSUPPORTED_SCHEMA_VERSION`,
`LIMIT_EXCEEDED`, `UNSAFE_RELATIVE_PATH`, `FPS_OUT_OF_RANGE`, `NON_FINITE_TRANSFORM`.

### Políticas

| Tema | Política |
|---|---|
| Versão | Só `schemaVersion` `"1.0"` (bundle e cada definição de cena). Outra → `UNSUPPORTED_SCHEMA_VERSION`, sem ler mais nada |
| Campo desconhecido | **Preservado** e reportado em `notices()`. A origem (zod) o descarta em silêncio |
| Tipo/receita/enum fora do vocabulário | **Recusado** com o código da origem (`nodes[].type` além de `group`/`image-plane`/`splat`, receita de câmera desconhecida, easing desconhecido, transição `dissolve`...) |
| Dados não interpretados | Preservados por valor no documento; regras adiadas não são validadas (lista em `DEFERRED_*`) |
| Round-trip | **Igual por valor e idempotente.** Formatação e ordem das chaves não são preservadas; chave JSON duplicada colapsa (a última vence); números preservam inteiro × decimal |
| Limites (`Limits`) | 32 MiB de texto, profundidade 128, 100 000 itens por array interpretado. Arrays em seções adiadas não têm esse limite individual. Aplicados antes/durante a leitura, nunca truncando |
| Pilha | Detecção de ciclo, matrizes e profundidade são **iterativas** (cadeia de 20 000 nós passa sem estouro) |

## Divergências da origem

- Desconhecidos são preservados (D2); a origem descarta.
- Erro com JSON Pointer completo; a origem dá caminho grosso.
- Limites de tamanho/profundidade/itens: a origem confia em arquivo local.
- Forma inválida interrompe antes da integridade; a origem às vezes reporta as duas (ex.: `fps.numerator = 0`
  dá `INVALID_VALUE` aqui, e `too_small` + `FPS_MISMATCH` lá — ambos recusam).
- Sem dependência de zod/three: matrizes são `Mat4` própria (mesma convenção `compose`/`multiply`).
- Caminhos de asset que saem da pasta do bundle (`..`, absoluto, esquema) são recusados.

## Pendências (explícitas)

- `DEFERRED_FIELDS`/`DEFERRED_RULES`: `LIGHT_*` exceto `LIGHT_PARENT_UNKNOWN`, clima, rigs, caminhos, efeitos, `SURFACE_*`/`NORMAL_MAP_*`/
  `RELIEF_*`/`SCENE_DEPTH_*`, `AUDIT_*`, narração/outline, payload dos canais de performance (BB03), checagem
  de FOV composto das camadas de câmera, `CONTINUOUS_*_JUMP` (comparação de pose nas junções).
- Avaliação de câmera/performance por frame (BB03), `finalize`/escrita atômica em disco (adaptador de `persist.rs`).
- Adaptadores: comando `scene.*`, headless JSONL, diálogo desktop, ponte para runtime/`document.rs`.
- Fixture própria **sintética** (`crates/core/tests/directed_scene_fixtures/layered_scene.json`); nenhum bundle ou
  mídia privados foram copiados.

## Evidências e status

| Superfície | Estado |
|---|---|
| Lib | **V** — `MBX_GC_MAX_TOTAL_SIZE=64GiB ./scripts/cargo test -p tessel-editor-core --locked` (suíte do core + 29 testes de `directed_scene`: estrutura, hierarquia/sockets/camadas, transformações (mundo do `prop-plane` = (7,4,2)), referências, round-trip, desconhecidos, ciclo, referência quebrada, ID duplicado, números inválidos, limites, 20 000 nós, mutação de campo único sem pânico, `CommandError`) |
| Qualidade | `cargo clippy -p tessel-editor-core --all-targets --locked -- -D warnings` limpo; `cargo fmt --all -- --check` limpo; `cargo tree -p tessel-editor-core`: só `serde`, `serde_json`, `flate2` (sem GPUI/wry) |
| Contra a origem | `directed_scene_oracle.rs`: 55/55 vereditos de importação + junções estruturais, referências pai/socket nos 4 casos válidos, round-trip e matrizes estáticas (tolerância absoluta `1e-10` + relativa `1e-12`). Performance animada e pose temporal de câmera não são comparadas. `LIGHT_PARENT_UNKNOWN` foi corrigido com teste que falhava antes. |
| CLI headless / UI | **P** — sem adaptador |

A fixture própria `layered_scene.json` foi também conferida pelo coordenador com as funções
reais da origem: schema e integridade passam, mas `finalizeBundle` a rejeita por
`CONTINUOUS_VELOCITY_JUMP`. Isto confirma a limitação deliberada das junções temporais: importar
e não encontrar problema estrutural **não** certifica o bundle completo para execução/export.
A futura avaliação BB03 deverá executar esse gate antes de habilitar cenas na UI.
