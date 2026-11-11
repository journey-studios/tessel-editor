# Experimento de latência real — BL157.1

Estado: plano preparado em 2026-11-10; **experimento executado em 2026-11-10** (branch `t3/async-viewport-io-latency`,
BL157.1). Resultado, hipóteses confirmadas/refutadas e limites: [relatório](../viewport-real-latency-ab-report.md);
contrato do que foi construído: [F063](../features/viewport-asset-io.md).
Registro de execução/status: [backlog BL157.1](../backlog.md).
Base observada da investigação: `3e48861e9914ed2fd6c71e13c62c5673f068768f`.
Antecedentes e traços: [investigação](../viewport-real-latency-investigation.md).
A latência permanece **não aceita**. Nenhum limite diagnóstico equivale a aceite do produto.

## Pergunta e previsão falsificável

O atendimento síncrono dos assets pelo protocolo da WebView atrasa a janela durante a pré-carga?
Se esse mecanismo for relevante, deslocar somente a leitura/resposta para workers deve reduzir
as pausas alinhadas à chegada dos assets, sem aumentar a espera do quadro atual nem mover o custo
para outro trecho do play. Se os spans de leitura não coincidirem com as pausas, ou o A/B não
mostrar esse efeito, rejeitar essa explicação como causa principal e investigar a próxima etapa.

Não pressupor a thread em que Wry executa o callback: medir a identidade da thread e sua coincidência
com a thread da UI antes de atribuir um bloqueio diretamente ao event loop GPUI.

## 1. Preparar um ensaio reproduzível

- Trabalhar no checkout isolado; preservar a raiz suja e mídia original. Reconfirmar main e
  diferenças relevantes desde a base acima; capturar SHA, árvore modificada, toolchain e parâmetros.
- Versionar o runner derivado da reprodução local, recebendo `--project` por argumento, sem
  caminho pessoal fixo, mídia ou metadados privados no Git. Saída local identificada por SHA/hash.
- Usar o projeto real de 25 s com seus 23 assets; validar presença e identidade das fontes.
  Abrir cópia somente do documento em workspace próprio, referenciando fontes só para leitura.
- Fonte real completa é o caso principal. Variantes sem títulos e sem áudio são controles de
  isolamento, não substitutos do caso principal nem resultados misturados na mesma estatística.
- Registrar os cortes pela timeline, não fixá-los cegamente: o projeto investigado tinha
  cortes em 186/268/410 e início da janela de pré-carga em 138/220/362.
- Janela/HOME/socket próprios, só semântica `scripts/ui` e handlers reais; não operar a janela
  do usuário. Dois locks GPU, espera limitada e erro BLOCKED se indisponíveis.
- Usar `./scripts/runtime setup` e `./scripts/cargo` com target padrão sob mbx; não definir
  target-dir, não limpar cache nem executar purge do sistema. Reserva 16 GiB/piso 12 GiB.
  Artefatos temporários próprios abaixo de ~500 MB; remover saídas grandes ao terminar.

## 2. Instrumentar antes de otimizar

Ativar traços somente no experimento, com custo idêntico nos dois braços e sem flush por evento.
Um buffer limitado registra spans e IDs; exportar ao terminar, sem pedir frames para medir.

| Etapa | Informação necessária |
|---|---|
| Tick de playback | instante previsto e executado, frame esperado, atraso do tick |
| Preparação | duração de plans_ahead, revisão, plano/corte e origem da requisição |
| Protocolo de assets | ID de request/asset não sensível, thread, Range, bytes, fila, leitura e resposta |
| Runtime | fetch/decode/upload/warm: início/fim, camada/asset, cancelamento e erros |
| Quadro | submissão, aceite e apresentação quando observável, frame pedido/mostrado |

Usar relógio monotônico do host para etapas Rust. JS informa durações locais e IDs correlacionáveis;
não subtrair timestamps JS/Rust/Python como se compartilhassem origem. Se não houver sinal real
para apresentação, rotular como aceite/submissão à apresentação, sem alegar latência até o fóton.

Separar intervalo normal de 41,7 ms a 24 fps do atraso adicional. O tempo entre aceite anterior e
pedido seguinte não é todo trabalho bloqueante: pode conter espera normal pelo próximo frame.
O campo start_ms antigo inclui espera do harness. Para Play→primeira imagem, usar marcação host
do comando e da imagem correspondente; até existir, não reportar esse número como startup.

## 3. Protótipo com uma variável de controle

Uma única build release oferece dois modos diagnósticos: protocolo síncrono atual e protocolo
assíncrono. O modo não é nova configuração de produto nem uma implementação já entregue.
Ambos reutilizam o mesmo serve/validação de assets e o mesmo runtime. Warm fica ligado nos dois.

No assíncrono, callback retorna sem esperar o I/O. Workers e fila são limitados, sem thread por
request e sem envio bloqueante na UI. Ponto inicial de engenharia: 2 workers e 16 pedidos em fila;
fixar esses parâmetros antes da medição e registrá-los. Não ajustá-los para cada braço ou corrida.
Saturação deve produzir erro explícito/testável, nunca descartar pedido silenciosamente; qualquer
saturação/erro torna a corrida funcionalmente inválida, não uma vitória de desempenho.

Preservar exatamente bytes, Range/Content-Range, status e autorização do caminho atual. Não alterar
limites de resposta/chunk, lookahead, pool de splats, MSAA, resolução, áudio, prioridade ou cache
no mesmo protótipo. Mudanças nessas regras são experimentos separados.

Cancelamento, encerramento e troca de projeto/sessão invalidam o resultado antigo. Workers encerram
sem prender a UI e sem responder duas vezes; não contabilizar fila velha no segmento novo.
Não introduzir serviço/porta de rede, bibliotecas novas ou dependência GPUI no core.

## 4. Testes funcionais e controle negativo

Antes do benchmark:

1. Mesma requisição válida nos dois modos retorna mesmos bytes/status/headers relevantes.
   Cobrir arquivo completo, Range parcial/sufixo/inválido, arquivo vazio, sumido e acesso recusado.
2. Testar fila cheia, cancelamento, projeto substituído com I/O em voo, encerramento e falha de
   leitura. Verificar limites de workers/fila/bytes retidos e ausência de resultado aplicado fora
   da sessão dona. Não afrouxar autorização para facilitar a prova.
3. Injetar atraso determinístico em uma leitura de teste: no modo síncrono ele precisa ser
   visível no sinal de atraso da janela; no modo assíncrono o pulso/ação semântica da janela deve
   continuar antes de a leitura terminar. Coordenar com barreiras, evitando só um sleep frágil.
   Esse controle prova o mecanismo, **não** que o disco seja a causa no projeto real.
4. Conferir pixels de quadro exato/refino/export e câmera iguais entre os modos. Qualquer imagem
   ausente, antiga, incorreta ou erro de carga invalida o braço, mesmo que pareça mais rápido.
5. Executar fmt/check/clippy/workspace/builds pertinentes ao código final, core tree sem GPUI/Wry,
   runtime Chrome sem skips e prova headless aplicável. Socket de janela não é headless.

Não fazer A/B com build quebrada ou reutilizar binário/log anterior à alteração.

## 5. Comparação no projeto real

- Quatro pares válidos, ordem A/B, B/A, A/B, B/A, mesma build e runtime; A=síncrono, B=assíncrono.
  Total: oito sessões isoladas. Conservar hashes e configuração de cada sessão.
- Em cada sessão: primeira reprodução completa e uma repetição para o cache residente.
  Comparar primeira com primeira e repetição com repetição; publicar separadamente.
- Processo novo não garante disco frio: não usar esse rótulo sem medir. Não purgar cache do SO.
- Rodar primeiro em condições comparáveis sem cargas pesadas desta tarefa em paralelo; registrar
  carga, CPU e atividade concorrente. Não matar processos alheios para obter números melhores.
- Corridas sob carga externa alta ficam visíveis em grupo separado. Não descartá-las porque
  tiveram resultado ruim nem misturá-las a uma suposta medição ociosa. Se condições mudarem
  muito, classificar o contraste como inconclusivo e declarar o motivo.
- Condições adversas controladas entram depois, em lote separado, com a mesma carga nos braços.
  Não repetir o custo inteiro só por uma alteração documental.

Medir por corte, por janela de pré-carga e por reprodução completa:

- p50/p95/máximo dos intervalos entre imagens e de pedido→aceite; cada falta/quadro perdido;
- número e localização dos intervalos >62,5 ms e >100 ms, como indicadores diagnósticos;
- atraso do tick, fila de I/O, duração/bytes da leitura, runtime e trânsito;
- RSS editor e processos auxiliares identificados, mediana e pico; memória GPU somente se
  medida por instrumento apropriado. Não chamar soma de RSS de memória física exclusiva;
- erros/cancelamentos, requests obsoletos e duração até primeira imagem após Play, se instrumentada.

Publicar os valores por corrida/entrada e diferenças pareadas. Frames da mesma corrida são
correlacionados; milhares de frames não são milhares de experimentos independentes. Quatro
pares são um teste de engenharia inicial, não garantia estatística de produção.

## 6. Decisão após os testes

**Seguir com o protocolo assíncrono:** só se as provas funcionais passarem e o contraste real
mostrar redução consistente das pausas associadas ao protocolo, em condições comparáveis,
sem transferi-las para cortes, outros frames ou filas; memória e erros devem continuar limitados.
Publicar magnitudes por corrida, não apenas uma média global ou um máximo escolhido.

**Rejeitar como solução principal:** se leitura/callback não explicarem as pausas ou não houver
melhora consistente. O protótipo não vira correção de latência só por ser assíncrono.

Próximo experimento conforme o span dominante, sempre uma variável por vez:

- plans_ahead/tick: cache por revisão dos planos de corte e preparação fora do tick;
- decode/upload: trabalho em parcelas, priorizando o quadro atual;
- warm/sort: orçamento e prioridade da ordenação antecipada, sem perder o warm útil no corte;
- escalonamento externo: medir sessão de condições comparáveis e separar efeito do produto
  daquele do ambiente; não prometer resolver carga arbitrária de outros processos.

**Aceite do produto continua separado:** 41,7 ms/frame é referência de orçamento, e 60/62,5 ms
não são limites aprovados pelo usuário. O relatório final deve mostrar a experiência completa
no projeto real e propor metas explícitas com os dados obtidos. Não fechar BL157 por este plano,
pelo merge anterior, por uma média, nem por provar apenas o fixture sintético.

## Entrega esperada

Runner versionado, controles negativos, um relatório A/B com dados brutos locais não sensíveis,
gráficos de intervalos por frame e spans da pré-carga, tabela por corrida/entrada, hipóteses
confirmadas/rejeitadas e recomendação. Atualizar contrato/matriz e somente BL157.1 no checkout
isolado. Nenhum merge/publicação faz parte desta etapa de planejamento.
