# P4a — Efeitos no escopo de plano (`shot`)

Data: 2026-11-11. Projeto: `journey-studios/tessel-editor`. Base conferida: `origin/main` em `9fc491b` (PR #45 luz pontual).
PRs abertas que tocam arquivos vizinhos: [#46](https://github.com/journey-studios/tessel-editor/pull/46) (handles 2D da vinheta, F061, BL082.35–.39),
[#44](https://github.com/journey-studios/tessel-editor/pull/44) (suíte de regressão de desempenho do gizmo) e
[#41](https://github.com/journey-studios/tessel-editor/pull/41) (diagnóstico de I/O do viewport). Este documento é o brief da implementação
(Cursor Sonnet 5.5). Complementa o [plano de paridade do piloto](pilot-parity-execution-plan.md) (fase **P4**, aqui só a primeira metade),
o [inventário do piloto](pilot-fx-shots-studio-features.md) (§1.1, §1.4 e §6), o contrato F016 (`docs/features/effects.md`), o inspector
F057 (`fx-inspector.md`) e as chaves F058 (`fx-keys.md`). Não cria outra fila: status e atribuição ficam no [backlog único](../backlog.md).

## Por que esta fatia

P0–P3 entregaram a vinheta, o inspector neutro com `fade`, as chaves e o `filmGrain`, **todos no escopo `output`**. Hoje o importador recusa
`scope: { kind: "shot" }` com `FX_SCOPE_NOT_YET` e qualquer `transitions[]` com `FX_TRANSITION_NOT_YET` (BL082.3 e BL082.4).
As 28 transições do catálogo do piloto (a maior parte do que falta) precisam de **dois decks** e de efeitos presos a um plano. O escopo `shot` é
o pré-requisito pequeno e revisável: dá ao efeito um alvo no plano (`ScenePlan`) e uma etapa de composição própria, e deixa a costura pronta
para a **P4b** (`crossDissolve`, dois decks). Esta entrega **não** inclui transições.

## O que o escopo `shot` significa (regras do piloto, lidas somente leitura)

Origem: repositório `tesselapp`, branch `t3/scene-engine-pilot`, commit fixado `d40ac3f7c0`, worktree
`/Users/wallyssonnunes/.t3/worktrees/tesselapp/t3-5de3852f` (**ler com `git show`/leitura, nunca editar, instalar ou executar o JS**; o que for lido
fica registrado como `ORIGIN_NOT_EXECUTED`, sem inventar golden). Arquivos: `packages/scene-engine/src/fx/fx-explain-clips.ts` (validação),
`fx-frame-decks.ts` e `fx-frame-state.ts` (resolução por deck), `fx-scope-text.ts` (grafia textual), `fx-command-clips.ts` (`addClip`, `setScope`),
`fx-commands-params.spec.ts` (~173–200) e `fx-commands-clips.spec.ts` (~280–310) (casos de equivalência).

1. **Forma no documento:** `scope: { "kind": "shot", "shotId": "<id>" }`. O escopo de saída é a **grafia canônica omitida**: o piloto não escreve `scope` para
   `output` e `setScope` de volta a `output` **remove a chave**. No Rust, as edições (`scene_edit/fx.rs`) operam sobre o **JSON bruto** do documento
   (`scene.document().clone()` + revalidação pelo importador), nunca reserializam um modelo: `add_clip` hoje não escreve `scope` e o novo `set_scope` escreve/remove a
   chave. Um `scope: {kind:"output"}` escrito à mão continua aceito pelo importador e preservado enquanto nenhuma op tocar o clipe. `set_scope` para o escopo que o
   clipe já tem é **no-op** (sem revisão nem undo, como os demais ops idempotentes).
2. **Validação (códigos do piloto, reaproveitados como estão):** `UNKNOWN_SHOT` quando o `shotId` não existe; `FX_SCOPE_MISMATCH` quando o clipe
   (`startFrame .. startFrame + durationFrames`) **não cabe inteiro** em `[shot.startFrame, shot.endFrameExclusive)`. Conferido em `9fc491b`: nenhum dos dois nomes
   existe hoje no core (se a `main` viva trouxer colisão, escolher outro nome e registrar). `FX_SCOPE_NOT_YET` continua para `node`. O `read_fx` já roda **depois** de
   `shots` em `directed_scene/parse.rs`: decidir e documentar onde o cruzamento mora (leitura do `fx` recebendo os shots já lidos, ou o passe de integridade
   `directed_scene/integrity.rs`), mantendo a **ordem dos issues estável** e o corpus de issues existente intacto.
3. **Sobreposição** continua por trilha, **independente do escopo** (uma trilha toca um efeito por vez).
4. **Composição por etapas (a regra que importa):** `decks → passes do deck (looks com escopo de plano) → [transição] → passes de saída (looks output)`.
   Dentro de cada etapa vale a ordem das trilhas. Logo, um clipe `shot` numa trilha **posterior** ainda é desenhado **antes** de um `output` numa trilha
   anterior. O importador já impõe que os shots cobrem a edição **sem buracos nem sobreposição** (`EDIT_RANGE_START`, `EDIT_GAP_OR_OVERLAP`, fim = duração da edição;
   fixture `invalid-edit-gap`), então cada quadro da edição tem **um único** shot e, sem transição, **um único deck**.
5. **Orçamento de passes** (`FX_PASS_BUDGET`, 8, só avisa): conta os passes de deck e de saída juntos no quadro.
6. **Fora desta fatia, mas registrado para a P4b:** durante uma janela de transição o deck desenha quadros fora do plano e um clipe `shot` que encosta
   na borda do plano "segura o quadro mais próximo" (`shotClipFrame`). Sem transição isso não existe; **não implementar**, só deixar a costura (ver adiante).
7. **Efeito `transform`** (escopo plano move os pixels do próprio deck): o catálogo Rust não tem `transform`; não existe nesta fatia.

## Decisões fechadas

| Tema | Decisão |
|---|---|
| Modelo (core) | `FxClip` ganha o escopo (`Output` ou `Shot(id)`); o tipo é extensível para `Node` (recusado como hoje, `FX_SCOPE_NOT_YET`). Sem segundo modelo: a camada `fx` do bundle continua sendo o único dono |
| Resolução | `Fx::resolve` fica pura e determinística, mas passa a conhecer os intervalos dos shots (parâmetro explícito, sem estado escondido). Saída: passes de deck na ordem das trilhas, depois os de saída. Seek = reprodução |
| Plano | cada passe leva `scope` (`{kind:"output"}` como hoje, ou `{kind:"shot", shotId}`); o validador do plano recusa passe de shot cujo `shotId` não é o do quadro, ordem fora de etapas (um `output` antes de um `shot`) e `amount` adulterado. Plano de uma cena sem clipe `shot` tem **os mesmos bytes e digests** de antes |
| Capacidade | nova `Capability::FxShotScope` (`fxShotScope`, a 13ª), exigida só quando algum passe tem escopo `shot`; um runtime sem ela recusa o plano pelo nome, nunca desenha sem o efeito. `Capability::ALL`, os testes que contam capacidades (core, render, runtime JS) e `SCENE_CAPABILITIES` do runtime sobem juntos |
| Runtime | desenha os passes na ordem do plano; separa o laço de passes de **deck** do de **saída** em funções próprias (a costura da P4b) e confere o escopo de cada passe (`shotId` ≠ do quadro ⇒ `unsupported` tipado). Sem pipeline novo nem alvos novos; o ping-pong de dois alvos continua |
| Edição | `scene.fx.edit` ganha `add_clip` com `scope` e o op novo **`set_scope`** (lote atômico, um undo; falha do candidato recusa o lote inteiro com os issues). Voltar a `output` remove a chave; ida e volta `output → shot → output` de um clipe sem `scope` devolve os mesmos bytes do documento; mesmo escopo = no-op |
| Leitura | `scene.fx.list`, `scene.fx.frame`, `scene.fx.catalog` e `scene.fx.inspect` trazem o escopo em texto (`output`, `shot:<id>`; o id é tudo depois do primeiro `:`: **testar ids de shot com `:`**, que o importador de shots pode aceitar) e a lista de shots do documento (`id`, `start_frame`, `end_frame_exclusive`, `intent`) com o shot do quadro exibido. `scopes()` passa a dizer `shot` desenhado e `node` recusado |
| Inspector | o escopo vira o **primeiro campo de escolha** do modelo neutro (F057 só tem escalares): `choice` com as opções do documento (`output` + um item por shot, rótulo pt-br/en/es + `intent`), `set_scope` como op de escrita, sem reset de tempo. Desenhado de forma genérica: a P4b vai reusá-lo (`alignment`, `easing`) |
| Janela | no clipe, seletor de escopo (Select do Kit) e a linha do escopo no título; ação para adicionar **no plano exibido** (desabilitada com `disabled_reason` se o quadro não cai em um shot); recusas aparecem como o aviso do painel. Nós: `scene.fx.clip:<id>` ganha `scope` (texto) e ação `set_scope:<destino>` ou equivalente; o nome exato é decidido e documentado no F062 |
| Recusa | importador: `UNKNOWN_SHOT`, `FX_SCOPE_MISMATCH` com `details` (shot e limites). Edição: os mesmos como códigos de comando tipados no estilo do `command_code` existente (`fx_scope_mismatch`, `unknown_shot`). Nada é "apertado" em silêncio: o `details` diz o intervalo legal para o adaptador sugerir |
| Quando o plano muda | importar/substituir um bundle com clipe `shot` fora do plano é recusado pelo importador como qualquer outro erro de `fx`; um plano que perde o shot (ainda não há ripple/trim, BL082.5/P5) não é caso desta entrega, mas o **teste do importador** cobre |

## Escopo funcional

1. **Core:** modelo, leitura/escrita canônica, validação, resolução em etapas, plano (+ capacidade + validação + JSON), orçamento de passes por etapa,
   ops `add_clip`/`set_scope`, leituras, inspector `choice`, mensagens pt-br/en/es.
2. **Runtime (`runtime/`):** aceitar o escopo `shot` com a capacidade, laço de deck separado do de saída, conferência do escopo; testes puros (`node --test`) e
   de navegador real; pixels do monitor e do export.
3. **Superfícies (regra do repositório):** biblioteca, CLI headless (JSONL, `./scripts/headless`) e janela (`./scripts/ui`, só comandos semânticos), com
   `Command` + nó em `state/tree.rs` + parser em `automation/commands.rs` + teste; handlers compartilhados, nunca estado paralelo na CLI. Nenhum diálogo novo
   (se surgir, marcar `Workflow::has_dialog`).
4. **Interação com as PRs vizinhas:** com a #46 já na `main`, o clipe `shot` de vinheta precisa funcionar com `scene.fx.select`/`scene.fx.handles` (o handle é 2D
   no quadro; o intervalo de exibição é o do clipe) — teste e registro; se a #46 ainda não estiver integrada, registrar a interação como pendência nova e **não**
   editar os arquivos dela.

## Fora do escopo

Transições (`transitions[]`, `crossDissolve`, dois decks, janelas e handles; **P4b**), escopo `node`, `transform`, ripple/roll/trim de shots (BL082.5, P5), clipe que
atravessa shots (use `output`), `FX_SCOPE_SHARED_DEFINITION` e demais avisos do piloto, presets, os outros 56 efeitos do catálogo, timeline de FX (P6), ponteiro físico,
desempenho integrado (medição separada), Windows. Não modificar a origem nem copiar assets/dados privados.

## IDs e documentação

- **Feature nova F062** (F061 está reservada pela PR #46; conferir `docs/features/README.md` na `main` e nas PRs abertas): documento próprio
  `docs/features/fx-shot-scope.md` e linha em `docs/features/README.md`. Atualizar F016 (`effects.md`: "Só escopo `output`", plano, runtime, limites), F057
  (`scopes()` e o campo de escolha) e o catálogo de capacidades onde estiver documentado (`docs/features/viewport.md`/F055 se citarem a contagem).
- **Backlog:** pai **BL082.3** (atualizar a linha existente) e filhas novas a partir de **BL082.40** (BL082.35–.39 são da PR #46; conferir a `main` e as PRs antes de
  reservar). Pendências a registrar: P4b (BL082.4), costura do deck (handle frames), `node`, interação com handles se a #46 não estiver integrada, ponteiro físico, desempenho.
  **O `docs/backlog.md` da raiz do coordenador é dele:** no PR edite só as suas linhas e deixe o resumo das linhas na descrição para integração. Nada vira "validado"
  sem prova: sem prova fica "A validar" com o que falta.
- `docs/migration/pilot-parity-catalog.json` e `feature-migration-matrix.md`: atualizar a linha de escopos/efeitos se existir.

## Validação (cada item vira evidência registrada no F062)

1. **Biblioteca (core).** Casos extraídos do piloto fixado, com **valores escritos à mão no teste e a proveniência** (arquivo e SHA do blob, `ORIGIN_NOT_EXECUTED`):
   `setScope` para um shot e de volta (chave removida), `FX_SCOPE_MISMATCH` quando o clipe começa fora do shot, `UNKNOWN_SHOT`, `node` ainda recusado. Mais: clipe
   exatamente do tamanho do shot (válido) e um quadro além (inválido); plano de um quadro no shot A, na fronteira A/B e no shot B (só o clipe do shot do quadro desenha);
   ordem em etapas com `shot` em trilha **posterior** a um `output` e o inverso; `amount` adulterado e `shotId` trocado recusados pelo validador do plano; orçamento de
   passes somando as duas etapas; JSON do plano ida e volta; plano sem `shot` com os **mesmos digests** de hoje (`crates/core/tests/scene_plan_performance_golden.rs` e os corpora); undo/redo
   e uma entrada por `set_scope`; lote com um item inválido recusa tudo; seek ≡ reprodução; shot id com `:`; cena sem shot no quadro exibido.
2. **Headless real:** processo JSONL sem desktop, mesmo documento final que a janela e a biblioteca (`add_clip` com `scope`, `set_scope`, `scene.fx.frame`, `scene.plan`).
3. **UI/runtime:** testes GPUI (seletor, ação "no plano", recusa com aviso, troca de cena), testes JS puros do runtime (aceita `shot` com a capacidade, recusa sem ela, recusa
   `shotId` errado, laço de deck antes do de saída) e janela própria por `./scripts/ui` só com comandos semânticos. **Não declarar prova física** da WKWebView.
4. **Imagem (referência independente, escrita no teste, não o código sob teste):** com o runtime real num corpus de dois shots (`valid-minimal-continuous`: shot-1 `[0,48)`,
   shot-2 `[48,96)`; `valid-cut-instance-offsets`: shot-a `[0,40)`, shot-b `[40,100)`), (a) um clipe `shot` só escurece os quadros do seu shot e os hashes fora do shot são os
   do baseline; (b) sem outros passes, `shot` ≡ `output` nos mesmos quadros (hash igual); (c) **não comutatividade**: `fade` (cor clara) `shot` + vinheta `output` difere de
   `output`+`output` na ordem inversa e o pixel confere com a composição em etapas calculada pela referência já validada (`directed_scene::fx::reference`) **alimentada com a ordem das etapas escrita à mão no teste**, nunca com a ordem que o `Fx::resolve` devolve; (d) o hash de `scene.export`
   por quadro é igual ao de `scene.render` (monitor ≡ export), incluindo um quadro de cada lado da fronteira.
5. **Mutações discriminantes** (uma por vez, arquivo restaurado e **hash conferido**; modelo `docs/migration/g12-scale/mutate.py`: raiz pelo local do script, relatório com a pasta
   criada, `KILLED` só com teste nomeado que falhou, saída sem teste é erro de execução). Sugestões: limite do shot com off-by-one (`<=`/`>`), etapas ignoradas (ordem das trilhas
   pura), `shotId` não conferido no plano/runtime, capacidade não exigida, `scope` escrito para `output`, `set_scope` sem remover a chave, fronteira A/B pegando o shot errado, orçamento
   contando só uma etapa, runtime sem a conferência de escopo. Lições da G4.1: o runner **reconstrói o `runtime/dist`** antes de mutar JS (`./scripts/runtime build`); restaurar em
   `finally`/sinal e conferir `git status` limpo nos arquivos mutados no fim (uma execução morta deixou um arquivo mutado).
6. **Gates no head final** (preservar runs falhos/abortados): `cargo fmt --all -- --check`; `./scripts/cargo check --workspace --locked`;
   `./scripts/cargo clippy --workspace --all-targets --locked -- -D warnings`; **`./scripts/runtime build` antes de**
   `TESSEL_REQUIRE_RUNTIME=1 ./scripts/cargo test --workspace --locked --no-fail-fast` (o `runtime/dist` é ignorado pelo Git e envelhece entre ramos; já causou falhas falsas de
   `fxNoise`); `./scripts/cargo build -p tessel-editor --locked`; `cargo tree -p tessel-editor-core` sem gpui/wry; testes JS do runtime. A contagem de capacidades muda de 12 para
   13: **atualizar todos os testes que a fixam** (core, render e runtime JS: procurar pela lista e pela contagem, não só pelo nome).
7. **Desempenho (separado, sob o lock de GPU/build, só para medição):** custo de `Fx::resolve` e do plano com e sem clipes `shot` e quadros por etapa; lock ocupado = medição pendente,
   sem bloquear o funcional.
8. **Revisão independente:** tarefa nova do coordenador depois da PR; CodeRabbit `NEUTRAL`/`SKIPPED` não é aprovação.

## Ordem sugerida de implementação

1. **Contrato no F062 e testes de caracterização** (os casos do piloto e a ordem em etapas) antes do código; confirmar que o corpus `output` não muda de digest.
2. Modelo + importador + escrita canônica + `UNKNOWN_SHOT`/`FX_SCOPE_MISMATCH`.
3. Resolução em etapas, plano (`scope`, `Capability::FxShotScope`, validação, JSON) e orçamento.
4. Ops `add_clip`/`set_scope`, leituras e inspector `choice`.
5. Runtime (capacidade, laço de deck, conferência) e provas de imagem/export.
6. Janela (seletor, ação, nós, parsers, recusas) e provas headless/janela.
7. Mutações, gates, docs e backlog.

## Restrições de ambiente

- Cargo **só pelo wrapper** `./scripts/cargo` (mbx, target padrão); nunca `CARGO_TARGET_DIR`, `--target-dir`, `build.target-dir`; nunca `mbx clean`/`gc`. Disco: reserva de 16 GiB para iniciar
  lote pesado e piso de 12 GiB (`REFUSED_DISK` e entrega do diff se faltar; sem limpeza ampla de caches); sem artefatos em `/tmp` acima de ~500 MB; evidências grandes fora do Git.
- O lock de GPU/build só é exigido para **medições de performance**; implementação, builds e testes funcionais seguem sem a trava.
- Só `gpui-kit` na UI; sem serviços, credenciais, telemetria ou rede; Rust 1.99.1; preservar `Cargo.lock`.
- O harness pode injetar `Co-authored-by` nos commits: **conferir e remover antes do push** (só nos commits da entrega, sem reescrever os alheios nem usar force sobre trabalho de outros).
  Mensagens em inglês, `feat(fx):`/`test(fx):`/`docs(fx):`, sem mencionar assistentes (skill `commit`).
- Conferir heads e PRs abertos antes de editar arquivos compartilhados (#46, #44, #41); mudanças pequenas e aditivas nesses arquivos.

## Entrega

Branch `t3/fx-shot-scope` criada a partir de `docs/p4a-shot-scope-plan` (que contém este plano) ou, se este já estiver na `main`, de `origin/main`; antes de abrir a PR,
`git fetch` e integrar a `main` viva (a #46 pode ter entrado). Commits pequenos; push e **PR contra `main`**, sem mergear nem monitorar; `link_pull_request` logo após criar.
A descrição traz: o que entrou, decisões de contrato (grafia, códigos, etapas, capacidade, inspector), o que **não** foi provado (ponteiro físico, WKWebView, desempenho, Windows,
transições), gates com SHA do head e as linhas de backlog a integrar. Em ambiguidade de contrato, pare e pergunte em vez de inventar paridade.
