# Latência real — investigação e proposta (2026-11-10)

**Não aceita pelo usuário.** Merge de correções e prova sintética não são aceite de latência.
Este documento propõe a próxima solução; não declara essa solução implementada ou validada.

## Reprodução

Código `3e48861e9914ed2fd6c71e13c62c5673f068768f`, release construído com
`./scripts/cargo build -p tessel-editor --locked --release`, runtime reconstruído com setup.
Binário SHA-256 `c4103e7bce58e336ce582bedfda21abaccfcdabbb3f0344cd8f5333ff8b2b07d`.
Projeto real horizontal de 25 s, 23 assets presentes, quatro nuvens de 1 179 648 gaussianas,
títulos e áudio existentes. Só o documento foi copiado para a sessão isolada; fontes intactas.
Janela própria, comandos semânticos do harness `scene-warm-proof.py`, dois locks GPU,
`TESSEL_GL=gpu`. Nenhuma mídia privada foi adicionada ao repositório.

Comando executado: `python3 /tmp/tessel-latency-investigation/probe.py warm cold`.
Saída: `DIAGNOSTIC_RED True`, exit 1. Relatório bruto em
`/tmp/tessel-latency-investigation/real-report.json`. O gatilho de 60 ms é diagnóstico,
não meta aprovada pelo usuário. O harness original mede pedido→imagem aceita, não fóton.

| Corte (frame) | Sem aquecimento: pedido→aceite / intervalo de imagem | Com aquecimento: pedido→aceite / intervalo |
|---|---:|---:|
| 186 | 84,9 / 126,6 ms | 2,8 / 47,3 ms |
| 268 | 83,7 / 127,4 ms | 7,1 / 29,0 ms |
| 410 | 120,9 / 160,1 ms | 2,5 / 46,6 ms |

Com aquecimento ainda houve oito intervalos acima de 62,5 ms e máximo 128,8 ms.
Sem aquecimento foram quatorze, máximo 160,1 ms. Estes são um par exploratório,
com carga ~47, não uma distribuição de desempenho em máquina ociosa nem aceite.
O custo de CPU do runtime não é tempo de GPU.

## Onde investigar

No intervalo 138→140 de 128,8 ms, 96,7 ms ficaram entre o aceite anterior e o envio
seguinte; pedido→aceite foi 32,1 ms, dos quais o runtime reportou 11 ms de CPU.
As pausas em torno de 138, 220 e 362 coincidem com a janela de pré-carga de 48 frames
antes dos cortes 186, 268 e 410. Também aparecem sem `frame.warm`: não atribuir tudo à
ordenação antecipada. Isso é correlação temporal, não prova isolada da função responsável.

Retirada somente dos títulos: oito intervalos lentos, pior 172,8 ms. Retirada adicional
do áudio: um intervalo lento com warm (66,4 ms), oito sem warm (pior 284 ms).
Relatórios `no-titles-report.json` e `scenes-only-report.json` no mesmo diretório local.
A carga mudou ~47→70→105–121; não é válido atribuir essa diferença ao áudio nem comparar
os máximos como um A/B controlado. A reprodução reduzida ainda passa por nuvens reais.
O cenário mínimo causal não está completamente isolado.

**Correção da interpretação do início:** `start_ms` do harness inclui o espaço entre
uma imagem de frame 0 anterior e o primeiro tick; pode incluir espera do próprio ensaio.
Os ~0,9–1,2 s desse campo não comprovam latência ao apertar Play. Medir comando→primeira
imagem correspondente separadamente, sem subtrair clocks Python/Rust.

## Candidatos, em ordem e com previsão

1. Caminho de pré-carga/protocolo competindo com a janela: instrumentar tick, plans_ahead,
   despacho, início/fim de leitura de asset e callback IPC, com IDs e relógios coerentes.
   Predição: as pausas se alinham a uma dessas etapas antes do próximo pedido/aceite.
   Em `viewport.rs` há `with_custom_protocol` síncrono; `render::host::serve` faz
   `File::open`, seek e `read_to_end`. Isso identifica um caminho síncrono, mas **não prova**
   que ele consuma os intervalos observados. O Wry instalado oferece protocolo assíncrono.
2. Carga externa/escalonamento: repetir pares alternados e registrar carga/concorrência;
   previsão: atrasos de admissão/transporte variam sem aumento equivalente do CPU do runtime.
3. Ordenação/upload antecipado no runtime: correlacionar início/fim de warm/decode/upload,
   sem chamar flush de medida de GPU. Previsão: desligar apenas warm elimina pausas relacionadas
   ao warm; as atuais pausas de pré-carga nos dois braços já enfraquecem uma explicação só por warm.

## Solução proposta: proteger o quadro corrente

**Primeira entrega, com controle causal:** instrumentar as etapas acima e fazer um protótipo
com protocolo assíncrono e I/O em workers limitados (fila limitada, sem thread por pedido),
preservando autorização de assets, Range, cancelamento, erros e identidade projeto/sessão.
Comparar protocolo síncrono/assíncrono no mesmo build e projeto, em ordem alternada. Se o
atraso não se mover como previsto, não promover esse protótipo como correção causal.

**Segunda entrega, guiada pelo traço:** se plans_ahead/tick forem relevantes, preparar e
cachear planos dos próximos cortes por revisão/cena em worker. O tick só consulta planos
prontos; resultado de revisão/sessão antiga é descartado. Priorizar o quadro corrente;
carregar/decodificar/ordenar em segundo plano com concorrência e orçamento limitados.
Não simplesmente aumentar o lookahead: isso pode só deslocar a pausa no tempo.

**Terceira entrega, se necessária:** upload em parcelas que cedem execução, priorização da
carga do frame atual e aquecimento antecipado compatível com orçamento/owners. O export e o
refino final continuam exatos. Não prometer redução de pico de memória nem usar ordem velha
como imagem exata para esconder atraso.

## Critérios propostos para julgar a solução

- Gravar cada entrada e cada intervalo, com p50/p95/máximo, faltas e quadros perdidos;
  separar tempo até pedido, pedido→aceite e aceite→apresentação. Nunca aprovar pela média.
- Proposta inicial de orçamento: um quadro a 24 fps corresponde a 41,7 ms. Usar isso como
  referência explícita de projeto; o usuário ainda não aprovou uma meta numérica de aceite.
- Nenhuma troca de cena deve ganhar uma pausa equivalente dois segundos antes dela.
  Avaliar o play completo, títulos, áudio e crossfades, não só o primeiro frame dos cortes.
- Pelo menos quatro pares alternados válidos, mesma build, projeto real e condições comparáveis,
  com execuções frias e quentes. Registrar resultados sob carga separadamente dos de máquina quieta.
- Imagens finais/refino/export iguais com controles negativos; cancelamento e troca de projeto
  não podem aplicar resultado antigo. Medir memória mediana **e pico**, RSS e limites da estimativa.
- Scripts semânticos em janela própria; core sem GPUI/Wry; testes e builds apropriados ao código
  realmente alterado. Sem exigir aceite humano/hardware que não foi executado.

Nada de produção foi modificado nesta investigação. Não foi rerodada a suíte completa: o trabalho
executado aqui foi build release, reprodução real e minimização parcial, não implementação.
