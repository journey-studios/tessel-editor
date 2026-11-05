# Pendências de otimização de timing e apresentação

> Novas atribuições e mudanças de estado ficam no [backlog compartilhado](backlog.md),
> BL031–BL035 e BL043–BL045. As tabelas abaixo preservam o histórico técnico de OPT01–OPT10.


Estado consolidado em 2026-11-05 (atualizado após a execução de [viewport-performance-next-report.md](viewport-performance-next-report.md)). Este é o índice das próximas melhorias e validações;
**nenhum item desta lista deve ser tratado como implementado apenas por estar documentado**.
Plano de execução aprovado: [próximas otimizações](viewport-performance-next-plan.md).
As operações de preview pertencem a F021/F007; cada implementação atualiza esses contratos
e a matriz de features, com evidências por biblioteca, CLI e UI.

## Base já entregue

Coalescing de seeks, caminho interativo sem hash/readback, retenção limitada de texturas,
materiais/programas e texto, rigs sem splat, alvos por camada, gerações/invalidação,
telemetria e benchmarks em release estão entregues. A correção posterior de vídeo também
adicionou JPEG em sequências e mediu PNG 540p. Não repetir esses itens como pendências.

Fontes atuais: [implementação](viewport-performance-implementation-report.md),
[correção de vídeo/contexto](viewport-bugfix-report.md),
[contrato da viewport](features/viewport.md) e [seek](features/seek.md).
Os pareceres do Opus medem snapshots/protótipos anteriores; não substituem aceite do código final.

## Backlog priorizado

| ID | Prioridade | Item | Estado | Critério de aceite |
|---|---|---|---|---|
| OPT01 | Alta | Escolha automática de proxy para preview/scrub | **Entregue (2026-11-05)**: política `auto`/`original` (`op preview.policy`, nó `monitor.preview.quality`), job de proxy 540p JPEG/PNG, refino e export com original. Arrasto 2 s a 60 Hz em 1080p: 67 → 120 quadros, atraso 3 → 0 quadros. Evidência: [relatório](viewport-performance-next-report.md#opt01-proxy-automático-de-previewscrub) | Mesmo tempo, enquadramento e proporção do original; melhora p95 de seek/miss em vídeos representativos; troca para original/frame final sem imagem obsoleta; export usa original |
| OPT02 | Alta | Cache de render target de Spark parado | **Entregue (cena parada)**: cache do resultado com chave completa; 1M 35 → 2 ms drenado, 3M 68 → 3 ms; em movimento nada muda. [Relatório](viewport-performance-next-report.md#opt02-resultado-de-cena-spark-parada) | Cache por camada com geração e orçamento em bytes; invalidar por câmera, tempo relevante, animação, asset, efeitos e contexto; pixels iguais ao caminho sem cache, inclusive múltiplas cenas |
| OPT03 | Alta | Backpressure alinhado à apresentação | **Medido e removido**: sem ganho de atraso/intervalo na janela real (gate por `frame.presented`). [Relatório](viewport-performance-next-report.md#opt03-backpressure-pela-apresentação-medido-e-removido) | Menos frames calculados sem oportunidade de apresentação, sem piorar latência/convergência; seguir a cadência real, sem teto universal de 60; não travar janela oculta, reload ou render offline |
| OPT04 | Média | `notify`/redesenho seletivo do GPUI | **Entregue**: `cx.notify` só quando cabeçalho/erro mudam; contadores em `render.status.ui`; sem alegação de FPS. [Relatório](viewport-performance-next-report.md#opt04-repaint-seletivo) | Reduzir redraws sem mudança visual e medir ganho em release; cabeçalho/status/seleção continuam atualizando; sem substituir medições por supressão de UI |
| OPT05 | Média | Primeiro frame e recursos frios | **Medido, nada mantido**: o pico de ~400 ms é decode/alvo do primeiro original 1080p, programas já compilados; sem experimento que se pague. [Relatório](viewport-performance-next-report.md#opt05-primeiro-frame-e-recursos-frios) | Separar load/decode/upload/compilação em spans; experimento de preparação/compilação antecipada só se medido; preservar diagnóstico de falha e controlar custo/memória/tempo de abertura |
| OPT06 | Média | Decode/cache miss de vídeo | **Resolvido no scrub pelo proxy** (36 → 8 ms/quadro); prefetch medido e removido. [Relatório](viewport-performance-next-report.md#opt06-decode-e-cache-miss) | Medir vídeo real, GOPs, resoluções e ordens de seek; reduzir miss com orçamento de memória/CPU; sem prefetch que concorra com o frame atual, quadros incorretos ou perda de cancelamento |
| OPT07 | Média | Spark em movimento: LOD/MSAA e sort | **Gancho opt-in** (`TESSEL_VIEWPORT_SCENE_SAMPLES`): 1M órbita 35 → 27 → 21 ms com qualidade de borda pior; padrão inalterado; LOD/sort em movimento **não feitos**. [Relatório](viewport-performance-next-report.md#opt07-spark-em-movimento) | Matriz de nuvens/câmeras/overdraw; ganho sustentado com qualidade explicitada e comparação de frames; preview pode ter política própria, frame final/export mantém qualidade configurada; sem promessa universal de 1–4 ms |
| OPT08 | Validação | Seek-to-photon | **Pendente (hardware)**: método publicado, nada medido. [Relatório](viewport-performance-next-report.md#opt08-seek-to-photon-pendente) | Câmera de alta velocidade/procedimento humano com cena sintética; publicar resolução temporal, amostras e limites; ack, rAF e captura de tela não contam como pixel físico |
| OPT09 | Validação | Spark 3M em UHD 4K, inclusive SwiftShader | **Parcial**: 3M/4K em WebKit (1 passada, orçamento declarado) e soak de 480 s estável; SwiftShader/UHD export e determinismo 3M **não rodaram**. [Relatório](viewport-performance-next-report.md#opt09-3m4k-e-longa-duração) | Budget explícito de disco/memória/tempo; verificar pixels/determinismo, cancelamento e consumo; registrar plataformas e casos que não rodaram |
| OPT10 | Validação | Série longa e outros displays/cargas | **Pendente**: só uma tela; método publicado. [Relatório](viewport-performance-next-report.md#opt10-outros-displays-e-cargas-pendente) | Ensaios repetidos em release, quente/frio, carga documentada e timestamps por frame; p50/p95/p99, memória estável e cadência de displays diferentes; não medir polling como FPS |

OPT01/OPT06 devem avaliar **PNG e JPEG** suportados pelo core atual. A medição anterior de
JPEG 540p não comprova automaticamente o comportamento de proxies PNG nem de vídeos reais.
O PNG 540p já foi medido em B7; a política automática do produto foi integrada e validada em OPT01 (JPEG e PNG, ver o relatório).

OPT02 não é a retenção de alvos por camada já entregue: é reutilizar o **resultado renderizado
de uma cena com splat estática**, sem executar novamente o Spark quando nada relevante mudou.

## Ideias que não são trabalho obrigatório pendente

- Reusar apenas `Scene`, câmera, geometria ou `setSize`: sem ganho isolado relevante medido.
- Grafo de cena incremental: não justificado pelo gargalo encontrado.
- Prefetch indiscriminado: refutado como melhoria geral; pode piorar decode/latência.
- Trocar Three.js/Spark por outro motor: sem evidência que justifique essa migração.
- Remover espera apenas para reduzir o indicador: não prova ganho de GPU ou de apresentação.

Essas ideias só voltam ao plano com hipótese nova e experimento que demonstre benefício.

## Limitações de correção relacionadas

O caminho nativo direto de vídeo tem contrato CFR/PTS zero e confia nos metadados declarados;
vídeo estático pode ser recusado pela heurística de prova de leitura. O import via FFmpeg é
o caminho que verifica/conforma a mídia. A leitura nova com janela oculta não foi medida.
Esses limites estão no [relatório de bugs](viewport-bugfix-report.md); não marcar como
resolvidos por otimizar cache. O timeout de seek de vídeo também precisa de regressão específica
se ainda não coberto no cenário final adotado.

## Regra de conclusão

Para cada item: reproduzir baseline → experimento isolado → implementação com invalidação e
limites → testes de correção/pixels e falha → benchmark do build final → atualizar status e
contratos. Registrar tempo de CPU, limite de GPU drenada, bridge, aceitação e apresentação
separadamente. Não promover resultado de protótipo a prova de produto. Guardar dados compactos
e proveniência no repositório, sem depender só de `/tmp`.
