# Relatório da frente A — BB01/BB02 (cena dirigida)

2026-11-05. Rodada paralela da base de migração ([plano](parallel-implementation-plan.md),
[roadmap](phased-roadmap.md)). Checkout `tessel-editor` em `cb1412f` (baseline `59d618a`). Esta frente não
fez `git add`/commit/push/switch/reset/clean, não alterou `Cargo.toml`/`Cargo.lock`, `document.rs`, runtime,
comandos, headless nem desktop, e não modificou `tesselapp` nem a origem.

## Entrega

Módulo `tessel_editor_core::directed_scene`: **importar, validar, consultar e regravar** `render-bundle` 1.0
(D2) como tipo novo de cena em camadas (D3). Contrato completo, políticas e divergências em
[`docs/features/directed-scene.md`](../features/directed-scene.md).

### Arquivos (todos dentro do ownership)

- `crates/core/src/lib.rs` — apenas `pub mod directed_scene;`
- `crates/core/src/directed_scene/` — `mod.rs` (API, `Limits`, `DEFERRED_*`), `read.rs` (leitor que coleta
  todos os problemas com JSON Pointer e detecta campos desconhecidos), `parse.rs` (estágio 1), `integrity.rs`
  (estágio 2), `graph.rs` (estágio 3: hierarquia, sockets, ciclos, matrizes), `joins.rs`, `refs.rs`
  (referências/referenciadores/ordem de pintura), `transform.rs` (`Vec3`/`Quat`/`Transform`/`Mat4`),
  `model.rs`, `issue.rs`
- `crates/core/tests/directed_scene.rs` (29 testes) e `crates/core/tests/directed_scene_fixtures/layered_scene.json`
  (bundle **sintético**: 24000/1001 fps, 4 assets, 1 cena com plano com socket, 2 shots, câmera com keys e
  camada de órbita, áudio e campos desconhecidos)
- `docs/features/directed-scene.md`, `docs/migration/foundation-a-report.md`

### Decisões aplicadas

- **D2:** bundle como intercâmbio; o documento canônico é o `serde_json::Value` importado e nunca é alterado
  pela visão tipada; desconhecidos **preservados + `Notice`**; `.tessel.json` intacto.
- **D3:** planos de imagem em camadas + hierarquia/socket + shots + câmera segmentada, só na estrutura
  necessária; luz/clima/rig/caminhos/narração/`frameAudit` preservados e não interpretados.
- Reuso: `time::Rate`/`Ratio` (FPS e tempo), `CommandError` (`From<ImportError>`). `Id` **não** reusado (internado
  e com vazamento por valor; inadequado para entrada não confiável).

## Verificação (A com Cargo exclusivo)

| Comando | Resultado |
|---|---|
| `MBX_GC_MAX_TOTAL_SIZE=64GiB ./scripts/cargo test -p tessel-editor-core --locked` | OK: 161 unit + 29 `directed_scene` + 2 headless + 8 project + 8 proof + 7 video_sound + 1 doctest, 0 falhas |
| `./scripts/cargo clippy -p tessel-editor-core --all-targets --locked -- -D warnings` | limpo |
| `cargo fmt --all -- --check` | exit 0 |
| `./scripts/cargo tree -p tessel-editor-core --locked` | só `flate2`, `serde`, `serde_json` (0 ocorrências de gpui/wry) |

Sem build GPU/desktop e sem execução pesada do workspace. Disco livre ~9,8 GiB (piso de 6 GiB respeitado);
artefatos próprios pequenos (compilação incremental só do core). `mbx clean/gc` não usado; HOME/RUSTUP_HOME/CARGO_HOME
inalterados.

### Cobertura dos testes públicos

Estrutura e consulta; hierarquia/sockets/camadas; transformações (mundo do plano em socket = (7, 4, 2));
referências e referenciadores; round-trip (valor, desconhecidos, número inteiro × decimal, idempotência);
campos desconhecidos; seções adiadas preservadas; ID duplicado; ciclo (`PARENT_CYCLE`,
`EFFECTIVE_GRAPH_CYCLE`); referência quebrada; quaternion degenerado/não normalizado; cadeia de 20 000 nós sem
estouro de pilha; transformação que estoura (`NON_FINITE_TRANSFORM`); erros de forma/número; FPS; versões;
vocabulário não suportado; caminhos inseguros; forma esconde integridade; shots que não ladrilham;
câmera; áudio/legado; junções; limites; `CommandError`; dados do chamador intactos em falha; e **mutação
de campo único** (≈ toda posição do fixture trocada por 11 valores ruins e removida: nunca entra em pânico
e toda recusa traz ao menos um `Issue`).

### Comparação ad-hoc com o corpus da frente B (não é dependência)

Sonda temporária, **removida** (não versionada), rodou o importador sobre `docs/migration/fixtures/` (55 bundles
com esperados gerados pelo código real da origem). O corpus da B ainda era provisório. Resultado: veredito
(aceita/recusa) igual em **54/55**; entre os 54, os códigos esperados da origem estão contidos nos emitidos em 53 (a exceção é `invalid-fps-zero-numerator`, recusado aqui por `INVALID_VALUE` e lá por `too_small` + `FPS_MISMATCH`); **1 aceito indevidamente**: `invalid-light-parent-unknown` (`LIGHT_PARENT_UNKNOWN` é regra
de luz, adiada). Nos 10 bundles aceitos pelos dois lados, as regras de junção coincidem. Reexecutar quando a B congelar o corpus.

## Limitações e dependências

- **Fixtures sintéticos, não validados contra o zod real** (nada rodou na origem). A evidência contra a origem é
  só a sonda acima, sobre o corpus da B; formalizar quando a B terminar (o teste de equivalência pertence ao
  coordenador/B, para que A não dependa de B).
- Falsa aceitação conhecida: regras adiadas (`DEFERRED_RULES`): `LIGHT_*`, clima/rig/caminhos/efeitos,
  `SURFACE_*`/`NORMAL_MAP_*`/`RELIEF_*`/`SCENE_DEPTH_*`, `AUDIT_*`, narração, payload de canais (BB03), FOV composto das
  camadas de câmera, `CONTINUOUS_*_JUMP`. Um bundle com esses defeitos importa em A e é recusado pela origem.
- Round-trip é por valor (chave duplicada colapsa; ordem/formatação não preservadas).
- Pendente (próxima etapa, adaptadores): comando `scene.*`, headless JSONL, diálogo desktop, ponte com
  `document.rs`/runtime, escrita atômica via `persist.rs`, avaliação por frame (BB03).
- Reservado ao coordenador: `Cargo.toml`/lock (nenhuma mudança necessária) e matriz.

## Linha exata para a matriz (`docs/features/README.md`)

```
| F031 | [Cena dirigida: importar, validar e regravar render-bundle 1.0 (BB01/BB02)](directed-scene.md) | P | P | P | V¹⁰ | Só lib `tessel_editor_core::directed_scene`; sem comando/headless/UI ainda |
```

¹⁰ Lib validada por 29 testes públicos executados (core inteiro, clippy e fmt limpos); CLI headless e UI **P** (sem
adaptador). Fixtures sintéticos; regras adiadas listadas em `directed-scene.md`.

Colunas: UI, CLI da janela, CLI headless, Lib. O número da nota `¹⁰` é sugestão; o coordenador renumera conforme as notas existentes.

## Revisão e integração pelo coordenador

- Consumidor permanente do corpus B: `crates/core/tests/directed_scene_oracle.rs`.
- Testes de regressão falharam antes da correção de `LIGHT_PARENT_UNKNOWN` e passaram depois.
  A referência de pai da luz agora é verificada, mesmo com render e demais regras de luz adiados.
- 55/55 vereditos do corpus coincidem ao combinar importação com junções estruturais.
- Os 4 casos válidos comparam referências pai/socket e round-trip. Matrizes estáticas também
  são comparadas, com tolerância absoluta `1e-10` + relativa `1e-12`; performance animada excluída.
- Core: 219 testes incluindo doctest, zero falhas (161 unitários, 29 próprios, 3 do oráculo,
  2 headless, 8 project, 8 proof, 7 video_sound e 1 doctest).
- Fixture própria executada contra a origem pelo harness: schema/integridade passam,
  `finalizeBundle` rejeita `CONTINUOUS_VELOCITY_JUMP`. Não constitui fixture totalmente válida
  da origem; ela exercita o subconjunto atual e as regras temporais continuam pendentes.
- Matriz usa **Parcial**, sem afirmar validação completa ou adapters entregues.

A sonda provisória 54/55 acima é histórica e foi substituída pelo consumidor versionado.
