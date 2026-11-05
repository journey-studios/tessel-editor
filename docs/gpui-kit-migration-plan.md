# Migração da UI para GPUI Kit

Status: executado; ver `docs/gpui-kit-migration-report.md` para o resultado e as evidências.
Projeto alvo exclusivo: `/Users/wallyssonnunes/www/tessel-editor`.

## Resultado esperado

Substituir Guise e a autoria Tailor por GPUI Kit de verdade: bootstrap, temas,
controles e docking devem usar suas APIs. Preservar o editor demonstrativo,
um único monitor central, timeline customizada e controle integral pela CLI.
A troca não é um alias de crate nem uma camada que mantém Guise escondido.

Referência visual: a workstation Splice já documentada no plano de componentes.
Manter densidade de ferramenta desktop, cabeçalho unificado, controles nativos
macOS e as regiões Assets/Biblioteca, Monitor, Timeline e Inspector.

## Base existente e proteção contra regressões

- Rust 1.94.1; GPUI 0.2.2; Guise 1.9.1; Tailor 0.4.3 para o cabeçalho.
- Estado puro em `state/`, sem dependência de UI, e enum `Command` compartilhado.
- `workstation/` contém as regiões e microcomponentes reativos.
- CLI/socket local: tree/get/actions/status/set/move/invoke/theme, incluindo
  resize nativo (`set window size 960x640`). Socket privado, nenhum HTTP.
- 69 testes passaram no último baseline, incluindo 4 testes de eventos reais
  no test platform GPUI para o trim: arraste normal/curto, lado esquerdo, Escape.
- Bug que não pode voltar: o movimento que inicia o drag para a propagação;
  o drop de trim deve usar a posição final do ponteiro mesmo sem preview anterior.
- Snapshot anterior ao trabalho em `/tmp/tessel-editor-before-gpui-kit-20261104.tar.gz`.
  Não restaurar o workspace inteiro por cima de alterações novas.
- Repositório local inicializado sem primeiro commit; arquivos sem tracking são
  trabalho pré-existente, não lixo. Não limpar, resetar nem criar worktree.

## Compatibilidade verificada e fontes

Consultado em 2026-11-04:

- https://gpui-kit.com/docs/installation/
  Documenta `gpui-kit = "0.9.0"`, APIs reexportadas por `gpui_kit::*` e snapshot
  alinhado `gpui-pre = 0.3.11`. A aplicação normalmente não declara GPUI separado.
- https://gpui-kit.com/component/dock/
  DockSkin/DockArea, BasePanel + Panel, panel_handle, DockLayout e DockAreaState.
- https://gpui-kit.com/component/button/#sizeable
  `Button`, variants e `Sizable` para componentes compactos.
- https://gpui-kit.com/docs/getting-started/
- https://gpui-kit.com/docs/testing/
- https://gpui-kit.com/docs/webview/
  Integração experimental; gpui-fast e gpui-webview exigem features compatíveis.

Ler fonte/manifests da versão escolhida. Fixar versão compatível e Cargo.lock;
não misturar `gpui 0.2.2` com o snapshot novo. Se a versão publicada tiver um
bloqueio concreto, registrar o erro e usar somente release/revisão oficial
compatível, sem substituir a biblioteca solicitada por outra. Rust só deve ser
atualizado se uma dependência realmente exigir, com justificativa documentada.
Não editar os arquivos do Cargo registry como solução.

## Mapa de migração

| Frente | Mudança | Aceite |
|---|---|---|
| M01 Dependências | Adotar gpui-kit e suas features coerentes; retirar Guise e GPUI antigo de app/testes. | Resolver e compilar sem duas famílias incompatíveis de GPUI. |
| M02 Bootstrap | application/init/open_window, assets/fontes/ícones, Root e infra de overlays conforme Kit. | Janela real com texto, imagens, foco, menus e controles nativos. |
| M03 Tema | Trocar Theme Guise por ActiveTheme/tokens Kit; preservar dark/light e CLI. | Alterar tema na instância aberta muda controles e conteúdo. |
| M04 Cabeçalho | Reimplementar o cabeçalho gerado pelo Tailor com Kit e botões compactos. | Sem sobreposição em 960×640 e 1440×900; labels/tooltips acessíveis. |
| M05 Controles | Button/Toolbar, Input, Slider, Select, Tabs, Accordion/Collapsible, Tree, Tooltip e scrolls Kit; wrappers pequenos quando necessários. | Usar componentes Kit efetivos, com teclado, seleção e estado; não só copiar a aparência. |
| M06 Dock | Biblioteca, monitor único, timeline e inspetor como painéis reais de DockArea/DockSkin; divisórias redimensionáveis e drag/tab/splits. | Mover/encaixar painéis, redimensionar e restaurar layout padrão. Mesmo Dock, sem segunda árvore CLI divergente. |
| M07 Timeline | Portar desenho e eventos customizados para o GPUI do Kit; manter ruler, playhead, clips/thumbs, waveforms e handles de trim. | Move/trim/reorder, snap, limites/overlap e Escape preservados; não reinventar engine. |
| M08 Automação | Preservar IDs/contratos existentes e expor layout vivo do Dock. | CLI inspeciona/move/ativa/oculta/redimensiona painéis, consulta layout e restaura padrão. GUI e CLI compartilham operações. |
| M09 Testes/runtime | Portar os testes GPUI para o backend compatível com Kit; manter os testes puros. | Comandos, gestos e Dock têm provas comportamentais; build + UI reais. |
| M10 Limpeza/docs | Remover Tailor da cadeia ativa e Guise das dependências/imports; atualizar README/AGENTS/DESIGN e docs CLI. | Nenhum script ativo reintroduz Guise/Tailor; histórico do design pode ficar documentado/arquivado. |

## Ordem de execução

1. Registrar baseline e inspecionar a versão oficial; obter uma janela mínima Kit
   compilável no projeto. Não criar um app desconectado e parar nele.
2. Migrar bootstrap, imports e tema; portar a renderização e testes para a nova
   família GPUI, mantendo estado e comandos independentes.
3. Migrar microcomponentes para os componentes Kit com wrappers compactos.
4. Integrar Dock real com layout inicial: biblioteca esquerda, um monitor no
   centro, timeline abaixo e inspetor direita; status e cabeçalho continuam fixos.
5. Expor operações semânticas de Dock na CLI, sobre o DockArea vivo. Incluir
   inspecionar layout, mover painéis para grupos/posições, ativar, fechar/reabrir,
   redimensionar splits e resetar. Validar IDs, alvos e limites; retornar erros
   estruturados sem alterar layout no erro. Um dump/load JSON explícito é útil;
   não acrescentar armazenamento em disco automático sem necessidade.
6. Portar e validar timeline e input. Verificar trim rápido e cancelamento, além
   de sliders/selects/tabs/pastas. Confirmar não haver estado duplicado nos widgets.
7. Retirar a cadeia ativa Tailor e atualizar documentação, relatório e comandos.
   Não desinstalar Tailor do computador nem alterar configurações globais.
8. Validar no aplicativo real e deixá-lo aberto com layout padrão e valores limpos.

## WebView / Three.js — etapa separada

A arquitetura desejada é UI/docking Rust com uma futura viewport WebView capaz
de hospedar Three.js/splats. Nesta migração, separar a interface da viewport do
layout para permitir essa integração e documentar a decisão de backend padrão
ou gpui-fast. Não carregar engine do monorepo nem chamar provedores externos.

Não migrar o motor Three.js, exportação, processamento ou playback contínuo como
parte desta entrega. Documentar uma POC posterior com: página local determinística,
WebGL/Three.js, bridge Rust↔JS versionado, resize repetido, overlays sobre WebView,
medição de frame time e comunicação durante playback. Marcar tudo isso como não
validado enquanto não tiver sido executado; não confundir suporte de docs com prova.

## Limites

- Somente local; sem push, PR, merge, publicação, deploy ou alteração no tesselapp.
- Não remover os testes de regressão para facilitar o porte, nem desabilitar lint.
- Não usar computer use por coordenadas para operar a UI. CLI é o canal de ações;
  observação/captura visual nativa é permitida para verificar layout.
- A captura `screencapture` do shell pode falhar. A observação nativa do aplicativo
  via ferramenta cua funcionou nesta sessão; não afirmar indisponibilidade geral
  só por falha desse comando. Não contornar bloqueios explícitos de permissão.
- Sem playback/exportação reais; comandos ausentes continuam indisponíveis com motivo.
- Imagens públicas ProUI são demonstrativas; manter origem/licença sem presumir redistribuição.

## Validação e evidência final

- `cargo fmt --all -- --check`, `cargo check --workspace --locked`,
  `cargo clippy --workspace --all-targets --locked -- -D warnings`,
  `cargo test --workspace --locked`, `cargo build --locked -p tessel-editor`.
- Preservar os 69 casos comportamentais de baseline (ou correspondentes portados
  com justificativa), adicionando testes do Dock/CLI e dos controles alterados.
- Testar contra app aberto: IDs/valores da tree, tema, seleção, accordions, sliders,
  selects, move/trim de clipes e erro atômico, panels Dock move/resize/reset/layout.
- Conferir UI em 960×640 e 1440×900, menus abertos, sliders, monitor único, timeline
  e handles. Confirmar o estado CLI em controles renderizados, sem estado paralelo.
- Validar regeneração/build sem dependência Tailor, `cargo tree` e busca de imports
  comprovando que Guise e o GPUI antigo deixaram a cadeia ativa.
- Escrever `docs/gpui-kit-migration-report.md`: versões/backend exatos, mapeamento
  dos componentes, comandos Dock, testes executados e suas saídas resumidas,
  arquivos, limitações e status separado da POC WebView.
- Não terminar em plano ou biblioteca parcialmente trocada: completar a migração
  da UI e Dock até os critérios acima, ou apontar um bloqueio concreto comprovado.
