# Backlog compartilhado do Tessel Editor

Registro único de execução das pendências: building blocks, migração, produto, decisões e provas.
Atualizado em 2026-11-05, após o merge `6cc4392` (PR #1). A rodada A–G terminou;
A próxima rodada H/I/J/K foi autorizada; atribuições e links constam abaixo após o despacho. Responsável `—` significa não atribuído.

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
| BL006 | BB05 | Definir plano avaliado de cena e contrato core → runtime, versão/capabilities | Pendente | BB02/BB03 existentes | — | Core resolve o frame; runtime só executa estado explícito, com erros tipados |
| BL007 | BB05 | Renderer da cena dirigida em preview e headless; geração/cancelamento/invalidação | Pendente | BL006, BL010 | — | Mesmo conteúdo em seek fora de ordem, troca de projeto e recuperação de contexto |
| BL008 | BB05 | Exportar cenas dirigidas pelo mesmo plano e original | Pendente | BL007 | — | Comparação de frames preview/headless/export e export curto reproduzível |
| BL009 | BB06 | Resolver dependências transitivas: imagens/mapas/splats/fontes e base de caminhos | Pendente | Modelo BB01/BB02 | — | Closure de recursos explícita, validação de caminhos e diagnóstico de ausentes |
| BL010 | BB06 | Jobs/load/cache/release por geração e dependentes da cena | Pendente | BL009 | — | Troca/release invalida dependentes; orçamento de memória e cancelamento |
| BL011 | BB06 | Save/package/restore/relink/interchange de recursos de cena | Pendente | BL009, BL010 | — | Pacote abre sem arquivos originais; só então retirar a guarda de pacote incompleto |
| BL012 | BB07 | Seleção por entidade, árvore de cena e tool state | Pendente | BB02, BL004 | — | Mesmo alvo por ID em UI/lib/CLI; batch independe de seleção |
| BL013 | BB07 | Gestos com preview/commit/cancel e histórico agrupado | Pendente | BL012 | — | Escape cancela, undo único e comando semântico equivalente ao gesto |
| BL014 | BB08 | Inspector ligado ao catálogo e às propriedades vivas | Pendente | BL004, BL012 | — | UI/CLI/lib compartilham comandos e valores; controle indisponível explica motivo |
| BL015 | BB08 | Overlays/handles: coordenadas, foco, hit testing e resize sobre WebView | Pendente | BL007, BL013, BL014, BL042 | — | Handle e comando dão mesmo resultado; validar foco/resize nativos |
| BL016 | BB09 | Transporte/mix do modelo dirigido: shots, janela de edição e relógio | Pendente | BL006, BL008 | — | Frame e amostra correspondentes em play/seek/pause; offline sem dispositivo |
| BL017 | BB09 | Narração/takes/legendas e monitor gain | Pendente | BL016, BL026 | — | Tempo, texto e mix persistem e coincidem na saída |
| BL018 | BB10 | Expandir corpus: tolerâncias JOIN, FPS variados contra a origem e to_world_matrix | Parcial | BB10 existente | — | Expectativas executadas na origem, tolerâncias prévias e casos de fronteira |
| BL019 | BB10 | Provas de plano/pixels/áudio/performance por novo domínio | Parcial | Cada entrega | — | Código final e proveniência; não usar resultado de protótipo como aceite |
| BL020 | Adapters | Expor avaliação completa de nós por frame no acesso programático de cena | Pendente | BB03 existente | — | Lib e JSONL retornam câmera/nós/unevaluated de forma coerente |
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

Plano e ownership: [H/I/J/K](migration/next-round-plan.md). Status de execução e links serão registrados pelo coordenador ao despachar; ainda não são entregas concluídas.
