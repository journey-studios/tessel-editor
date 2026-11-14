# F038 — Recursos da cena dirigida: manifesto, resolução e materialização (BB06, fatia inicial)

## Status

Lib **Parcial** (27 testes públicos core + 39 media passaram pelo Cargo do coordenador;
o gate completo e os limites abaixo continuam explícitos). CLI headless **P** (nenhum comando). UI **P** (nenhum controle). BB06 continua
**parcial**: esta fatia entrega o manifesto, a resolução com preflight e a materialização num
diretório de transporte. **Não** é `package.create` completo, não toca `host.rs`, pacote nem
intercâmbio, e **não** retira as guardas de pacote incompleto (BL011).

## Objetivo e limites

Dado um `DirectedScene` importado ([F031](directed-scene.md)):

1. **Core** (`tessel_editor_core::scene_resources`, puro: sem IO, sem caminho absoluto, sem GPUI,
   sem dependência de `scene_plan`) deriva um **manifesto determinístico** de todo arquivo de que
   o bundle depende, com dependências transitivas e consulta por `assetId`.
2. **Media** (`tessel_editor_media::directed_resources`) **resolve** o manifesto contra uma raiz de
   origem (preflight, só leitura) e **materializa** os recursos suportados numa raiz de transporte
   autorizada, com rollback.

Identidade = `assetId` (o áudio da mix, sem `assetId`, é `ResourceKey::AudioMix`). `relativePath`,
`revisionId` e `contentHash` são metadado e nunca chave. Cada recurso conserva o `assetId` e o
`relativePath` da origem; não há armazenamento de conteúdo duplicado (um arquivo compartilhado por
vários `assetId` é copiado uma vez e atende a todos).

### O que é classificado

| Status | O que é | Transporte |
|---|---|---|
| `Supported` | `assetRef` de nó `imagePlane` (PNG), de nó `splat` (SPZ) e `audio.mixRelativePath` | Carregado |
| `Uninterpreted` | arquivos citados só por seções que o importador preserva sem interpretar: `surface`, `normalMap`, `sceneDepth` (grade f32 + PNG), `relief` (grade f32) e takes de narração | **Listado em `skipped`, nunca copiado** (transporte parcial, `complete == false`) |
| `Unreferenced` | declarado em `assets[]`, citado por nada | Listado em `skipped` |

Dependências transitivas: o asset de um nó puxa os assets dos mapas citados por ele
(`dependencies`, `closure`). Só erros em recursos `Supported` bloqueiam o transporte; avisos
(`REFERENCE_IGNORED`, ...) informam.

### Caminhos de transporte (`ResourcePath`)

Mais estrito que o importador: `\` vira `/`, segmentos `.`/vazios caem; recusa `..`, absoluto/drive,
caracteres de controle e `<>:"|?*`, ponto/espaço final, nomes de dispositivo do Windows, o prefixo
reservado `.tessel-staging-`, mais de 1024 bytes (255 por segmento, 32 segmentos). Colisões:
mesmo caminho e mesmo hash = um arquivo compartilhado (não é colisão); mesmo caminho com hash
diferente, caminhos que só diferem em caixa e "arquivo vs pasta" são `PathCollision`. Hash exigido:
64 hex minúsculos (SHA-256), como a origem compara.

### Limites declarados

- PNG: assinatura e IHDR (dimensões declaradas), **sem decodificar pixels nem alfa**. SPZ via
  `splat::inspect`. Grade f32 e áudio: só hash.
- `collision_key` usa minúsculas, **sem dobra de normalização Unicode**.
- Identidade de arquivo aberto (dev/ino) da **materialização e dos carimbos** (`Stamp`) é **unix**;
  sem equivalente Windows nesta fatia (BL078). Resta uma janela TOCTOU para quem escreve na raiz
  concorrentemente; hardlinks não são detectáveis. O **ledger de claims** (BL111) é outra coisa:
  no Windows a identidade do `ledger.lock` (volume + índice do arquivo) é conferida desde BL167.6,
  e o diário aceita registros incompletos; limites e provas em
  [windows-ledger-report](../migration/windows-ledger-report.md).
- Cancelamento é observado entre blocos de cópia e antes de publicar, **não durante a publicação**
  (passo de publicação por hard links, parar no meio exigiria o mesmo rollback).
- Campos desconhecidos e luz/clima/efeitos/rigs/paths/`frameAudit` não são varridos em busca de
  arquivos (`CAPABILITIES.unscanned`).
- Hashes sintéticos das fixtures do oráculo (`aaa…`) **bloqueiam** o transporte por serem
  inválidos como SHA-256 hex de 64; os testes de mídia reescrevem o bundle com hashes reais.

## Contrato do core / biblioteca

```rust
use tessel_editor_core::directed_scene::DirectedScene;
use tessel_editor_core::scene_resources::ResourceManifest;

let scene = DirectedScene::import_str(text)?;
let manifest = ResourceManifest::from_scene(&scene);      // puro e determinístico
let hero = manifest.resource("hero");                      // por assetId
let found = manifest.lookup(&["hero", "ghost"]);           // found / unknown
let closure = manifest.closure("hero");                    // com dependências transitivas
let plan = manifest.transport_plan();                      // Err(Vec<ManifestIssue>) se bloqueado
```

| Item | Contrato |
|---|---|
| `ResourceManifest::from_scene` | Não muda a cena; ordem independente da ordem dos `assets[]` |
| Consultas | `bundle_id`, `revision_id`, `len`, `entries`, `supported`, `get(&ResourceKey)`, `resource(asset_id)`, `audio_mix`, `lookup(&[&str]) -> ResourceLookup{found, unknown}`, `uses_of`, `dependencies`, `dependencies_of`, `closure`, `for_scene`, `for_node`, `uninterpreted`, `issues`, `blocking_issues`, `collisions`, `is_transportable`, `is_complete`, `to_json` |
| `ResourceEntry` | `key`, `kind` (Image/Splat/Grid/Audio), `status`, `format`, `revision_id`, `content_hash`, `relative_path` (origem), `transport_path`, `path_error`, dimensões e `uses` (papel + ponteiro JSON) |
| `CAPABILITIES` / `MANIFEST_VERSION = 1` | O que esta versão lê e o que declara não ler |
| Códigos de issue | `RESOURCE_PATH_INVALID`, `RESOURCE_PATH_COLLISION`, `RESOURCE_HASH_INVALID`, `RESOURCE_FORMAT_MISMATCH`, `REFERENCE_UNRESOLVED`, `REFERENCE_MISMATCH`, `REFERENCE_IGNORED`, `LISTING_TRUNCATED` |

### Media

```rust
use tessel_editor_media::directed_resources::{materialize, preflight, Options};

let report = preflight(&manifest, &source_root, &Options::default(), &token, &mut |p| {})?;
if !report.ok() { /* report.problems (todos), report.manifest_issues */ }
let done = materialize(&manifest, &source_root, &transport_root, &Options::default(), &token, &mut |p| {})?;
```

- `preflight` não escreve. Coleta **um problema por arquivo** (`missing`, `symlink_rejected`,
  `escapes_root`, `not_regular_file`, `too_large`, `hash_mismatch`, `dimension_mismatch`,
  `invalid_content`, `unreadable`, `changed_during_check`) e um problema do conjunto (`too_large`,
  caminho vazio) se a soma passa de `max_total_bytes`.
- `materialize` em 3 passes: (1) decidir sem escrever (destino com o hash esperado é **reutilizado**,
  outro conteúdo é `destination_conflict`, livre exige origem verificada); (2) copiar para uma pasta
  de staging **dentro** da raiz de transporte com `File::create_new`, hash e cabeçalho conferidos na
  cópia; (3) publicar por `hard_link` (sem sobrescrever; sem fallback por rename) com diário que
  remove só o que esta chamada criou. Qualquer erro deixa a raiz como estava.
- Contenção da origem: caminho percorrido componente a componente com `symlink_metadata`,
  canonicalização e `starts_with` da raiz canônica, arquivo regular, limite de tamanho e conferência
  dev/ino do arquivo aberto (unix). `SymlinkPolicy::Reject` (padrão) recusa qualquer link;
  `AllowContained` segue só links que continuam dentro da raiz. O destino **nunca** é escrito por
  link nem sobrescrito.
- Erros (`JobError`): `manifest_blocked`, `resources_invalid` (`details.problems`), `root_unavailable`,
  `root_not_directory`, `disk_full`, `cancelled`, `destination_conflict`, `permission_denied`,
  `io_error` (`details.rollbackLeftovers` se o próprio rollback falhar).

## CLI headless

O módulo puro não faz I/O. A integração de operações `scene.resources.*`, pacote e intercâmbio está em [F040](scene-resource-integration.md), consumida pelo renderer F041.

## UI e CLI da janela

O módulo puro não tem apresentação. O fluxo da janela usa o mesmo host F040; controles dedicados de carregar/anexar ainda são parciais.

## Evidências e status

| Superfície | Evidência | Estado |
|---|---|---|
| Core | `crates/core/tests/scene_resources.rs` (27) e 6 testes unitários de `path.rs` | Passaram via Cargo `--locked` no gate do coordenador |
| Media | `directed_resources.rs` (19) e `directed_resources_transport.rs` (20) | Passaram via Cargo `--locked` em Unix; o teste de rollback pula se executado como root |
| Qualidade/gate | `test --workspace --locked`, `clippy --workspace --all-targets --locked -- -D warnings`, fmt, check, release e `cargo tree -p tessel-editor-core` | Passaram localmente ([relatório](../migration/round-hijkl-report.md)); sem CI de testes e sem Windows |

O comportamento de links usa unix; os módulos de symlink continuam sem execução em Windows
(criar link exige privilégio, BL167.1).

### Claims no Windows (BL167.6)

O ledger de claims que protege estes arquivos (`scene_resources::claims`, [F040](scene-resource-integration.md))
foi executado no Windows real por esta fatia. Estado: **implementado e provado no Windows; Mac a
revalidar** (a mudança de `claims.journal` também roda no Unix).

| Superfície | Evidência | Estado |
|---|---|---|
| Lib (core/media, sem janela) | `claims/tests.rs`: 48 testes, entre eles `trackers_never_hold_an_emptied_and_refilled_ledger_at_the_same_time`, `a_lock_file_replaced_at_its_path_is_not_the_one_that_was_opened`, `an_emptied_and_refilled_ledger_reads_the_same_every_time` e a corrida `emptying_and_refilling_a_ledger_is_never_mistaken_for_damage` | Windows: passam; antes a corrida ficava mais de 30 min com CPU ocupada |
| Dois processos reais | `scene_claims_processes` (14), `scene_claims_journal` (13, antes `cfg(unix)`), `job_discard_claims` (13), `scene_claims_integrity` (11), `scene_claims_jsonl` (6) | Windows: passam; `scene_claims_failure` depende do ajuste de `adopt.rs` que o [relatório](../migration/windows-ledger-report.md) propõe |
| CLI headless / janela | Sem mudança de contrato; `scene.resources.*` já era coberto pelos testes acima (processos `tessel-media` reais) | Sem prova nova de janela no Windows |

Limites que esta fatia **não** fecha: `scene_claims_budget` continua só Unix (mede tempo do tick, exigiria
a rodada de desempenho com o lock de GPU); no Windows o peer de `a_peer_stopped_while_holding_the_ledger_lock…`
segura o lock mas não é suspenso (sem `SIGSTOP`, e `unsafe` é proibido); `Stamp` continua sem
dev/inode (BL078); o atraso com que o SO libera um lock ao fechar o arquivo não foi demonstrado.
O `ledger.lock` é disputado por polling (`LOCK_POLL`), sem fila: sob contenção contínua um chamador pode
esperar quase todo o orçamento de 1,5 s dos jobs (medido: até 1,3 s com retenções abaixo de 11 ms). É
inanição, não violação de exclusão; o teste de exclusividade usa um orçamento próprio de 60 s e o limite de espera
tem testes próprios. Um open que o Windows recusa com "acesso negado" (remoção pendente do arquivo) é repetido até
50 vezes antes de a recusa ser acreditada.

## Integração pelo coordenador

Exports core/media adicionados. Gate direcionado: 27 testes core, 19+20 media e 1 teste de
integração H/I (4 bundles × 4 consultas de frame) passaram pelo Cargo --locked. O teste H/I usa
planos parciais explícitos quando necessário; isso prova identidade de recursos, não render completo.

Publicação exige hard links no filesystem de destino: falhas retornam erro sem fallback por
rename. Uma regressão que falhava com o fallback confirma esse fechamento seguro. O chamador
precisa controlar as raízes com exclusividade durante a operação; os testes de symlink não
provam proteção contra troca hostil concorrente de diretórios-pais. PNG é verificação de
cabeçalho, não decode de pixels/alpha. Pacote/intercâmbio continuam com as guardas atuais.
