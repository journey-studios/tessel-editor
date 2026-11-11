# Tessel Editor

Projeto desktop independente em Rust + GPUI Kit (`gpui-kit` 0.9.0, GPUI `gpui-pre` 0.3.11).

## Limite atual

Escopo autorizado: reprodução visual da UI do editor Tessel existente em GPUI Kit (origem
`apps/scene-studio`, Three.js) e, desde a fundação da viewport, o fluxo de projeto/mídia/export listado abaixo.
Não portar o motor antigo inteiro nem integrações de serviço. Não copiar assets ou dados privados. Não modificar o repositório de origem como parte
desta preparação.

Projetos de conteúdo (autorais, privados ou com mídia pesada) ficam em `projects/<nome>/`, que o Git ignora
(`projects/README.md`). O repositório é público: nunca use `git add -f` nesse caminho nem copie esse conteúdo para
`docs/`, `crates/` ou commits.

A fundação da viewport (projeto/IDs/assets, plano racional, bridge v1, runtime Three.js local,
frame headless e WebView persistente no monitor) está entregue; ver `docs/features/viewport.md`
e `docs/viewport-foundation-report.md`. O escopo foi ampliado depois (aprovação do usuário) para
persistência, undo, jobs, import de mídia, pacote, intercâmbio, Spark, playback e export de vídeo:
status e lacunas em `docs/media-project-export-report.md`, contratos em `docs/features/` (F018, F019, F022–F030),
setup em `docs/media-setup.md`. Continua fora do escopo: o motor antigo inteiro, colaboração/cloud, importação de
formatos de outros editores. Qualquer novo controle precisa de `Command` + nó em `tree.rs` +
parser em `core::commands` + teste; frames reais exigem `./scripts/runtime setup`.

## Desenvolvimento

- Use Cargo neste workspace Rust, **pelo wrapper** `./scripts/cargo build|check|clippy|test ...`
  (cache compartilhado entre worktrees; ver [docs/cargo-worktrees.md](docs/cargo-worktrees.md)).
  Não adicione uma camada Node sem necessidade. Não rode `mbx clean`/`gc` como parte de tarefas de feature.
- A única dependência de UI é `gpui-kit` (que reexporta GPUI via `gpui_kit::*`). Não declare
  `gpui`, `gpui-component` nem Guise: misturar famílias de GPUI quebra a compilação.
  `test-support` do Kit existe só como dev-dependency.
- GPUI fica em `crates/desktop`; regras de domínio ficam em `crates/core` e funcionam sem GPUI.
- Crie novos crates somente quando existir uma necessidade concreta.
- Não adicione serviços, credenciais, telemetria ou dependências do monorepo nesta etapa.
- Preserve `Cargo.lock` e atualize versões de forma intencional. Rust 1.99.1 é o pin do
  build validado com `gpui-pre`; 1.94.1 falhou por `std::hint::cold_path`.
  1.99.1 é o pin testado, não uma afirmação de versão mínima; não rebaixe sem validar.
- Valide com `cargo fmt --all -- --check`, `./scripts/cargo check --workspace --locked`,
  `./scripts/cargo clippy --workspace --all-targets --locked -- -D warnings`,
  `./scripts/cargo test --workspace --locked` e `./scripts/cargo build -p tessel-editor --locked`.
- Testes Rust seguem a convenção nativa; o macro `#[gpui_kit::test]` dos testes GPUI sombreia
  `#[test]` com `use gpui_kit::*`: nos módulos de teste, importe `core::prelude::v1::test`.
- Nunca inclua referências a assistentes em commits. Use a skill `commit` antes de commitar.
- **Lock de GPU/build somente para desempenho** (regra do usuário, 2026-11-10; substitui instruções anteriores de scripts e docs):
  `/tmp/tessel-scene-gpu.lock` (`with-gpu-lock.py`) só é exigido por validações de **desempenho** (latência, FPS, custo, benchmarks). Edição,
  `cargo fmt/check/clippy/test/build`, o runtime Chrome funcional e as provas funcionais não esperam por ele nem o tomam. Não apague o lock nem mate
  outro processo; não inicie medição concorrente. Se o Chrome precisar de isolamento funcional use instância, socket e `HOME` próprios. Um script que
  mistura resultado funcional e de desempenho deve separá-los: sem o lock, o desempenho não é declarado aceito.

## UI

- Use componentes do Kit (`Button`, `Toolbar`, `TabBar`, `Slider`, `Select`, `Input`, `Tree`,
  `Accordion`, `Switch`, scroll) e seus ícones (`gpui_kit::assets::IconName`). O `Tab` do Kit
  mostra ícone **ou** rótulo, nunca os dois.
- Layout em painéis do `DockArea`/`DockSkin` (`workstation/dock.rs`): biblioteca, monitor (um
  só), timeline, inspetor. O `DockArea` vivo é o único dono do layout; a CLI o lê e edita, sem
  árvore paralela. Cabeçalho e status ficam fora do Dock.
- Tema: `ActiveTheme`/`Theme` do Kit (`theme.rs`); tokens de workstation em `workstation/tokens.rs`
  derivam dele. Nunca hex solto.
- Todo gesto vira um `Command` (ou `LayoutCommand`) e passa por `Workstation::execute*`.
- Controles de funcionalidades ausentes ficam desabilitados com `disabled_reason`; nunca com callback
  vazio. Um controle só é habilitado quando executa a operação real (Abrir, Salvar, Importar, Exportar, Empacotar,
  Tarefas, Desfazer, Refazer, Play/Pause estão ligados a `workstation/workflow.rs` e `playback.rs`).
- Diálogos são `Root` dialogs do Kit com corpo em `Entity` própria (`FlowView`, `JobsView`). A WebView nativa pinta
  acima de qualquer elemento GPUI: **todo diálogo novo precisa marcar `Workflow::has_dialog`** para o monitor
  escondê-la. Seletor nativo de arquivos é só uma entrada de caminhos; tudo é acessível por `flow.*`/`dialog.*`.
- Controles da janela (fechar/minimizar/maximizar) são nativos e ficam integrados no cabeçalho.
- ProUI é referência adicional; não é dependência desta implementação.
- Tailor foi retirado da cadeia ativa (sem `src/ui` gerado, sem script de export, sem MCP de
  projeto). `docs/archive/editor.tailor.txt` é só histórico.

## UI automation requirement

- Interact through `./scripts/ui` instead of coordinate-based computer use.
- Every UI feature exposes inspectable live state and semantic commands (`tree`, `get`, `set`, `move`, `invoke`, `dock`).
- New controls: add the `Command`, its node in `state/tree.rs`, its parser in `automation/commands.rs`, and tests.
- CLI and GUI actions must share application handlers; never fake a separate CLI state.
- Local socket IPC is authorized infrastructure; do not expose it over the network.
- See `docs/ui-automation.md` for the contract and validation boundaries.
- Observação visual nativa é permitida (`screencapture -l <windowid>`); não opere a UI por cliques de coordenadas.

## Escopo atualizado — microcomponentes interativos

A pedido do usuário, os componentes de `docs/component-implementation-plan.md` têm estado
demonstrativo em memória: pastas/accordions, sliders/selects, tabs e drag and drop de
tracks/clipes. Isto não autoriza migração do motor, processamento de mídia, backend ou
publicação. Manter um único monitor central e toda interação acessível pela CLI com handlers
compartilhados.

## Migração para GPUI Kit — concluída

`docs/gpui-kit-migration-plan.md` foi executado; evidências, versões, comandos de Dock e limites
em `docs/gpui-kit-migration-report.md`. As regras históricas de Guise/Tailor deixaram de valer.

## Contrato obrigatório de features e core

Antes de implementar ou alterar qualquer feature, leia a skill local
[`.agents/skills/tessel-feature-contract/SKILL.md`](.agents/skills/tessel-feature-contract/SKILL.md).
Ela se aplica a todo trabalho de feature neste repositório, inclusive quando não for invocada pelo nome.

- Toda feature exige uma linha em [docs/features/README.md](docs/features/README.md) e um
  documento `.md` próprio em `docs/features/`, atualizados na mesma entrega.
- Biblioteca, CLI sem janela e UI consomem as mesmas regras em `tessel-editor-core`.
  O core monta e controla o sistema de domínio independentemente da forma de acesso.
- Regra de negócio, processamento e validação pertencem ao core; adapters só traduzem
  entradas/apresentam resultados. Não depender de GPUI, janela ou socket para editar.
- Uma feature de edição só está completa com fluxo pela lib, CLI headless e UI e evidência
  em cada superfície. Não marcar socket da janela como headless. Gaps existentes ficam
  registrados, sem inventar paridade. Capacidades de apresentação usam N/A justificado.
- `./scripts/headless` executa uma sessão JSONL sem desktop. `./scripts/ui` controla a janela.
- Processamento pesado (FFmpeg, hash, ZIP, render) vive em `crates/media` como adapters injetados no
  `Extension` do core; o core só tem regras, planos, presets e o registro de jobs. `cargo tree -p tessel-editor-core`
  não pode listar gpui/wry. Backend de prova: `scripts/live-flow-proof.py` (janela real, só requisições semânticas).
- Contrato e limitações atuais: [docs/core-architecture.md](docs/core-architecture.md).

## Migração por etapas — rodada paralela autorizada

O usuário autorizou a implementação da base descrita em
`docs/migration/parallel-implementation-plan.md`, com agentes com ownership exclusivo de arquivos (limitação local do T3 registrada no plano).
Esta rodada cobre documento/grafo mínimo, corpus de equivalência e robustez da viewport.
Não autoriza portar famílias visuais inteiras nem modificar a origem.
Commits locais das entregas estão autorizados; nesta rodada somente o coordenador
faz staging/commits e integração.

## Backlog único obrigatório para todos os agentes

- Leia e atualize [docs/backlog.md](docs/backlog.md) antes de iniciar e ao concluir uma tarefa.
- Toda pendência encontrada (feature, bug, dívida, decisão, paridade ou validação) recebe um ID
  nesse arquivo, com status, responsável, dependências, aceite e link para os detalhes.
- Não mantenha filas de execução independentes em relatórios. Docs por feature mantêm contratos
  e evidências; `docs/backlog.md` centraliza status e atribuição. Reutilize IDs existentes.
- Marque em andamento somente tarefas efetivamente iniciadas; registre commit/PR e evidências
  antes de concluir. Uma implementação sem prova permanece a validar.
- Em paralelo, cada agente entrega as atualizações de suas linhas ao coordenador, que integra
  o arquivo comum sem apagar alterações de outros agentes. Nenhuma tarefa é atribuída só por
  estar descrita no plano; o coordenador registra link da delegação e ownership de arquivos.
