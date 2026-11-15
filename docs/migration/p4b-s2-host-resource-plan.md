# P4b S2-H — união de recursos nos quatro carregadores do host

Data: 2026-11-15. Base: main 17d03933c3092acfac99850f88aa81e8f07288b3 (PR71/C0b integrada).
Backlog: BL082.4.4, fatia H; T10/T18 do plano canônico. Este documento fixa o contrato antes
do código. Status e atribuição ficam no backlog, sem uma fila concorrente neste relatório.

## Objetivo e fronteira

Os dois decks já têm plano e recursos próprios no core. O host ainda percorre apenas
plan.resources, logo perde um asset usado exclusivamente por transition.other.
Migrar os quatro carregadores para ScenePlan::resource_union(), preservando aprovação,
identidade, orçamento e atomicidade e acrescentando diagnóstico com usos por deck.

Esta PR entrega H, não R nem S2A. Refuse público continua fechado, F064 produto P,
Rust14/JS13, sem novo Domain, comando, controle de UI ou capability JS.
Não implementar shader, parser JS, blend, cache de desenho, render de transição,
export de transição ou preflight do intervalo inteiro. Não tocar a origem.

C0b está integrada com revisão Windows R1/fechamento e Mac por SHA:
215 transições + 8 Xestado em ff39b2e nos dois perfis; produção idêntica a c2959ce.
Mac core1731/0/7 e Windows core1728/2/7 pertencem a4ee9370, não a esta futura PR.
Os dois gizmos Windows da base continuam vermelhos. Workspace global não foi executado.
A pesquisa de H foi só leitura sobre d5d5dd3: nenhuma prova de loader novo foi executada.

## Decisões obrigatórias

1. O core já é o único dono da união: BTreeMap por assetId, ordenada, com UnionResource
   e used_by: Vec<DeckUse>. DeckUse é (Primary|Other,node_id). Mesmo node_id em dois decks
   são dois usos; não deduplicar por node_id cru. Não fundir as tabelas serializadas
   dos decks e não inventar ResourceId cruzando cenas.
2. A chave de aprovação continua (scene_id da cena ANEXADA ao projeto, Asset(asset_id)).
   scene_id de approved_map/resource_map não é source.scene_definition_id, instanceId,
   shotId nem o papel do deck. Os dois decks pertencem à mesma revisão de bundle.
3. Usar a identidade/igualdade de metadados já julgada por resource_union: revisão,
   hash, caminho, formato, dimensões, alpha e espaço de cor. Mesmo assetId com conteúdo
   divergente é invalid_plan no pointer do other, antes de resolver arquivos ou alterar slots.
   Não copiar a comparação do core para os adapters.
4. Aprovação, hash, header e containment da materialização continuam no media existente.
   approved_map/resource_map só consultam o resolver aprovado; não abrir, canonicalizar,
   hashear ou ler arquivo na UI para fingir uma prova adicional.
5. Recurso compartilhado tem uma identidade, um slot e um custo de bytes/decodificação.
   O custo de splats por quadro conta TODOS os usos qualificados, inclusive IDs de nó iguais
   nos dois decks. Preservar o teto de splats, slots e bytes, sem aumentar orçamento.
   Contagem/custo não pode dar wrap ou panic quando soma ambos os decks; se a união revelar
   overflow alcançável, recusar com orçamento sem mutar estado e registrar o conserto restrito.
6. Todo erro atribuível a um recurso inclui seu asset_id e usos {deck,node_id}.
   O campo aditivo de contexto será details.resource:
   { "asset_id": id, "used_by": [{ "deck": "primary|other", "node_id": id }, ...] }.
   Preservar detalhes existentes, sem apagar pointer, expected/got, geração ou backend_code.
   Mensagem também identifica o(s) deck(s), porque o caminho JobError não preservava detalhes.
   Para recurso compartilhado informar ambos os usos. O contexto acompanha o papel atual
   antes/depois do corte, sem codificar A/B ou from/to como se fossem Primary/Other fixos.
7. Preservar taxonomia de erro: approved_map mantém ResolveError.code(), não encaminhar
   tudo por core::headless::backend_error, que converteria resource_unknown em render_failed.
   Converter PlanError pela regra existente, com pointer qualificado. Recursos globais
   (root inválido, orçamento total) não ganham atribuição inventada a um asset.
   Em scene_export::job_error manter código/cancelamento/backend_code; carregar o novo
   contexto resource até o JobError sem alargar desnecessariamente outros contratos de erro.
8. requires do topo JÁ é união dos decks no core. Renderer/monitor já negociam esse conjunto.
   Não recalcular capacidades nos adapters, alterar Domain, contornar negociação ou anunciar
   fxTransition no runtime. Um runtime antigo deve continuar recusando essa capacidade.
9. Plano sem transição mantém comportamento, mapas e protocolo antigos salvo o contexto
   aditivo nos erros de recurso. Os bytes do ScenePlan e os fixtures/goldens continuam intactos.

Antes do código, salvar este plano e documentar assinaturas/modelo de wanted/error helper
que serão usados. Commit próprio de contrato. Ajustes de nomes dentro dessas regras são
rotina; registrar antes da implementação, sem pedir confirmação desnecessária.

## Costuras e ownership

| Entrada | Arquivo | Mudança |
|---|---|---|
| SceneResourceMap::from_root | crates/render/src/scene.rs (~162/173) | Iterar união; preservar canonicalização e recusa de symlink que sai da raiz |
| SceneSlots::resolve | mesmo arquivo (~561/568) | União como wanted, identidade/custo por recurso e usos de splat por deck |
| scene_operation::approved_map | crates/render/src/scene_operation.rs (~11/17) | Resolver pelo scene_id anexado + asset_id da união, preservando códigos |
| scene_export::resource_map | crates/render/src/scene_export/mod.rs (~31/38) | Mesmo contrato da união; contexto chega ao erro do job |

scene.rs tem777linhas nesta base; scene_operation79 e scene_export124.
resolve hoje devolve Vec<(&PlanResource,Identity,PathBuf,u64)>.
Consumir UnionResource por valor (ou um wanted privado pequeno que o retenha), mantendo
used_by qualificado. picture_bytes/load_message podem receber &UnionResource; para formato
usar AssetFormat::Spz. Não converter a união de volta a PlanResource inventando Vec<String>
para os usos. Não criar outro modelo de plano ou recurso público.

Permitidos: esses três arquivos; helper privado pequeno de contexto/recursos em crates/render/src;
lib.rs apenas para declarar módulos; testes render dedicados/fixtures de teste gerados em memória.
Se um teste privado for necessário, módulo sob cfg(test) real; nome *_tests.rs sozinho não exclui
código de produção. Accept oculto só em teste, jamais introduzido em src ativo para exercitar host.
Extensão de caller estritamente necessária (ex.: CLI que já chama from_root) deve ser registrada
com motivo e diff mecânico; não implementar regras em desktop/media ou mexer nos dados do core.

Preferir novos módulos focados <800linhas. Não deixar Rust >1000linhas crescer; extrair antes
se necessário. Evitar engrossar scene.rs com centenas de linhas de helpers/testes. Nenhum
crate/dependência nova; Cargo.toml/Cargo.lock inalterados. Prosa contratual pode crescer onde
é necessária, com exceção documental identificada, sem alegação genérica falsa sobre tamanhos.

Docs permitidos: este plano; relatório próprio docs/migration/p4b-s2-host-resource-report.md;
docs/features/{fx-transitions,scene-plan,scene-resource-integration,scene-export}.md nas cláusulas
afetadas. Backlog/matriz/índice ficam com coordenador: entregar PATCH LITERAL das linhas
BL082.4.4, F064 e notas F040/F041/F042 pertinentes, com escopo interno/limites e link do relatório.
Não criar IDs por conta própria nem novas filas em relatórios; pendências novas são propostas.

## Marco A — mapas e diagnósticos

- Todos os quatro caminhos percorrem a mesma união derivada do core. Manter uma ordem estável
  de resolução. Slots e mensagens compartilham o mesmo recurso quando o assetId/metadados coincidem.
- SceneResourceMap::from_root mantém raiz explícita/aprovada e containment real por canonicalize;
  revisar root inválida, arquivo ausente e symlink para fora. Não afrouxar checagem Windows.
- approved_map precisa de sessão media real nos testes quando declarar aprovação real.
  resource_map pode usar ResourceResolver instrumentado, e uma prova com aprovação media real.
  Não confundir um fake resolver com arquivos validados por hash/header pelo media.
- Mapear corretamente geração/revisão/hash/path. Resolver sempre com o scene_id de projeto
  recebido, mesmo que source do other tenha outra scene_definition_id/scene_instance_id.
- As quatro portas recusam metadados divergentes no mesmo assetId pela união antes de consultar
  o resolver ou alterar o livro de slots. Teste instrumentado deve provar ordem/fail-closed.
- O contexto novo é derivado de used_by; nenhum scan de nós pelos IDs crus para adivinhar deck.
  Verificar que chega na mensagem e nos detalhes do CommandError/BackendError/JobError pertinente.
  Não aumentar a superfície da API só para expor uma private function ao teste.

## Marco B — slots, bytes, splats e atomicidade

- SceneSlots::prepare é a porta pública tipada do livro; resolve pode ser testado por módulo
  interno. Toda falha acontece antes da primeira mutação do livro/generation/clock/held.
- Dois decks com um recurso compartilhado e um exclusivo: union size, occupied, carga uma vez,
  mensagens de carga/uso e geração estável. Repetir quadro não deve gerar carga duplicada.
- Trocar geração conserva semântica atual de stale/release. Falha em outro asset/other não
  libera um slot válido já retido nem deixa novo recurso parcialmente carregado.
- Teto de slots/bytes da UNIÃO: exato aceita, uma unidade insuficiente recusa com mesma
  taxonomia, e estado/mensagens permanecem atômicos. Contar bytes compartilhados uma vez.
- SPZ real e pequeno: dois nós com o mesmo ID em decks diferentes contam dois usos no
  frame_splats; bytes do arquivo/decode contam uma vez. Testar limite exato e uma unidade
  abaixo; não testar só nós de IDs diferentes. Preservar inspeção de formato e orçamento.
- Manter casos sem transição como regressão de mapa/identidade/mensagens/custos. Não
  associar essas provas a renderização real da transição ou liberação de leases do runtime.

## Matriz mínima de provas (T10/T18, sem promover paridade)

Fixture sintético importável via core Accept SOMENTE no teste, com PNGs reais pequenos
(e SPZ real quando testar splats), hashes reais e mapa/sessão aprovado pelo media onde alegado.
Não reutilizar hash sintético de um teste de core como se fosse aprovação de arquivo real.
Asserções independentes e dados esperados literais; não chamar a função de contagem sob teste
para certificar seu próprio resultado. Sem regravar corpus, fixtures ou baselines existentes.

1. Asset exclusivo do other presente nas quatro portas; após o corte o mesmo asset é Primary
   e a união continua correta. Conferir IDs/paths/revisão/hash/geração e ordenação.
2. Asset exclusivo ausente:
   - from_root: arquivo ausente => recusa real de canonicalização com contexto other;
   - slots: entrada ausente e, separadamente, arquivo removido depois da aprovação => recusa;
   - approved_map/resource_map: ausente do mapa APROVADO => resource_unknown/not_carried
     conforme contrato atual, com contexto other. NÃO alegar verificação de existência física.
3. Arquivo removido depois da aprovação: os dois mappers ainda retornam mapa aprovado,
   slots recusam ao verificar is_file. Remover antes de media::load só prova preflight do
   bundle inteiro; esse caso não prova por si só que um mapper percorreu o other.
4. Mesmo assetId/metadados compartilhados e mesmos nodeIds em ambos: uma carga e dois usos,
   planos/tabelas serializados intactos. Mesmo assetId com metadados divergentes => invalid_plan
   com pointer do other antes de resolver/mutar. Não inventar novo namespace por deck para assets.
5. Revisão/hash/generation errados e caminho inválido preservam códigos e atomicidade; detalhes
   existentes e novo contexto coexistem. A mensagem de export/job identifica o deck também.
6. Recursos do other somados ao orçamento; SPZ compartilhado com dois usos; exato/abaixo.
7. Sem transição: comportamento anterior. Entrada pública real (headless) continua recusando
   transitions, se reexecutada. Nenhum Accept público ou capability JS novo para facilitar teste.

approved_map/scene_export::resource_map são helpers de host que aceitam plano tipado.
scene_operation::prepare ainda não pode receber uma cena Accept pela entrada pública.
Testes desses helpers e SceneSlots::prepare não são monitor público nem CLI positiva de transição.
Não inventar estado paralelo ao core nem fake de GPU/runtime para alegar essa cobertura.
O parser/desenho da transição e o preflight do intervalo inteiro continuam nas fases R/S2A.

## Mutantes discriminantes e controles

Um por vez, com restauração por bytes/hash e mtime tocado antes do controle limpo:
- Voltar cada um dos quatro loops a plan.resources deve falhar em seu teste específico.
- Usar source.scene_definition_id/instanceId em vez de scene_id de projeto deve falhar.
- Deduplicar uso de splat por node_id cru ou contar só primary deve falhar no limite.
- Cobrar bytes compartilhados duas vezes deve falhar no limite; ignorar recursos do other
  também. Quando o código existente já saturar/checar, não introduzir wrap no teste.
- Perder contexto other na mensagem/detalhe, achatar resource_unknown para render_failed,
  ou descartar contexto no JobError deve ser detectado. Testar só frases sem validar código
  e payload não fecha a prova.
- Remover a fase de validação antes de mutar slots deve ser detectado por estado intacto.
  Se um mutante for equivalente/inatingível, explicar e não chamá-lo morto.
Erro de compilação, zero testes, timeout, falha de ambiente ou orçamento não contam como
mutante morto. Relatar sobreviventes e distinguir controles de injeção de IO real.

## Gates e entrega

- Antes/depois: cwd, origin, branch/base/head, status, espaço e folga de memória virtual.
  Projeto Windows dedicado preparado via T3; não criar checkout/worktree/clone pelo shell.
  Não encerrar processos alheios nem fazer limpeza, reparo SQLite, restart ou pagefile.
- Cargo pelo ./scripts/cargo via Git Bash REAL C:\Program Files\Git\bin\bash.exe, target padrão
  controlado pelo mbx. Nunca CARGO_TARGET_DIR/--target-dir/lane/clean/gc manual.
  Sem artefatos/TEMP >500MB; fixtures sintéticos pequenos e limpeza somente dos próprios.
- fmt, check workspace, clippy workspace all-targets -Dwarnings, workspace --no-run e
  build -p tessel-editor. Executar testes render de recursos/host/slots e os novos testes H;
  core focado 215transições/8Xestado e baseline, tree core sem gpui/wry.
- Como regressão do transporte de recursos, executar o target render scene_export com
  TESSEL_REQUIRE_RUNTIME=1 sobre planos antigos sem transição, usando setup oficial do runtime
  se necessário. Isso é regressão funcional do caminho antigo, não prova de transição/pixel/GPU.
  Se faltar ferramenta real ou o ambiente bloquear, reportar o gate como bloqueado; nunca skip
  silencioso nem fallback falso. Nenhuma mudança em JS/deps/FFmpeg para fazer teste passar.
- Workspace global Windows tem falhas de base conhecidas; não chamá-lo verde. Core completo
  não precisa ser repetido se fontes/deps core não mudaram; não herdar suas contagens como nova
  execução. Relatar checks compilados vs executados, cache e skips, ambiente e cabeçalho de logs.
- Commitar contrato antes do código (incluindo modelo privado wanted e forma exata do contexto).
  Usar skill commit se disponível, mensagens limpas sem referência/trailer a assistentes.
  Se harness injetar trailer, sanear só commits locais não publicados com árvore idêntica,
  via plumbing; nunca force. Branch própria feat/p4b-s2h-host-resources.
- Push normal, PR contra main; link_pull_request com URL completo como próxima toolcall.
  Filho não faz watch/poll/merge. Entregar report porSHA, comandos/contagens, 4portas/matriz,
  mutantes, hashes de dados antes/depois, ownership ampliado se houve e patches literais.
- Coordenador fará revisão independente Windows e gates Mac porhead antes do aceite. H interna
  validada não libera R nem S2A e não transforma F064 em produto habilitado.

## Contrato final de assinaturas, modelo wanted e forma de erro (fixado ANTES do código)

Registro da implementação; nenhuma regra acima muda. Base lida e conferida com
`(Get-Content f).Count`: `scene.rs` 777 linhas, `scene_operation.rs` 79, `scene_export/mod.rs` 124
(as do plano).

### 1. Assinaturas públicas: INALTERADAS

```rust
SceneResourceMap::from_root(generation: u64, plan: &ScenePlan, root: &Path) -> Result<Self, BackendError>
SceneSlots::prepare(&mut self, plan: &ScenePlan, resources: &SceneResourceMap) -> Result<Prepared, BackendError>
scene_operation::approved_map(plan: &ScenePlan, scene: &str, session: &SceneResourceSession) -> Result<SceneResourceMap, CommandError>
scene_export::resource_map(plan: &ScenePlan, scene_id: &str, resolver: &dyn ResourceResolver, generation: u64) -> Result<SceneResourceMap, BackendError>
```

Nenhuma função, tipo ou campo público novo. Os callers existentes (CLI, desktop, `SceneRenderer`)
não mudam.

### 2. Módulo privado novo `crates/render/src/resource_union.rs`

Declarado em `lib.rs` como `mod resource_union;` (única edição de `lib.rs`). Itens `pub(crate)`:

```rust
/// resource_union() em ordem ascendente de assetId (a do BTreeMap), por valor.
fn union_of(plan: &ScenePlan) -> Result<Vec<UnionResource>, PlanError>;
/// {"asset_id": id, "used_by": [{"deck": "primary"|"other", "node_id": id}, ...]}  (ordem de used_by)
fn resource_context(resource: &UnionResource) -> Value;
/// Contexto de um PlanError da união: só para o pointer exato `/transition/other/resources/{n}`
/// (o do conflito de metadados): asset_id = other.resources[n].asset_id; used_by = usos Primary
/// do mesmo assetId (se o primário o tem) + usos Other, tudo vindo das listas `used_by`.
fn union_failure_context(plan: &ScenePlan, error: &PlanError) -> Option<Value>;
/// Anexa o contexto de UM recurso a um erro já tipado, sem trocar o código.
fn attribute_backend(error: BackendError, resource: &UnionResource) -> BackendError;
fn attribute_command(error: CommandError, resource: &UnionResource) -> CommandError;
```

Regras de `attribute_*`:

- `code`, `target`, `expected/got/generation/backend_code` e todo campo de `details` existente
  ficam. `details` objeto → ganha a chave `resource`; sem `details` → `{ "resource": … }`;
  `details` não objeto → `{ "detail": <o antigo>, "resource": … }`.
- A mensagem ganha o sufixo único
  ``" [asset `<id>` used by: primary deck node `<n1>`, other deck node `<n2>`]"``
  (um item por uso de `used_by`, na ordem da união: primário primeiro). Um recurso exclusivo do
  other diz só `other deck node …`; o papel é o ATUAL do plano (depois do corte o mesmo asset
  aparece como `primary`).
- Erros globais (raiz inválida, `resource_budget` de slots/bytes/splats, `resource_stale`,
  `unsupported`, `bundle`, plano inválido fora do pointer acima) NÃO são atribuídos a um asset.

### 3. Conversão do erro da união (regra existente de cada adapter)

| Porta | `PlanError` da união → |
|---|---|
| `from_root`, `prepare`, `resource_map` | `scene::plan_error(&e)` (`BackendError` de código `invalid_plan`, mensagem com o pointer) + `details.resource` via `union_failure_context` |
| `approved_map` | `CommandError::from(e)` (a regra do core, `invalid_plan`) + `details.resource` via `union_failure_context` |

`approved_map` mantém `ResolveError::code()` (`resource_unknown`, `resource_not_carried`,
`scene_resources_not_loaded`, `stale_generation`, `resource_budget_exceeded`) e passa por
`attribute_command`. O erro de `ResolvedResource` com caminho relativo (`resource_path_invalid`)
mantém o mapeamento `core::headless::backend_error` que já tinha (`render_failed`), agora com o
contexto. `resource_map` mantém `BackendError::new(error.code(), …)` + `attribute_backend`.

### 4. Modelo `wanted` privado de `scene.rs`

```rust
struct Wanted { resource: UnionResource, identity: Identity, path: PathBuf, bytes: u64 }
fn resolve(&self, plan: &ScenePlan, resources: &SceneResourceMap) -> Result<Vec<Wanted>, BackendError>
```

- `resolve` itera `union_of(plan)`; a falha da união acontece antes de qualquer consulta ao mapa
  de recursos e antes de qualquer mutação do livro. `prepare` já chamava `plan.validate()`
  (que contém a união) antes; as duas portas dão o mesmo `invalid_plan`.
- `picture_bytes(&UnionResource)` e `load_message(.., &UnionResource, ..)`; formato splat é
  `AssetFormat::Spz`. Uma identidade, um slot, uma carga e um custo de bytes por `assetId`.
- Usos de splat por quadro: `resource.used_by.len().max(1)`, sobre a lista QUALIFICADA (dois nós
  `cloud-node` em decks diferentes são 2). Bytes do arquivo e do decode contam uma vez.
- Todos os erros de recurso de `resolve` (`resource_unresolved`, `resource_mismatch` por
  revisão/hash, não arquivo e formato splat) passam por `attribute_backend`. Os dois tetos
  globais não.
- Endurecimento restrito de overflow (decisão 5): a soma de bytes do quadro e `held + incoming`
  passam a `saturating_add`. Antes, dois recursos com `widthPx × heightPx × 4` saturado
  (`u64::MAX`) somavam com `Iterator::sum` (panic em debug, wrap em release e aceite). Agora o
  total satura e o quadro é recusado com `resource_budget`, sem mutar o livro.

### 5. `scene_export::job_error`

Código, cancelamento e `backend_code` como estavam. Se `BackendError.details["resource"]` existe,
ele é copiado para `JobError.details.resource` (nada mais de `details` é copiado). A mensagem do
`BackendError` (com o sufixo de decks) já é a do `JobError`.

### 6. Ordem da porta e testes

Ordem de resolução: ascendente por `assetId`. Os testes novos são de integração
(`crates/render/tests/scene_host_union*.rs`) e usam só API pública; a cena com duas decks é
importada pelo Accept oculto SOMENTE nos testes (`crates/render/tests`, fora do que o scanner de
`crates/*/src` lê). Os arquivos são PNGs/SPZ reais pequenos (`core::fixtures`), hashes reais
(`media::fsx::hash_file`) e aprovação por `media::scene_resources::{load, SceneResourceSession}`.

### 7. Arquivos tocados (registro prévio)

`crates/render/src/{scene.rs, scene_operation.rs, scene_export/mod.rs, lib.rs, resource_union.rs}`;
testes novos em `crates/render/tests/`; docs: este plano, o relatório e as cláusulas de
`docs/features/{fx-transitions,scene-plan,scene-resource-integration,scene-export}.md`.
Nenhum caller de `from_root` precisa mudar. `Cargo.toml`/`Cargo.lock` inalterados.
