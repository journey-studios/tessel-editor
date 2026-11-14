# P4b S1b — relatório: avaliação de dois decks e `ScenePlan.transition` (BL082.4.3)

Plano de execução: [p4b-s1b-evaluation-plan.md](p4b-s1b-evaluation-plan.md). Contrato canônico:
[p4b-cross-dissolve-plan.md](p4b-cross-dissolve-plan.md). Feature: [F064](../features/fx-transitions.md) e
[F037](../features/scene-plan.md). Base `origin/main e3c20cb7`, ramo `feat/p4b-s1b-evaluation`.

**Status: S1b concluída parcial interna, a validar review.** Produto F064 segue **P**. S2 e S3a não começaram;
BL082.4.15 segue Pendente para a S3. A admissão pública **não** foi aberta.

## 1. O que foi implementado (core, sem UI)

| Marco | Entrega | Arquivos |
|---|---|---|
| M0 | Baseline de planos sem transição, medido na base sem alteração, por perfil (debug/release) + negativo público | `tests/scene_plan_transition_baseline.rs`, `tests/scene_plan_transition_baseline/no-transition-plans.json` |
| M1 | `evaluate_shot_at_offset`: shot nomeado num quadro fora do seu intervalo; instância e câmera presas **separadamente**, `held` só quando o grampo mudou o quadro; luz/cena no quadro real | `directed_scene/temporal/shot_offset.rs` (+ `frame.rs`, `mod.rs`) |
| M2 | `Fx::resolve_frame` (topo, vizinho, mistura, saída uma vez), `progress` suavizado, `amount = clamp01(intensity)`, orçamento só-aviso | `directed_scene/fx/{frame,budget}.rs` (+ `fx/mod.rs`) |
| M3 | `ScenePlan.transition` (chave aditiva), leitor defensivo, tetos (4 194 304 bytes no texto, 20 000 itens, profundidade 1) | `scene_plan/transition{,_json,_validate}.rs` + ganchos em `model/json/build/validate/timeline/mod` |
| M4 | Recursos por deck, `resource_union()` derivada (`used_by` qualificado por deck), `requires` com `fxTransition` (14ª capacidade, **fora** da lista do JS) | `scene_plan/transition.rs`, `validate.rs` |
| M5 | Corpus `dissolve-cases.json` lido em Rust e mutantes essenciais | `tests/pilot_dissolve_corpus.rs`, `tests/scene_fx_transition/*` |

Compatibilidade: um plano sem transição mantém bytes e digests (`PLAN_VERSION` 1, `FX_CATALOG_VERSION` 1, `Domain::ALL`
13). `Fx.transitions` continua vazio na entrada pública (`TransitionAdmission::Refuse`); `Accept` só existe no caminho
`import_*_admitting` interno e o scanner de fronteira proíbe seu uso em produção. Planos reais do corpus: ~6 KB
(spec edit, frames 42–53), muito abaixo dos tetos.

## 2. Provado (Windows x64, Rust 1.99.1, head de código desta entrega)

| Prova | Resultado |
|---|---|
| `cargo fmt --all -- --check` | ok |
| `./scripts/cargo check --workspace --locked` | ok |
| `./scripts/cargo clippy --workspace --all-targets --locked -- -D warnings` | ok |
| `./scripts/cargo test --workspace --locked --no-run` | ok (compila todos os alvos) |
| `./scripts/cargo build -p tessel-editor --locked` | ok |
| `cargo tree -p tessel-editor-core` | sem `gpui`/`wry` |
| `scene_fx_transition` | 106 testes ok, **com e sem** `serde_json/preserve_order` (e com `TESSEL_EXPECT_PRESERVE_ORDER` 1/0) |
| `pilot_dissolve_corpus` | 10 testes ok (116 quadros analíticos, tabelas de progresso, pixels de referência, união de recursos) |
| `scene_plan_transition_baseline` | ok; golden de plano e de performance inalterados |
| core lib | 594 ok |
| core completo (`--no-fail-fast`) | tudo ok **exceto 2 falhas de base**: `scene_gizmo_rotation`, `scene_gizmo_scale` (reproduzidas na base sem alteração com `git stash -u`; não são desta entrega e não foram tocadas) |
| desktop `scene_fx` (GPUI, negativo proporcional) | 80 ok |
| `tessel-headless` real | continua **recusando** `transitions` com exit 1 (teste `public::a_real_headless_process_refuses_transitions…`) |

O workspace inteiro **não** é declarado verde: há pendências externas (W1/claims) fora do escopo; só `--no-run` foi
exigido e passou. Nenhuma medição de desempenho foi feita (o lock de GPU não foi necessário nem tomado).

### Mutantes essenciais (M5; aplicar, rodar `lib` + `scene_fx_transition` + `pilot_dissolve_corpus`, reverter)

Os 18 foram **mortos**; o controle (código sem mutação) passa, e a árvore volta limpa após cada um.

| Mutante | Resultado (1º teste a falhar) |
|---|---|
| meio quadro (`+0,5`) removido | morto, 29 falhas (`the_analytic_scenes_agree_with_the_oracle_frame_by_frame`) |
| easing ignorado | morto, 29 |
| primário trocado | morto, 31 |
| grampo conjunto de instância e câmera | morto, 6 (`deck_frame_rows_agree_with_the_oracle`) |
| relógio da luz preso ao quadro retido | morto, 28 |
| `held` sempre verdadeiro | morto, 12 (`shot_offset::tests::holding_is_to_the_first_and_last_frame_…`) |
| quadro de borda do clipe vizinho sem grampo | morto, 5 |
| saída duplicada no outro deck | morto, 4 |
| semente do vizinho com o quadro retido | morto, 1 (`the_noise_seed_of_the_neighbour_follows_the_real_frame_…`) |
| recursos do outro deck ignorados | morto, 4 |
| ids de nó sem qualificação por deck | morto, 3 |
| conflito de `assetId` na união aceito | morto, 2 |
| `requires` sem a capacidade do outro deck | morto, 1 |
| `requires` sem `fxTransition` | morto, 4 |
| profundidade 2 aceita pelo validador | **sobreviveu** na 1ª rodada (o teste só passava pelo leitor); asserção direta em `plan.validate()` adicionada; agora morto |
| profundidade 2 aceita pelo leitor | morto, 2 |
| teto de itens fora por um | morto, 1 |
| teto de bytes fora por um | morto, 1 |

Achado extra da revisão própria: sob `preserve_order` (perfil do workspace), a bomba de 1000 níveis construída **pelo
próprio teste** estourava a pilha na thread do teste (não no código sob teste); o teste passou a rodar em thread com
pilha de 64 MiB.

## 3. Divergências e limites registrados

1. **Cenas `lit-*-held` (3)**: o oráculo dá ao shot A uma instância de 12 quadros sob um shot de 30; a origem tolera
   (prende), o importador do Tessel recusa (`SHOT_EXCEEDS_INSTANCE_DURATION`, contrato da S1a). O teste recusa isso e
   reconstrói a cena com instância que cobre o shot, comparando só o que não depende do comprimento (câmera/heldCamera de A, luz no quadro real,
   `progress`, `amount`, janela, relógios do vizinho não alterado).
2. **Linha `covers-and-reaches-before-the-shot` do clipe de plano**: o importador recusa um clipe `shot` que sai do
   próprio shot (F062); o caso da origem não tem documento de destino e é **pulado** (asserido por nome).
3. **Câmera curta (BL082.4.14)**: a variante Rust usa o quadro 70 (instância 20+50 de 200, câmera presa em 47 de 48).
4. **Teto de itens**: 20 000 nós excedem 4 MiB; o limite exato de itens é provado pelo **pré-check** pelos comprimentos
   dos arrays, não por um documento de 20 000 itens que passe do teto de bytes.
5. **Alpha (D20)** e a ordem lexical dos `params` (diverge da ordem de autoria da origem) seguem como em F064.
6. **Pixels**: a linha de pixels do corpus é um cálculo de referência (decodificação sRGB, mistura, codificação, meio
   para cima). Nada aqui é saída de GPU/runtime. `ORIGIN_NOT_EXECUTED` e `GOLDEN_NOT_COMPARABLE` permanecem; toda célula
   não executada fica **A validar**. Aceite humano pendente.
7. A matriz de células por fase da F064 **não** foi alterada neste PR (é do coordenador).

### Extensões de ownership (puramente mecânicas)

- `directed_scene/fx/transition_explain.rs`: `type Reach = ShotReach` (o tipo passou a viver em `shot_offset.rs`).
- `scene_project/fx.rs`: chave aditiva `transition` na resposta interna `scene.fx.frame` e `pass_budget_in` na listagem.
- `directed_scene/mod.rs`: reexportações.
- `tests/scene_fx_shot_scope/plan.rs` e `tests/scene_plan.rs`: a contagem de capacidades passou de 13 para 14
  (`ALL[13] == FxTransition`) e o filtro de exclusão ganhou `FxTransition`.

Coesão: nenhum arquivo acima de 1000 linhas cresceu; `camera.rs` (1014) não foi tocado; `scene_plan/json.rs` foi de 828
a 923 linhas (<1000), com o texto das transições em módulos próprios.

## 4. Patch literal para o coordenador (BL082.4.3 / F064)

Não editei `docs/backlog.md`, `docs/features/README.md` nem `docs/README.md`.

- **BL082.4.3**: status **Concluída parcial interna, a validar review**. Evidência: este relatório e o PR da S1b
  (commits `feat(core): evaluate two decks…`, `test(core): prove…`). Aceite pendente: review independente; S2 (runtime,
  shader, pixels reais) e aceite humano. Dependências: S2 depende desta entrega. BL082.4.15 segue **Pendente** (S3).
- **F064** (linha em `docs/features/README.md`): produto **P**; acrescentar «S1b: avaliação de dois decks e
  `ScenePlan.transition` internas no core, admissão pública fechada; a validar review». S2 e S3a: **não iniciadas**.

Nits para o coordenador: (a) `fxTransition` está no Rust (14) e ausente do `SCENE_CAPABILITIES` do JS até a S2 (há teste
de compatibilidade que lê o JS como texto); (b) as 2 falhas de gizmo da base merecem BL próprio, se ainda não houver.

## 5. Coordenador: correção da baseline Mac (BL082.4.16)

A execução nativa Mac em73775c3 passou106transições e10corpus. A baseline
falhou com "no baseline pinned for the debug profile": o JSON só continha
windows-debug/windows-release, enquanto o teste seleciona debug/release
fora do Windows. Nenhuma divergência de bytes Mac foi comparada nessa falha.

O coordenador capturou os dois perfis MacARM num clone separado do commit
e3c20cb7c8d3e32692719a9c71adb1b65eb3c392. O probe temporário só emtests
reutilizou a função de registro; core/src eCargo.lock estavam inalterados
antes/depois, e o probe foi removido (status final limpo). A captura Mac
é posterior no tempo àimplementação, mas o código executado é o anterior.
Não há expectativa obtida pelo planner S1b. Os perfis Windows e os metadados
existentes foram preservados por comparação canônica ehash.

Com a fixture enriquecida, no candidatoMac passaram106transições/10corpus
e baseline debug; baseline release também passou. O comentário do teste
foi corrigido para distinguir as capturasWindowsM0 eMac posterior sobre
a mesmabase. Produção não mudou; o head limpo será validado antes do push.
Linux não foi executado nem esses perfis constituem paridade multiplataforma
com o motor de origem. O ajuste não fecha BL082.4.17: três kitslit-held e
o clipe fora do shot continuam com limites de evidência declarados acima.

## 6. Revisão independente e reforço do coordenador

A revisão Windows de73775c3 executou106transições/10corpus/594lib e80GPUI, estáticos,
no-run/build; core1584passaram e2gizmos da base falharam. Achados: baselineMac ausente,
aceitação indevida de dois None por overflow, três guardas sem teste e contexto do vizinho.
Nenhum resultado de runtime ou pixels reais. A aprovação é condicionada e só interna.

No head limpo dd92281, o coordenador executou no Mac106transições/10corpus/baseline1 em
debug e baseline1 em release; check/clippy/build passaram. A correção posterior de produção
não herda esses gates. Ela adiciona regressões do overflow, dos três guardas e de versão/splat
em ambos os decks, e migra todos os orçamentos para pass_budget_in. A câmera e heldCamera de A
nos30 quadros adaptados passam a ser comparados; só instanceFrame/heldInstance ficam de fora.
A nova referência representável tem ownership separado (BL082.4.17); a S0 fica intacta.

Os erros do vizinho passam a Invalid com ponteiro do deck; a transformação de splat do topo
preserva seu erro original. O wire não carrega C: a S2 precisa fechar o contrato de validação
de primary antes de aceitar planos externos (BL082.4.20). Os limites de bytes são conferidos
no texto com chave literal, ou depois do parse se escapada; itens/profundidade são conferidos
na árvore. Não se provou plano válido aceito de exatamente20000 itens (.21).

BL082.4.18/.19/.22 estão em correção pelo coordenador. Testes/gates do novo head e rechecagem
independente continuam a validar. S2 e S3a não foram iniciadas; F064 permanece P.

### Resultado negativo do reforço da comparação de câmera

Core completo no Mac em61efdba:1593 passaram,1 falhou,7 ignorados (64 resultados).
A falha era no teste ampliado do coordenador: lit-center-held f23 tem contador de câmera23
no plano primário legado e11 no oráculo preso. A câmera curta tem12quadros. O construtor
primário preserva a avaliação estrita anterior; evaluate_shot_at_offset clampa separadamente.
A afirmação da revisão de que esses contadores de A podiam sempre ser comparados era incorreta.
O teste passa a conferir o contador primário legado pela fórmula cameraIn+delta, o contador
de offset contra X e heldCamera contra X. Para A vizinho, o contador do plano continua comparado
diretamente com X. A divergência de metadado fica BL082.4.23; não se alterou produção ou acervo.
Este resultado vermelho foi preservado e não conta como aprovação; novas execuções são necessárias.
