# P4b S2-H — relatório: união de recursos nos quatro carregadores do host

Backlog **BL082.4.4**, fatia H (T10/T18 do plano canônico). Branch `feat/p4b-s2h-host-resources`, base `main`
`17d03933c3092acfac99850f88aa81e8f07288b3` (PR71/C0b integrada). Plano e contrato fixados **antes** do código:
[p4b-s2-host-resource-plan.md](p4b-s2-host-resource-plan.md). Status: **implementado, a validar** (revisão independente,
rechecagem Windows final pendente após R1 condicional em19c5ea1; Mac e regressão `scene_export` com runtime concluídos em8a489bf; ver §14).

## 1. Contrato final

- Assinaturas públicas **inalteradas**: `SceneResourceMap::from_root`, `SceneSlots::resolve`/`prepare`,
  `scene_operation::approved_map`, `scene_export::resource_map`, `job_error` (privada) e os códigos de erro.
- Os quatro percorrem `ScenePlan::resource_union()` (BTreeMap por `assetId`, `used_by` qualificado por deck) por meio do
  módulo **privado** `crates/render/src/resource_union.rs`. Um `assetId` usado só por `transition.other` deixa de se
  perder; um `assetId` compartilhado é resolvido, aprovado e carregado **uma vez**.
- Chave de aprovação: `(cena do projeto, Asset(assetId))`. Nunca `source.scene_definition_id`, `scene_instance_id`,
  shot, id de nó ou papel de deck.
- Metadados divergentes para o mesmo `assetId`: recusados antes de resolver, abrir arquivo ou tocar a raiz
  (`/transition/other/resources/N`, código do adapter inalterado). `from_root` calcula a união **antes** de
  canonicalizar a raiz.
- Erro atribuível a um recurso: `details.resource = {"asset_id": id, "used_by": [{"deck": "primary|other", "node_id": id}]}`,
  detalhes existentes preservados e mensagem com o sufixo `[asset `<id>` used by: primary deck node `a`, other deck node `b`]`.
  O código é preservado (`approved_map` mantém `ResolveError::code()`; não passa por `backend_error`, que achataria para
  `render_failed`). `job_error` copia só `details.resource` para o `JobError` (junto de `backend_code`).
  Erros globais (raiz inexistente, tetos totais de slots/bytes/splats) **não** recebem atribuição inventada.
- Custo: o teto de splats soma **cada uso qualificado** dos dois decks (ids de nó iguais em decks diferentes contam duas
  vezes); bytes de um arquivo compartilhado contam uma vez. Custos e soma total não representáveis são **recusados**, inclusive
  com orçamento `u64::MAX`, antes de mudar slots. A eviction exige soma verificada. A saturação inicial do autor
  não fechava esse caso no orçamento máximo e foi corrigida pelo coordenador (§11).
- A admissão pública de transições continua `Refuse` (`invalid_bundle`, issue `FX_TRANSITION_NOT_YET`). A admissão
  oculta `Accept` só aparece em `crates/render/tests`.

## 2. Diff e ownership originais (7e7c83f; reforços em §§11–16)

`git diff --stat 17d03933..7e7c83f` (somente ownership da fatia; arquivos comuns **não** foram editados):

| Arquivo | Linhas | Papel |
|---|---|---|
| `crates/render/src/resource_union.rs` | +161 | módulo privado: `union_of`, contexto, atribuição |
| `crates/render/src/scene.rs` | 203 linhas alteradas | `from_root`, `SceneSlots` (`Wanted`), splats, somas saturadas |
| `crates/render/src/scene_operation.rs` | +14 | `approved_map` |
| `crates/render/src/scene_export/mod.rs` | +29 | `resource_map`, `job_error` |
| `crates/render/src/lib.rs` | +1 | `mod resource_union;` |
| `crates/render/tests/scene_host_union_common/mod.rs` | +338 | fixture compartilhada (só testes) |
| `crates/render/tests/scene_host_union_maps.rs` | +460 | 10 testes |
| `crates/render/tests/scene_host_union_slots.rs` | +627 | 13 testes |
| `docs/migration/p4b-s2-host-resource-plan.md` | +333 | plano e contrato |
| docs de feature (este PR) | — | `fx-transitions`, `scene-plan`, `scene-resource-integration`, `scene-export` |
| este relatório | — | — |

`Cargo.toml` e `Cargo.lock` inalterados (`git diff 17d03933..HEAD -- Cargo.toml Cargo.lock` vazio; SHA-256 atuais
`Cargo.lock` 2CA1AB56…DB63, `Cargo.toml` 407B3BD9…01A7). Nenhum arquivo Rust novo passa de 1000 linhas
(maior arquivo tocado: `scene_host_union_slots.rs`, 627; `scene_seam.rs`, 1145, é anterior e não foi tocado).
`cargo tree -p tessel-editor-core` não lista `gpui`/`wry` (0 ocorrências). Sem origem, serviço, runtime JS/shader,
capability, UI, preflight de intervalo nem ativação S2A.

## 3. As quatro portas

| Porta | IO | Códigos preservados | Testes (`crates/render/tests/`) |
|---|---|---|---|
| `SceneResourceMap::from_root` | **arquivo**: `canonicalize` do root e de cada `relativePath`, recusa de escape (link/junção) | os códigos existentes de `from_root` (plano da união, arquivo ausente, escape da raiz) com o contexto do deck; raiz inexistente global | `scene_host_union_maps`: carrega o recurso só-vizinho antes/depois do corte; arquivo ausente nomeia o deck `other`; raiz inexistente sem atribuição; junção que sai da raiz recusada; conflito de metadados antes de qualquer IO |
| `SceneSlots::resolve`/`prepare` | **arquivo** (`is_file`; inspeção de SPZ) e **livro** de slots; hash é declarado pelo mapa aprovado | `invalid_plan`, `resource_budget`, `resource_mismatch`, revisão/hash/ausente, `stale_generation` | `scene_host_union_slots`: compartilhado + exclusivo = uma entrada cada e carregados uma vez no corte; geração nova; recusas atômicas (livro não se move); removido após aprovação; tetos de slots/bytes no limite exato; overflow; eviction com imagem enorme; splats |
| `scene_operation::approved_map` | **aprovado** (`SceneResourceSession`): nenhum arquivo é aberto | `resource_unknown`, `resource_not_carried`, `scene_resources_not_loaded`, `stale_generation`, `resource_budget_exceeded` | ausência no aprovado → `resource_unknown` com o deck; chave = cena do projeto; caminho aprovado relativo mantém o código; removido depois da aprovação ainda mapeia |
| `scene_export::resource_map` (+ `job_error`) | **aprovado** (`ResourceResolver`); o job lê o arquivo depois | idem, mais `render_failed`/`backend_unavailable` no `JobError` com `backend_code` | os mesmos casos nos três tipos de erro (`CommandError`, `BackendError`, `JobError`); regressão de plano sem transição |

Regressões existentes executadas no head `7e7c83f`: `scene_seam` 28/28, `scene_flow_resources` 4/4, lib do render 16/16.

## 4. Custos e splats

Plano do fixture (bundle público `valid-cut-instance-offsets`, mais uma `crossDissolve` de 12 quadros): `backdrop`
64×36×4 = 9216, `figure` 32×32×4 = 4096, `sky` 8×8×4 = 256 → união 13568 bytes.

- Teto de bytes: exatamente 13568 aceito, 13567 recusado (`resource_budget`); teto de 2 slots recusa 3 recursos.
- Imagem compartilhada conta **uma vez** nos bytes e nos slots (mutante M12).
- Splat compartilhado sob o mesmo id de nó nos dois decks: 2 usos × 100 = 200 aceito, 199 recusado; bytes do arquivo uma
  vez. Splat só no vizinho e só no primário: 100 aceito / 99 recusado (mutantes M09–M11).
- Overflow inicial em7e7c83f: os testes só fechavam o orçamento padrão por saturação. A correção
  posterior e os controles de orçamento máximo/custo individual estão em §§11–16; M23/M24 são históricos.

## 5. Mutantes do autor em7e7c83f (30 relatados, um de cada vez)

Cada mutante é aplicado por substituição exata de texto na árvore de trabalho, roda os dois alvos novos
(`scene_host_union_maps` + `scene_host_union_slots`, `--no-fail-fast`), é revertido por `git checkout --` e a restauração é
conferida pelo **SHA-256** do arquivo, com `mtime` tocado. Nenhum `RESTORE-HASH-MISMATCH`; a árvore terminou limpa.

**Resultado relatado pelo autor em7e7c83f: 30/30 mortos, 0 sobreviventes; não é prova dos reforços posteriores.** Uma primeira rodada deu `BUILD-ERROR` em M01–M11 por um `LNK1104`
(exe aberto durante o link, no Windows) e foi **descartada** como "não é morte": a rodada foi interrompida, a árvore
restaurada (`scene.rs` voltou por `git checkout`), o script ganhou repetição para `LNK1104` e classificação separada
(`LINK-LOCKED`, `BUILD-ERROR`, `NO-RESULT` nunca contam como morte) e **todos** os 30 foram reexecutados. M29 não se aplicou
na reexecução por um trecho de texto errado, foi corrigido e reexecutado sozinho (morto por 3).

| Mutante | Resultado |
|---|---|
| M01 from_root walks plan.resources | morto por 6 teste(s) [FAILED 6p/4f; FAILED 11p/2f] |
| M02 slots resolve walks plan.resources | morto por 10 teste(s) [ok 10p/0f; FAILED 3p/10f] |
| M03 approved_map walks plan.resources | morto por 13 teste(s) [FAILED 5p/5f; FAILED 5p/8f] |
| M04 resource_map walks plan.resources | morto por 5 teste(s) [FAILED 5p/5f; ok 13p/0f] |
| M05 approved_map keyed by source.scene_definition_id | morto por 15 teste(s) [FAILED 4p/6f; FAILED 4p/9f] |
| M06 approved_map keyed by source.scene_instance_id | morto por 15 teste(s) [FAILED 4p/6f; FAILED 4p/9f] |
| M07 resource_map keyed by source.scene_definition_id | morto por 6 teste(s) [FAILED 4p/6f; ok 13p/0f] |
| M08 resource_map keyed by source.scene_instance_id | morto por 6 teste(s) [FAILED 4p/6f; ok 13p/0f] |
| M09 splat uses deduplicated by raw node id | morto por 1 teste(s) [ok 10p/0f; FAILED 12p/1f] |
| M10 splat uses count only primary (max 1) | morto por 1 teste(s) [ok 10p/0f; FAILED 12p/1f] |
| M11 splat uses count only primary (no max) | morto por 2 teste(s) [ok 10p/0f; FAILED 11p/2f] |
| M12 shared picture bytes charged once per use | morto por 2 teste(s) [ok 10p/0f; FAILED 11p/2f] |
| M13 bytes of neighbour-only resources ignored | morto por 3 teste(s) [ok 10p/0f; FAILED 10p/3f] |
| M14 attribute_backend drops details.resource | morto por 8 teste(s) [FAILED 6p/4f; FAILED 9p/4f] |
| M15 attribute_command drops details.resource | morto por 3 teste(s) [FAILED 7p/3f; ok 13p/0f] |
| M16 attribute_backend drops the message suffix | morto por 4 teste(s) [FAILED 8p/2f; FAILED 11p/2f] |
| M17 attribute_command drops the message suffix | morto por 2 teste(s) [FAILED 8p/2f; ok 13p/0f] |
| M18 approved_map flattens resource_unknown to render_failed | morto por 3 teste(s) [FAILED 7p/3f; ok 13p/0f] |
| M19 job_error drops details.resource | morto por 3 teste(s) [FAILED 7p/3f; ok 13p/0f] |
| M20 message names every use as primary | morto por 6 teste(s) [FAILED 6p/4f; FAILED 11p/2f] |
| M21 details.resource names every use as primary | morto por 9 teste(s) [FAILED 5p/5f; FAILED 9p/4f] |
| M22 book moved before the plan is resolved | morto por 8 teste(s) [ok 10p/0f; FAILED 5p/8f] |
| M23 byte sum back to Iterator::sum | morto por 2 teste(s) [ok 10p/0f; FAILED 11p/2f] |
| M24 held + incoming back to plain add | morto por 1 teste(s) [ok 10p/0f; FAILED 12p/1f] |
| M25 approved_map generation 0 | morto por 3 teste(s) [FAILED 9p/1f; FAILED 11p/2f] |
| M26 resource_map ignores the generation | morto por 1 teste(s) [FAILED 9p/1f; ok 13p/0f] |
| M27 resource_map resolves in reverse order | morto por 1 teste(s) [FAILED 9p/1f; ok 13p/0f] |
| M28 union conflict loses its resource context | morto por 2 teste(s) [FAILED 9p/1f; FAILED 12p/1f] |
| M29 identity generation 0 in the book | morto por 3 teste(s) [ok 10p/0f; FAILED 10p/3f] |
| M30 from_root ignores the union failure context | morto por 1 teste(s) [FAILED 9p/1f; ok 13p/0f] |

Limites: os mutantes são do autor (não independentes); a lista não prova a ausência de outros defeitos nem cobre o
runtime. Um mutante só conta como morto se o alvo **compilou** e um teste falhou.

## 6. Gates (head de código `7e7c83f826388fd21b545256e6c06106c97a2c31`; docs em seguida)

Windows, Git Bash real, `./scripts/cargo`, alvo padrão/mbx, sem `CARGO_TARGET_DIR`/lane/clean/gc manual.

| Gate | Resultado | Cache mbx / observação |
|---|---|---|
| `cargo fmt --all -- --check` | exit 0 | — |
| `check --workspace --locked` | exit 0 (55 s) | 564 hits / 34 misses |
| `clippy --workspace --all-targets --locked -- -D warnings` | exit 0 | 52 hits / 141 misses |
| `test --workspace --locked --no-run` | exit 0 (**compilou** todos os alvos; **não** executou) | 323 hits / 141 misses |
| `build -p tessel-editor --locked` | exit 0 | 0 hits / 19 misses |
| render: lib + `scene_seam` + `scene_flow_resources` + 2 alvos H | **16 + 28 + 4 + 10 + 13 = 71 passaram, 0 falharam**, executados | — |
| core focado, perfil isolado (`TESSEL_EXPECT_PRESERVE_ORDER=0`) | `scene_fx_transition` **215/215**, `pilot_dissolve_corpus` 10/10, `scene_fx_target` 10/10 (F061, não X estado), `scene_plan_transition_baseline` 1/1 | 4 hits / 5 misses |
| core focado, `--features serde_json/preserve_order` (`=1`) | os mesmos 215/10/10/1, todos verdes | 0 hits / 14 misses |
| `cargo tree -p tessel-editor-core` | sem `gpui`/`wry` | — |
| `scene_export` render com `TESSEL_REQUIRE_RUNTIME=1` | **BLOQUEADO** (12/12 falharam **pelo guarda**, não por regressão) | ver §8 |

Pulos: nenhum teste foi pulado silenciosamente. Não executei o workspace global nem o rotulei de verde: dois gizmos Windows da
base continuam vermelhos no core completo (já registrado em C0b), e não repeti o core completo.

## 7. Dados antes/depois

Nenhum dado, fixture, `Cargo.*` ou arquivo comum mudou. Os testes constroem os recursos em diretório temporário próprio
(removido ao fim, `Drop`), a partir de `tessel_editor_core::fixtures::{encode_png, splat_spz}` e do bundle público; o
`Accept` oculto não existe em `src`. Mensagens de erro de recursos de planos **sem** transição ganham o sufixo de atribuição
(aditivo; os testes de selo existentes continuam verdes).

## 8. Bloqueios e limites honestos na entrega do autor (histórico de 7e7c83f)

- **Relato de bloqueio do autor, corrigido pela R1:** o alvo com TESSEL_REQUIRE_RUNTIME=1
  falhou12/12 pelo guarda no ambiente que o autor preparou. Ele atribuiu isso a runtime e
  FFmpeg ausentes. A R1 encontrou FFmpeg9.0.2 no layout WinGet documentado, executou setup
  oficial e obteve10/12 com ferramentas reais. A ausência de FFmpeg nesta máquina **não se
  confirmou**; as duas falhas restantes são NotFound de shasum nos testes de áudio, BL167.3,
  anteriores aos loaders e com arquivo de teste idêntico à main. Não chamar10/12 de verde.
- Sem pixel, sem runtime, sem GPU, sem desempenho (nenhum lock de GPU tomado). Sem Windows/Mac independentes: a
  validação aqui é do próprio autor.
- Admissão pública segue fechada; a S2A e o preflight de intervalo seguem pendentes (R/S2A).
- Estado do ambiente: a captura "antes" (espaço/memória) não foi preservada no contexto desta sessão; o estado final é
  `E:` 287.1 GB usados / 193.0 GB livres, RAM 40.4 GB livres de 66.8 GB, origin
  `https://github.com/journey-studios/tessel-editor.git`, branch `feat/p4b-s2h-host-resources`. O `%TEMP%` do usuário
  é grande (e não é meu): somente os meus `s2h-*` foram criados e removidos.
- Commits locais passaram por saneamento do trailer injetado pelo harness (plumbing, árvore idêntica, nunca force): ver
  o histórico, sem trailer de assistente.

## 9. Pendências propostas pelo autor (histórico; estado atual em §12)

- Validar por revisão independente e em Windows/Mac; rodar `scene_export` com FFmpeg e runtime.
- `docs/features/scene-renderer.md` (linhas 93 e 285) ainda descreve `from_root`/`approved_map` por "cada recurso do plano":
  sugerir a nota "(a união dos dois decks, S2-H)". Não editei: fora dos docs desta fatia.
- BL082.4.4: H passa a "implementada, a validar"; R e S2A seguem pendentes.

## 10. Arquivos comuns reconciliados pelo coordenador

Os patches originais de7e7c83f foram substituídos pela integração nos próprios arquivos
comuns: BL082.4.4 e BL167.3 no backlog, F040–F042 e F064 na matriz, e índice dos docs.
As versões antigas ficam no histórico Git; não aplicar um patch antigo de23 testes/saturação
ou a alegação de FFmpeg ausente. Estado, atribuição e aceite continuam centralizados no backlog.

## 11. Integração do coordenador — código 267c659

Merge `b8442f6` incorpora a main c5400bb/PR72 sem conflito de conteúdo, preservando a recusa
de instância curta nos dois shots. Código `267c659ddecc9cb06a767439adb1c87e6367bf4d` corrige
um caso aberto pela saturação: com orçamento u64::MAX, o custo saturado podia ser aceito.
O coordenador escreveu negativos que compilaram e falharam por asserção (11 passam/4 falham)
na implementação inicial. Uma primeira tentativa dos testes não compilou por acessos de API;
foi corrigida e não conta como prova de recusa. A correção rejeita custo e soma total não
representáveis antes de mutar o livro e exige soma verificada para manter recursos na eviction.

O alvo de slots passa de 13 para 15 testes. Orçamento padrão/MAX, soma de dois custos exatos
2^63, livro já ocupado preservado na recusa e eviction com custos antigo/novo representáveis
são cobertos. O teste de custo usa metadados tipados e o de eviction injeta novas identidades;
não são prova de alocação enorme, novos hashes aprovados ou desempenho.

Mac do coordenador no conteúdo de 267c659: mapas 10/10 e slots 15/15. A primeira execução
Mac dos mapas deu 9/10 porque a limpeza usava remove_dir em um symlink Unix; a recusa do escape
já passava. O teste agora usa remove_file no Unix e remove_dir para junction Windows.
Gates Mac completos, runtime export e revisão independente Windows ainda pendentes.

Correção de atribuição: os 10 testes scene_fx_target do autor são de F061, não do consumidor
X. O consumidor correto é pilot_dissolve_representable_state (8 testes) e precisa de execução
própria. Os 215 do autor pertencem à base sem PR72; a integração deve executar 216.

O setup oficial do runtime no Mac passou, com FFmpeg existente e pnpm congelado. Isso ainda
não é prova de execução scene_export. O bloqueio Windows do autor é preservado como fato
por máquina; nenhuma falha ou guarda foi tratada como verde. Workspace global não executado,
sem pixel de transição, origem, GPU, desempenho ou UI física; Refuse/F064 P.

## 12. Gates Mac do coordenador em 19c5ea1

Head `19c5ea168699d307e541388d1b0e8781bf1bf6aa`, código do host `267c659`, core igual à main
c5400bb/PR72. Native Mac, clone dedicado em `/Volumes/Extreme Pro/www/tessel-pr63-mac-20261114`,
árvore limpa antes/depois. Os quatro negativos de custo compilados no merge b8442f6 (host ainda
igual ao original) deram 11/4; após267c659, mapas10/slots15. A tentativa que não compilou foi descartada.

| Gate | Resultado |
|---|---|
| fmt/check workspace/clippy workspace all-targets -D warnings | exit0 |
| Render: lib / scene_flow_resources / mapas H / slots H / scene_seam | 19 / 4 / 10 / 15 / 29, total77; 0 falhas/ignorados |
| Core: dissolve corpus / X import / X estado / transições / baseline | 10 / 5 / 8 / 216 / 1, isolado EXPECT0 e preserve_order EXPECT1 |
| scene_export, TESSEL_REQUIRE_RUNTIME=1, serial | 12/12, 0 ignorados, exit0; Node + Chrome + FFmpeg reais |
| test workspace --no-run e build -p tessel-editor | exit0; workspace apenas compilado |
| cargo tree core | 0 ocorrências gpui/wry |

O runtime foi preparado por `./scripts/runtime setup` com lock congelado; fontes JS e dependências
versionadas não mudaram. O alvo scene_export testa o transporte/export EXISTENTES, com MP4
1080p/UHD, oráculo de pixels antigo, hashes cena/export, cancelamento e sessão JSONL real.
Não desenha crossDissolve: a transição continua recusada publicamente e o runtime não anuncia
fxTransition. Este gate fecha a regressão funcional requerida no Mac; não transforma a ausência
de FFmpeg do relato Windows em execução Windows. Não houve desempenho, GPU real, UI física,
áudio ouvido, origem executada ou workspace global executado.

Logs em `~/.cache/tessel-pr73-mac-20261115/`: runtime-setup.log, overflow-red.log,
summary.json e um arquivo por gate. Check/clippy/no-run podem aproveitar cache mbx; os logs
mantêm essa distinção. Não houve target override, clean/gc manual ou limpeza de terceiros.
Core, Cargo.lock, fixtures, runtime, media e desktop são idênticos à main c5400bb.

Revisão independente Windows iniciada em19c5ea1, ainda sem veredito. Os 30 mutantes do autor
continuam atribuídos a7e7c83f e não são um resultado novo destes gates. H interna permanece
a validar por revisão; R/S2A não iniciadas, F064 P e Refuse fechado.

## 13. Diagnóstico estrutural por deck — código05f37da

O OpenCodeReview apontou corretamente que um ponteiro exato `/transition/other/resources/N`
também pode vir da validação estrutural (por exemplo widthPx=0), e não só de conflito da união.
O helper tratava os dois casos como conflito e culpava também o primário quando só a entrada
do vizinho era inválida. Uma regressão compilada reproduziu a atribuição errada: os dois usos
apareciam no lugar do único uso esperado. A primeira ordenação do teste também demonstrou
que a estrutura inválida do primário ficava sem contexto; essa lacuna foi coberta no mesmo teste.

Código `05f37da1937ba58bed2d0e0f31198642cf5f977d`: ponteiros de tabela/entrada/campo identificam
o deck da falha. Os usos dos dois decks só são unidos se o erro recebido for igual à recusa
produzida por `ScenePlan::resource_union()`; não há comparação com mensagem literal nem
nova regra de metadados no host. Os detalhes existentes, código e ponteiro na mensagem ficam
intactos. A regressão cobre tamanho inválido e relativePath inválido, primary/other, antes/depois
do corte e livro retido sem mutação. Os testes existentes continuam exigindo ambos os decks
no conflito real de metadados.

Mac local no conteúdo05f37da: lib19, mapas10, slots16, scene_seam29 e scene_flow_resources4,
total78, todos passam. Os 77 e o export12 do §12 pertencem a19c5ea1; gates nativos completos
para a nova fonte e a revisão Windows ainda precisam fechar. O total H agora é26 testes
(10 mapas +16 slots). Core/fixtures/deps/runtime/media/desktop continuam intactos.

## 14. Fechamento Mac em8a489bf (código05f37da)

Após a correção de atribuição, todos os gates foram repetidos no head
`8a489bfd03309ca193858212b8b329b52018dd81`, com árvore limpa antes/depois e core/fixtures/deps/
runtime/media/desktop idênticos à main c5400bb. A fonte do host é05f37da.

- fmt/check workspace/clippy workspace all-targets -D warnings: exit0.
- Render: lib19, scene_flow_resources4, mapas10, slots16, scene_seam29 =78, 0 falhas/ignorados.
- Core focado: corpus10/Ximport5/Xestado8/transições216/baseline1, isolado EXPECT0 e preserve_order EXPECT1.
- scene_export real, TESSEL_REQUIRE_RUNTIME=1, serial:12/12, 0 ignorados, exit0.
- Workspace --no-run e build desktop: exit0. Tree core: sem gpui/wry.

Logs `final-*.log` e `final-summary.json` no diretório do §12. O export foi repetido por mudança
no helper de erros que seus negativos atravessam. Os 12 cobrem o caminho antigo sem transição,
incluindo processos reais e MP4; não provam parser/blend/runtime de crossDissolve. Workspace
global e core completo não foram executados nesta rodada; nada de GPU real, desempenho,
UI física, áudio ouvido ou origem. A revisão Windows começou na fonte19c5ea1 e não é herdada
pela05f37da sem rechecagem. H interna continua em validação, com produto F064 P e Refuse fechado.

## 15. Revisão independente Windows R1 em19c5ea1

Aprovada com condições apenas para H interna. Gates fmt/check/clippy/no-run/build passaram;
render73 (lib16/seam28/flow4/mapas10/slots15) e core216/10/5/8/1 em isolado/preserve passaram.
Core, media, desktop, Cargo, runtime, scripts e fixtures iguais à main c5400bb. Workspace global
não executado. R1 declarou não ter lido AGENTS/skill; a rechecagem deve cumprir essas leituras.

A R1 encontrou FFmpeg9.0.2 no layout WinGet e preparou o runtime oficial: scene_export real
10/12, com as duas falhas por shasum de BL167.3, antes dos loaders. Os12/12 Mac em8a489bf
continuam separados por plataforma e satisfazem a regressão com ferramentas reais no Mac.
Não instalar shim nem excluir essas falhas para declarar Windows verde.

R1 relatou20/23 mutantes mortos, com O3/O4 (custo individual) e A3 (validate do prepare)
sobreviventes. A lista de IDs narrada aparenta outra contagem: conferir logs na R2, sem
normalizar números por suposição. Não herdou os30 do autor. O achado de contexto de campos
corresponde ao reforço05f37da, que a R1 não revisou; não promover seu aceite para essa fonte.

## 16. Fechamento dos controles de custo e guarda de validação

Commit de testes `fe14cfc14282415de2a7312044c8012612950f8e`, produção idêntica a05f37da.
Novo módulo individual_cost isola o custo do backdrop: a figura de altura1e-10 custa0 no
modelo existente. Sem mudar produção, cobre custo infinito com dimensões finitas, custo
exato2^64 recusado e o maior f64 abaixo dele (2^64−2048) aceito. São sondas de contabilidade
com metadados tipados, não imagens enormes aprovadas ou alocadas pelo media.

Mac do coordenador: O3 (Some sem guardas) falhou0/2; O4 (<= no limite) falhou1/1;
A3 (sem validate no prepare) falhou0/1 pela regressão estrutural de05f37da. Todos compilaram,
fontes voltaram por bytes/SHA e mtime atualizado, e cada controle limpo voltou a passar.
A nova regressão de contexto também fecha a lacuna A3; não foi necessário mudar produção.
Logs r1-close-*.log e r1-close-mutants.json no diretório dos gates Mac.

Emfe14cfc: render80 (lib19/flow4/mapas10/slots18/seam29) e clippy render all-targets -Dwarnings
passaram; fmt/diff-check0. H agora tem28 testes (10 mapas+18 slots). Os gates Mac completos e
scene_export12 de8a489bf pertencem à mesma produção05f37da; não foram repetidos após uma
mudança só de teste/docs. Rechecagem Windows do contexto05f37da, O3/O4/A3 e dos docs ainda
pendente. Produto P e Refuse fechado; runtime de transição/R/S2A não habilitados.
