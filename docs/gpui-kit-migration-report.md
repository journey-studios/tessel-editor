# Relatório — migração da UI para GPUI Kit

Data: 2026-11-04. Alvo: `/Users/wallyssonnunes/www/tessel-editor`. Plano: [gpui-kit-migration-plan.md](gpui-kit-migration-plan.md).
Escopo: UI e Dock, local. Sem push, PR, merge, deploy ou publicação; nenhum arquivo do tesselapp foi tocado.
O repositório continua **sem primeiro commit**: nada foi commitado.

## Resultado

A aplicação demonstrativa roda sobre GPUI Kit: bootstrap, tema, controles, ícones e **docking real**
(`DockArea`/`DockSkin`). Guise, GPUI 0.2.2 e Tailor saíram da cadeia ativa. O motor, o playback, a
exportação e o WebView **não** foram migrados (etapa separada, ver "WebView POC").

## Versões e backend

| Item | Versão | Observação |
|---|---|---|
| `gpui-kit` | 0.9.0 (`=0.9.0`) | única dependência de UI declarada; features `default` (`component`, `assets`) |
| `gpui-component` / `gpui-base` / `gpui-kit-assets` | 0.9.0 | puxadas pelo Kit |
| GPUI | `gpui-pre` 0.3.11 (snapshot do Kit) | reexportado por `gpui_kit::*`; sem `gpui` 0.2.2 no `Cargo.lock` |
| Backend | **padrão (`gpui-pre`)** | `gpui-fast` não habilitado (ver decisão abaixo) |
| Rust | **1.99.1** (`rust-toolchain.toml`) | era 1.94.1 |
| Cargo | `./scripts/cargo` (mbx 1.29.2) | cache entre worktrees, `docs/cargo-worktrees.md` |

**Por que Rust 1.99.1:** com `RUSTUP_TOOLCHAIN=1.94.1` o build falha em `gpui-pre 0.3.11`
(`profiler.rs:475`, E0658: `std::hint::cold_path` instável). É uma exigência da dependência, não uma
preferência. Os manifestos não declaram `rust-version`; 1.95 é onde `cold_path` se estabiliza, mas só
1.94.1 (falha) e 1.99.1 (passa) foram exercitados. Os arquivos do registry não foram alterados.

**Cadeia limpa:** `cargo tree -p tessel-editor -e normal,build,dev` não contém Guise, Tailor nem
`gpui` 0.2.x (`gpui_util`, `gpui_collections`, …); `Cargo.lock` tem 0 ocorrências desses nomes e
uma única família `gpui-pre`. `rg 'guise|gpui::'` em `crates/` não encontra nada: todo import vem de
`gpui_kit`. Remanescentes de texto: avisos históricos em docs antigos e `docs/archive/editor.tailor.txt`.

## Componentes migrados

| Frente | Kit usado | Onde |
|---|---|---|
| Bootstrap | `gpui_kit::application().with_assets(AllAssets)`, `gpui_kit::init`, `gpui_kit::open_window` | `main.rs` |
| Tema | `Theme::change(ThemeMode)`, `ActiveTheme` | `theme.rs`; tokens da workstation derivam dele |
| Cabeçalho | `Toolbar`, `Button` compacto (`Sizable`), tooltip = nome acessível | `header.rs` (substitui o gerado pelo Tailor) |
| Botões de ferramenta | `Button` ghost/xsmall com `tooltip` | `controls.rs` |
| Busca | `Input`/`InputState` | `library_panel.rs` |
| Pastas | `Tree`/`TreeState` | `library_panel.rs` |
| Abas | `TabBar`/`Tab` (underline no inspetor, pill no modo de workspace) | `controls.rs` |
| Sliders | `Slider`/`SliderState` (propriedades e zoom) | `inspector_panel.rs`, `timeline_panel.rs` |
| Selects | `Select`/`SelectState` | `inspector_panel.rs` |
| Seções | `Accordion` | `inspector_panel.rs` |
| Boolean | `Switch` | `inspector_panel.rs` |
| Scroll | `ScrollableElement` | painéis |
| Dock | `DockArea`, `DockSkin`, `BasePanel`/`Panel`, `DockLayout`, `DockAreaState` | `workstation/dock.rs` |
| Timeline | desenho/eventos próprios portados (ruler, playhead, clips, miniaturas, waveforms, handles de trim) | `timeline_panel.rs`, `drag.rs` |

Os widgets não guardam estado duplicado: `Workstation` ressincroniza sliders/selects/tree a partir de
`EditorState` a cada render, e só reconstrói a `Tree` quando o modelo mudou.

Corrigidos durante a validação visual: (1) `Tab` do Kit desenha **ícone ou rótulo**, então as abas
Editar/Cor/Entregar apareciam só como ícones numa pílula; agora usam rótulo. (2) `Switch` com `label`
quebrava a linha "Ken Burns" e sobrepunha a linha de Rotação; o rótulo visível saiu (nome acessível
mantido). (3) Docks laterais esvaziados ficavam abertos como uma faixa em branco; agora colapsam e
reabrem quando um painel volta (comandos e gestos, `DockEvent::LayoutChanged`).

## Bug de tema encontrado e corrigido

`./scripts/ui theme dark` aplicava o tema **oposto**, e `invoke toolbar.theme toggle` não alternava:
`theme::apply` recebe `dark`, mas o servidor passava `target == "theme.light"`. Nenhum teste cobria. Corrigido
(`automation/server.rs`, nova `theme::toggle`) e coberto por dois testes GPUI em `theme.rs`. Confirmado no app:
`theme light` → `light`, `theme dark` → `dark`.

## Docking

Layout inicial (verificado por teste e no app): biblioteca à esquerda (232 px), monitor único no centro,
timeline abaixo (296 px), inspetor à direita (272 px). Cabeçalho e status ficam fora do Dock. Headers
arrastáveis, tabs, splits e divisórias vêm do Kit; a CLI usa os mesmos caminhos.

| Comando | Efeito |
|---|---|
| `dock` / `get dock` | layout vivo: docks, grupos, splits, painéis, ocultos, `layout.dump` |
| `dock show\|hide\|activate <panel>` | reabre/oculta/ativa a aba |
| `dock move <panel> --to left\|right\|bottom\|center` | move para o 1º grupo do dock |
| `dock move <panel> --group group:N\|panel:P [--index N]` | entra num grupo de abas |
| `dock move <panel> --beside group:N\|panel:P --side …` | cria split |
| `dock resize left\|right\|bottom <px>` · `dock open\|close <dock>` | tamanho/colapso |
| `dock split split:N px,px,…` | divisórias |
| `dock load <json \| @file>` · `dock reset` | restaura dump / padrão |

Equivalentes: `invoke panel:<id> hide|show|activate`, `set dock:left size 300`, `set split:N sizes …`,
`move panel:<id> --to bottom`, `invoke dock reset`. Painéis e docks têm IDs fixos; `group:N`/`split:N` são
numerados pelo `DockArea` e mudam após reset/load (consultar `dock` antes de usar). Erros são estruturados
(`unknown_element`, `out_of_bounds` com `min`/`max`, `invalid_value`, `not_applicable`) e não alteram o layout.

## Provas

**Validação final (todas com `--locked`, saída resumida):**

| Comando | Resultado |
|---|---|
| `cargo fmt --all -- --check` | ok |
| `./scripts/cargo check --workspace --locked` | ok |
| `./scripts/cargo clippy --workspace --all-targets --locked -- -D warnings` | ok, 0 avisos |
| `./scripts/cargo test --workspace --locked` | **96 passed, 0 failed, 0 ignored** |
| `./scripts/cargo build --locked -p tessel-editor` | ok |

**Testes (baseline 69 → 96).** Comparação por nome com o snapshot `/tmp/tessel-editor-before-gpui-kit-20261104.tar.gz`:
65 dos 69 nomes continuam; os quatro testes de input GPUI do trim (drag contínuo, drag curto de um movimento,
trim à esquerda, Escape) passam no backend do Kit sem alteração de comportamento. Os 4 que não existem mais
testavam código manual substituído por componentes Kit:

| Baseline | Destino |
|---|---|
| `workspace_tabs_and_panels_hold_state` | dividido: `workspace_tabs_hold_state` + testes de visibilidade de painel no modelo de layout/Dock |
| `tabs_parse_and_switching_closes_the_open_select` | parse mantido em `tabs_parse_and_switching_reports_the_change`; o popup do select agora é do Kit `Select` |
| `fractions_clamp_to_the_element` | slider manual removido; a conversão ponteiro→valor é do Kit `Slider`; a nossa quantização segue testada (`pointer_values_snap_to_the_step_and_stay_in_range`, `f32_slider_artefacts_return_to_the_step_grid`) |
| `select_arrows_stop_at_the_ends` | teclado do select é do Kit `Select` |

Esses comportamentos agora são responsabilidade da biblioteca e **não** têm teste próprio aqui.
Novos: 11 testes de Dock (layout inicial, hide/show, mover entre docks, entrar em grupo de abas, split,
tamanho/colapso, reset, dump/load, erros atômicos, CLI = UI, colapso de dock vazio), os de layout/CLI puros
e 2 de tema.

**Smoke com o app aberto** (CLI contra a instância viva, relançada a cada rebuild; os smokes de Dock e de edição rodaram antes do ajuste de colapso do dock vazio, que foi reverificado no build final):
- Dock: `move inspector --to bottom` (vira aba ao lado da timeline), `resize left 300`, `hide/show library`,
  `activate inspector`, `move library --beside panel:timeline --side right` (split 434/434), dump → `reset`.
  Cinco comandos inválidos (painel/grupo inexistentes, tamanhos 5 e 99999, `hide nonexistent`) devolveram erro
  estruturado e o layout ficou idêntico (comparação do JSON completo antes/depois).
- Edição: `set trim:first-light:end frame 120`, `…:start frame 24` (extremo oposto preservado); frames −4 e 9999 →
  `out_of_bounds`, frame 24 no `end` → `invalid_duration`, estado inalterado. `move` sobreposto → `overlap`.
  `property:zoom` 250 e 110, `property:blend multiply`, valor inválido → `invalid_value`; pastas, abas do
  inspetor, busca, abas de workspace e seção `inspector.cropping` responderam como na tabela do `ui-automation.md`.
- Trim por **gesto** (curto de um movimento e Escape): coberto pelos 4 testes GPUI de input, que enviam
  mouse down/move/up e Escape pela plataforma de teste; **não** foi repetido com ponteiro real.
- Dock vazio (build final): `move library --to right` → `dock:left closed`; `move library --to left` → reabriu.

**Observação visual** (`screencapture -l <window id>`, janela nativa):
- 1440×900 e 960×640, tema escuro: sem sobreposição no cabeçalho nem no inspetor; um único monitor; a barra de
  status mostra o último comando da CLI (ex.: `CLI · panel:library → move`), ou seja, o estado da CLI aparece na UI.
- Tema claro e escuro conferidos nas capturas; layout reorganizado (inspetor em aba embaixo, biblioteca à direita,
  dock esquerdo colapsado) conferido em captura.
- As capturas ficaram em `/tmp` e **não** foram incluídas no repositório (contêm a imagem pública ProUI,
  sem licença de redistribuição presumida).

## Limites e o que NÃO foi validado

- **Drag real de headers/divisórias com ponteiro** não foi exercitado: os gestos são do Kit e a CLI usa as
  mesmas operações do `DockArea`, mas nenhum teste ou smoke arrastou com o mouse. Os testes de Dock chamam
  `execute_layout`.
- **Menus e popups abertos** (dropdown do `Select`, menu `…` dos painéis, tooltips) não foram abertos nem
  fotografados; a CLI não os abre.
- Durante a verificação o estado do app mudou duas vezes **sem comando meu** (aba Cor, zoom 201): havia input
  de ponteiro externo na janela. O app final foi relançado limpo.
- Só macOS (Metal) foi construído e executado. Linux e Windows não foram validados.
- Sem playback, exportação, processamento, persistência nem undo; esses botões seguem desabilitados com motivo.
  Imagens `assets/demo` são referência pública ProUI, sem licença comercial presumida
  (`docs/proui-reference.md`).
- O pill selecionado das abas e os acentos seguem o `primary` do tema do Kit (neutro no tema padrão); não há
  tema Tessel customizado ainda.

## WebView POC (etapa separada, **não iniciada**)

**Seam preparado, nada integrado.** `workstation/viewport.rs::render_viewport` é o único ponto que pinta a
imagem do monitor (hoje um still ou um placeholder); o painel do monitor (cabeçalho, transporte), o Dock, o
estado e a CLI não dependem de como a imagem é desenhada. Uma viewport WebView troca só essa função.

**Backend padrão × `gpui-fast`.** Decisão registrada: manter o **padrão (`gpui-pre`)** nesta entrega, pois não há
WebView. Pela página oficial (https://gpui-kit.com/docs/webview/, consultada em 2026-11-04), `gpui-webview`
(Wry) é **experimental** (macOS WKWebView, Windows WebView2, Linux só X11). No `gpui-pre`, a view nativa **cobre**
popovers, diálogos, menus e tooltips na sua área e não é recortada por contêineres; com `gpui-fast`, macOS e
Linux registram a view na árvore de composição e overlays GPUI aparecem acima. Como o editor tem Dock, menus e
tooltips sobre o monitor, **a POC deve usar `gpui-fast`**, habilitado em `gpui-kit` **e** `gpui-webview` (features
que precisam concordar). O manifesto do `gpui-kit` 0.9.0 não tem feature de WebView; `gpui-webview` é crate
separado e **não foi baixado nem compilado** aqui.

Evidência mínima sobre `gpui-fast`, **apenas compilação/testes**: numa cópia descartável do projeto (fora do repo,
já removida) com `gpui-kit = { version = "=0.9.0", features = ["gpui-fast"] }` e `--offline`, `cargo check` e
`cargo test --workspace` passaram (96 testes). Nenhuma janela foi aberta nesse backend. Habilitar a feature muda
o `Cargo.lock` (falha com `--locked`), então a troca precisa ser uma atualização deliberada do lock.

**Roteiro da POC (tudo não validado até ser executado):**
1. Página local determinística com WebGL/Three.js, servida de arquivo local, sem rede nem provedores.
2. Bridge Rust↔JS **versionada** (IPC do Wry com tipos de mensagem e tamanho limitados, origem verificada).
3. Resize repetido do painel do monitor (Dock e janela) mantendo `set_bounds`/clipping corretos.
4. Overlays GPUI (tooltips, menus, popup do `Select`, arraste de painel do Dock) sobre a viewport.
5. Medição de frame time com a viewport ativa e com o playhead movendo durante playback simulado.
6. Comunicação durante playback (estado do playhead → Three.js, eventos de volta), foco/teclado entre inputs
   GPUI e a página, política de navegação/downloads/novas janelas, falha de carga e fechamento da janela.

Limites: experimental em todas as plataformas; Wayland sem suporte; a documentação não prova o comportamento no
nosso app, só uma execução real prova.

## Arquivos

Novos/alterados nesta migração (todos sem tracking): `Cargo.toml`, `Cargo.lock`, `rust-toolchain.toml`,
`crates/desktop/Cargo.toml`, `crates/desktop/src/{main,shell,theme,header}.rs`,
`crates/desktop/src/workstation/{mod,dock,dock_tests,controls,viewport,monitor,library_panel,inspector_panel,timeline_panel,status_bar,tokens,drag,geometry,media,waveform,input_tests}.rs`,
`crates/desktop/src/state/layout.rs` (modelo puro do Dock), `automation/*`, `scripts/ui`, e docs
(`README.md`, `AGENTS.md`, `DESIGN.md`, `PRODUCT.md`, `docs/ui-automation.md`, `docs/migration-scope.md`,
este relatório). Removidos da cadeia ativa: `crates/desktop/src/ui/`, `scripts/export-tailor.py`, `.mcp.json`
(só continha o servidor Tailor); `design/editor.tailor` foi arquivado em `docs/archive/editor.tailor.txt`.
`scripts/cargo`, `scripts/mbx`, `.mbx.toml` e `docs/cargo-worktrees.md` são do pai e foram preservados.
Tailor, Guise e a configuração global não foram desinstalados nem alterados.

## Revisão após a entrega delegada

A revisão local acrescentou três testes com eventos reais da plataforma de testes do GPUI Kit:
confirmação do select por teclado, cancelamento de uma escolha não confirmada com Escape e arraste do
slider para além de seus dois limites. As verificações conferem o estado compartilhado do editor;
o cancelamento também confere o valor selecionado no widget. Uma sonda de geometria no contêiner do
slider permite localizar o controle sem coordenadas fixas.

Resultado atualizado: **99 testes passaram**, `cargo fmt --all -- --check` e
`./scripts/cargo clippy --workspace --all-targets --locked -- -D warnings` passaram. Os testes foram
executados com `./scripts/cargo test --workspace --locked`. Isso complementa os 96 testes da entrega
delegada; não substitui a validação manual de drag dos painéis e divisórias, nem comprova menus
abertos na janela nativa. Nenhum commit ou push foi feito nesta revisão.
