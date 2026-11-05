# Plano de execução — próximas otimizações de timing

Data: 2026-11-05. Escopo aprovado: executar os pontos de
[viewport-performance-backlog.md](viewport-performance-backlog.md), OPT01–OPT10.
Estado inicial: **planejado**. A implementação e as provas finais continuam pendentes.

## Objetivo e invariantes

Melhorar tempo de resposta e uso de recursos em preview, mantendo correção de frames,
qualidade final e execução programática. Não perseguir apenas o número do indicador.
Three.js/Spark permanecem; não portar motor nem criar grafo incremental sem evidência.

- Core define intenção, tempo e política de preview; adapters executam decode/render.
  UI, CLI e biblioteca usam o mesmo contrato e podem inspecionar política ativa e recursos.
- Preview pode usar proxy durante gesto/reprodução conforme política explícita; ao parar,
  converge para original/qualidade final. Export e PNG explícito usam original por padrão.
- Cena parada não é sinônimo de playhead parado: considerar animações, efeitos, câmera,
  recursos dinâmicos e tempo local das fontes para qualquer reutilização de resultado.
- IDs, geração de asset/projeto/contexto, tamanho, parâmetros e qualidade entram na
  identidade dos caches. Limites em bytes, evicção e ownership são obrigatórios.
- Resultado antigo não substitui intenção atual. Preservar coalescing do scrub, erros,
  cancelamento, timeout e requests explícitos com probes/hash/PNG.
- Dados e assets do usuário e projetos de origem ficam intactos. Testes usam fixtures
  próprias e instâncias isoladas, sem fechar ou substituir a janela/projeto do usuário.

## Etapas e ordem

| Etapa | Backlog | Entrega | Depende de | Prova de saída |
|---|---|---|---|---|
| T00 | Todos | Snapshot/fingerprints, baseline release, orçamento e testes existentes | Estado atual | Baseline reproduzível, versões e métricas rotuladas |
| T01 | OPT01/OPT06 | Política automática de proxy, seleção de fonte e integração de geração em jobs | T00 | p95 de seek/miss melhora; mesmo frame/tempo; frame final e export originais |
| T02 | OPT02 | Cache do resultado de Spark estático, por camada | T00 | Reuso medido, pixels iguais; invalidação/evicção/contexto e duas cenas corretos |
| T03 | OPT03 | Backpressure conforme cadência real da página | T00 | Menos trabalho descartado/não apresentável sem piorar convergência; testes de ocultação |
| T04 | OPT04 | Notificações GPUI seletivas | T03 | Menos redraws em release; estado visual e comandos continuam atualizando |
| T05 | OPT05 | Experimentos de primeiro frame e preparação antecipada | T01/T02 | Ganho frio medido sem apenas transferir custo para abertura; memória controlada |
| T06 | OPT06 | Ajustes de decode/miss e prefetch somente se justificado | T01 | Ganho em casos reais/representativos, sem competir com quadro solicitado |
| T07 | OPT07 | Sort/LOD/MSAA de Spark em movimento | T02 | Perfil mensurado e política de qualidade explícita; frame final/export completos |
| T08 | OPT09/OPT10 | Matriz pesada, duração estendida e telas/cargas disponíveis | T01–T07 | Correção + recursos + p50/p95/p99 do código final, com casos não executados identificados |
| T09 | OPT08 | Prova de latência física ou procedimento de medição | T08 | Seek-to-photon só com captura física adequada; se não disponível, status pendente |
| T10 | Todos | Regressão integrada, docs e relatório final | Etapas executadas | Critérios por item e por superfície, sem confundir protótipo com produto |

## T01/T06 — vídeo e proxies

1. Inventariar o pipeline atual de import/decode/proxy e evitar uma segunda implementação.
2. Policy de preview consultável: original/proxy/automático, resolução/tamanho máximo,
   qualidade/formato, memória e motivo da seleção. Automático durante gesto é a primeira
   hipótese; comparar PNG e JPEG suportados pelo core, inclusive trade-off de qualidade.
3. Derivados vinculados ao original e sua revisão, com índice de tempo correto. Não
   depender do cwd nem exigir `media.prepare` manual escondido do usuário.
4. Gerar/obter proxies fora da thread UI, com cancelamento, progresso e cache limitado.
   Se proxy ainda não existe, definir comportamento explícito e responsivo, não bloquear.
5. Seek rápido, reversão e término de gesto não deixam frame proxy obsoleto na tela;
   export, requests de medição em tamanho final e playback usam a fonte adequada ao contrato.
6. Medir hit/miss, primeira abertura, GOPs/formatos, 540p/720p/1080p/4K conforme capacidades.
   Prefetch é hipótese condicionada à vazão; não ativar indiscriminadamente. Não alterar o
   mapeamento de tempo como otimização. Preservar os limites de vídeo nativo de B1–B4.

## T02/T07 — Spark

1. Reutilizar o resultado de uma camada somente quando sua imagem não pode ter mudado.
   Chave inclui câmera, assets/gerações, parâmetros, estado de animação e efeitos, tempo
   relevante, tamanho, alpha/composição, qualidade e contexto. Camadas independentes têm RTs próprios.
2. Orçamento em bytes e política de evicção, resize, troca de projeto e perda de contexto;
   recursos ainda usados por frame em andamento não podem ser liberados por outra solicitação.
3. Comparar com cache desligado em tempos fora de ordem, cenas múltiplas, assets com mesmo
   ID substituídos, animação parada/ativa, câmera alterada e cenários de erro/cancelamento.
4. Em movimento, medir sort, overdraw, draw e espera; distinguir CPU e limite de GPU drenada.
   LOD/MSAA têm política visível e programática; manter qualidade original como opção.
   Não reduzir silenciosamente a qualidade de export para atingir uma meta de preview.
5. Testar nuvens sintéticas e representativas 100k/1M/3M, diversas câmeras; nenhum alvo
   universal de 1–4 ms para Spark em movimento é requisito de aceitação.

## T03/T04 — cadência e UI

Medir cadência real por rAF/timestamps e oportunidades de apresentação, sem limite fixo de
60 Hz para todas as telas. Separar acks de conclusão, eventos de rAF e frame fisicamente
mostrado. Os clocks de processos diferentes têm incerteza e não podem ser subtraídos sem controle.

O gate interativo precisa de caminho seguro para página oculta/minimizada, diálogos, resize,
contexto perdido e timeout. Render offline/PNG explícito não espera uma tela visível.
Testar notificações seletivas por alteração visual, garantindo atualizações de status,
erros, cabeçalho, seleção e ações CLI. Rejeitar experimento sem benefício consistente.

## T05 — frio

Instrumentar carga, decode, upload, compilação e primeiro frame. Testar preparação
antecipada apenas com prazo/cancelamento, geração e escopo de recursos. Relatar tempo de
abertura total e primeiro uso, não apenas mover o custo para antes do temporizador.
Manter falhas observáveis; não pré-carregar todo acervo ou compilar combinações ilimitadas.

## Protocolo de medição e limites

- Release é referência de produto; debug é diagnóstico separado. Registrar hashes do código,
  binários e bundle, motores/versões, máquina/GPU/tela/carga, resolução e configurações.
- Quente/frio separados; A/B/A em várias passadas. Entrada e observação independentes:
  timestamp por frame, coleta fora do gesto e testes de taxas 30/60/120 Hz quando viável.
- CPU, decode/upload/sort, GPU drenada (com overhead declarado), bridge, accepted, rAF e
  captura física são métricas distintas. Reportar média/p50/p95/p99/máximo e memória/disco.
- Executar testes de correção antes de comparar velocidade; incluir export curto com áudio,
  frames finais/ffprobe, cancelamento e snapshot imutável. Preservar regressões existentes.
- Ensaios longos e 3M/4K precisam de estimativa de recursos. Proposta operacional inicial:
  até 3 GiB de artefatos próprios por rodada e piso de 6 GiB livres; não iniciar carga que
  ultrapasse o orçamento. Se espaço cair, encerrar só testes próprios e limpar só artefatos próprios.
- Não executar todas as suítes pesadas ao mesmo tempo; conferir disco/carga antes de build.
  Não remover caches globais, target gerenciado ou arquivos do usuário para viabilizar teste.
- Se outra tela, máquina ou câmera física não estiver disponível, publicar o procedimento
  e a lacuna; simulação rAF não comprova outro monitor nem seek-to-photon.

## Documentação e entrega

Criar/atualizar os contratos de política/proxy/cache/presentação em `docs/features/` e
matriz, preservando IDs existentes. Atualizar o backlog OPT01–OPT10 com evidência por item:
implementado, validado, experimento sem ganho, bloqueado ou pendente. Esta implementação
não deve reescrever resultados históricos como se tivessem sido obtidos no código novo.

Relatório novo: `docs/viewport-performance-next-report.md`, com arquivos, APIs/CLI reais,
configurações de UI, baseline, resultados compactos duráveis, testes, builds disponíveis,
ganhos confirmados e limitações. Artefatos pesados ficam fora do Git e têm cleanup próprio.
Conclusão exige fluxo lib/CLI/headless/UI; não basta uma flag interna do runtime.
Sem commit, push, publicação ou migração adicional do motor nesta entrega.
