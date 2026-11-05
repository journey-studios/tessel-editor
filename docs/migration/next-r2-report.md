# R2 — Anexar cena pela interface (BL086, F043)

Relatório da frente R2 da [próxima entrega](next-delivery-ten-agents.md). Estado de execução vive só em
[`docs/backlog.md`](../backlog.md); aqui ficam seams, evidências, limites e patches para o coordenador.
Contrato da feature: [F043](../features/scene-import-ui.md).

## 1. O que existe

Um diálogo Kit "Anexar cena" (cabeçalho, `node:26`) e um diálogo "Recursos da cena" (reparo). Os dois
**só montam pedidos** e os enviam pelas operações que já existem; nenhuma regra de domínio foi duplicada. Adopt/resolve rodam em jobs; leitura/parse do attach ainda são síncronos (BL100).

```
attach (scene.attach, na hora, 1 entrada de desfazer)
  → adopt   (scene.resources.adopt, job)    só se a pasta de recursos foi informada
  → resolve (scene.resources.resolve, job)
  → select  (cena ativa + inspetor em modo cena)
  → show    (monitor desenha a cena, só se o mapa foi instalado)
```

Quem reage aos jobs é o `workflow_tick` de 250 ms (`host.poll`); a cadeia é uma máquina de estados pura
(`SceneImport`) que só olha eventos de job do seu próprio id.

## 2. APIs e seams

Arquivos (todos da minha propriedade): `workstation/scene_import.rs` (novo), `scene_import_tests.rs`
(novo), `flows.rs`, `workflow.rs`, `header.rs`, `workstation/mod.rs` (só `mod scene_import;`),
`scripts/scene-import-proof.py`, `docs/features/scene-import-ui.md`, este relatório.

| Seam | Onde |
|---|---|
| `FlowKind::{AttachScene, SceneResources}` + campos `bundle/id/name/source/show/scene` | `flows.rs` (`fields()`, `build_request`, `import_plan`) |
| `Picker::Directory` (seletor de pasta; o painel nativo só devolve caminho) | `flows.rs`, `workflow.rs::choose_path` |
| `ImportPlan`, `SceneImport`, `State`, `Failure` | `scene_import.rs` |
| `Workstation::scene_import_{submit,begin,start,receive,present,settle,nodes}` | `scene_import.rs` |
| Nó `scene_import` (estado `idle/running/attached/loaded/failed`, `step`, `done`, `error`, `presentation_error`) | `scene_import_nodes()` ← `workflow_nodes` |
| Botão do cabeçalho `node:26` "Anexar cena" | `header.rs` (`NODES`=27, `ACTIONS`=9), `workflow_invoke` |
| `flow.attach_scene`, `flow.scene_resources`, `dialog.field.*`, `dialog.submit/cancel` | já são nós via `flows::nodes()`/`workflow_nodes` |

Respeito a `Workflow::has_dialog`: o diálogo é um Kit `Root` dialog, então o WebView do monitor sai da
tela enquanto ele está aberto (provado, §3).

Preparação para `scene.export` (contrato R1): **nada habilitado**. A cadeia não conhece export; quando a
ação existir basta um `FlowKind` novo com `build_request` próprio. Nenhuma ação inexistente foi ligada.

## 3. Evidências

### Testes executados

| Comando (wrapper `cargo-serial.py`) | Resultado |
|---|---|
| `test -p tessel-editor --locked scene_import` | **18 passaram** |
| `test -p tessel-editor --locked` (crate inteiro) | 185 passaram, **1 falhou** (ver §5, não é desta frente) |
| `clippy -p tessel-editor --all-targets --locked -- -D warnings` | limpo |
| `fmt -p tessel-editor -- --check` | meus arquivos limpos; restam diffs de `scene_lifecycle_tests.rs`, `scene_model.rs`, `scene_panel.rs`, `scene_visual_tests.rs` (outras frentes) |
| `python3 scripts/directed-scene-proof/with-gpu-lock.py python3 scripts/scene-import-proof.py` (binário debug, HOME curto próprio, só IPC semântico) | **33 checks, ok** |

Os 18 testes (`scene_import_tests.rs`): o diálogo envia os mesmos pedidos que um script; semântica do
plano; nós; cadeia adopt→resolve; falhou/cancelado/`not_installed`; passo que não consegue começar; e,
contra o `MediaHost` real e uma cópia privada do corpus sintético: arquivo válido, bundle inválido,
recurso faltante, fonte alterada, arquivo gerenciado alterado, só anexar, ocupado/cena desconhecida,
sem workspace, nós da árvore e do cabeçalho (`node:26`).

### Prova viva (socket, janela nativa própria)

Corpus sintético de `scripts/directed-scene-proof/corpus`, copiado para uma pasta descartável. Prova:

- os nós `flow.attach_scene`, `flow.scene_resources`, `scene_import`, e cada campo, existem na árvore viva;
- abrir o diálogo tira o WebView da tela (`monitor.viewport.visible` vai a `false`) e fechá-lo o devolve;
- **bundle inválido** → `invalid_bundle`; **caminho inexistente** → `bundle_unreadable`; nos dois o diálogo
  fica aberto com o erro e **não** há cena, entrada de desfazer, pasta de recursos nem cadeia;
- **arquivo válido** → 1 entrada de desfazer; jobs `adopt` e `resolve` concluídos e aplicados pela janela,
  passos em ordem, três PNG copiados e verificados para dentro do projeto, `scene.resources.status`
  `loaded/current`, jobs no histórico compartilhado;
- inspetor em modo cena na cena nova; o **monitor mostra a cena e o hash do que desenhou é o de um
  `scene.render` explícito do mesmo quadro** (`TESSEL_VIEWPORT_INSPECT=1` liga a leitura do monitor);
- **recurso faltante** → `resources_invalid` no passo `adopt`, a mensagem nomeia o arquivo, a cena segue
  anexada, nada foi copiado, nada carregado, e o nó `scene_import` guarda a falha;
- **fonte alterada** (hash) → recusada e nunca copiada;
- **reparo** pelo segundo diálogo carrega a mesma cena sem novo `scene.attach`.

### Achado corrigido durante a prova

O host mapeia todo erro de job sem nome no vocabulário de comandos para `job_failed` e guarda o código
real em `details.job_code`. A primeira versão mostrava `job_failed`. `Failure::of_event` agora prefere
`details.job_code`, anexa à mensagem o primeiro problema por arquivo (`path — detail`) e guarda os
`details` inteiros. Coberto por teste puro e pela prova viva.

## 4. Não executado / não provado

- **Painel nativo de arquivos do macOS** (precisa de humano); o campo de caminho é o equivalente provado.
- **Ponteiro físico e aparência** (cabeçalho estreito, espaçamento do diálogo): não inspecionado.
- Corpo do diálogo em teste unitário: janelas de teste não têm Kit `Root`; coberto só pela prova viva.
- Gate completo do workspace (`pnpm`-equivalente do repositório): não rodado, só o crate desktop.
- Windows/Linux: nada verificado.

## 5. Limitações e riscos

1. `scene.attach` lê o bundle na thread que chama (limite de 32 MiB por texto de bundle; 128 MiB é o orçamento agregado de cenas). Attach assíncrono pede
   mudança no core.
2. Não há attach+carga atômico: entre os passos a cena existe sem recursos. Em falha após o attach a cena
   **fica** (decisão: é uma entrada de desfazer legítima, e re-anexar duplicaria).
3. Cena só com dados não carrega sem pasta: `scene.resources.resolve` sem base dá `root_unavailable`.
4. Um `resolve` que falha mantém instalado o mapa anterior (comportamento do host).
5. Botão no cabeçalho pode apertar vizinhos em janelas estreitas (não medido).
6. O teste `scene_tests::visual::dragging_a_slider_in_the_stacked_layout_is_one_gesture_and_one_undo_entry`
   (`scene_visual_tests.rs`, arquivo novo de R9, ainda em edição durante esta rodada) falha com
   `no slider on screen to wheel over` num inspetor de 120 px. Não toca cabeçalho, fluxos nem
   `scene_import`; não alterei nem investiguei além de confirmar que falha isolado.
7. Aviso de `clippy` anterior sobre `same_folder` em `media/scene_resources/import.rs` (WIP alheio) sumiu
   na última execução; erros transitórios de `core/directed_scene/temporal/frame.rs` também (WIP alheio).

## 6. Patches para o coordenador (não aplicados: arquivos que não são meus)

### 6.1 `crates/desktop/src/workstation/scene_panel.rs` (R9) — ligar o botão do inspetor

R9 deixou o seam pronto (`ATTACH_SCENE_OPENER`). Com `FlowKind::AttachScene` e `open_flow` existentes:

```rust
const ATTACH_SCENE_OPENER: Option<AttachSceneOpener> = Some(|this, window, cx| {
    // O diálogo mostra o próprio erro; um erro aqui só diria que já há um diálogo aberto.
    let _ = this.open_flow(super::flows::FlowKind::AttachScene, window, cx);
});
```

Depois, atualizar o teste de R9 que espera o botão desabilitado com razão (se existir) e o texto do
comentário acima da constante.

### 6.2 `docs/backlog.md`

- **BL086**: `Em andamento` → `Parcial` ou `Concluído` depois do 6.1; evidência: F043, 18 testes
  `scene_import`, prova viva de 33 checks. Pendente: painel nativo de arquivos e aparência.
- **BL089** (ou o ID de recursos de cena sem base): cena só com dados não carrega (`root_unavailable`) —
  dono core/media (R5 ou coordenador).
- Sugestão de ID novo: **attach assíncrono e atômico attach+carga** (core).

### 6.3 `docs/features/README.md` e matriz

Linha nova:

```
| F043 | [Anexar cena pela interface](scene-import-ui.md) | V no corpus | V | V no corpus | Parcial | `scene_import`, `flow.attach_scene`, `flow.scene_resources`, `node:26`; painel nativo de arquivos e aparência pendentes |
```

Em `docs/migration/feature-migration-matrix.md`: marcar "Anexar/carregar cena por diálogo" como UI
parcial, sem tocar nas colunas de lib/headless.

### 6.4 `docs/ui-automation.md` (linha ~106)

Acrescentar na lista de botões do cabeçalho: ``Anexar cena `node:26` `` (ícone `node:27`), e nas
tabelas de fluxos `flow.attach_scene`/`flow.scene_resources`.
