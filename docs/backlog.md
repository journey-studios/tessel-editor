# Backlog compartilhado do Tessel Editor

Registro único de execução das pendências: building blocks, migração, produto, decisões e provas.
Atualizado em 2026-11-05, após o merge `914c830` (PR #3). Rodadas A–G, H–L e M–P integradas.
Próxima rodada autorizada: dez frentes R1–R10, com contratos/ownership no plano abaixo. Responsável `—` significa não atribuído.

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
| BL006 | BB05 | Definir plano avaliado e contrato core → runtime | Concluído | BB02/BB03 | Coordenador | F037/F041 integrados na PR #3; plano completo, capabilities, gates e consumidor real. Domínios adicionais e export permanecem em seus próprios itens |
| BL007 | BB05 | Renderer da cena dirigida em preview e headless; geração/cancelamento/invalidação | Parcial | BL006, BL010 | M / coordenador | F041: plano real no Three.js, CLI e monitor WKWebView; 184 checks headless (com plano) e 40 checks de pixels nativos. Lifecycle Node e 3 testes do host. Contexto/reload nativos dirigidos ainda a ampliar |
| BL008 | BB05 | Exportar cenas dirigidas pelo mesmo plano e original | Concluído | BL007 | R1 / coordenador | R1 reporta 10 testes Chrome real com 1080p/4K, pixels e CLI. Coordenador reconferiu core 15/15, media 7/7 e desktop integrado 190/190; diálogo/host ligados, R10 reporta 23/23 fases CLI/socket/diálogo; prova repetida com binários finais e gate global passou; áudio dirigido recusado salvo omissão explícita  Gate final: [R1–R10](migration/round-r1-r10-report.md); [PR #4](https://github.com/journey-studios/tessel-editor/pull/4), merge `c099f10`; [gate final](migration/round-r1-r10-report.md). |
| BL009 | BB06 | Resolver dependências transitivas: imagens/mapas/splats/fontes e base de caminhos | Parcial | Modelo BB01/BB02 | I / F038 ([relatório](migration/foundation-i-report.md)) | Manifesto/closure/preflight implementados; 27 core + 39 media e plano × manifesto passaram. Mapas/f32/narração só listados; fontes fora da fatia |
| BL010 | BB06 | Jobs/load/cache/release por geração e dependentes da cena | Parcial | BL009 | N / coordenador | F040: MediaHost com sessão, geração, orçamento, resolve/release/invalidate; 22 testes de integração/publicação reconferidos. Consumo por mapa aprovado e pixels comprovados em F041 |
| BL011 | BB06 | Save/package/restore/relink/interchange de recursos de cena | Parcial | BL009, BL010 | N / coordenador | F040: adopt/relink, base gerenciada, pacote Directory/Archive e project.import_scenes; teste de restauração sem origem/workspace passou. UI e formatos BL077 pendentes; guardas mantidas fora do subconjunto |
| BL012 | BB07 | Seleção por entidade, árvore de cena e tool state | Parcial | BB02, BL004 | J / F039 ([relatório](migration/foundation-j-report.md)) | 21 testes públicos e unidade core passaram; executável headless real (13 requests) validou queries/seleção/gestos/undo; UI/input GPUI e socket integrados na PR #3; ponteiro físico/handles pendentes |
| BL013 | BB07 | Gestos com preview/commit/cancel e histórico agrupado | Parcial | BL012 | J / F039 ([relatório](migration/foundation-j-report.md)) | 21 testes públicos e unidade core passaram; executável headless real (13 requests) validou queries/seleção/gestos/undo; UI/input GPUI e socket integrados na PR #3; ponteiro físico/handles pendentes |
| BL014 | BB08 | Inspector ligado ao catálogo e às propriedades vivas | Concluído | BL004, BL012 | O / coordenador | Catálogo atual de oito propriedades ligado ao inspector, árvore, gesto/candidato/reset e erros tipados. 190 testes desktop, 50 capturas nativas; mínimo claro/escuro e dock 100 px inspecionados. Catálogo ampliado em BL004, ponteiro físico em BL043. [PR #4](https://github.com/journey-studios/tessel-editor/pull/4), merge `c099f10`; [gate final](migration/round-r1-r10-report.md). |
| BL015 | BB08 | Overlays/handles: coordenadas, foco, hit testing e resize sobre WebView | Pendente | BL007, BL013, BL014, BL042 | — | Handle e comando dão mesmo resultado; validar foco/resize nativos |
| BL016 | BB09 | Transporte/mix do modelo dirigido: shots, janela de edição e relógio | A validar | BL006, BL008 | S9 / frente S9 | Subconjunto entregue: **mix offline** do WAV declarado no export de cena ([F042A](features/scene-export-audio.md)): campos reais do bundle, tempo racional frames/amostras, intervalo/offset e conversão de frame rate e de sample rate, AAC com a duração do vídeo, `audio:false` omite com aviso, formatos não suportados recusados antes do job. Evidência: core 20+4, mídia 15 (FFmpeg real, amostras exatas e espectro medido), CLI headless com Chrome, oráculo da R10 17/17. **Falta**: play/seek/pause com o relógio de amostras sobre o modelo dirigido (BL016.1), prova viva da janela com mix (BL016.2), narração/takes no mix (BL017), fonte > 48 kHz/formatos comprimidos (BL016.3) |
| BL016.1 | BB09 | Transporte dirigido: play/seek/pause com relógio de amostras sobre shots e janela do bundle | Pendente | BL016, BL015 | — | Frame e amostra correspondentes em play/seek/pause no monitor; o plano offline de F042A é a referência do mapeamento amostra↔instante |
| BL016.2 | F042A/UI | Provar na janela real o export de cena com mix (`flow.export_scene`, `audio=yes`) | Pendente | BL016, BL086 | — | Diálogo exporta com `audio=yes` e o MP4 passa pelo mesmo oráculo; a fase `ui_export_flow` da R10 hoje espera a recusa do padrão e precisa de ajuste (S10) |
| BL016.3 | F042A | Mix com fonte > 48 kHz (passa-baixa) e formatos comprimidos (FLAC/MP3/AAC) | Pendente | BL016 | — | Conversão sem aliasing provada contra um oráculo espectral; formatos novos decodificados por adapter injetado, nunca no core |
| BL016.4 | F042A/jobs | Nomear estaticamente os códigos de job `resource_changed`, `audio_source_short`, `audio_window_*` em `From<JobError> for CommandError` | Pendente | BL016 | — | `scene.export` falho responde o código tipado em vez de `job_failed` + `details.job_code`; teste em `jobs.rs` |
| BL017 | BB09 | Narração/takes/legendas e monitor gain | Pendente | BL016, BL026 | — | Tempo, texto e mix persistem e coincidem na saída |
| BL018 | BB10 | Expandir corpus: tolerâncias JOIN, FPS variados e matrizes mundo | Parcial | BB10 existente | K / F032 ([relatório](migration/foundation-k-report.md)) | 47 casos estendidos + 55 base verificados contra origem; 49 testes Node e 15 testes Rust passaram. Matriz da câmera usa Three isolado; limite do runtime em BL071 |
| BL019 | BB10 | Provas de plano/pixels/áudio/performance por domínio | Parcial | Cada entrega | Coordenador | PR #3: 408 checks no fluxo de cena, headless/socket/pixels/pacote; sem prova de export dirigido ou áudio. R10 amplia para export; contexto nativo dirigido em BL092 |
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
| BL064 | BB05/performance | Avaliação de muitos frames sem reconstruir NodeTimeline; medir tamanho/custo do plano | Concluído | BL006, consumidor BL007/BL008 | R6 / coordenador | PlanTimeline aditiva recebida; coordenador reconferiu 10/10 testes de equivalência e 4/4 do runner; agente mediu plano 3–5x, plano+JSON 1,09–1,24x (câmera pesada 28x/2,7x). Equivalência, golden por perfil e memória documentados em next-r6-report.md. PlanTimeline adotada pela R1 no job e coleta de assets; ganho no export real não medido (otimizações adicionais BL107/BL108); gate global passou  Gate final: [R1–R10](migration/round-r1-r10-report.md); [PR #4](https://github.com/journey-studios/tessel-editor/pull/4), merge `c099f10`; [gate final](migration/round-r1-r10-report.md). |
| BL065 | BB05/domínios | Expandir plano aos domínios recusados e ao desempate por profundidade | Pendente | BL003, BL023–BL027 | — | Capabilities/versão e estado completo por domínio, com provas; não remover unsupported sem implementação |

H recebida: plano não executa gates de admissão temporal/estrutural; o adapter consumidor deve
executá-los antes de habilitar render. Unsupported inclui mapas de dados e splat dirigido,
independentemente do Spark básico já entregue. BL062 permanece exclusivamente o bug de sliders;
os novos IDs de H foram reconciliados como BL063–BL065.

## Pendências adicionais identificadas por J

| ID | Área | Pendência | Estado | Depende de | Responsável | Critério de aceite |
|---|---|---|---|---|---|---|
| BL066 | BB07/segurança de sessão | Evitar colisão de IDs de gesto após abrir/criar projeto | Concluído | BL013 | R3 / coordenador | IDs opacos por sessão; teste de projeto novo e reaberto recusa pedido antigo com other_session. Coordenador reconferiu scene_session 15/15 e scene_interaction 21/21, incluindo executável JSONL; workspace/desktop e provas finais passaram.  Gate final: [R1–R10](migration/round-r1-r10-report.md); [PR #4](https://github.com/journey-studios/tessel-editor/pull/4), merge `c099f10`; [gate final](migration/round-r1-r10-report.md). |
| BL067 | BB07/prova de erro | Commit de gesto rejeitado pelo orçamento de bytes | Concluído | BL013 | R3 / coordenador | Update público acima de 128 MiB recusa sem mutação. Commit acima do orçamento é ramo de defesa provado só por teste unitário white-box, inalcançável via API pública normal; não alegar prova de commit por JSONL.  Gate final: [R1–R10](migration/round-r1-r10-report.md); [PR #4](https://github.com/journey-studios/tessel-editor/pull/4), merge `c099f10`; [gate final](migration/round-r1-r10-report.md). |
| BL068 | BB07/observabilidade | Expor seleção e gesto no summary compartilhado | Concluído | BL012/BL013 | R3 / coordenador | summary adiciona scene_interaction apenas quando há seleção/gesto, lista até 64 com contagem total; leitura sem poda/histórico. Workspace/desktop e provas finais passaram.  Gate final: [R1–R10](migration/round-r1-r10-report.md); [PR #4](https://github.com/journey-studios/tessel-editor/pull/4), merge `c099f10`; [gate final](migration/round-r1-r10-report.md). |
| BL069 | BB07/seleção | Poda/invalidação imediata no undo, além da checagem preguiçosa atual | Concluído | BL012/BL013, coordenação com L | R3 / coordenador | Poda imediata de seleção em undo/redo/rollback elimina dependência de consulta intermediária; redo não revive seleção. Gesto permanece invalidado até uso/cancel, por decisão documentada.  Gate final: [R1–R10](migration/round-r1-r10-report.md); [PR #4](https://github.com/journey-studios/tessel-editor/pull/4), merge `c099f10`; [gate final](migration/round-r1-r10-report.md). |

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
| BL086 | F039/UI | Anexar cena por diálogo na janela | Concluído | scene.attach existente | R2 / coordenador | F043 recebida: cabeçalho node:26, flow.attach_scene/resources e cadeia adopt/resolve/monitor; coordenador reconferiu 18/18 testes; 33 checks nativos repetidos no build final. Hook do inspector integrado após R9; painel de arquivos do SO/ponteiro físico não provados; capturas do inspector em BL087. Attach síncrono em BL100  Gate final: [R1–R10](migration/round-r1-r10-report.md); [PR #4](https://github.com/journey-studios/tessel-editor/pull/4), merge `c099f10`; [gate final](migration/round-r1-r10-report.md). |
| BL087 | F039/UI | Inspeção visual da árvore e inspector de cena | Concluído | BL014 e build integrado | R9 / coordenador | Agente reporta 50/50 capturas nativas e desktop 188/188; coordenador inspecionou mínimo escuro e dock 100 px claro e reconferiu 11 testes visuais após ligar Anexar. Cabeçalho ajustado e captura final BL104 concluída; ponteiro físico BL043; gate global passou  Gate final: [R1–R10](migration/round-r1-r10-report.md); [PR #4](https://github.com/journey-studios/tessel-editor/pull/4), merge `c099f10`; [gate final](migration/round-r1-r10-report.md). |

`monitor.show_scene` reutiliza BL007/BL063 (seam documentado em foundation-o-report.md), sem nova fila.
Modo Clipe/Cena é apresentação local exposta pelo socket; avaliar necessidade de Command/persistência junto de F014, sem criar regra de domínio artificial.
Limites BL074/BL075 também precisam ser considerados nos sliders de cena.

### Limites de recursos/pacote recebidos da frente N

| ID | Área | Pendência | Estado | Dependências | Responsável | Aceite |
|---|---|---|---|---|---|---|
| BL088 | Pacote/atomicidade | Preservar pacote anterior se a substituição falhar antes do rename | Concluído | F026 | R4 / coordenador | Publicação preserva anterior: rename único para arquivo; backup+rollback para pasta/troca de formato. Coordenador reconferiu package_atomic 10/10; agente também executou JSONL real. Gate integrado passou. Queda entre renames/recuperação automática em BL098.  Gate final: [R1–R10](migration/round-r1-r10-report.md); [PR #4](https://github.com/journey-studios/tessel-editor/pull/4), merge `c099f10`; [gate final](migration/round-r1-r10-report.md). |
| BL089 | Recursos/jobs | Limpar staging/materialização de import de cena cujo resultado fica obsoleto | Parcial | BL010/BL011, BL094/BL111 | Integração / coordenador | PendingAdoption integrado ao host; cleanup29/ownership23/host13 cobertos e gate global passou. Claims seguros no mesmo processo e SHA-256 antes de remover; limite de multiprocesso BL111 e custo UI BL112 permanecem. [Relatório](migration/integration-cleanup-report.md). |

F040 recebida em [foundation-n-report.md](migration/foundation-n-report.md). Coordenador reconferiu 18 testes de integração e 4 de publicação em 2026-11-05. Sem prova de pixels/UI nessa entrega.

### Limites do renderer dirigido recebidos da frente M

| ID | Área | Pendência | Estado | Dependências | Responsável | Aceite |
|---|---|---|---|---|---|---|
| BL090 | F041/qualidade | Política de alpha de saída e mipmaps/minificação de planos | Pendente | BL008/BL018 | Coordenador | Fundo preto opaco atual declarado; definir alpha e comparar minificação com origem antes de prometer equivalência visual completa |
| BL091 | Runtime/testes | Falha intermitente de splat.test.mjs em concorrência padrão | Concluído | Harness runtime | R7 / coordenador | Corrida do teste entre dois page.evaluate e carga já concluída; gate de fetch comprova load em voo. Agente reporta 2/6 falhas antes, 12/12 suítes de 85 testes depois; runtime intocado. Coordenador reconferiu 6/6 testes splat/splat-concurrency com concorrência padrão. Gate global passou. [Relatório](migration/next-r7-report.md)  Gate final: [R1–R10](migration/round-r1-r10-report.md); [PR #4](https://github.com/journey-studios/tessel-editor/pull/4), merge `c099f10`; [gate final](migration/round-r1-r10-report.md). |
| BL092 | F041/lifecycle | Prova nativa dirigida de perda de contexto, reload e recursos invalidados com frame em voo | Concluído | BL007 | R8 / coordenador | Vazamento de slots corrigido; coordenador reconferiu host 13/13 e harness 7/7. Agente reporta 218 checks nativos em duas execuções Apple GPU, perda forçada; gate global passou. Ready atrasado em BL102; orçamento ampliado em BL103. Relatório next-r8-report.md  Gate final: [R1–R10](migration/round-r1-r10-report.md); [PR #4](https://github.com/journey-studios/tessel-editor/pull/4), merge `c099f10`; [gate final](migration/round-r1-r10-report.md). |

O contrato e evidências integradas da rodada estão em [round-mnop-report.md](migration/round-mnop-report.md).

### Gate local da rodada M/N/O/P

Entregas integradas na branch feat/directed-scene-end-to-end: workspace 827 testes verdes, 2 ignorados; scene_flow repetido 8/8 após adicionar regressão de candidato; clippy/fmt/build release limpos. Prova final **408/408** inclui pixels headless/nativos, monitor interativo, pacote restaurado sem origem e seleção semântica. Relatório e hashes em [round-mnop-report.md](migration/round-mnop-report.md).
Isso não fecha BL008 (export dirigido) ou BL087 (inspeção visual). BL092 recebeu prova nativa forçada na R8, ainda sujeita ao gate integrado. App/projeto do usuário preservados.

Runtime final: 79/79 reconferidos pelo coordenador em execução serializada; sem skips. BL091 continua aberto para concorrência padrão.

Entrega de implementação: commit `6a8b030`; revisão em [PR #3](https://github.com/journey-studios/tessel-editor/pull/3). Gates descritos são locais; revisão/merge são estados separados.

### Revisão da PR #3

| ID | Área | Pendência | Estado | Dependências | Responsável | Aceite |
|---|---|---|---|---|---|---|
| BL093 | Revisão/robustez | Corrigir 10 apontamentos e nit de admissão por referência da PR #3 | Concluído | PR #3 | Coordenador | Arc retido no cache, import obsoleto recusado na lib, CLI relativa, orçamento concorrente, corpus portátil, JSONL com timeout/stderr drenado e docs reproduzíveis; 68 Rust + 19 runtime + 19 Python direcionados passaram; prova integrada repetida 408/408; PR #3 mergeada em 914c830; incremental pulada por cota, sem aprovação nova desse head |

BL089: troca de projeto agora é recusada por `scene_resources::import::apply` com stale_result e descarte das pastas próprias; regressão via job.commit passou. Outros caminhos de descarte/cancelamento ainda precisam de auditoria antes de encerrar o item inteiro.

## Próxima entrega — dez frentes R1–R10

Usuário autorizou iniciar dez agentes Cursor Sonnet 5.5. [Contrato, ownership e aceite](migration/next-delivery-ten-agents.md). Somente delegação iniciada ganha status Em andamento; coordenador insere os links abaixo.

| Frente | Itens | Delegação | Estado |
|---|---|---|---|
| R1 | BL008 | <t3-thread://v2/8ac99a47-6712-4c08-aa4a-d8e73a792f19/thread:delegated-task:command%3Amcp%3Ad3856989-26f9-4933-b607-9945895abf30%3Adelegate-task%3Atessel-next-914c830-r1> | Integrada na PR #4 e merge c099f10; gate final passou |
| R2 | BL086 | <t3-thread://v2/8ac99a47-6712-4c08-aa4a-d8e73a792f19/thread:delegated-task:command%3Amcp%3Ad3856989-26f9-4933-b607-9945895abf30%3Adelegate-task%3Atessel-next-914c830-r2> | Integrada na PR #4 e merge c099f10; gate final passou |
| R3 | BL066–BL069 | <t3-thread://v2/8ac99a47-6712-4c08-aa4a-d8e73a792f19/thread:delegated-task:command%3Amcp%3Ad3856989-26f9-4933-b607-9945895abf30%3Adelegate-task%3Atessel-next-914c830-r3> | Integrada na PR #4 e merge c099f10; gate final passou |
| R4 | BL088 | <t3-thread://v2/8ac99a47-6712-4c08-aa4a-d8e73a792f19/thread:delegated-task:command%3Amcp%3Ad3856989-26f9-4933-b607-9945895abf30%3Adelegate-task%3Atessel-next-914c830-r4> | Integrada na PR #4 e merge c099f10; gate final passou |
| R5 | BL089 | <t3-thread://v2/8ac99a47-6712-4c08-aa4a-d8e73a792f19/thread:delegated-task:command%3Amcp%3Ad3856989-26f9-4933-b607-9945895abf30%3Adelegate-task%3Atessel-next-914c830-r5> | Integrada na PR #4 e merge c099f10; gate final passou; BL089 permanece parcial por BL111/BL112 |
| R6 | BL064 | <t3-thread://v2/8ac99a47-6712-4c08-aa4a-d8e73a792f19/thread:delegated-task:command%3Amcp%3Ad3856989-26f9-4933-b607-9945895abf30%3Adelegate-task%3Atessel-next-914c830-r6> | Integrada na PR #4 e merge c099f10; gate final passou |
| R7 | BL091 | <t3-thread://v2/8ac99a47-6712-4c08-aa4a-d8e73a792f19/thread:delegated-task:command%3Amcp%3Ad3856989-26f9-4933-b607-9945895abf30%3Adelegate-task%3Atessel-next-914c830-r7> | Integrada na PR #4 e merge c099f10; gate final passou |
| R8 | BL092 | <t3-thread://v2/8ac99a47-6712-4c08-aa4a-d8e73a792f19/thread:delegated-task:command%3Amcp%3Ad3856989-26f9-4933-b607-9945895abf30%3Adelegate-task%3Atessel-next-914c830-r8> | Integrada na PR #4 e merge c099f10; gate final passou |
| R9 | BL087 | <t3-thread://v2/8ac99a47-6712-4c08-aa4a-d8e73a792f19/thread:delegated-task:command%3Amcp%3Ad3856989-26f9-4933-b607-9945895abf30%3Adelegate-task%3Atessel-next-914c830-r9> | Integrada na PR #4 e merge c099f10; gate final passou |
| R10 | BL019/BL008 | <t3-thread://v2/8ac99a47-6712-4c08-aa4a-d8e73a792f19/thread:delegated-task:command%3Amcp%3Ad3856989-26f9-4933-b607-9945895abf30%3Adelegate-task%3Atessel-next-914c830-r10> | Integrada na PR #4 e merge c099f10; gate final passou |

### Pendências recebidas na limpeza R5

| ID | Área | Pendência | Estado | Dependências | Responsável | Aceite |
|---|---|---|---|---|---|---|
| BL094 | Jobs/resultados | Descartar explicitamente resultado finalizado não aplicado e proteger prune de resultado pendente | Pendente | BL089, host compartilhado após R1 | Coordenador | job.discard/semântica equivalente limpa somente arquivos próprios e tem estado consultável; prune não perde dono do resultado |
| BL095 | Recursos/rollback | Garantir Journal de materialize com unwinding de panic | Concluído | F038 | Integração / coordenador | ArmedJournal desfaz links em panic de publicação; regressão e mutação reportadas, gate global passou; ver integration-cleanup-report.md  Gate final: [R1–R10](migration/round-r1-r10-report.md); [PR #4](https://github.com/journey-studios/tessel-editor/pull/4), merge `c099f10`; [gate final](migration/round-r1-r10-report.md). |
| BL096 | Recursos/concorrência | Impedir que descarte apague arquivos reutilizados por outro job no mesmo processo | Concluído | BL089/BL079 | Integração / coordenador | Claims e transferência de posse ao aceitar, ordens determinísticas de aceitar/descartar cobertas. Limite explícito: dois processos não compartilham registro (BL111); gate global passou  Gate final: [R1–R10](migration/round-r1-r10-report.md); [PR #4](https://github.com/journey-studios/tessel-editor/pull/4), merge `c099f10`; [gate final](migration/round-r1-r10-report.md). |
| BL097 | Recursos/identidade | Preservar arquivo reescrito em lugar com mesmo inode e tamanho antes do cleanup | Concluído | BL089 | Integração / coordenador | SHA-256 e metadata antes/depois; leitura/falha incerta mantém arquivo. Mutação do hash causa 3 falhas. Mesmos bytes e escritor concorrente após última conferência ficam fora da garantia; custo UI BL112  Gate final: [R1–R10](migration/round-r1-r10-report.md); [PR #4](https://github.com/journey-studios/tessel-editor/pull/4), merge `c099f10`; [gate final](migration/round-r1-r10-report.md). |

Patch de host R5: aplicado manualmente no follow-up de integração e coberto por 13 testes de MediaHost; o arquivo [next-r5-host.patch](migration/next-r5-host.patch) é referência histórica e não deve ser reaplicado. Não há prova visual específica de cleanup.

### Limites recebidos de R3/R4

- R3: IDs de gesto são opacos. Sal de processo é probabilístico; época distingue sessões do mesmo processo. Provas de orçamento de update público e commit white-box têm escopos distintos. Relatório: [next-r3-report.md](migration/next-r3-report.md).
- R4: arquivo/arquivo tem troca por rename único; pasta/troca de formato usa dois renames, com intervalo sem destino. O anterior é preservado em backup se rollback falhar e seu caminho vem no erro. Não declarar operação inteiramente atômica contra queda de processo. Relatório: [next-r4-report.md](migration/next-r4-report.md).

| ID | Área | Pendência | Estado | Dependências | Responsável | Aceite |
|---|---|---|---|---|---|---|
| BL098 | Pacote/recuperação | Recuperar replace interrompido entre renames e tratar resto de pacote anterior não apagável | Pendente | BL088 | Coordenador | Falha de processo deixa anterior identificável e recuperável; rotina segura não apaga backup/vizinho arbitrário nem oculta pacote incompleto; testar crash/rollback e restos parciais |

### Limite recebido da bancada Spark R7

| ID | Área | Pendência | Estado | Dependências | Responsável | Aceite |
|---|---|---|---|---|---|---|
| BL099 | Spark/lifecycle | Provar release durante decodificação após fetch, antes de ready | Pendente | BL091 | Coordenador | Gate determinístico durante decodificação, sem trocar por serial/skip/timeout maior; resposta cancelled e nenhum recurso retido. GPU real reusa BL054 e tem evidência separada de SwiftShader |

A correção R7 não altera runtime/src nem substitui provas de WKWebView ou GPU real. As taxas da bancada são observações sob carga variável, não garantia estatística de ausência de outras falhas.

### Limites recebidos da importação visual R2

| ID | Área | Pendência | Estado | Dependências | Responsável | Aceite |
|---|---|---|---|---|---|---|
| BL100 | F043/latência | Mover leitura/parse/validação pesada do attach para job sem bloquear UI | Pendente | BL086/core/media | Coordenador | Arquivo grande não congela UI; resultado aplica somente no projeto/revisão apropriados, com cancelamento/erro tipado; não duplicar validação no adapter |
| BL101 | F040/recursos vazios | Carregar cena sem arquivos externos sem exigir diretório inexistente | Pendente | BL010/BL086 | Coordenador | Manifesto sem dependências resolve sem root_unavailable; cenas que nomeiam arquivos continuam exigindo raiz e verificação; teste lib/headless/monitor |

R2 não faz attach+carga atômicos: em falha de recursos a cena anexada permanece para reparo/undo; isso é comportamento explícito, não fluxo completo bem-sucedido. A prova do painel nativo de arquivos e aparência fica pendente em BL087/BL043.
Falha observada em teste de slider/layout (arquivo R9 ainda WIP) foi encaminhada à frente responsável; não considerar crate desktop inteiro verde enquanto persistir. [Relatório R2](migration/next-r2-report.md).

| ID | Área | Pendência | Estado | Dependências | Responsável | Aceite |
|---|---|---|---|---|---|---|
| BL102 | F041/identidade IPC | Ecoar identidade de carga em asset.state e recusar ready atrasado de slot reutilizado | Pendente | BL092/runtime/host | Coordenador | Contrato aditivo compatível; teste injeta ready antigo após substituir carga no mesmo slot e prova que não desbloqueia o frame novo; ver next-r8-report.md |
| BL103 | F041/orçamento nativo | Exercitar mais de 32 recursos distintos com LRU e perda de contexto | Pendente | BL092 | Coordenador | Prova WKWebView com recursos acima do teto, contadores host/runtime coerentes e pixels atuais após evicção e restauração; corpus atual tem só 3 recursos |

| ID | Área | Pendência | Estado | Dependências | Responsável | Aceite |
|---|---|---|---|---|---|---|
| BL104 | F043/cabeçalho | Validar captura final em 960 px após compactar ações e ligar Anexar | Concluído | BL086/BL087 | Coordenador | Capturas finais selected-min-dark/light inspecionadas: cabeçalho sem sobreposição em 960 px. 50/50 capturas feitas, gate global passou; ponteiro físico não provado. [Relatório](migration/round-r1-r10-report.md).  [PR #4](https://github.com/journey-studios/tessel-editor/pull/4), merge `c099f10`; [gate final](migration/round-r1-r10-report.md). |
| BL105 | F039/automação | Expor rolagem semântica do inspector e árvore | Pendente | BL087 | Coordenador | CLI usa os scroll handles vivos, sem coordenadas; permite provas nativas da propriedade fora da vista sem duplicar estado |
| BL106 | F005/layout | Avaliar reutilização do layout compacto de propriedades no inspector de clipe | Pendente | BL087 | Coordenador | Conferir controls.rs::property_row em docks estreitos e compartilhar layout se o mesmo defeito for reproduzido; não alterar regras de domínio |

| ID | Área | Pendência | Estado | Dependências | Responsável | Aceite |
|---|---|---|---|---|---|---|
| BL107 | F037/JSON | Medir e implementar escritor direto do plano sem alterar formato | Pendente | BL064 | Coordenador | Mesmos bytes de to_json_string no corpus/golden, erros e versão preservados; medir ganho total sem confundir montagem do plano com export real; JSON domina a bancada R6 |
| BL108 | F037/bridge | Avaliar redução do plano enviado por frame | Pendente | BL064/BL008 | Coordenador | Medir bytes e custo no export real antes de decidir; fragmentos estáticos ou matrizes derivadas exigem contrato versionado e compatível; planos atuais chegam a 2,2 MiB por frame, não alterar formato sem prova de pixels |

| ID | Área | Pendência | Estado | Dependências | Responsável | Aceite |
|---|---|---|---|---|---|---|
| BL109 | F042/recursos | Proteger export contra mudança de bytes após aprovação do mapa | Pendente | BL008/F040 | Coordenador | Hoje só presença/tamanho são conferidos no export; testar troca de mesmo tamanho após resolve e durante job e definir cópia imutável ou verificação apropriada sem bloquear UI; mapa congelado não garante bytes congelados |
| BL110 | F042/manutenção | Compartilhar helpers de encode e resolução aprovados entre exports | Pendente | BL008 | Coordenador | Deduplicar encode_args/png_size/kill/read_tail/encode_error e avaliar approved_map com dyn ResourceResolver, preservando testes de ambos exports; evitar ampliar refactor sem necessidade |

### Aceitação independente R10 recebida

`migration/next-r10-report.md` e `scripts/directed-scene-export-proof/results.json` registram 23/23 fases com exit 0: export CLI 1080p/UHD/NTSC, socket, diálogo semântico, política de áudio, cancelamento, snapshot e pacote restaurado sem origem. Os binários foram reconstruídos antes da prova, mas novas edições posteriores tornam a repetição final obrigatória. Não prova ponteiro físico, roteamento node:16, preflight do diálogo ou GPU real. O follow-up integrou o patch do host e cobriu BL096/BL097 no escopo de um processo; o gate final e a repetição com binários finais seguem registrados na entrega integrada. Limites BL111/BL112 permanecem.

Follow-up de integração BL089/BL096/BL097 delegado ao Cursor Sonnet 5.5: <t3-thread://v2/8ac99a47-6712-4c08-aa4a-d8e73a792f19/thread:delegated-task:command%3Amcp%3Ad3856989-26f9-4933-b607-9945895abf30%3Adelegate-task%3Atessel-next-integration-cleanup-fix>. Inclui aplicar e provar PendingAdoption no host após R1. Coordenador reconferiu os 16/16 self-tests do harness de export R10; gate final após esse follow-up.

| ID | Área | Pendência | Estado | Dependências | Responsável | Aceite |
|---|---|---|---|---|---|---|
| BL111 | F040/multiprocesso | Coordenar ownership de recursos pendentes entre editores/processos | Pendente | BL096/BL079 | Coordenador | Registro atual é só em memória do processo; dois editores/CLI sobre o mesmo workspace não podem ter garantia de cleanup compartilhado. Persistir claims sob lease e provar com dois processos reais antes de declarar suporte |
| BL112 | F040/responsividade | Mover hash e descarte de resultados grandes para job fora da UI | Pendente — prioridade alta | BL094/BL097 | Coordenador | Medição debug: descartar 256 MiB levou 8,7 s; não executar leitura pesada no destrutor da UI. job.discard/limpeza assíncrona deve preservar ownership até terminar; medir release e garantir encerramento seguro |

### Gate integrado da rodada R1–R10

Concluído localmente: fmt/check/clippy, workspace 1027 testes verdes (8 ignorados), runtime85, builds debug/release. Repetição final: export23 fases/570 checks, lifecycle218, import33 e capturas50, com binários atuais. [Relatório e limites](migration/round-r1-r10-report.md). Validação local não significa CI, revisão ou merge. BL089 segue parcial por BL111/BL112; nenhuma pendência desses limites foi apagada.

### PR da entrega e bloqueio de revisão

[PR #4](https://github.com/journey-studios/tessel-editor/pull/4), implementação `f6311d2`, gate local acima. CodeRabbit pulou a revisão: 121 arquivos selecionados, teto de 100 e capacidade/créditos indisponíveis ([comentário](https://github.com/journey-studios/tessel-editor/pull/4#issuecomment-6669704948)). PR aberta, sem merge ou aprovação do serviço. Os limites BL111/BL112 permanecem pendentes. A configuração nova de filtros de evidências só entra para futuras execuções que a usem; a execução atual leu a base.

## Reconciliação após merge da PR #4

16 itens concluídos no escopo documentado: BL008, BL014, BL064, BL066–BL069, BL086–BL088, BL091–BL092, BL095–BL097 e BL104. Merge `c099f102d230271923684bc4e27a970a2bdfbe2a`. O CodeRabbit pulou a revisão e o usuário autorizou o merge; isso não foi aprovação do serviço.

BL089 continua parcial: claims são locais ao processo e hashing no descarte pode bloquear UI; BL094/BL111/BL112 continuam pendentes. Blocos amplos de domínios, recursos e migração continuam parciais conforme seus critérios. Relatórios de recebimento e registros de prova preservam o estado histórico anterior ao merge.

## Rodada S1–S10 autorizada — PR por frente

Plano e contratos: [wave-s1-s10-plan.md](migration/wave-s1-s10-plan.md). R1–R10 e follow-up encerrados, sem execução/filas antigas; resultados preservados. Novas frentes em worktrees isolados com Cursor Sonnet 5.5, PRs próprias e aceite somente após revisão CodeRabbit executada, correções e conferência do coordenador. SKIPPED não é aprovação.

| Frente | Itens | Estado | Gate de entrega |
|---|---|---|---|
| S1 | BL094/BL112 | Preparada para delegação | descarte assíncrono, sem hashing na UI |
| S2 | BL111 | Preparada para delegação | claims persistentes e dois processos reais |
| S3 | BL109 | Preparada para delegação | bytes imutáveis em export e cancelamento |
| S4 | BL102 | Preparada para delegação | ack antigo de slot reutilizado recusado |
| S5 | BL100/BL101 | Preparada para delegação | attach off-thread e recursos vazios |
| S6 | BL004/BL005 | Preparada para delegação | catálogo e pose inicial equivalentes à origem |
| S7 | BL025 (subconjunto) | Preparada para delegação | Spark dirigido em preview/pacote/export |
| S8 | BL015/BL042 | Preparada para delegação | gizmo de translação e comando semântico equivalentes |
| S9 | BL016 | Entregue para revisão (PR desta branch) | mix offline dirigido e áudio no export: core 20+4, mídia 15, CLI headless real, oráculo R10 17/17; janela e transporte pendentes (BL016.1–.4) |
| S10 | BL019 (ampliação) | Preparada para delegação | provas independentes e controles negativos |
