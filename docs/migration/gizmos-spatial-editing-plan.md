# Plano de gizmos e edição espacial

Data: 2026-11-10. Projeto: `journey-studios/tessel-editor` (editor de vídeo Rust/GPUI).
Base inicial: `3e48861`. Base da próxima entrega conferida em 2026-11-10: `origin/main` em `308f1a2c1b79ffba8305032288cc04a71ecef820` (PR #40 integrada).

> **Nota histórica da fatia G1 (PR #40, preservada).** A cópia de consulta que acompanhou a PR #40
> declarava: "A branch `t3/scene-gizmo-rotation` entrega **somente** a primeira fatia de G1: **rotação de
> um nó** (anéis por eixo, ferramenta selecionável, orientação local/mundo, pivô na origem do nó e
> `scene.rotate` sem ponteiro), reutilizando o gesto de F055. **Não** entram: escala, snapping, seleção
> múltipla, grupos, luzes, câmera nem FX." A escala é a fatia G1.2 (última seção).

## Objetivo

Permitir editar diretamente no monitor os parâmetros espaciais das entidades e dos
efeitos. Cada recurso espacial deve chegar com representação selecionável, handles
adequados, edição numérica equivalente e regras compartilhadas no core.

Este documento complementa o [plano S1–S10](wave-s1-s10-plan.md), o
[plano de paridade](pilot-parity-execution-plan.md) e o contrato F055
(`docs/features/scene-gizmo.md` na main). Não substitui o contrato nem cria outra
fila: status, atribuição e evidências ficam no [backlog único](../backlog.md).
As fases organizam o escopo; a atualização ao final identifica o que foi entregue e o próximo recorte autorizado.

## Estado verificado

| Capacidade | Situação e limite |
|---|---|
| Translação X/Y/Z e plano da tela | Entregue na PR #25, merge `caeeeae9`; um nó por vez, com regras no core |
| Seleção e gesto de edição | Candidato, cancelamento, revisão obsoleta e um undo por arrasto implementados |
| Lib, headless e janela por comandos semânticos | Provas históricas conferidas na entrega S8; não são nova execução nesta conferência |
| Rotação de um nó, local/mundo | Entregue na PR #40; auxiliares semânticos validados, prova física continua pendente |
| Escala | Próxima implementação autorizada, BL015.1; contrato abaixo |
| Snapping, seleção múltipla e pivôs de grupo | Pendentes, BL015.1 |
| Ponteiro físico, foco, captura e resize na WKWebView | Sem aceite físico, BL015.2 / BL043 / BL044 |
| Luzes editáveis, sombras e seus marcadores | Domínio ainda pendente, BL023; não é somente adicionar um desenho de gizmo |
| Câmera | Já possui entrega de keyframes, F054; isso não prova frustum, alvo ou gizmo espacial completo |
| FX | Vinheta e fade/inspetor neutro integrados pela PR #39; P3 animado/film grain na PR #42, fora deste recorte |

A S8 registrou workspace 1562/0/12, runtime 175/175 e provas headless 56/56 e
janela semântica 84/84, em heads e manifestos descritos no contrato F055.
Esses números não validam antecipadamente nenhuma expansão deste plano.

## Qual gizmo cada recurso precisa

| Recurso | Representação e edição direta | Edição pelo inspetor |
|---|---|---|
| Planos, objetos e splats | Translação, anéis de rotação, escala; eixos locais ou de mundo | Transformação numérica equivalente |
| Luz pontual | Marcador selecionável, posição e esfera de alcance | Cor, intensidade e demais parâmetros suportados |
| Luz spot | Posição, direção/alvo e cone de abertura/alcance | Cor, intensidade, abertura e suavidade suportadas |
| Luz direcional | Orientação e indicação de direção; posição não deve sugerir efeito físico inexistente | Cor, intensidade e sombras quando implementadas |
| Luz ambiente/global | Indicador no editor, sem handle de posição sem significado | Cor e intensidade |
| Câmera | Posição, orientação/alvo, frustum e campo de visão | Mesmas propriedades e keyframes existentes |
| Efeitos espaciais, volumes e emissores | Centro, direção, dimensões, limites e trajetória conforme o domínio | Parâmetros correspondentes, sem controles fictícios |
| Efeitos de imagem, como vinheta | Handles 2D de centro e raio quando pertinentes | Intensidade, suavidade, cor e valores numéricos |
| Efeitos sem parâmetro espacial | Não precisam de gizmo 3D | Controles adequados no inspetor/timeline |

Um marcador que apenas identifica uma luz não substitui seu gizmo de edição.
Os auxiliares do editor não entram no frame de exportação nem no documento como geometria.

## Ordem de implementação proposta

| Fase | IDs existentes | Entrega revisável | Critério específico de aceite |
|---|---|---|---|
| G0 — consolidar a base | BL015, BL015.2, BL043, BL044 | Reconciliar F055 com o merge e conferir seleção, captura, cancelamento, teclado e resize | Provas semânticas reproduzíveis; prova física identificada separadamente e mantida pendente se não observada |
| G1 — transformações reutilizáveis | BL015.1 | Rotação e escala de um nó, seleção de ferramenta, orientação local/mundo e pivô definido | Arrasto e comando numérico produzem a mesma transformação; pai rotacionado/escalado e transformações não invertíveis têm resultado ou recusa explícita |
| G2 — luzes com edição visual | BL023, BL002, BL003, BL065 | Primeira fatia: luz pontual; depois spot e direcional, cada tipo com modelo, avaliação, desenho e handles próprios | Posição/direção/alcance alteram o resultado esperado; preview, render e export concordam com referência independente |
| G3 — câmera espacial | BL015.1, BL140 | Reutilizar F054 para alvo/orientação/frustum/FOV, distinguindo câmera editada da câmera de observação | Editar a câmera sem depender de navegar por ela; transformação, avaliação por frame e undo preservam as chaves existentes |
| G4 — efeitos com handles | BL028, BL082.1, BL024 | Vinheta com handles 2D como primeira fatia; domínios 3D ganham volumes/handles ao serem implementados | Centro/raio pelo monitor e pelo inspetor têm o mesmo valor e resultado; nenhum catálogo ainda ausente é habilitado |
| G5 — produtividade e grupos | BL015.1 | Snapping, seleção múltipla, pivô de seleção e transformação de grupo | Operação atômica, um undo por gesto, regras explícitas de hierarquia e de pivô |
| G6 — ampliar domínios | BL024, BL025, BL027 | Partículas, clima espacial, paths, rigs e integração com splats conforme seus contratos | Só anunciar suporte ao domínio que realmente desenha e responde ao gizmo; limitações de iluminação de splats explícitas |

G0 acompanha as demais fases; sua parte física não deve ser declarada automática.
G1 começa por rotação e depois escala. G2 pode definir o contrato de luz pontual em
paralelo ao desenho de G1, mas não deve duplicar a matemática nem os gestos do gizmo.
G4 deve preservar o inspetor/fade P2 já integrado pela PR #39 e o trabalho ativo de
FX animados/film grain (PR #42); não duplicar nem alterar esse domínio nesta entrega.
G5 não bloqueia entregar uma luz editável individualmente. G6 não autoriza portar
todos os domínios de uma só vez.

## Contrato técnico obrigatório

- O core define propriedades, unidades, transformações, picking, restrições e gestos.
  GPUI e runtime traduzem entrada e desenham o estado retornado; não mantêm regras paralelas.
- Reusar o ciclo begin/update/commit/cancel: updates são candidatos, commit produz um
  único undo e no-op não cria histórico. Entrada inválida preserva o último candidato legal.
- Identidade inclui projeto/cena, revisão, frame, seleção e época da página. Reload,
  troca de cena, undo externo, Escape, perda de captura e desaparecimento do monitor
  não podem aplicar um gesto antigo nem confirmar outro controle.
- Definir unidades e convenções antes de implementar: graus/radianos, espaço local/mundo,
  pivô, escala mínima, escala negativa e composição com pais. Preservar recusas atuais
  de transformações que o renderer, inclusive Spark, ainda não suporta.
- Definir como a edição interage com keyframes: propriedade base ou chave no frame atual.
  Não criar chaves, mudar interpolação ou sobrescrever animação silenciosamente.
- Preservar a política atual de ocultar o overlay na reprodução até decisão explícita
  em BL015.3. Evitar reconstruir planos por frame e isolar erros de overlay da sessão de render.
- Cada controle novo exige comando, parser, nó inspecionável e handler compartilhado.
  Permitir ligar/desligar auxiliares e escolher ferramenta sem depender de coordenadas físicas.
- Atualizar F055 e os contratos dos domínios afetados, matriz e IDs do backlog na mesma
  entrega. Conferir IDs de features antes de reservar; não inventar uma nova numeração aqui.

## Validação de cada entrega

1. **Biblioteca:** matemática e invariantes, pais transformados, erro atômico, cancelamento,
   no-op, gesto obsoleto, undo/redo e avaliação temporal quando aplicável.
2. **Headless real:** processo JSONL sem desktop, mesmos comandos e mesmo documento final.
3. **UI/runtime:** seleção, hit test, arrasto, teclado do adapter, resize, captura perdida,
   diálogo e reload; testes não substituem observação física da WKWebView.
4. **Imagem:** referência independente para o efeito de luz/FX; auxiliares ausentes no export,
   sem inferir iluminação de splats a partir de uma luz aplicada só a planos/meshes.
5. **Desempenho:** medir cena com auxiliares visíveis e ocultos, cena pequena e densa;
   reter todas as amostras. O benchmark antigo sem overlay não prova custo dos novos handles.
6. **Gates:** fmt, check, clippy com `-D warnings`, testes pertinentes/workspace, build e árvore
   do core sem GPUI/Wry, no head final ou com equivalência de produção demonstrada.
7. **Revisão:** nova revisão independente após implementação, findings tratados e evidências
   ligadas ao SHA. CodeRabbit NEUTRAL/SKIPPED não é aprovação.

Usar mbx com target padrão: nunca `CARGO_TARGET_DIR`, `--target-dir`, `build.target-dir`
ou wrapper antigo que os defina. Conferir o wrapper antes de usá-lo. Respeitar reserva de
16 GiB para iniciar lote pesado e piso de 12 GiB. Conforme autorização posterior do usuário,
o lock compartilhado de GPU/build é obrigatório **somente para medições de performance**;
implementação, builds e testes funcionais seguem sem essa trava. Não criar
artefatos/cache/builds em `/tmp` acima de aproximadamente 500 MB nem limpar caches alheios.

## Sequência de entregas

**G1: rotação de um nó + orientação local/mundo**, mantendo a translação existente,
seguida de escala em uma entrega delimitada. Próxima entrega de domínio: **luz pontual
com posição e alcance editáveis**, sem habilitar spot/direcional antes de implementá-las.

Antes de iniciar cada entrega, conferir main, PRs e ownership ativos. A implementação
usa Cursor Sonnet 5.5; a revisão independente é uma nova tarefa. O backlog registra
a atribuição efetiva. A próxima entrega não assume ownership do domínio de FX.

## Atualização G1 — 2026-11-10

Rotação de um nó em local/mundo integrada pela [PR #40](https://github.com/journey-studios/tessel-editor/pull/40), merge `308f1a2c1b79ffba8305032288cc04a71ecef820`. Biblioteca, headless e UI semântica validadas conforme backlog e contrato; ponteiro físico e desempenho integrado permanecem pendentes. G1 ainda não está integralmente concluída: próxima fatia é escala de um nó. Snapping, múltiplos/grupos, translação local e luzes não foram implementados por esta PR.


## Próxima entrega autorizada — G1.2: escala de um nó

Autorização do usuário em 2026-11-10: seguir com o plano e encaminhar ao Cursor Sonnet 5.5.
ID reutilizado: **BL015.1**; ampliar F055 sem reservar outra feature para a mesma ferramenta.

### Escopo funcional

- Ferramenta **Escalar** junto de Mover/Girar, três handles por eixo e um handle de
  escala uniforme. Estado e comandos inspecionáveis nas três superfícies.
- Preservar o formato existente `localScale`; a operação semântica (nome a fechar no
  contrato, por exemplo `scene.scale`) e o arrasto usam a mesma função do core.
- Pivô inicial: origem do próprio nó. Escala não deve deslocar sua posição nem criar
  shear, trocar sua rotação ou alterar a transformação do pai silenciosamente.
- Definir fatores multiplicativos relativos à base do press; fator 1 é no-op e a
  escala uniforme aplica o mesmo fator aos três componentes. Não acumular deltas por move.
- Escala por eixo local é obrigatória. Escala em mundo só é habilitada quando a
  transformação desejada puder ser representada no TRS existente, sem aproximação;
  combinação que produziria shear deve ter recusa tipada e motivo consultável.
- Definir limites numéricos no core antes do código: valores finitos, fator positivo,
  resultado não degenerado e limite máximo coerente com o documento. Não permitir
  atravessar zero e espelhar pelo arrasto. Preservar escala negativa preexistente se
  o domínio a aceitar; não anunciar criação de espelhamento nesta entrega.
- Manter as restrições do Spark: escala não uniforme de splat não vira suportada por
  existir um handle. Escala uniforme válida deve funcionar; recusas são visíveis.
- Não escrever chaves automaticamente. Escala animada ou composição de pai que impeça
  edição fiel exige recusa ou comportamento explícito no contrato, com teste.
- Reusar candidato/commit/cancel, um undo por gesto, erro sem escrita e preservação do
  último candidato válido. Não interferir em sliders de propriedade ou gestos de FX.
- Playback, frame/revisão/seleção/epoch antigos, perda de captura, reload, diálogo e
  troca de projeto mantêm as guardas já entregues em G1/S8.

### Entregáveis e aceite

1. API pública do core, JSONL sem janela, ferramenta/estado no desktop e desenho no
   canvas da WebView. Cada controle tem comando, parser, nó e handler compartilhado.
2. Testes com valores esperados independentes: uniformizar, eixo local com nó e pai
   rotacionados, pai com escala, no-op, limites, candidato inválido, undo/redo/cancel,
   gesto obsoleto e recusa de shear/Spark/animação pertinentes. Arrasto e comando
   sem ponteiro deixam documento equivalente.
3. Prova headless real e janela própria por comandos semânticos; Chrome cobre seleção,
   handles, captura e resize. Referência de imagem verifica transformação e ausência
   dos auxiliares no export. Não promover isso a prova física da WKWebView.
4. Gates no head final: fmt/check/clippy/workspace com runtime exigido/builds/core tree
   sem GPUI/Wry e testes JS pertinentes. Mutações discriminantes das regras novas,
   uma por vez com restauração conferida. Preservar todos os runs falhos ou abortados.
5. Custo de desenho/plano é uma validação separada, sob lock de performance. Não
   bloquear implementação ou testes funcionais por lock ocupado; registrar a medição
   pendente, sem usar resultados de rotação como prova do custo de escala.
6. Contrato F055, matriz e BL015.1 atualizados com SHAs e evidências. PR própria desta
   fatia e nova revisão independente; CodeRabbit NEUTRAL não é aceite.

### Ownership e fronteiras

Ownership: módulos de gizmo no core, overlay, adapter desktop do gizmo, parsers/nós
mínimos, testes e F055. Preservar FX animados/film grain da PR #42 e diagnóstico I/O
na PR #41; conferir heads atuais antes de editar arquivos compartilhados. O backlog
sujo da raiz é do coordenador e não pode ser substituído pelo de outra branch.

Fora desta PR: luzes, câmera espacial, snapping, seleção múltipla, grupos, edição de
keys, translação em eixos locais e correção geral da flaky de vídeo B2. Após escala,
a próxima entrega de domínio continua sendo luz pontual com posição e alcance.

## Atualização G1.2 — escala de um nó (implementação, a validar)

Branch `t3/gizmo-node-scaling`, base `308f1a2c1b79ffba8305032288cc04a71ecef820`. Implementada em F055
([seção "Escala de um nó"](../features/scene-gizmo.md#escala-de-um-nó-g12-bl0151)): ferramenta **Escalar**
(eixos `x`, `y`, `z` e `uniform`), orientação local/mundo, `scene.scale` sem ponteiro, o mesmo `ScaleFrame::scale` para
arrasto e comando. **Status: a validar** (implementação e testes funcionais no head; nova revisão independente, ponteiro
físico na WKWebView **não** foram feitos; o desempenho foi medido uma vez com ruído alto, sem conclusão, BL015.18). Decisões fechadas no contrato: escala em mundo só quando um
eixo do nó corre ao longo do eixo da cena (senão `world_axis_shear`, handle desabilitado); limites `[1e-3, 1e3]` por
componente; nunca atravessa zero nem espelha; splat só uniforme; `localScale` animada é recusada (`animated_scale`) sem criar chave.
Continuam fora: snapping, seleção múltipla, grupos, luzes, câmera espacial, edição de chaves, translação local, a flaky
de vídeo B2 e o custo de desenho da escala (uma medição ruidosa em BL015.18; sem página nem WKWebView).

## Atualização G2.1 e próxima entrega — 2026-11-11

- **G1.2 (escala de um nó)** integrada pela [PR #43](https://github.com/journey-studios/tessel-editor/pull/43), merge `cbe9e1b`.
- **G2.1 (luz pontual com posição e alcance)** integrada pela [PR #45](https://github.com/journey-studios/tessel-editor/pull/45),
  merge `9fc491b` (contrato F060, `scene-light.md`). Spot, direcional, ambiente, sombras e iluminação de splats seguem fora
  (BL023.x). Ponteiro físico, desempenho e aceite humano continuam pendentes e não foram declarados.
- **Próxima entrega: G4.1, handles 2D da vinheta** (centro, raio e suavidade no monitor), brief em
  [g4-vignette-handles-plan.md](g4-vignette-handles-plan.md). Escolhida por ser isolada, reusar o gesto de efeito (BL082.6) e o
  overlay existente e provar um alvo que não é nó 3D. Ficam depois, sem ordem fixa: G2.2 (spot/direcional), G3 (câmera
  espacial) e G5 (snapping, múltiplos, grupos).

## Atualização — escala integrada (2026-11-11)

Escala de um nó integrada pela [PR #43](https://github.com/journey-studios/tessel-editor/pull/43), merge `cbe9e1b393861febf073633fbbaa0da9857780b6`, por autorização explícita do usuário. Revisão sem achado de código acionável, threads resolvidas. Os gates completos das últimas correções continuam pendentes por disco; não confundir merge com validação integral. Benchmark de escala e prova física continuam pendentes. Próximo domínio planejado: luz pontual com gizmos de posição e alcance.

## Suíte de regressão de performance autorizada — 2026-11-11

O usuário pediu testes unitários de performance e validação do mesmo cenário de abrir
visualmente o app. Próxima tarefa: implementar a suíte, não apenas registrar pendências.
Reutilizar BL015.7/BL015.12/BL015.18 e relacionar os gates finais da escala ainda pendentes.

- Um corpus sintético compartilhado e uma sequência única: abrir projeto, anexar/carregar
  cena, aguardar quadro apresentado, selecionar, alternar ferramentas/espaços, mover/girar/
  escalar, atualizar candidato, confirmar/cancelar, undo/redo, trocar cena, resize/diálogo.
- Executar a mesma sequência pela lib real, pelo host GPUI real em teste e pela janela
  própria/WKWebView. Registrar quais etapas são equivalentes e quais exigem apresentação
  nativa. Mocks e resposta IPC não provam pintura; não declarar ponteiro físico observado.
- Testes unitários de custo estrutural por operação e de orçamento temporal no core,
  preservando ausência de I/O/render pesado síncrono no tick, memoização e isolamento do
  overlay. Testes de tempo controlados em release, com aquecimento, amostras completas e
  assertivas de orçamento; não tornar o CI funcional aleatoriamente sensível à carga.
- Cenários pequeno/denso, frio/quente, sem seleção/auxiliar oculto, translação/rotação/
  escala, pai transformado e splat onde suportado. Os testes devem executar os handlers
  reais; medir abertura fria separada de interação quente, sem esconder o primeiro frame.
- Baseline atual main `cbe9e1b` e candidato comparados com mesmo corpus/build/backend,
  ordem alternada e condições registradas. Isso detecta regressão desta entrega; para
  atribuir custo histórico dos gizmos comparar também origem anterior quando possível.
- Declarar métricas, orçamentos e regras de aceitação antes da medição do candidato,
  reutilizando limites já contratados. Mediana/p95/p99/máximo, amostras perdidas, ticks
  longos e erros preservados; nenhuma conclusão por melhor execução ou média isolada.
- Medição real de desenho/presentação e entrada até quadro apresentado na janela, além
  do benchmark headless/IPC. Scripts devem distinguir fase do core, host, runtime e
  compositor; evento semântico apresentado não vira prova física de latência ponta a ponta.
- Medições temporais precisam do lock de performance; implementação, compilação e testes
  funcionais não. Reserva 16 GiB/piso 12 mantida. Se sem recursos, entregar implementação
  revisável e registrar performance não validada, sem afirmar ausência de regressão.
- Mutantes que removem memoização/replanejam por tick/adicionam trabalho ao handler devem
  reprovar testes pertinentes; separar detector estrutural de detector de tempo. Nenhuma
  duração artificial pode substituir a execução do cenário real como prova de performance.
- Evidências por SHA/binário/corpus/build/hardware/backend/comando/cwd/env/UTC/exit e
  todas as amostras; revisão independente nova. Não prometer garantia universal com
  uma máquina. Otimizar somente gargalos medidos, com regressão funcional protegida.

**Entrega (F055, [scene-gizmo-performance.md](../features/scene-gizmo-performance.md), BL015.21):** a PR #44 entregou a
suíte **estrutural**: `crates/core/tests/gizmo_perf_support/`, `crates/core/tests/scene_gizmo_perf_structure.rs`,
`crates/core/tests/scene_gizmo_perf_threads.rs`, `crates/desktop/src/workstation/scene_gizmo_perf_tests.rs`,
`scripts/gizmo-perf/corpus.py` (e `hygiene.spec.py`) e o fixture (`docs/migration/fixtures/gizmo-perf/`). A pilha de tempo (`run.py`, A/B, mutantes, janela nativa, orçamentos) tem contrato em
[scene-gizmo-performance-timing.md](../features/scene-gizmo-performance-timing.md) (BL015.23) e **não** foi validada. O que já está provado e o que continua pendente (janela/WKWebView
física, compositor, BL015.2/43/44) fica no documento da feature e em `docs/backlog.md`.
