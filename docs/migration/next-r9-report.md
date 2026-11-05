# R9 — Inspeção visual real do inspector, árvore e nota do monitor (BL087)

Feature: [features/scene-inspector.md](../features/scene-inspector.md#inspeção-visual-nativa-bl087).
Estado de execução fica só no backlog; este relatório entrega o resultado e os patches para o coordenador.

## Resultado

**Verificado por captura nativa da instância própria, com 11 defeitos de layout achados e corrigidos.**
Janela larga (a tela limitou 1680x1000 a 1680x879) e mínima 960x640; escuro e claro; árvore expandida, selecionado,
candidato de gesto, erro tipado, vetor, valor extremo, última linha; docks de 100/180/272/628 px no mínimo.
50 capturas do SO (`captured: 50/50`), cada uma com recorte do inspector, antes e depois das correções, lidas uma a uma.

O que **não** está provado: ponteiro/roda de dispositivo real (BL043), escala de tela diferente de 2x, e o estado "rolando"
(não há comando semântico de rolagem, então o scroll é evento injetado no GPUI + checagem de geometria).

## Defeitos corrigidos

Tabela completa D1–D11 em [scene-inspector.md](../features/scene-inspector.md#inspeção-visual-nativa-bl087). Os que mudam
comportamento visível: árvore em linhas inteiras e proporcional à janela; seleção por ID abre ancestrais e rola até a linha
(uma vez); valor nunca sobre a trilha; trilha ≥ 64 px em qualquer dock; layout empilhado abaixo de 232 px; motivos na tela;
botão "Anexar" e recuo da árvore cabem em 100 px.

Achado real do teste de GPUI: a altura da linha do `ListItem` do Kit vem da fonte (27,5 px na plataforma de teste), então
a altura da árvore em "linhas inteiras" cortava a última. A linha agora tem altura fixa `TREE_ROW_HEIGHT`; um teste mede o
passo real das linhas.

## API e seams

| Item | Onde |
|---|---|
| `PanelFit::{of, tree_height}`, `TREE_ROW_HEIGHT`, `REGULAR_MIN_WIDTH`, `MIN_TRACK`, `MAX_VALUE`, `PILL_MIN_WIDTH` | `scene_model.rs` |
| `TreeEntry::ancestors`, `Snapshot::{take_reveal, single_selection}` (revelação uma vez por seleção) | `scene_model.rs` |
| `AttachSceneOpener`, `ATTACH_SCENE_OPENER: Option<_> = None`, `attach_availability` | `scene_panel.rs` |
| Probes `scene:tree`, `scene:monitor-reason`, `scene:attach`, `scene:attach-reason`, `value:<elemento>` | `scene_panel.rs` |
| `scripts/scene-visual-proof.py` | novo; sob `with-gpu-lock.py` |
| `scene_visual_tests.rs` (módulo `scene_tests::visual`, declarado por `#[path]` em `scene_tests.rs`) | novo |

Nenhum arquivo fora do meu ownership foi editado. `Cargo.*`, Kit e a11y intactos.

## Falha apontada pela R2

`dragging_a_slider_in_the_stacked_layout_is_one_gesture_and_one_undo_entry` falhava com "no slider on screen to wheel over"
numa versão intermediária do teste, que procurava um slider visível para rolar por cima. Em 960x640 com inspector de 120 px
**nenhum** slider fica visível sem rolar (árvore + nota + "Anexar" ocupam a altura), então a premissa do teste era errada, não
o painel. Corrigido: o teste rola sobre texto liso (o motivo de "Anexar") até o fim, confere que a trilha entrou na vista e
só então arrasta. As asserções de gesto/undo não mudaram (gesto aberto, documento intocado a meio do arrastar, **uma** entrada
no soltar, undo volta). Mensagem removida do código; 3 execuções seguidas verdes.
Um teste novo prova que a roda sobre um slider rola o inspector (120 e 272 px).

## Achado fora do meu ownership: cabeçalho em 960 px (R2, `node:26`)

O botão "Anexar cena" do cabeçalho está presente e legível, mas a barra de ferramentas (`region:toolbar`, 855 px,
`flex_shrink_0`) começa em x = 97 numa janela de 960 e **passa por cima do título "Tessel Editor"**; "Abrir" fica impresso
sobre o título (visível em `after/selected-min-{dark,light}.png`, recorte `/tmp/te-r9/after-header-crop.png`). Antes do botão
a barra cabia (`before/selected-min-dark.png`: título e "Abrir" separados). Largura mínima da janela: a barra precisa de ~78 px
a menos. Patch sugerido para `header.rs`, a decidir pela R2/coordenador: nas larguras < ~1040 px mostrar os botões de ação só com
ícone (tooltip + `accessibility_label` já existem), ou tirar o rótulo de "Anexar cena" e "Empacotar" nessa faixa; e dar ao grupo
do título `flex_shrink` com `overflow_hidden` em vez de `flex_shrink_0` para que um excesso futuro corte o título, nunca
sobreponha botões. Nenhum teste da R2 mede sobreposição; sugestão: asserir `region:toolbar.x >= título.right` em 960 px.

## Patches para o coordenador

1. **Ligar "Anexar"** (`scene_panel.rs`, uma linha):
   ```rust
   const ATTACH_SCENE_OPENER: Option<AttachSceneOpener> = Some(|this, window, cx| {
       let _ = this.open_flow(FlowKind::AttachScene, window, cx);
   });
   ```
   e trocar `NO_SCENE_REASON` em `scene_ui.rs` (ainda diz "a janela ainda não tem diálogo de anexar").
2. **Comando semântico de rolagem** (`ui op scroll` com alvo `scene.panel` / `scene.tree`): hoje a rolagem não é
   provável nativamente. Fora do meu ownership (`automation/*`).
3. **`property_row` de `controls.rs`**: rótulo de 76 px / valor de 38 px é a mesma família do defeito D3/D9 nos painéis de clipe;
   os sliders de cena já não o usam. Unificar é decisão do coordenador.
4. Backlog/matriz (IDs existentes ou novos, sem fila paralela):
   - `BL087` → verificado por captura nativa (50/50) + 11 testes de layout; resta ponteiro/roda real (BL043).
   - **novo** `BLxxx` — cabeçalho em 960 px com "Anexar cena" (R2): sobreposição título/toolbar.
   - **novo** `BLxxx` — comando semântico de rolagem para provas nativas.

## Testes

Rodados (via `cargo-serial.py`):

| Comando | Resultado |
|---|---|
| `test -p tessel-editor --locked` (pacote inteiro) | 188/188 |
| `test … scene_model` | 19/19 |
| `test … scene_tests::visual` | 11/11 (3 execuções seguidas) |
| `clippy -p tessel-editor --locked --all-targets -- -D warnings` | limpo |
| `fmt -p tessel-editor -- --check` | arquivos meus limpos; diferenças só em `scene_lifecycle_tests.rs` e `viewport_host.rs` (R8) |
| `scripts/scene-visual-proof.py` sob `with-gpu-lock.py` | 6 checagens ok, 50/50 capturas, `visual_validated: pending-human-or-agent-review` |

Não rodados: `pnpm`/build de release, o app do usuário (nunca tocado), prova de ponteiro físico, e `cargo test` do workspace
inteiro (`tessel-editor-core`/`media`/`render`) — arquivos de outros agentes estavam em edição.

Incidente: um `cargo fmt -p tessel-editor` (escrevendo) foi executado uma vez no pacote do desktop; os únicos arquivos fora do meu
ownership com diferença de formatação continuam diferentes (`viewport_host.rs`, `scene_lifecycle_tests.rs`), portanto não os
reescrevi; depois passei a usar só `--check`.

## Evidência

`/tmp/te-r9/before` (50 capturas do estado original) e `/tmp/te-r9/after` (50 após as correções), `/tmp/te-r9/report-after.json`.
Mantidos até a revisão do coordenador; removo só o que escrevi, sem tocar o resto de `/tmp`.
