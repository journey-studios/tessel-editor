# G4.1 — Handles 2D da vinheta no monitor (centro, raio, suavidade)

Data: 2026-11-11. Projeto: `journey-studios/tessel-editor`. Base conferida: `origin/main` em `9fc491b`
(PR #43 escala, merge `cbe9e1b`; PR #45 luz pontual, merge `9fc491b`; PR #42 FX animados/film grain, merge `bf4347d`).

Este documento é o brief da implementação (Cursor Sonnet 5.5). Complementa o
[plano de gizmos](gizmos-spatial-editing-plan.md) (fase **G4**, "vinheta com handles 2D como primeira fatia"), o contrato F055
(`docs/features/scene-gizmo.md`), o inspector de efeitos F057 (`fx-inspector.md`), as chaves F058 (`fx-keys.md`) e a luz F060
(`scene-light.md`). Não cria outra fila: status e atribuição ficam no [backlog único](../backlog.md).

## Por que esta fatia

G1 (rotação/escala) e G2.1 (luz pontual) estão integradas. Das próximas fases do plano, G4.1 é a menor e a mais
isolada: o efeito, o gesto de efeito com prévia de candidato (BL082.6), o inspector e o overlay já existem; falta ligar o
ponteiro no monitor ao mesmo caminho do slider. Não depende de decisões de domínio abertas (spot/direcional, câmera
espacial) e prova que o overlay serve a um alvo que **não é um nó 3D**, o que G3/G4 completo vão precisar.

## Escopo funcional

Alvo: um clipe de efeito `vignette` na camada de FX (escopo de saída, o único aplicável hoje) que esteja **selecionado no
inspector** e cujo intervalo contenha o quadro exibido.

Três handles, desenhados no canvas 2D do overlay (nunca no WebGL, nunca no quadro exportado, nunca no documento):

| Handle | Parâmetro | Posição no monitor | Arrasto |
|---|---|---|---|
| `center` | `center` (ponto 0–1, y para baixo, como o plano) | o próprio centro | 2D livre; o ponteiro vira `center` em frações do quadro |
| `radius` | `radius` (0–1,5) | onde a queda **começa**: sobre o eixo horizontal que passa pelo centro, na elipse `distance = radius` | a distância do ponteiro ao centro (mesma métrica do shader) vira `radius` |
| `softness` | `softness` (0,05–1) | onde a queda **termina**: elipse `distance = radius + softness` | `softness = distância − radius` |

`roundness`, `intensity` e `color` não têm handle: ficam no inspector (F057). Nenhum controle fictício.

Além dos três pontos, o overlay desenha as **duas elipses** (início e fim da queda) para o usuário ver o que arrasta.

### Geometria (decidir e fixar por teste antes do código; fórmula do projeto)

Do `vignette_shade` de referência (`crates/core/src/directed_scene/fx/reference.rs`), com `u,v` em frações do quadro (`v` para
baixo), `W×H` a saída em pixels e `roundness = ρ`:

- `stretch = 1 + (W/H − 1)·ρ`, `shape = (stretch, 1)`, `norm = max(√(stretch² + 1), 1)`.
- `distance(u,v) = √((2·(u−cx)·stretch)² + (2·(v−cy))²) / norm`.
- Elipse de raio `r`: semieixos em frações do quadro `a_u = r·norm / (2·stretch)` (horizontal) e `a_v = r·norm / 2` (vertical).
- O handle `radius` fica em `(cx + a_u(radius), cy)`; o `softness` em `(cx + a_u(radius + softness), cy)`.
- Inverso (arrasto): dado o ponteiro `p`, `distance(p)` é o novo `radius` (ou `radius + softness`), depois quantizado pela grade
  do `step` do catálogo (`quantize_control`, a mesma do slider e do teclado) e limitado à faixa do catálogo.

Esta matemática vive no **core** (módulo novo sob `scene_edit/` ou `directed_scene/fx/`, funções puras, sem GPUI). O
`W×H` vem da saída da sequência/cena que o core já conhece; o overlay não recalcula nada, só desenha o que o core devolve.

### Regras de edição

- **Mesma função no arrasto e no comando numérico.** O arrasto produz, a cada move, o `set_param` que `scene.fx.edit` produziria
  com os mesmos valores; documento final igual bit a bit ao do `scene.fx.edit` com os valores que o arrasto reportou.
- **Reusar o gesto de efeito** `scene.fx.gesture.begin|update|commit|cancel|status` (F016/BL082.6): candidato do core, nada
  escrito na prévia, **um undo por gesto**, zero entradas em no-op (voltar ao ponto de partida), proposta recusada mantém o
  último candidato legal, invalidação por cena/projeto/undo/revisão. Um gesto de handle e um gesto de slider não coexistem:
  abrir um com o outro aberto liquida ou recusa por regra explícita já existente (conferir `BL015.19` e o F057), nunca
  confirma o gesto errado.
- **Chaves (P3, F058).** Se o parâmetro tem chaves no clipe, o handle desse parâmetro fica **desabilitado** com motivo
  consultável (`animated_param`, exibido como `disabled_reason`). O handle **não** usa o caminho "`set_param` edita só a base
  com aviso `FX_STATIC_UNDER_KEYS`", e **não escreve chave sozinho**. Gravar chave pelo handle é outra entrega (registrar como
  pendência nova). Parâmetros sem chaves do mesmo clipe continuam editáveis.
- **Prévia ao vivo.** Durante o arrasto o monitor mostra o quadro do **candidato** (como os sliders da seção Efeitos já fazem,
  `scene.fx.frame` com `gesture`) e o overlay se redesenha na geometria do candidato. Provar que a prévia vem do candidato e
  que cancelar restaura o quadro da base.
- **Fora do quadro.** Um handle cuja posição cai fora do quadro (raio+suavidade até 2,5 numa vinheta larga) não é desenhado nem
  captura o ponteiro; vira `disabled_handles` com motivo `handle_off_picture`. O campo numérico do inspector continua valendo.
- **Alvo único por vez.** Com um clipe de FX selecionado, o gizmo de nó não aparece (e vice-versa); definir e documentar a
  regra de precedência pela seleção existente e provar a troca sem arrasto fantasma. Selecionar outro clipe, trocar de cena,
  undo/redo, reload, perda de captura, Escape, diálogo e troca de projeto cancelam o arrasto como já fazem os do gizmo.
- **Reprodução.** O overlay segue oculto durante a reprodução (política vigente, BL015.3); nenhum handle aparece nem recebe
  ponteiro com o playhead rodando.
- **Identidade.** O carimbo do overlay ganha, de forma aditiva, o clipe de FX alvo (e o que mais for preciso); mensagens de
  página recarregada, de outro clipe, de revisão anterior ou de quadro anterior são descartadas e contadas, como em F055.

### Superfícies (regra do repositório)

Biblioteca, CLI headless e UI usam os mesmos handlers; toda interação por comandos semânticos.

- **Core/lib:** leitura dos handles do clipe (geometria, estado habilitado/desabilitado + motivo) e conversão ponteiro →
  parâmetro, funções puras. Nome da op e forma do JSON: **decidir e documentar no F061** (sugestão: leitura
  `scene.fx.handles` e extensão **aditiva** do overlay v1 / de `scene.pointer.*` com alvo de efeito; manter `scene.capabilities`
  negociável por listagem).
- **Headless (JSONL):** o mesmo arrasto por pedidos semânticos em coordenadas de quadro `(u, v)`, sem ponteiro físico, e o
  mesmo documento final que a UI.
- **UI (janela):** handles no monitor; no inspector, os campos numéricos de `center`/`radius`/`softness` continuam sendo o
  caminho equivalente; nenhum controle novo sem `Command` + nó em `state/tree.rs` + parser em `automation/commands.rs` + teste.
  Nenhum diálogo novo (se surgir, marcar `Workflow::has_dialog`).

## Fora do escopo

Handle de `roundness`, `intensity`, `color`; gravar chaves pelo handle (nova pendência); `fade` e `filmGrain` (não têm
parâmetro espacial: sem gizmo); escopo shot/node de efeitos (`FX_SCOPE_NOT_YET`); transições; luzes spot/direcional
(G2.2); câmera espacial (G3); snapping, seleção múltipla e grupos (G5); ponteiro físico na WKWebView (BL015.2/BL043/BL044
seguem pendentes); desempenho integrado (validação separada); Windows.

## IDs e documentação

- Feature nova: **F061** (confira que está livre antes de reservar) — documento próprio `docs/features/fx-handles.md` e linha em
  `docs/features/README.md`. Atualizar F055 (o overlay passa a servir um alvo de efeito), F057 e `effects.md` onde pertinente.
- Backlog: umbrella **BL082.35** (G4.1; confira que está livre) com filhas para cada pendência que sobrar (chaves pelo handle,
  ponteiro físico, desempenho). Reutilizar BL015/BL082.1/BL028; não duplicar linhas. **O `docs/backlog.md` da raiz do
  coordenador é dele**: no PR edite só as suas linhas e deixe o resumo na descrição para integração. Nada vira "validado"
  sem prova: sem prova fica "A validar" com o que falta.
- Matriz `feature-migration-matrix.md`: atualizar a linha de gizmos/efeitos se existir.

## Validação (cada item vira evidência registrada no F061)

1. **Biblioteca (core):** oráculo **escrito no teste** (fórmula própria do teste, não o código sob teste): pontos dos handles
   sobre as elipses para vários `W×H` (16:9, 1:1, 9:16) e `roundness` 0, 0,6, 1; ida e volta ponteiro → valor → ponteiro;
   `vignette_shade` nos handles dá 0 em `radius` e 1 em `radius+softness` (checagem cruzada com a referência já validada);
   quantização pela grade do catálogo; limites (`radius` 0 e 1,5; `softness` 0,05 e 1); centro nos cantos; recusas tipadas;
   erro atômico; cancel; no-op; gesto obsoleto; undo/redo; parâmetro com chaves desabilitado; handle fora do quadro;
   clipe fora do intervalo no quadro exibido; arrasto ≡ `scene.fx.edit` bit a bit.
2. **Headless real:** processo JSONL sem desktop, mesmo documento final que a janela e que a biblioteca.
3. **UI/runtime:** testes GPUI (seleção, troca de alvo, cancelamento por Escape/blur/hidden), testes JS do overlay (desenho,
   hit test, arrasto por mouse do Chrome real, captura perdida, resize) e janela própria por `./scripts/ui` só com comandos
   semânticos. Testes não substituem observação física da WKWebView: **não declarar prova física**.
4. **Imagem:** com o runtime real, o perfil de escurecimento ao longo da linha horizontal pelo centro, em quadros de uma cena
   do corpus, confere com a referência independente nos pontos dos handles (intocado em `radius`, tingido por
   `intensity` em `radius+softness`, dentro de tolerância declarada) e **o hash do quadro exportado é igual com o overlay ligado
   e desligado** (auxiliares fora do export).
5. **Mutações discriminantes** das regras novas (uma por vez, arquivo restaurado com hash conferido; modelo em
   `docs/migration/g12-scale/mutate.py`: raiz pelo local do script, relatório com diretório criado, `KILLED` só com teste
   nomeado que falhou, saída sem teste é erro de execução). Sugestões: sem `norm`, sem `stretch`, softness sem somar radius,
   sem quantização, chave ignorada, off-picture desenhado, prévia lendo a base, undo por move, estado do overlay de nó
   convivendo com o de efeito.
6. **Gates no head final** (preservar runs falhos/abortados): `cargo fmt --all -- --check`;
   `./scripts/cargo check --workspace --locked`; `./scripts/cargo clippy --workspace --all-targets --locked -- -D warnings`;
   **`./scripts/runtime build` antes** de `TESSEL_REQUIRE_RUNTIME=1 ./scripts/cargo test --workspace --locked --no-fail-fast`
   (o `runtime/dist` é ignorado pelo Git e envelhece entre ramos; já causou falhas falsas); `./scripts/cargo build -p
   tessel-editor --locked`; `cargo tree -p tessel-editor-core` sem gpui/wry; testes JS do runtime.
7. **Desempenho** (separado, sob o lock de GPU/build, só para medição): custo do overlay com handles visíveis/ocultos e do
   quadro de candidato durante o arrasto; lock ocupado = medição pendente, sem bloquear o funcional; não reaproveitar
   medições de outros gizmos.
8. **Revisão independente:** tarefa nova do coordenador depois da PR; CodeRabbit `NEUTRAL`/`SKIPPED` não é aprovação.

## Ordem sugerida de implementação

1. Contrato no F061 (geometria, nomes das ops, regras de alvo único, chaves, fora do quadro) e testes de oráculo do core
   **antes** do código; funções puras.
2. Leitura dos handles + conversão ponteiro → parâmetro; arrasto ≡ `scene.fx.edit`; gesto reaproveitado; recusas.
3. Overlay: desenho das elipses e dos 3 handles, hit test, ponteiro, carimbo e guardas; prévia do candidato.
4. Adapter desktop: seleção ↔ alvo, `Command`/nó/parser, desabilitados com motivo; inspector sincronizado.
5. Provas: headless, janela semântica, imagem/export, mutações, gates, docs e backlog.

## Restrições de ambiente

- Cargo **só pelo wrapper** `./scripts/cargo` (mbx, target padrão); nunca `CARGO_TARGET_DIR`, `--target-dir`,
  `build.target-dir`; nunca `mbx clean`/`gc`. Disco: reserva de 16 GiB para iniciar lote pesado, piso de 12 GiB; sem
  artefatos em `/tmp` acima de ~500 MB; não limpar caches alheios; evidências grandes fora do Git.
- O lock de GPU/build só é exigido para **medições de performance**; implementação, builds e testes funcionais seguem sem a trava.
- Só `gpui-kit` na UI; sem serviços, credenciais, telemetria ou rede; Rust 1.99.1; preservar `Cargo.lock`.
- Não modificar a origem nem copiar assets/dados privados. Sem menção a assistentes em commits (use a skill `commit`).

## Entrega

Branch `t3/vignette-handles` criada a partir de `docs/g4-vignette-handles-plan` (que contém este plano) ou, se este já estiver
na `main`, de `origin/main`. Commits pequenos; push e **PR contra `main`**, sem mergear nem monitorar; `link_pull_request`
logo após criar. Descrição da PR: o que entrou, decisões de contrato, o que **não** foi provado (ponteiro físico, WKWebView,
desempenho, Windows), gates com SHA do head e as linhas de backlog a integrar. Pare e pergunte diante de ambiguidade de
contrato em vez de inventar paridade.
