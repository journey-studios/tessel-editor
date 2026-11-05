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
| BL007 | BB05 | Renderer da cena dirigida em preview e headless; geração/cancelamento/invalidação | Parcial | BL006, BL010 | M / coordenador | F041: plano real no Three.js, CLI e monitor WKWebView; 184 checks headless (com plano) e 40 checks de pixels nativos. Lifecycle Node e 3 testes do host. Contexto/reload nativos dirigidos ainda a ampliar |
| BL008 | BB05 | Exportar cenas dirigidas pelo mesmo plano e original | Pendente | BL007 | — | Comparação de frames preview/headless/export e export curto reproduzível |
| BL009 | BB06 | Resolver dependências transitivas: imagens/mapas/splats/fontes e base de caminhos | Parcial | Modelo BB01/BB02 | I / F038 ([relatório](migration/foundation-i-report.md)) | Manifesto/closure/preflight implementados; 27 core + 39 media e plano × manifesto passaram. Mapas/f32/narração só listados; fontes fora da fatia |
| BL010 | BB06 | Jobs/load/cache/release por geração e dependentes da cena | Parcial | BL009 | N / coordenador | F040: MediaHost com sessão, geração, orçamento, resolve/release/invalidate; 22 testes de integração/publicação reconferidos. Consumo por mapa aprovado e pixels comprovados em F041 |
| BL011 | BB06 | Save/package/restore/relink/interchange de recursos de cena | Parcial | BL009, BL010 | N / coordenador | F040: adopt/relink, base gerenciada, pacote Directory/Archive e project.import_scenes; teste de restauração sem origem/workspace passou. UI e formatos BL077 pendentes; guardas mantidas fora do subconjunto |
| BL012 | BB07 | Seleção por entidade, árvore de cena e tool state | A validar | BB02, BL004 | J / F039 ([relatório](migration/foundation-j-report.md)) | 21 testes públicos e unidade core passaram; executável headless real (13 requests) validou queries/seleção/gestos/undo; UI/socket e gesto físico pendentes |
| BL013 | BB07 | Gestos com preview/commit/cancel e histórico agrupado | A validar | BL012 | J / F039 ([relatório](migration/foundation-j-report.md)) | 21 testes públicos e unidade core passaram; executável headless real (13 requests) validou queries/seleção/gestos/undo; UI/socket e gesto físico pendentes |
| BL014 | BB08 | Inspector ligado ao catálogo e às propriedades vivas | A validar | BL004, BL012 | O / coordenador | [F039-UI](features/scene-inspector.md): árvore, oito propriedades, candidato/reset e erro tipado; agente reporta 137 testes desktop e clippy limpos, socket em instância própria. Gate local passou; falta inspeção visual |
| BL015 | BB08 | Overlays/handles: coordenadas, foco, hit testing e resize sobre WebView | Pendente | BL007, BL013, BL014, BL042 | — | Handle e comando dão mesmo resultado; validar foco/resize nativos |
| BL016 | BB09 | Transporte/mix do modelo dirigido: shots, janela de edição e relógio | Pendente | BL006, BL008 | — | Frame e amostra correspondentes em play/seek/pause; offline sem dispositivo |
| BL017 | BB09 | Narração/takes/legendas e monitor gain | Pendente | BL016, BL026 | — | Tempo, texto e mix persistem e coincidem na saída |
| BL018 | BB10 | Expandir corpus: tolerâncias JOIN, FPS variados e matrizes mundo | Parcial | BB10 existente | K / F032 ([relatório](migration/foundation-k-report.md)) | 47 casos estendidos + 55 base verificados contra origem; 49 testes Node e 15 testes Rust passaram. Matriz da câmera usa Three isolado; limite do runtime em BL071 |
| BL019 | BB10 | Provas de plano/pixels/áudio/performance por novo domínio | Parcial | Cada entrega | P / coordenador | [Corpus e harness](migration/directed-scene-proof.md) recebidos: modelo/corpus e 14 testes Python reconferidos; agente executou 80 checks headless e 80 socket, binários sinalizados como anteriores às fontes. Pixels/pacote/export bloqueados até BL007/BL011/BL008/BL063; reexecutar após build integrado |
| BL020 | Adapters | Expor avaliação completa de nós por frame no acesso programático de cena | A validar | BB03 existente | J / F039 ([relatório](migration/foundation-j-report.md)) | 21 testes públicos e unidade core passaram; executável headless real (13 requests) validou queries/seleção/gestos/undo; UI/socket e gesto físico pendentes |
| BL021 | Adapters | Provar `scene.*` pelo socket da janela e depois pela UI visual | A validar | BL012/BL014 para UI | O/P / coordenador | Socket reportado em instâncias próprias; O: controles semânticos, P: 80 checks. UI com 16 testes de input GPUI; aparência e ponteiro físico não conferidos. Build integrado: prova final 408/408, com 80 checks de socket e 2 da árvore semântica |

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

Plano e ownership: [H/I/J/K](migration/next-round-plan.md). H/J/K foram recebidas e passaram nos gates direcionados registrados acima; I recebida; gate local completo passou (718 testes Rust, 2 ignorados, clippy/fmt/check/release). Revisão do PR ainda é separada. L foi adicionada para BL062 e está concluída. Cargo e integração final são do coordenador.

| Frente | Itens | Responsável/tarefa | Estado |
|---|---|---|---|
| H | BL006 | <t3-thread://v2/8ac99a47-6712-4c08-aa4a-d8e73a792f19/thread:delegated-task:command%3Amcp%3Ad3856989-26f9-4933-b607-9945895abf30%3Adelegate-task%3Aa6ff05b3-f726-4031-ab83-3f9cc3045ebf> | Recebido — gate local conjunto passou; escopo restante explícito |
| I | BL009/BL010/BL011 | <t3-thread://v2/8ac99a47-6712-4c08-aa4a-d8e73a792f19/thread:delegated-task:command%3Amcp%3Ad3856989-26f9-4933-b607-9945895abf30%3Adelegate-task%3A9cb28c3e-6589-4f90-840f-b43f21c534ca> | Recebida — gate local conjunto passou; BB06 permanece parcial |
| J | BL012/BL013/BL020 | <t3-thread://v2/8ac99a47-6712-4c08-aa4a-d8e73a792f19/thread:delegated-task:command%3Amcp%3Ad3856989-26f9-4933-b607-9945895abf30%3Adelegate-task%3A975cf39a-05f7-40c5-a224-811495f83404> | Recebido — gate local conjunto passou; escopo restante explícito |
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
| BL063 | BB05/Adapters | Expor scene.plan por lib/JSONL/UI e integrar plano × manifesto | Concluído | BL006, BL009 | Coordenador | Operação compartilhada scene.plan com gates/gesture; mapa F040 consumido em scene.render e monitor. 7 testes scene_flow e 60 checks de plano headless passaram; gate local passou (827/2 no workspace, scene_flow 8/8 após teste extra) |
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

## Limites adicionais do transporte de recursos

| ID | Área | Pendência | Estado | Depende de | Responsável | Critério de aceite |
|---|---|---|---|---|---|---|
| BL077 | BB06/formatos | Interpretar/transportar mapas, grades f32, profundidade/relevo e narração | Pendente | BL001/BL003, BL009 | — | Capabilities/versão e verificação por formato (pixels/alpha, tamanho e endianness), sem omitir dependências |
| BL078 | BB06/portabilidade | Identidade de arquivo fora de Unix e colisões Unicode NFC/NFD | Pendente | BL009 | — | Equivalente Windows a dev/ino e política de normalização, com testes reais |
| BL079 | BB06/concorrência | Garantir controle exclusivo das raízes ou usar operações ancoradas em diretórios | Parcial | BL011 | N / coordenador | RootLease advisory e recusa root_busy testados; protege operações cooperativas. Escritor hostil trocando pai continua fora da garantia; operações ancoradas pendentes |
| BL080 | BB06/cancelamento | Observar cancelamento durante publicação e provar rollback parcial dessa fase | A validar | BL010 | N / coordenador | Cancelamento antes de cada publicação e rollback só dos arquivos próprios; 4 testes determinísticos de publicação reconferidos. Gate integrado da rodada pendente |

A proposta de I para ligar materialize ao pacote reutiliza BL011, sem duplicação. IDs sugeridos
BL070–BL072 foram reconciliados como BL011/BL077/BL078; BL079/BL080 explicitam os limites de
concorrência/cancelamento. O fallback check-then-rename foi removido pelo coordenador com
regressão falhando antes e passando depois; não permanece como alternativa de publicação.

## Paridade de produto já pendente na matriz

| ID | Área | Pendência | Estado | Depende de | Responsável | Critério de aceite |
|---|---|---|---|---|---|---|
| BL081 | F017 | Dividir clipe/corte interno no core, CLI e UI | Pendente | Modelo de timeline existente | — | Operação real com tempo racional, undo único e erro atômico; não confundir com edição de shots do piloto |
| BL082 | F016 | Efeitos de clipe e regras compartilhadas de processamento | Pendente | BL004/BL006 aplicáveis | — | Contrato por efeito, mesma avaliação offline/preview/export e controles só quando funcionais; alinhar BL024/BL028 sem duplicar migração |
| BL083 | F015 | Completar controles visuais de criação de tracks/clipes dinâmicos | Parcial | Comandos core existentes | — | Mesmos handlers lib/headless; UI habilitada só para operações reais, com prova de interação |
| BL084 | BB10/CI | Automatizar gates no PR além do CodeRabbit | Pendente | Ambiente de build/runtime reprodutível | — | Separar teste local de CI; instalar ferramentas/licenças intencionalmente; não alegar fonte-oráculo reexecutada sem acesso à origem |

Integração local H–L: [relatório consolidado](migration/round-hijkl-report.md). 718 testes
Rust passaram, 2 ZIP64 ignorados; 45 Node passaram, 55+47 casos verificados contra origem.
BL061 recebeu correções pontuais de README/índices e rótulo histórico A–E; ainda pede revisão
das descrições antigas restantes. Não há agente de implementação em andamento nesta rodada.

## Revisão do PR #2

| ID | Área | Pendência | Estado | Depende de | Responsável | Critério de aceite |
|---|---|---|---|---|---|---|
| BL085 | F032/integridade | Impedir aliases de saída do oráculo estendido de alterarem o corpus base | Concluído | — | Coordenador | Canonização/ancestrais + identidade de arquivos antes de escrever; 4 regressões em cópias temporárias; 49 Node e 16 Rust direcionados passaram; só hashes do gerador mudaram no manifest |

Os 12 comentários acionáveis e o nit de cobertura foram tratados: contratos e índices agora
refletem gates executados, IDs reconciliados e o teste correto de F039; comandos reproduzíveis
usam scripts/cargo; tabela do runner teve pipes escapados. Gate desta correção: 49 Node,
16 Rust direcionados e fmt. O último gate workspace completo continua o da rodada (718/2),
não foi reexecutado por estas mudanças em documentação, proteção Node e lista de frames de teste.

## Rodada de integração de cena dirigida — M/N/O/P

Autorizada pelo usuário em 2026-11-05. Contrato e ownership: [plano](migration/directed-scene-integration-plan.md).
Delegações serão registradas abaixo pelo coordenador. Não há conclusão por mera atribuição.

| Frente | Itens | Estado | Ownership |
|---|---|---|---|
| M | BL007/BL063 | Recebido e integrado — gate local passou | Renderer/runtime, scene.plan e ligação ao monitor pelo coordenador |
| N | BL010/BL011 | Recebido — 22 testes reconferidos | Recursos e mídia/pacote integrados ao renderer; formatos parciais documentados |
| O | BL014/BL021 | Integrado — gate local passou | Desktop/inspector/árvore; 140 testes desktop, socket/pixels; sem inspeção visual |
| P | BL019 | Recebido — integração parcial | Provas e fixtures sintéticas; 408 checks integrados, contexto/export dirigido pendentes |

As quatro delegações foram iniciadas no Cursor com `claude-sonnet-5-5`:

| Frente | Delegação | Estado |
|---|---|---|
| M | <t3-thread://v2/8ac99a47-6712-4c08-aa4a-d8e73a792f19/thread:delegated-task:command%3Amcp%3Ad3856989-26f9-4933-b607-9945895abf30%3Adelegate-task%3A00e21202-77b5-4542-9153-18a3a3e3a46c> | Recebido — headless/pixels; integração compartilhada pelo coordenador |
| N | <t3-thread://v2/8ac99a47-6712-4c08-aa4a-d8e73a792f19/thread:delegated-task:command%3Amcp%3Ad3856989-26f9-4933-b607-9945895abf30%3Adelegate-task%3Aa95284fe-dc6e-4fb3-86d9-d267f72e5c34> | Recebido — testes direcionados de recursos/publicação 22/22 reconferidos |
| O | <t3-thread://v2/8ac99a47-6712-4c08-aa4a-d8e73a792f19/thread:delegated-task:command%3Amcp%3Ad3856989-26f9-4933-b607-9945895abf30%3Adelegate-task%3A4a23d447-780f-4752-a38a-5b5c3047e216> | Recebido — código e relatório revisados; gate local passou; inspeção visual pendente |
| P | <t3-thread://v2/8ac99a47-6712-4c08-aa4a-d8e73a792f19/thread:delegated-task:command%3Amcp%3Ad3856989-26f9-4933-b607-9945895abf30%3Adelegate-task%3A3c548058-1010-46b8-8463-8225ed541d76> | Recebido — corpus e 14 testes Python reconferidos; 408 checks no build integrado, contexto/export dirigido fora do conjunto |

### Limites recebidos da frente P

- BL019: corpus atual não discrimina ordem relativa hero/echo (mesma textura e sem sobreposição). Ampliar caso antes de afirmar essa cobertura.
- BL021: agente reporta 80/80 checks por socket de instância própria; sem pixels e com binário sinalizado anterior às fontes. Revalidar no build final, separadamente da UI visual.
- BL007/BL008/BL011/BL063: operações propostas em `scripts/directed-scene-proof/interfaces.json` são adapters do harness, não contrato de produção já entregue; coordenador deve mapear APIs reais e reexecutar.

### Integração pendente da UI de cena — frente O

| ID | Área | Pendência | Estado | Dependências | Responsável | Aceite |
|---|---|---|---|---|---|---|
| BL086 | F039/UI | Anexar cena por diálogo na janela | Pendente | scene.attach existente | Coordenador | Entrada de arquivo e semântica pelo mesmo fluxo; erro sem mutação; respeitar ocultação da WebView no diálogo |
| BL087 | F039/UI | Inspeção visual da árvore e inspector de cena | A validar | BL014 e build integrado | Coordenador | Conferir layout amplo/menor e propriedades; não confundir input GPUI com inspeção visual |

`monitor.show_scene` reutiliza BL007/BL063 (seam documentado em foundation-o-report.md), sem nova fila.
Modo Clipe/Cena é apresentação local exposta pelo socket; avaliar necessidade de Command/persistência junto de F014, sem criar regra de domínio artificial.
Limites BL074/BL075 também precisam ser considerados nos sliders de cena.

### Limites de recursos/pacote recebidos da frente N

| ID | Área | Pendência | Estado | Dependências | Responsável | Aceite |
|---|---|---|---|---|---|---|
| BL088 | Pacote/atomicidade | Preservar pacote anterior se a substituição falhar antes do rename | Pendente | F026 | Coordenador | Falha injetada mantém pacote anterior legível; publicação e limpeza próprias recuperáveis |
| BL089 | Recursos/jobs | Limpar staging/materialização de import de cena cujo resultado fica obsoleto | Pendente | BL010/BL011 | Coordenador | Troca de projeto durante import não deixa pasta órfã nem apaga arquivo pré-existente; teste determinístico |

F040 recebida em [foundation-n-report.md](migration/foundation-n-report.md). Coordenador reconferiu 18 testes de integração e 4 de publicação em 2026-11-05. Sem prova de pixels/UI nessa entrega.

### Limites do renderer dirigido recebidos da frente M

| ID | Área | Pendência | Estado | Dependências | Responsável | Aceite |
|---|---|---|---|---|---|---|
| BL090 | F041/qualidade | Política de alpha de saída e mipmaps/minificação de planos | Pendente | BL008/BL018 | Coordenador | Fundo preto opaco atual declarado; definir alpha e comparar minificação com origem antes de prometer equivalência visual completa |
| BL091 | Runtime/testes | Investigar falha intermitente de splat.test.mjs em concorrência padrão | Pendente | Harness runtime | Coordenador | Reproduzir/diagnosticar corrida; suíte serial de 79 passa, isso não prova concorrência |
| BL092 | F041/lifecycle | Prova nativa dirigida de perda de contexto, reload e recursos invalidados com frame em voo | A validar | BL007 | Coordenador | WebView real converge ao frame atual e rejeita respostas antigas; distinguir testes Node/host roteirizado de prova nativa |

O contrato e evidências integradas da rodada estão em [round-mnop-report.md](migration/round-mnop-report.md).

### Gate local da rodada M/N/O/P

Entregas integradas na branch feat/directed-scene-end-to-end: workspace 827 testes verdes, 2 ignorados; scene_flow repetido 8/8 após adicionar regressão de candidato; clippy/fmt/build release limpos. Prova final **408/408** inclui pixels headless/nativos, monitor interativo, pacote restaurado sem origem e seleção semântica. Relatório e hashes em [round-mnop-report.md](migration/round-mnop-report.md).
Isso não fecha BL008 (export dirigido), BL087 (inspeção visual) ou BL092 (lifecycle dirigido nativo forçado). App/projeto do usuário preservados.

Runtime final: 79/79 reconferidos pelo coordenador em execução serializada; sem skips. BL091 continua aberto para concorrência padrão.

Entrega de implementação: commit `6a8b030`; revisão em [PR #3](https://github.com/journey-studios/tessel-editor/pull/3). Gates descritos são locais; revisão/merge são estados separados.
