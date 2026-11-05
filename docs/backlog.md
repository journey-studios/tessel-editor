# Backlog compartilhado do Tessel Editor

Registro único de execução das pendências: building blocks, migração, produto, decisões e provas.
Atualizado em 2026-11-05, após o merge `6cc4392` (PR #1). A rodada A–G terminou;
A rodada H/I/J/K foi iniciada com quatro agentes Cursor Sonnet 5.5; atribuições e links abaixo. Responsável `—` significa não atribuído.

## Como todo agente deve usar

1. Ler este arquivo antes de começar. Reutilizar o ID existente; não criar uma segunda lista de status.
2. Antes de implementar, registrar responsável/link da tarefa, escopo, dependências e arquivos de ownership.
   Só marcar **Em andamento** quando houver execução iniciada, nunca apenas por existir um plano.
3. Qualquer bug, limitação, falta de paridade ou prova ausente encontrada entra aqui na mesma entrega,
   com referência ao documento/código. Dividir itens grandes antes de delegar; preservar o ID pai.
4. Atualizar o item ao encerrar: implementação, superfícies lib/headless/UI, testes executados,
   commit/PR e limites restantes. Implementado sem prova fica **A validar**, não concluído.
5. Em trabalho paralelo, cada agente fornece ao coordenador o patch das linhas que possui.
   O coordenador reconcilia este arquivo; não sobrescrever atualizações dos outros agentes.
6. Docs de features continuam donos do contrato e das evidências; inventário MIG continua dono
   da classificação da origem. Aqui fica o estado de execução e seus links. Relatórios antigos
   são históricos, não autoridade para reabrir trabalho já entregue.

Estados: **Pendente**, **Parcial**, **A validar**, **Decisão**, **Bloqueado (hardware)**,
**Em andamento**, **Concluído**, **Descartado**. Sem prazo ou prioridade implica backlog,
não autorização automática de escopo. Cada item herda a fonte indicada no grupo.

## Base entregue — não reimplementar

- BB01/BB02: import/round-trip do subconjunto de bundle 1.0, IDs/referências/grafo e matrizes.
- BB03: câmera/tempo racional e canais de nós `cycle`/`event`/`static`, com oráculo.
- BB04 mínimo: 8 propriedades, lote/dry-run e gates compartilhados; G conecta projeto,
  histórico, save/reopen e CLI headless. UI visual continua pendente.
- BB10 mínimo: 55 bundles, funções reais da origem e consumidores Rust de grafo/câmera/animação.
- Preview atual: retenção, coalescing, proxy/refino, resultado Spark parado e repaint seletivo.
  Lifecycle com proxy/context loss/hide/show foi provado pela frente C; não reabrir essa lacuna.
- B1–B8 tratados; as limitações abaixo são distintas dos bugs corrigidos.
- Revisão da base: 526 testes Rust passaram, 2 ZIP64 ignorados; 3 testes Python passaram.
  CodeRabbit configurado; revisão incremental completa encontrou limite de cota gratuita.

## Próxima base compartilhada

Fontes: [roadmap](migration/phased-roadmap.md), [matriz](features/README.md),
[cena no projeto](features/scene-project.md), [edição](features/scene-edit.md),
[animação](features/scene-animation.md), [tempo](features/scene-temporal.md).

| ID | Bloco | Pendência | Estado | Depende de | Responsável | Critério de aceite |
|---|---|---|---|---|---|---|
| BL001 | BB01 | Completar validações adiadas conforme cada domínio migrar (`DEFERRED_RULES/FIELDS`) | Parcial | Domínio escolhido | — | Erros/limites explícitos e oráculo por regra; preservar desconhecidos |
| BL002 | BB02 | Ampliar entidades/grafo para luzes, rig, paths e demais domínios | Parcial | BL001/domínio | — | Referências e transformações verificadas sem GPUI |
| BL003 | BB03 | Avaliar domínios ainda em `unevaluated`: luz, clima, efeitos, rig, paths, bind/reveal | Parcial | Domínio escolhido | — | Estado por frame equivalente; nada omitido como se estivesse avaliado |
| BL004 | BB04 | Ampliar catálogo: componentes, renderOrder, keys/segmentos e propriedades dos novos domínios | Parcial | BL001–BL003 aplicáveis | — | Tipo/unidade/faixa/default compartilhados; rollback e undo por lote |
| BL005 | BB04 | Compatibilizar escrita de pose inicial com a conversão de movimentos/keys da origem | Pendente | BL004 | — | Semântica escolhida documentada e comparada com a origem |
| BL006 | BB05 | Definir plano avaliado de cena e contrato core → runtime, versão/capabilities | A validar | BB02/BB03 existentes | H / F037 ([relatório](migration/foundation-h-report.md)) | 29 testes públicos, doctest e clippy lib passaram. Renderer BL007 e recursos BL009 continuam pendentes; gate completo da rodada ainda será executado |
| BL007 | BB05 | Renderer da cena dirigida em preview e headless; geração/cancelamento/invalidação | Pendente | BL006, BL010 | — | Mesmo conteúdo em seek fora de ordem, troca de projeto e recuperação de contexto |
| BL008 | BB05 | Exportar cenas dirigidas pelo mesmo plano e original | Pendente | BL007 | — | Comparação de frames preview/headless/export e export curto reproduzível |
| BL009 | BB06 | Resolver dependências transitivas: imagens/mapas/splats/fontes e base de caminhos | Em andamento | Modelo BB01/BB02 | I / Cursor Sonnet 5.5 (ver rodada) | Closure de recursos explícita, validação de caminhos e diagnóstico de ausentes |
| BL010 | BB06 | Jobs/load/cache/release por geração e dependentes da cena | Em andamento | BL009 | I / Cursor Sonnet 5.5 (ver rodada) | Troca/release invalida dependentes; orçamento de memória e cancelamento |
| BL011 | BB06 | Save/package/restore/relink/interchange de recursos de cena | Em andamento | BL009, BL010 | I / Cursor Sonnet 5.5 (ver rodada) | Pacote abre sem arquivos originais; só então retirar a guarda de pacote incompleto |
| BL012 | BB07 | Seleção por entidade, árvore de cena e tool state | A validar | BB02, BL004 | J / F039 ([relatório](migration/foundation-j-report.md)) | 21 testes públicos e unidade core passaram; executável headless real (13 requests) validou queries/seleção/gestos/undo; UI/socket e gesto físico pendentes |
| BL013 | BB07 | Gestos com preview/commit/cancel e histórico agrupado | A validar | BL012 | J / F039 ([relatório](migration/foundation-j-report.md)) | 21 testes públicos e unidade core passaram; executável headless real (13 requests) validou queries/seleção/gestos/undo; UI/socket e gesto físico pendentes |
| BL014 | BB08 | Inspector ligado ao catálogo e às propriedades vivas | Pendente | BL004, BL012 | — | UI/CLI/lib compartilham comandos e valores; controle indisponível explica motivo |
| BL015 | BB08 | Overlays/handles: coordenadas, foco, hit testing e resize sobre WebView | Pendente | BL007, BL013, BL014, BL042 | — | Handle e comando dão mesmo resultado; validar foco/resize nativos |
| BL016 | BB09 | Transporte/mix do modelo dirigido: shots, janela de edição e relógio | Pendente | BL006, BL008 | — | Frame e amostra correspondentes em play/seek/pause; offline sem dispositivo |
| BL017 | BB09 | Narração/takes/legendas e monitor gain | Pendente | BL016, BL026 | — | Tempo, texto e mix persistem e coincidem na saída |
| BL018 | BB10 | Expandir corpus: tolerâncias JOIN, FPS variados e matrizes mundo | Parcial | BB10 existente | K / F032 ([relatório](migration/foundation-k-report.md)) | 47 casos estendidos + 55 base verificados contra origem; 45 testes Node e 15 testes Rust passaram. Matriz da câmera usa Three isolado; limite do runtime em BL071 |
| BL019 | BB10 | Provas de plano/pixels/áudio/performance por novo domínio | Parcial | Cada entrega | — | Código final e proveniência; não usar resultado de protótipo como aceite |
| BL020 | Adapters | Expor avaliação completa de nós por frame no acesso programático de cena | A validar | BB03 existente | J / F039 ([relatório](migration/foundation-j-report.md)) | 21 testes públicos e unidade core passaram; executável headless real (13 requests) validou queries/seleção/gestos/undo; UI/socket e gesto físico pendentes |
| BL021 | Adapters | Provar `scene.*` pelo socket da janela e depois pela UI visual | A validar | BL012/BL014 para UI | — | Socket e UI têm evidências distintas; headless já está validado |

Primeiro marco pendente: cena mínima → plano → monitor/headless → pacote sem origem →
export curto. BB05 e BB06 podem avançar em paralelo após acordar o contrato de recursos;
BB07/BB08 podem preparar modelos/controles, mas o aceite visual depende do renderer.

## Fases funcionais posteriores

Fonte: [M1–M8 e MIGs](migration/phased-roadmap.md),
[matriz MIG001–MIG121](migration/feature-migration-matrix.md).
Os itens desta tabela são agregadores: abrir subitens com MIGs exatos antes de implementar.
MIG marcado AUS/PLAN/“não migrar” não vira obrigação por aparecer no inventário.

| ID | Fase | Pendência | Estado | Depende de | Responsável | Critério de aceite |
|---|---|---|---|---|---|---|
| BL022 | M1 | Completar cenas/direção de câmera: shots, receitas, segmentos, keys/easing e controles | Parcial | BL004–BL016 | — | Produção mínima editável/renderizável pelas três superfícies |
| BL023 | M2 | Luzes, sombras, mapas e marcadores | Pendente | BB02–BB08/BB10 | — | Parâmetros e resultado equivalentes, com preview/export |
| BL024 | M3 | Clima, partículas, grade e pós-produção | Pendente | Base e iluminação aplicável | — | Parâmetros animados no core; desenho no runtime e provas por efeito |
| BL025 | M4 | Integração de splats com recursos/luz/clima de cena | Pendente | BL023/BL024 aplicáveis | — | Preservar câmera e composição; não reimplementar Spark básico |
| BL026 | M5 | Takes, narração, legendas e fluxo de edição/mix | Pendente | BB03/BB04/BB06/BB09 | — | Fluxo programático e visual; TTS só se autorizado em BL037 |
| BL027 | M6 | Rig, IK, paths e follow | Pendente | Base, BL039 | — | Pose/tempo comparados à origem e solver escolhido explícito |
| BL028 | M7 | Efeitos/transições e edição de shots/gestos do piloto | Decisão | BL036 | — | Escopo/snapshot congelado; reabrir código além do catálogo antes de portar |
| BL029 | M8 | i18n, acessibilidade, atalhos e organização visual | Pendente | BL040 e features correspondentes | — | Cobertura de fluxos/teclado e idiomas escolhidos; sem alegar paridade só por testes de comandos |
| BL030 | Fechamento | Compatibilidade e regressão do conjunto, matriz MIG/F reconciliada | Pendente | Fases escolhidas | — | Cada MIG tem destino, exclusão justificada ou aceite com evidência |

## Provas e otimizações ainda abertas

Fontes: [detalhamento OPT](viewport-performance-backlog.md),
[execução OPT](viewport-performance-next-report.md), [vídeo](viewport-bugfix-report.md),
[lifecycle](viewport-lifecycle-report.md). A tabela OPT permanece como histórico técnico;
novas atribuições e mudanças de estado ficam neste arquivo.

| ID | Origem | Pendência | Estado | Depende de | Responsável | Critério de aceite |
|---|---|---|---|---|---|---|
| BL031 | OPT07 | LOD/sort de Spark em movimento; MSAA já tem opt-in | Pendente | Experimento justificado | — | Ganho sustentado com qualidade/budget explícitos, sem promessa universal de 1–4 ms |
| BL032 | OPT08 | Seek-to-photon físico | Bloqueado (hardware) | Câmera rápida/procedimento humano | — | Amostras e resolução temporal; rAF/hash não substituem fóton |
| BL033 | OPT09 | Determinismo 3M e export UHD/SwiftShader não executados | A validar | Budget de GPU/disco/tempo | — | Pixels, cancelamento e recursos registrados; falha de ambiente explícita |
| BL034 | OPT10 | Série longa, displays externos 120/240 Hz e carga concorrente | Bloqueado (hardware) | Displays/ambiente | — | Release, repetições, p50/p95/p99 e memória estável |
| BL035 | Vídeo/provas | CFR/PTS declarados, vídeo estático recusado, leitura com janela oculta e timeout de seek | A validar | Harness de vídeo | — | Reproduzir cada limite atual antes de corrigir; erro tipado e quadro correto |
| BL043 | B6 | Reproduzir perda física de MouseUp no macOS | A validar | Input nativo | — | Gesto real; teste injetado já passou e não equivale a ponteiro físico |
| BL044 | Lifecycle | Popover/tooltip/foco/atalhos sobre a WebView | A validar | BL042, BL015 | — | Abrir overlay e navegar por teclado; provar composição/foco nativos |
| BL045 | Performance | Investigar falha da bancada Chrome e medir recursos completos do WebKit/Node/Chrome | A validar | Bancada reproduzível | — | Causa/logs e medições CPU/GPU/filhos; não reportar não medido como zero |

## Decisões de arquitetura/escopo

Fonte: [D1–D11](migration/README.md#decisões-que-precisam-ser-tomadas).
D2/D3 já fundamentam o import/round-trip e o tipo DirectedScene; não pedir a mesma decisão de novo.
D4 deve ser respeitada no contrato BL006: regras no core, execução visual no runtime.

| ID | Decisão | Estado | Responsável | Saída necessária |
|---|---|---|---|---|
| BL036 | D1: quais itens do piloto e qual snapshot entram | Decisão | — | Lista explícita de MIG100–MIG105 selecionados; não confundir merge do piloto na origem com migração |
| BL037 | D5: permitir adapter de TTS HTTP local | Decisão | — | Escopo autorizado; serviços continuam proibidos enquanto não houver decisão |
| BL038 | D6: manter pipelines ML fora do app | Decisão | — | Registrar formatos aceitos e limite de responsabilidade; não incorporar pipeline por acidente |
| BL039 | D7: solver IK JS ou Rust | Decisão | — | Escolha com tolerâncias e execução headless; não portar alterando pose silenciosamente |
| BL040 | D9: idiomas e política de i18n | Decisão | — | Idiomas escolhidos e contrato de chaves |
| BL041 | D10: multi-projeto/abas | Decisão | — | Confirmar manter projeto único ou definir escopo de abas e bloqueio de escrita |
| BL042 | D11: overlays no runtime e eventos semânticos | Decisão | — | Contrato de desenho/hit testing/eventos, com core validando toda mutação |

## Produto/mídia e manutenção — revalidar lacunas históricas

Fontes: [mídia §7](media-project-export-report.md#7-lacunas-explícitas),
[export](features/video-export.md), [arquitetura](core-architecture.md).
**A validar** significa que o relato histórico precisa ser confrontado com o código atual;
não é uma afirmação de bug presente. Não reabrir B1–B8, proxy ou lifecycle já entregues.

| ID | Área | Pendência | Estado | Responsável | Critério de aceite |
|---|---|---|---|---|---|
| BL046 | Áudio | Saída de baixa latência, início real e sincronismo A/V ouvido | A validar | — | Medição do dispositivo e escuta humana; processo afplay vivo não prova áudio ouvido |
| BL047 | Package | ZIP64 >4 GiB gravado em disco e extraído externamente | A validar | — | Budget de disco seguro; arquivo real, integridade e extração por ferramenta externa |
| BL048 | Export | Áudio embutido em vídeo no mix exportado | A validar | — | Conferir aviso video_audio_not_mixed atual e provar mix por amostras/escuta |
| BL049 | Import | VFR/FPS misto e PTS deslocado, conformação e políticas | A validar | — | Distinguir detecção FFmpeg já existente de suporte ao fluxo completo e ao caminho nativo |
| BL050 | UI/jobs | Tirar trabalho pesado síncrono de prepare/verify/restore/preflight da thread UI | A validar | — | Confirmar caminho atual; janela responsiva, cancelamento e progresso sem regras duplicadas |
| BL051 | UI | Seletor visual de sequência | A validar | — | Reusar sequence.activate; seleção e estado persistentes coerentes |
| BL052 | Export | D8: contrato de cor BT709/transferência/faixa | A validar | — | Conferir código/ffprobe e pixels; adotar/verificar marcação escolhida |
| BL053 | Texto | Fontes reais, métricas e redistribuição/empacotamento | A validar | — | Confrontar fonte bitmap atual; fontes transportáveis e licença documentada |
| BL054 | Portabilidade | Linux/Windows/outras GPUs e codecs | A validar | — | Builds e fluxos reais por plataforma; formatos indisponíveis falham explicitamente |
| BL055 | Distribuição | Empacotamento/licença do FFmpeg | Decisão | — | Política de distribuição e backend/ferramentas detectáveis, sem presumir licença |
| BL056 | Export | Retomada, granularidade de cancelamento, HDR/alpha e formatos extras | Decisão | — | Separar subitens aprovados; não prometer capacidade ainda ausente |
| BL057 | Documento | Migração de versão de presets e compatibilidade futura | A validar | — | Separar versões de preset de schema 1/2 do projeto já entregue |
| BL058 | Qualidade | Revisão humana visual/áudio dos exports finais | A validar | — | Aceite humano separado de ffprobe/hash/testes técnicos |
| BL059 | Core | IDs internados não liberados | A validar | — | Medir crescimento; política de ciclo de vida sem invalidar IDs/snapshots existentes |
| BL060 | Tooling/disco | Resíduos de testes/Puppeteer e teto/coleta mbx coordenada | A validar | — | Tempdirs limpos inclusive em falha; builds não perdem target; não executar gc/clean às cegas |
| BL061 | Documentação | Reconciliar textos antigos A–E, índices e README com F/G | Pendente | — | Remover promessas de adapters pendentes já entregues e de app apenas demonstrativo; preservar relatórios históricos rotulados |

## Referências de execução

- [Matriz atual de features](features/README.md): estado por UI, socket, headless e lib.
- [Inventário completo](migration/source-feature-inventory.md) e [matriz MIG](migration/feature-migration-matrix.md): 121 itens, incluindo excluídos/ausentes, sem converter todos em trabalho.
- [Rodadas A–G](migration/parallel-implementation-plan.md): ownership e entregas anteriores.
- [Roadmap](migration/phased-roadmap.md): dependências E0–E3/M1–M8; não é uma fila de agentes ativos.

Não reabrir como pendência: prefetch genérico, trocar Three.js/Spark, grafo incremental ou
reuso isolado de objetos sem hipótese nova. Experimentos refutados ficam descartados nos relatórios OPT.

## Rodada atual

Plano e ownership: [H/I/J/K](migration/next-round-plan.md). H/J/K foram recebidas e passaram nos gates direcionados registrados acima; a entrega de I aguarda recebimento. L foi adicionada para BL062 e está concluída. Cargo e integração final são do coordenador.

| Frente | Itens | Responsável/tarefa | Estado |
|---|---|---|---|
| H | BL006 | <t3-thread://v2/8ac99a47-6712-4c08-aa4a-d8e73a792f19/thread:delegated-task:command%3Amcp%3Ad3856989-26f9-4933-b607-9945895abf30%3Adelegate-task%3Aa6ff05b3-f726-4031-ab83-3f9cc3045ebf> | Recebido — gates direcionados passaram, integração total pendente |
| I | BL009/BL010/BL011 | <t3-thread://v2/8ac99a47-6712-4c08-aa4a-d8e73a792f19/thread:delegated-task:command%3Amcp%3Ad3856989-26f9-4933-b607-9945895abf30%3Adelegate-task%3A9cb28c3e-6589-4f90-840f-b43f21c534ca> | Em andamento |
| J | BL012/BL013/BL020 | <t3-thread://v2/8ac99a47-6712-4c08-aa4a-d8e73a792f19/thread:delegated-task:command%3Amcp%3Ad3856989-26f9-4933-b607-9945895abf30%3Adelegate-task%3A975cf39a-05f7-40c5-a224-811495f83404> | Recebido — gates direcionados passaram, integração total pendente |
| K | BL018 | <t3-thread://v2/8ac99a47-6712-4c08-aa4a-d8e73a792f19/thread:delegated-task:command%3Amcp%3Ad3856989-26f9-4933-b607-9945895abf30%3Adelegate-task%3A72230e00-998b-473e-aa85-12aacc2a00d7> | Recebido — Node e 15 testes Rust validados |

Ambiente: checkout compartilhado por limitação da delegação local; ownership exclusivo no plano. Nenhum agente altera o projeto aberto do usuário. O app foi reconstruído e aberto em release antes da integração desta rodada.

## Correção priorizada durante a rodada — sliders

| ID | Área | Pendência | Estado | Depende de | Responsável | Critério de aceite |
|---|---|---|---|---|---|---|
| BL062 | F005/F024 | Agrupar sliders por gesto com preview ao vivo e commit único | Concluído | — | L + coordenador ([relatório](slider-gesture-history-report.md)) | 28 core + 13 GPUI + CLI real + 12 checks socket/janela isolada passaram; regressão desktop 108/108; clippy/fmt/release OK. App do usuário atualizado com snapshot preservado. Mouse físico permanece BL043 |

Tarefa L: <t3-thread://v2/8ac99a47-6712-4c08-aa4a-d8e73a792f19/thread:delegated-task:command%3Amcp%3Ad3856989-26f9-4933-b607-9945895abf30%3Adelegate-task%3A6b408492-699c-46af-b4f2-06a6933bd3dc>

Ownership: L possui state/history.rs; J mantém scene_project e ops.rs. Em state/mod.rs,
J adiciona apenas lifecycle/campo de cena; L altera Command/execute/apply e seu campo de gesto,
com patches pontuais para preservar trabalho concorrente. Parser de headless e exports novos
serão integrados pelo coordenador. A janela do usuário não é reiniciada durante a investigação.

## Pendências adicionais identificadas por H

| ID | Área | Pendência | Estado | Depende de | Responsável | Critério de aceite |
|---|---|---|---|---|---|---|
| BL063 | BB05/Adapters | Expor scene.plan por lib/JSONL e depois UI; integração plano × manifesto | Pendente | BL006, BL009 | — | Mesmo parser/erros compartilhados; required_asset_ids do plano contidos no manifesto; domínios recusados explícitos |
| BL064 | BB05/performance | Avaliação de muitos frames sem reconstruir NodeTimeline; medir tamanho/custo do plano | Pendente | BL006, consumidor BL007/BL008 | — | Hipótese medida antes de otimizar; mesma saída e budget explícito no export curto |
| BL065 | BB05/domínios | Expandir plano aos domínios recusados e ao desempate por profundidade | Pendente | BL003, BL023–BL027 | — | Capabilities/versão e estado completo por domínio, com provas; não remover unsupported sem implementação |

H recebida: plano não executa gates de admissão temporal/estrutural; o adapter consumidor deve
executá-los antes de habilitar render. Unsupported inclui mapas de dados e splat dirigido,
independentemente do Spark básico já entregue. BL062 permanece exclusivamente o bug de sliders;
os novos IDs de H foram reconciliados como BL063–BL065.

## Pendências adicionais identificadas por J

| ID | Área | Pendência | Estado | Depende de | Responsável | Critério de aceite |
|---|---|---|---|---|---|---|
| BL066 | BB07/segurança de sessão | Evitar colisão de IDs de gesto após abrir/criar projeto | Pendente | BL013 | Coordenador | Pedido antigo não consegue atualizar/confirmar gesto novo; teste cruzando sessões/projetos |
| BL067 | BB07/prova de erro | Commit de gesto rejeitado pelo orçamento de bytes | A validar | BL013 | Coordenador | Rejeição preserva cena/histórico/candidato e devolve erro tipado |
| BL068 | BB07/observabilidade | Expor seleção e gesto no summary compartilhado | Pendente | BL012/BL013 | — | Estado efêmero consultável sem criar histórico; refletir invalidação |
| BL069 | BB07/seleção | Poda/invalidação imediata no undo, além da checagem preguiçosa atual | Pendente | BL012/BL013, coordenação com L | — | Avaliar necessidade antes de alterar; se implementada, sem restaurar apresentação antiga no undo |

As propostas de J para automação/controles, prova de socket e gate JSONL reutilizam BL012/BL014,
BL021 e o status A validar, respectivamente; não abrem tarefas duplicadas. F036 foi atualizada
para apontar F039, removendo a limitação histórica de consultas apenas de câmera.

## Divergências e cobertura adicional de K

| ID | Área | Pendência | Estado | Depende de | Responsável | Critério de aceite |
|---|---|---|---|---|---|---|
| BL070 | BB10/compatibilidade | Formalizar política de segundos→frame: racional exato do destino versus floor em float da origem | Decisão | BL018 | — | Manter comportamento atual até decisão; documentar divergência e testar os 14 artefatos, sem alterar expectativas para esconder diferença |
| BL071 | BB10/câmera | Medir matriz da câmera no runtime real da origem, além de Three.lookAt isolado | A validar | BL018, bancada WebGL | — | Proveniência distingue source runtime de three-library; comparar método real antes de alegar equivalência |
| BL072 | BB10/FPS | Formalizar teto de 1000 fps versus origem sem esse limite | Decisão | BL018 | — | Manter limite atual até decisão; teste explícito FPS_OUT_OF_RANGE e documentação de compatibilidade |
| BL073 | BB10/junções | Ampliar fronteiras de JOIN com easing não linear e sondas de 1 ulp | Pendente | BL018 | — | Resposta efetiva da origem registrada; preservar tolerâncias prévias e controles negativos |

K validada no coordenador: base 55 e extensão 47 regeneradas byte a byte; Node 45/45;
consumidor Rust 15/15. FPS>1000 e as 14 diferenças de piso NTSC são divergências explícitas,
não bugs corrigidos nem equivalência integral. A câmera em Three isolado não prova o runtime.

## Limites adicionais da correção de sliders

| ID | Área | Pendência | Estado | Depende de | Responsável | Critério de aceite |
|---|---|---|---|---|---|---|
| BL074 | F005/a11y | Increment/Decrement do Kit alteram estado sem emitir evento de aplicação | Pendente | BL062 | — | Ponte local ou correção upstream; uma ação acessível aplica/valida e gera uma entrada |
| BL075 | F005/UX | Eliminar salto visual do thumb ao cancelar até release | Pendente | BL062 | — | Thumb e preview coerentes após Escape sem reabrir gesto; teste de estado |
| BL076 | F005/socket | Provar property.gesture.* no socket e documentar acesso cruzado | Concluído | — | Coordenador | scripts/slider-live-proof.py: 12/12 na janela nativa isolada; docs de UI automation/project creation atualizadas; prova não usa ponteiro físico |

As propostas de L para mouse físico reutilizam BL043/BL062, sem novo ID. BL070–BL073
pertencem ao corpus K; pendências adicionais de L foram reconciliadas como BL074–BL076.
