# Relatório da frente I — recursos da cena dirigida (BL009 / BL010 / BL011 / BB06 inicial / F038)

2026-11-05. Rodada H/I/J/K ([plano](next-round-plan.md)). Checkout compartilhado `tessel-editor`.
Contrato da feature: [`docs/features/scene-resources.md`](../features/scene-resources.md) (F038).
Origem (`tesselapp-image-coverage`, snapshot `d6adac15`) só lida para a regra de hash; nada foi
editado nela nem em `tesselapp`. Contrato de H (`7f925f7`, `ScenePlan::required_asset_ids()`)
recebido; **H não foi editada** e esta entrega não depende dela.

**Sem Cargo.** Não rodei `cargo`. Não fiz `git add`/commit/push/PR, branch/worktree, nem toquei
`core/src/lib.rs`, `media/src/lib.rs`, `Cargo.toml/lock`, `docs/backlog.md`, matriz ou índices.
Nenhum subagente. `rustfmt` só nos meus arquivos. **O Rust não está declarado verde.**

## Arquivos (ownership exclusivo)

| Arquivo | Conteúdo |
|---|---|
| `crates/core/src/scene_resources/{mod,model,manifest,scan,query,transport,path}.rs` | Manifesto, classificação, dependências/closure, consulta por `assetId`, `ResourcePath`, plano de transporte |
| `crates/core/tests/scene_resources.rs` | 27 testes públicos (+ 6 unitários em `path.rs`) |
| `crates/media/src/directed_resources/{mod,source,check,destination,preflight,materialize}.rs` | Resolução, preflight, materialização com staging/diário/rollback |
| `crates/media/tests/directed_resources.rs`, `directed_resources_transport.rs`, `directed_resources_common/mod.rs` | 19 + 20 testes e fixtures procedurais (PNG/SPZ mínimos, sem imagem privada) |
| `docs/features/scene-resources.md`, este relatório | docs |

## Implementação

- **Manifesto** (core, sem IO/caminho absoluto/GPUI): `ResourceManifest::from_scene(&DirectedScene)`
  lê o bundle tipado e as seções cruas preservadas (`surface`, `normalMap`, `sceneDepth`, `relief`,
  narração). Entradas por `assetId` (+ `AudioMix`), com status `Supported`/`Uninterpreted`/
  `Unreferenced`, usos com ponteiro JSON, dependências transitivas (asset do nó → mapas), `closure`,
  consultas por cena/nó, issues ordenadas, colisões, JSON determinístico. Serialização à mão
  (o `serde` do core não tem `derive`). Independente de `scene_plan`.
- **Transporte** (`transport_plan`): lista arquivos distintos com o hash esperado e os recursos que
  apontam para cada um; preservados e não referenciados vão para `skipped`, nunca em silêncio.
- **Caminhos**: validação mais estrita que a do importador (ver feature doc), com chave de colisão.
- **Media**: `preflight` (só leitura, todos os problemas) e `materialize` (3 passes: decidir sem
  escrever, copiar para staging dentro da raiz com hash/cabeçalho conferidos na cópia, publicar por
  `hard_link` sem sobrescrever, diário que desfaz só o que a chamada criou). Contenção da origem por
  caminhada componente a componente, canonicalização e dev/ino; política de links `Reject` (padrão) ou
  `AllowContained`; destino nunca escrito por link. Cancelamento por token; limites de arquivo e total.
- Recursos reutilizados se o destino já tem o hash esperado; destino diferente é conflito, não
  sobrescrita. Sem armazenamento de conteúdo duplicado.

## Testes: escritos × executados

| | |
|---|---|
| **Escritos** | Core 27 + 6 unitários. Media 19 (preflight, hash, PNG, SPZ, limites, links) + 20 (materialização, reuso, conflito, cancelamento, rollback, contenção de links na origem e no destino) |
| **Executados** | **Não pelo Cargo.** Compilei avulso com `rustc`/`clippy-driver` (toolchain 1.99.1 do repo) fora do checkout (`/tmp/i-scratch`), contra os `rlib` já existentes de `target/release/deps`, com um core reduzido (módulos `directed_scene`, `time`, `splat`, `scene_resources` + stubs de `state`, `jobs`, `assets`, `fixtures`). Passaram 27 + 6 (core), 19 + 20 (media); clippy sem avisos. É evidência de trabalho, **não** o gate |
| **Não executados** | Doctest de `scene_resources` (só compilado em contexto reduzido), `cargo tree` (sem GPUI), clippy `-D warnings` do workspace, Windows |

Comandos do gate (coordenador): `cargo test -p tessel-editor-core --locked --test scene_resources`, `--lib scene_resources`, `--doc scene_resources`; `cargo test -p tessel-editor-media --locked --test directed_resources --test directed_resources_transport`; clippy `-D warnings` dos dois crates.

Pontos que, se falharem no Cargo, provavelmente são expectativa de ambiente: o teste de rollback pula
sozinho se rodar como root; os helpers de fixtures reais (`encode_png`, `splat_spz`) do core são usados
em vez dos stubs que usei fora do Cargo.

## Limites

- PNG: só assinatura e IHDR; **sem decodificar pixels nem alfa**. Grade f32 e áudio só têm hash.
- Mapas, `sceneDepth`, `relief` e narração são listados e entram na closure, mas **não são
  transportados** (transporte parcial, `complete == false`).
- Colisão por minúsculas, sem dobra Unicode. Identidade dev/ino só em unix. TOCTOU residual contra
  quem escreve na raiz durante a execução; hardlinks não detectáveis.
- Cancelamento não é observado durante a publicação.
- Hashes sintéticos das fixtures (`aaa…`) bloqueiam o transporte (hex de 64 minúsculos exigido).
- Campos desconhecidos e luz/clima/efeitos/rigs/paths/`frameAudit` não são varridos por arquivos.
- Não integra `host.rs`, pacote nem intercâmbio; guardas de pacote incompleto intactas. Sem job/cache/
  release por geração (BL010) nem salvar/restaurar/relink (BL011).

## Contratos para integrar

1. **Coordenador — `crates/core/src/lib.rs`:** `pub mod scene_resources;` (após `scene_project`, ordem
   alfabética). **`crates/media/src/lib.rs`:** `pub mod directed_resources;`. Sem dependência nova
   (core: serde/serde_json/flate2; media: core, serde_json, sha2, crc32fast, já existentes).
2. **API principal:** `ResourceManifest::{from_scene, resource, lookup, closure, transport_plan, ...}`;
   `directed_resources::{preflight, materialize, Options, SymlinkPolicy, ProblemKind}`; erros `JobError`
   com os códigos do feature doc.
3. **Frente H (`ScenePlan`):** consultar o manifesto pelos mesmos `assetId` de
   `ScenePlan::required_asset_ids()`; identidade é `assetId`, caminho/revisão/hash são metadado.
4. **Teste de integração futuro (dono: BL063, não escrito aqui para não depender de H):** para cada
   bundle válido do corpus e cada frame, `let plan = ScenePlan::build(&scene, frame)?;`
   `let ids: Vec<&str> = plan.required_asset_ids().iter().map(String::as_str).collect();`
   `let found = ResourceManifest::from_scene(&scene).lookup(&ids);` e `assert!(found.unknown.is_empty())`;
   mais a recíproca fraca: todo `Supported` de um nó da cena do frame aparece em `required_asset_ids`
   (se o plano estrito recusar mapas/splat, usar `allow_partial` e documentar).

## Patches propostos (o coordenador integra)

### `docs/features/README.md`

```markdown
| F038 | [Recursos da cena dirigida: manifesto, preflight e materialização (BB06)](scene-resources.md) | P | P | P | Parcial¹⁵ | `scene_resources::ResourceManifest`; `directed_resources::{preflight, materialize}`; sem pacote/UI/CLI |
```

```markdown
¹⁵ Manifesto determinístico por `assetId` com closure, e materialização com rollback de PNG/SPZ/mix
suportados numa raiz de transporte autorizada (links e traversal contidos). Mapas, profundidade,
relevo e narração só listados. 27+6 (core) e 39 (media) testes escritos, executados apenas fora do
Cargo; gate do coordenador pendente.
```

### `docs/backlog.md`

```markdown
| BL009 | BB06 | Resolver dependências transitivas: imagens/mapas/splats/fontes e base de caminhos | A validar | Modelo BB01/BB02 | Frente I (F038, [relatório](migration/foundation-i-report.md)) | Closure por `assetId`, validação de caminhos/colisões e diagnóstico de ausentes (preflight) implementados; falta gate Cargo. Mapas/profundidade/relevo/narração apenas listados; fontes fora de escopo |
| BL010 | BB06 | Jobs/load/cache/release por geração e dependentes da cena | Em andamento | BL009 | Frente I | Só cancelamento, limites e rollback da materialização (orçamento por arquivo/total). Faltam geração, invalidação de dependentes e release |
| BL011 | BB06 | Save/package/restore/relink/interchange de recursos de cena | Em andamento | BL009, BL010 | Frente I | Só a materialização inicial (F038). Faltam package.create/restore/relink, intercâmbio e a retirada da guarda de pacote incompleto |
```

Novas linhas sugeridas (BL066–BL069 já existem; usei BL070–BL072, renumerar se houver colisão):

```markdown
| BL070 | BB06 | Ligar `preflight`/`materialize` ao fluxo real de pacote/salvar/restaurar (`host.rs`, intercâmbio) e provar abrir sem os originais | Pendente | BL009, BL011 | — | Mesma fonte de erros nas superfícies; guarda de pacote incompleto só cai com prova de restauração |
| BL071 | BB06 | Transportar mapas de dados, `sceneDepth`/`relief` (grades f32) e narração quando houver interpretação | Pendente | BL009, BL025 | — | Capabilities/`MANIFEST_VERSION` sobem; verificação de grade f32 (tamanho/endianness) e PNG com alfa decodificado |
| BL072 | BB06 | Identidade de arquivo e colisão fora de unix / dobra Unicode | Pendente | BL009 | — | Equivalente Windows do dev/ino; colisão por normalização NFC/NFD |
```

`BL063` (já atribuído por você) cobre a integração `scene.plan` × manifesto descrita acima;
`BL064`/`BL065` (custo/timeline, domínios/profundidade) e `BL062` (só undo de sliders) ficam como
informados. Sem outras mudanças em estados.

### Índices

Se `docs/README.md` listar features, acrescentar F038 → `features/scene-resources.md`.

## Integração pelo coordenador

Exports core/media adicionados. 27 testes core + 19/20 media passaram via Cargo --locked;
integração H/I passou em 4 bundles × 4 frames, com required_asset_ids contidos no manifesto.
Clippy workspace com -D warnings e formatação passaram; suíte completa em execução neste registro.

Removido fallback check-then-rename: hard-link failure agora falha sem mover/publicar outro
objeto. Regressão unitária falhou antes e passou depois. Precondição explícita: raízes sob
controle exclusivo; checagem de symlink não é garantia contra troca hostil concorrente de pais.
F038 e BB06 continuam parciais. BL011/BL077–BL080 são os IDs consolidados; BL070–BL072 já são K.

Gate completo final: 718 testes Rust passaram, 2 ignorados; fmt/check/clippy workspace e release passaram. Guardas de pacote mantidas. Ver [relatório consolidado](round-hijkl-report.md).
