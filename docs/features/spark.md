# F030 — Gaussian splats com Spark 2.4.0 (P04)

## Status

Lib/CLI headless **V** (Chrome real); UI **V** (WKWebView real, `scripts/live-proof.py`).
Spark **2.4.0** fixado em `runtime/package.json` com Three.js 0.187.0. Sem CDN: o pacote vem de
`node_modules` e o servidor de assets só entrega arquivos registrados.

## Contrato

- Formatos: `.splat`, PLY Gaussian e `.spz`. O core identifica **pelo conteúdo**
  (`crates/core/src/splat.rs`); `meta.format` (`splat|ply|spz`) vai ao runtime, que o converte no
  nome de arquivo que o Spark exige (`runtime/src/splat-format.js`). PLY de pontos comum é
  rejeitado (a mensagem lista as propriedades Gaussian ausentes), PLY comprimido/ASCII também; nada é aceito como splat por extensão.
- Estágio (`runtime/src/splat-stage.js`): um `SparkRenderer` por contexto WebGL
  (`minSortIntervalMs: 0`, `autoUpdate: false`). Um frame só é devolvido depois do **settle**: o
  loop chama `spark.update` até o sort terminar e o centro ordenado coincidir com a câmera do
  pedido (tolerância 1e-4, máx. 400 ordenações iniciadas por este quadro e 60 s de espera por ordenações que outro quadro deixou
  rodando; esgotar falha com `render_failed`). O `settle` só chama `update` com o sort ocioso: cada `update` suja o sort de novo, então
  chamá-lo com um sort rodando mantém outro na fila e a ordem nunca ficaria ociosa para a câmera pedida. Quadro de movimento (BL130)
  não espera: ver a seção abaixo. **Um sort que falha é uma falha do quadro** (BL130.5): ver "Ordenação que falha".
- Malhas emprestadas (BL125, BL131): cada uso de um asset num quadro (objeto de cena, nó de cena dirigida) recebe uma
  `SplatMesh` do asset (`AssetStore.splatLease`) com a própria transformação. Todas leem os **mesmos** `packedSplats`,
  decodificados uma vez: `SplatMesh.dispose()` descarta os dados compartilhados, então só a malha que os decodificou
  (`pool.source`) os descarta, junto com o asset; as demais soltam a referência (`disposeSharedSplatMesh`). Ao fim do quadro
  a malha volta ao asset com a transformação reiniciada (`resetLentMesh`) e serve o quadro seguinte: até
  `LIMITS.splatMeshes` (16) sobressalentes por asset (o excedente é criado e liberado com o quadro, como antes), de modo que
  um crossfade sobre o mesmo asset de efeito não decodifica nem cria gerador/programa Spark por quadro. `hello.retain: false`
  volta ao caminho que libera as emprestadas ao fim do quadro. Perda de contexto, `asset.release` e recarga descartam o
  estágio e **todas** as malhas do asset (também as que estavam emprestadas, que se liberam ao voltar); o primeiro empréstimo
  depois reconstrói dos bytes guardados com **uma** decodificação por asset, não uma por empréstimo. O contador `cache` do
  quadro informa `splat_decoded`, `splat_mesh_created` e `splat_mesh_reused`.
- Load cancelado (`asset.release` durante o load) responde `cancelled`; decodificação inválida ou
  com 0 splats responde `decode_failed` com o nome do asset.
- `capabilities.runtime.spark` informa a versão carregada.

## Evidência

- `./scripts/runtime test`: `runtime/test/splat.test.mjs` (Chrome real) com encoders de
  `.splat`/`.ply`/`.spz` — contagem de splats e cor idêntica nos três contêineres, ordenação por
  câmera (frente/trás) com visita em ordens opostas e hash igual bit a bit, splats translúcidos
  (mistura dependente da ordem), movimento por tempo (0→2→1→0 repete o frame), oclusão por caixa
  opaca da cena, o mesmo asset duas vezes na cena, arquivo malformado (`decode_failed`), load
  cancelado (`cancelled`, nada fica para trás) e perda/restauração de contexto (mesmo frame).
- `runtime/test/splat-shared.test.mjs` (BL131, Chrome real): duas cenas de um crossfade e vários objetos de uma cena leem um
  asset de efeito em posições, escalas e `spin` diferentes; depois da carga nenhum quadro decodifica de novo nem constrói malha
  (as emprestadas voltam ao asset); página quente e página fria desenham os mesmos bits em duas ordens de visita; soltar um uso
  não toca os dados dos outros e soltar o asset leva todas as suas malhas; perda de contexto reconstrói com uma decodificação por
  asset e o quadro seguinte é igual ao anterior; uma malha devolvida volta sem a pose do último uso; com `retain: false` as malhas
  extras são liberadas com o quadro (caminho de reversão) e a volta de `retain: true` reaproveita de novo.
- Medição no editor release com o projeto de exemplo (hash dos quadros idêntico antes/depois, 1,87 → 0 programas Spark novos
  por quadro de crossfade, ≈ 0,5 ms de CPU por quadro, sem mudança mensurável de `render_ms`):
  [bl131-shared-effect-leases](../viewport-performance-results/bl131-shared-effect-leases/README.md).
- `runtime/test/preview-sort.test.mjs` (BL130, Chrome real, 9 testes): o quadro de movimento usa a ordem velha só com o mesmo
  mapeamento e avisa (`stale_sort_layers`), enquanto o primeiro quadro e o final são exatos; o quadro final é o mesmo
  seja qual for a sequência de quadros de preview antes dele; um quadro de movimento nunca é servido do cache de resultado como o
  exato; uma cena cujos splats mudaram nunca é desenhada com os dados velhos; um crossfade reaproveita a ordem de cada uma das duas
  cenas mesmo com um arquivo de efeito compartilhado; `preview_sort: exact` mantém os quadros de preview exatos (só a ordem); cena sem splat nunca
  é velha; perda de contexto não deixa ordem velha (o quadro depois da restauração é exato); **todo `hello` diz a ordem inteira**
  (ausente = `latest` de novo; nome desconhecido ou de outro tipo = `exact`). Núcleo: `bridge` (`PreviewSort`,
  `stale_sort_layers` ⇒ reduzido), sessão (refino de ordem velha; quadro explícito nunca candidato) e host desktop (rótulo
  " · sort"; `TESSEL_VIEWPORT_PREVIEW_SORT` inválido vira `exact` com aviso).
- `runtime/test/splat-lineages.test.mjs` (BL130.2/BL130.7, Chrome real, 7 testes): o orçamento de estágios com 3 e com 4 cenas (no máximo
  `splatStages` + o compartilhado vivos, a terceira camada em diante exata e nunca velha, o hash do quadro exato igual ao de um
  runtime que nunca fez preview, o estágio fica com a camada que já o tem e o despejo abre espaço antes de criar), três camadas da
  mesma cena, `retain:false` sem linhagem; e as malhas: `spareFor` puro (própria, sem dono, de dono que não pede, nenhuma), cena dirigida
  antes de uma cena com o mesmo splat, cena nova entrando na frente da que está na tela. Mutações executadas (código restaurado com o mesmo sha256 do `dist`): sem orçamento de
  estágios reprova os 3 testes de orçamento; empréstimo LIFO sem dono reprova os 3 testes de malhas.
- `runtime/test/splat-failures.test.mjs` (BL130.5/BL130.8, Chrome real, 6 testes, `readbackDepth` do `SparkRenderer` na página
  substituído por uma rejeição ou por um sort que nunca responde): sort que rejeita atrás de quadros de movimento → o pedido
  seguinte falha com o erro (não desenha mais um quadro velho), o estágio é descartado e o pedido depois dele sorteia num estágio novo
  (exato, depois movimento de novo e o hash final igual ao exato); quadro exato depois da falha recebe o erro na hora (não espera os
  60 s); falha do próprio sort do quadro exato; erro que o Spark reporta por `dropErrorIfDisposed` registrado, o primeiro mantido; sort
  que nunca responde (quadro superado para de esperar, o orçamento de tempo encerra o seguinte, o estágio é aposentado); estágio compartilhado
  trocado sem tocar nos das cenas com estágio. Todos com 0 `unhandledrejection` na página. Mutações executadas: a falha engolida (`started.catch(() => {})` como era) reprova os 2
  primeiros testes de propagação; a espera sem limite reprova o do sort que nunca responde.
- `runtime/test/sort-order-evidence.test.mjs` (sem navegador, 4 testes): a ferramenta de evidência recusa < 2 planos, planos/assets que não
  são o que a ferramenta lê, tamanho/ritmo/amostras/folha inválidos e opção ausente, com a mensagem de uso e **status 2**, antes de
  renderizar, subir o worker ou criar a pasta de saída.
- `cargo test -p tessel-editor-render --test frames` (7) e `crates/render/tests/export.rs` (2).
- `scripts/live-proof.py`: 56/56 na janela macOS real (splat incluso).

## Splat numa cena dirigida (F048)

O mesmo Spark, o mesmo estágio e o mesmo `settle` desenham um splat de um plano de cena dirigida:
o plano ([F037](scene-plan.md)) carrega a matriz de mundo do nó e a câmera avaliadas e o recurso
`spz` aprovado; o runtime só os desenha. Contrato e limites em [scene-splat.md](scene-splat.md).

## MSAA em movimento (BL129)

Enquanto o playhead anda (playback e scrub do monitor) uma cena com splat é desenhada com `preview_scene_samples` (padrão 0) em
vez do MSAA final (padrão 4); ao parar, o monitor se refina sozinho para o final. `render.frame`, export, PNG e probes seguem
o final. Contrato, bridge e provas em [viewport.md](viewport.md#msaa-por-movimento-bl129).

## Ordem dos splats em movimento (BL130)

O `settle` acima custa a ordenação inteira (CPU + leitura da GPU) no caminho crítico de **cada** quadro: com 1 M de gaussians o quadro
de playback/scrub levava ≈ 16 ms (take) e ≈ 43 ms (crossfade) de desenho no runtime. Enquanto o playhead anda (quadro `Preview`,
`hello.preview_sort: "latest"`, o padrão) o quadro é desenhado **na hora com a ordem da última ordenação terminada** e a ordenação
para a câmera do quadro roda atrás dele; ao parar, o refino do BL129 pede o quadro exato.

- **Quando a ordem velha é segura** (`SplatStage.holdsOrderFor`): o Spark já tem uma textura de ordem **e** o que está em exibição é o
  mapeamento mais recente (`display.mappingVersion === latestMappingVersion`): a ordem é de outro ponto de vista, mas indexa os
  **mesmos** splats, cujas posições são as deste quadro. Primeiro quadro de uma cena, mapeamento mudado ou dados de outra cena
  ainda em exibição não desenham com a ordem velha: esperam como o quadro exato.
- **Uma linhagem de ordem por camada de cena, com orçamento** (`SplatStage` por `(asset da cena, n-ésima camada desse asset no plano)`):
  um `SparkRenderer` compartilhado troca de mapeamento a cada camada de um crossfade e nunca tem uma ordem válida para a seguinte; com
  uma linhagem por cena cada camada reaproveita a própria. Como cada `SparkRenderer` guarda na GPU os dados ordenados da sua cena, o
  orçamento é de estágios **vivos**, não só de ociosos: `LIMITS.splatStages` (2) por plano. O plano é lido inteiro antes do primeiro
  empréstimo (`Compositor.planLineages`): as camadas que **já têm** estágio ficam com ele, depois as demais na ordem do plano, até o
  orçamento; **toda outra camada de cena com splat usa o estágio compartilhado e é desenhada exata** (espera a ordenação da própria
  câmera, nunca `stale`). No máximo `splatStages` + 1 `SparkRenderer`s existem (`cache.stage_entries` + `cache.shared_stage_entries`;
  contadores `stage_created`, `stage_evicted`, `stage_shared_layers`), e criar um estágio abre espaço antes (despeja o ocioso menos
  recente), sem passar do orçamento nem por um instante. Duas camadas da mesma cena são duas linhagens. Cenas dirigidas (F048) ficam no
  estágio compartilhado e no `settle` exato; `retain:false` não tem linhagem (estágio compartilhado) e também fica exato.
- **Malhas por dono:** `splatLease(id, owner)` (`owner` = `linhagem#índice do objeto`) devolve, nesta ordem, a malha sobressalente que o
  dono teve da última vez, uma que **ninguém** possui, uma cujo dono não pede neste quadro, e senão uma malha nova; o compositor informa antes
  do primeiro empréstimo os donos que o quadro vai pedir (`reserveSplatOwners`). Uma cena dirigida (sem dono) ou uma cena que entra
  num crossfade, mesmo listada antes, não leva a malha de uma cena que está na tela: o mapeamento dela não muda de um quadro ao seguinte
  e a ordem continua valendo. Sem isso o quadro continuava correto, só perdia o ganho (cada quadro de movimento esperava a ordenação exata).
- **O quadro avisa:** `frame.done.source.stale_sort_layers` (campo aditivo; ausente = 0) conta as camadas desenhadas com a ordem de
  outro ponto de vista; `FrameDetail::is_reduced()` o conta, então a sessão agenda o refino exato (sem `quality`), e o cabeçalho
  mostra " · sort". O sinal é conservador: câmera parada com a ordem ainda por refinar também conta. **Um quadro assim nunca entra
  no cache de resultado** (a chave é a mesma do exato quando as amostras do preview são as finais).
- **Reversão:** `hello.preview_sort: "exact"` / `TESSEL_VIEWPORT_PREVIEW_SORT=exact` volta ao `settle` exato em todo quadro.
  `render.frame`, export, PNG, probes e o refino nunca usam a ordem velha (`quality` ausente ⇒ `motion` falso no runtime).
  **`exact` só decide a ordenação.** O quadro de preview continua podendo ter MSAA reduzido (`preview_scene_samples`, padrão 0) e
  proxies de vídeo, então **não** é o quadro final: para o original durante o movimento use a política `preview.policy` com
  `mode: "original"` (`monitor.preview.quality`), que o desenha em qualidade original.
- **Valores:** cada `hello` diz a ordem inteira: sem `preview_sort` é `latest` (inclusive num segundo `hello` depois de um `exact`, não
  "mantém o último"); um nome que não é `latest` nem `exact` (ou outro tipo) é lido como `exact`, a ordem que não mostra a ordenação de outra
  câmera, e o runtime avisa no console. No editor, `TESSEL_VIEWPORT_PREVIEW_SORT` vazio ou ausente deixa o padrão; um valor que não é
  nenhum dos dois nomes (um `exato` digitado de reversão) vira `exact` **com aviso em stderr** e nunca deixa a ordem reaproveitada ligada
  sem querer. O MSAA reduzido do BL129 é outra variável (`TESSEL_VIEWPORT_PREVIEW_SCENE_SAMPLES`).

### Ordenação que falha (BL130.5)

O Spark 2.4.0 não limpa o estado quando a leitura de profundidade ou o worker de ordenação rejeita: `driveSort` põe `sorting = true`
e, se o `await` seguinte lança, a flag fica **para sempre**, nada mais é ordenado, o erro das ordenações que o próprio Spark encadeia
(ou dispara por timer) vai a `dropErrorIfDisposed`, que o relança numa promessa sem dono (*unhandled rejection*), e o `settle` exato
esperaria os 60 s inteiros por um erro que já aconteceu. O runtime **não escreve em `sorting`**; o estágio registra a falha e é
descartado:

- `SplatStage.watchSort` substitui `dropErrorIfDisposed` da instância (o gancho do Spark para o que ninguém espera) por um registro, e o
  `update` do próprio quadro rejeita para quem o aguarda (`begin`, que nunca rejeita: guarda a causa no estágio). Nenhuma rejeição fica sem
  dono, também com o quadro cancelado ou superado.
- Todo pedido a um estágio falho (`settle`, qualquer qualidade) falha **na hora** com `render_failed` e a causa, antes de qualquer espera;
  um quadro de movimento não desenha mais com a ordem de uma ordenação que nunca termina. A propagação é no pedido seguinte ao erro (o
  quadro de movimento que viu a ordem velha já foi entregue).
- O compositor descarta o estágio falho (`stage_discarded`; o de uma linhagem ou o compartilhado), então o pedido depois dele cria um
  `SparkRenderer` novo e ordena para a própria câmera (exato; depois, movimento reaproveita de novo). Nada do estado quebrado é reutilizado.
- A espera por uma ordenação é limitada: um quadro **cancelado ou superado** confere o cancelamento a cada 25 ms (a latência depende do agendamento) (antes, ficava preso até o fim
  da ordenação e segurava a fila de quadros) e uma ordenação que não termina em 60 s (ou 400 ordenações) falha o quadro e aposenta o estágio,
  para o seguinte não gastar o mesmo orçamento.

Evidência (release, M3 Pro, GPU compartilhada, load average 14–44 de outros agentes, o mesmo projeto 1280×720/24 fps com splats
de 1.179.648 gaussians nos dois lados, sem mídia nova; 3 sessões × 3 repetições; `scripts/preview-quality-proof.py`; tabelas e
comandos em [bl130-spark-settle](../viewport-performance-results/bl130-spark-settle/README.md)):

| Medida (média das repetições) | Antes (`main` com BL129/BL131) | `exact` (este PR, reversão) | `latest` (este PR, padrão) |
|---|---|---|---|
| Desenho no runtime, take (p50) | 16 ms | 17 ms | **1 ms** |
| Desenho no runtime, crossfade (p50) | 43 ms | 48 ms | **1 ms** |
| Take 24 fps, quadros sem imagem | 3,4 % (0–39) | 3,3 % (0–43) | **0 %** (0–0) |
| Crossfade, quadros sem imagem | 8,0 % (0–21) | 17,9 % (7–36) | **0 %** (0–0) |
| Scrub do take, imagens/s | 42,1 (38–46,5) | 37,8 (19,5–43,5) | **49,9** (49,5–50) |
| Scrub do crossfade, imagens/s | 38,9 (35,5–42,5) | 33,6 (19,5–36,5) | **49,8** (49–50) |
| Atraso do scrub (quadros, mediana) | 1 | 1 | **0** |
| Parou → quadro final de volta (playback) | 297 ms | 327 ms | 304 ms |
| Soltou o scrub → quadro final | 262 ms | 302 ms | **355 ms** |

O custo do `latest` está na última linha: o quadro exato depois de soltar o scrub chega ≈ 90 ms mais tarde que antes, porque a
ordenação exata deixou de ser feita durante o gesto. Depois do playback o tempo é o mesmo (a ordenação do último quadro já estava em
curso).

Invariantes (executados): `render.frame` dos quadros 5, 60, 100, 193, 300 e 450 com o mesmo hash antes e depois
(`--inspect`, só invariante, não medida); **o último quadro que o runtime devolveu ao monitor depois de parar o playback e depois de
soltar o scrub tem o mesmo hash que o `render.frame` do mesmo quadro** (4 playbacks e 4 scrubs, take e crossfade;
`final_source.stale_sort_layers` 0); export de 6 quadros de crossfade (190–195, `fullhd`) com o mesmo MD5 por quadro e o mesmo sha256 do
arquivo antes e depois. **O hash é o do quadro lido pelo runtime (`frame.done.hash`), não uma captura dos pixels da tela**: a apresentação
visual no monitor nativo **não foi verificada** por esta prova (revisão visual nativa pendente, BL130.6).

Câmera animada (`runtime/scripts/sort-order-evidence.mjs`, o worker real com `TESSEL_GL=gpu`, 960×540, planos reais de quadros
consecutivos do projeto; cada quadro de movimento contra o exato do mesmo quadro). **Dado histórico, não aceitação:** a primeira versão
da ferramenta media só médias do quadro inteiro (diferença média por canal 0,0028–0,0384/255, PSNR ≥ 50,2 dB, "popping" ≤ 0,0039/255 contra
0,95–10,0/255 de mudança do exato), e uma média sobre o quadro inteiro é diluída por todos os blocos que os splats nunca alcançam (o pior
canal isolado chegou a 149/255). Por isso este documento **não afirma "sem popping"** nem limite superior; a ferramenta passou a medir por
bloco de 32×32 (pior bloco e quantos passam de 2/255, também na mudança de um quadro ao seguinte) e os números locais estão no
[README da medição](../viewport-performance-results/bl130-spark-settle/README.md#diferença-local-por-bloco-câmera-animada). O quadro final
refinado é idêntico ao exato em todas as execuções; o script termina com erro (status 1) se o exato mudar com a ordem de visita ou se o final
não for igual ao exato, e com status 2 se a entrada não permite medir (menos de 2 planos etc.). Os passos `s4`/`s8`/`s16` (planos de 4, 8 e 16
quadros de distância) **aproximam** um movimento de câmera maior; não são um playback medido.

Limites (não esconder):

- O quadro de movimento **não é** o exato: a diferença é medida, não zero. Quem exige o quadro exato em movimento usa `exact` **e**
  `preview.policy` `original` (ver "Reversão": `exact` só decide a ordenação, o MSAA reduzido e os proxies continuam).
- Uma corrida isolada da primeira versão (crossfade com `draw` p50 de 2,4 s e 78 % de quadros perdidos) **não se reproduziu** em
  as dezenas de repetições seguintes (v2, v3 e a versão final); a hipótese (um `setTimeout(0)` limitado a ≈ 1 s numa página escondida) motivou trocar o timer por
  esperar a promessa do `update` já iniciado, mas a causa não foi confirmada (BL130.1).
- Em máquina carregada o `settle` exato demora segundos de relógio por ordenação (SwiftShader/CPU disputada): por isso o orçamento é
  de ordenações e de tempo (60 s), não de voltas de `setTimeout`.
- A ferramenta de evidência lê cada quadro como PNG, o que espera a GPU e a ordenação e **encurta** o atraso da ordenação em relação ao
  monitor: a diferença medida lá **não é** um limite superior do que o monitor mostra, e mudar o ritmo (`--pace-ms` 41,67, 16 e 10)
  não a estressa (dados iguais). O quadro de playback no editor real não é capturável por PNG (o campo `done` é a submissão, não a exibição).
- **Revisão visual nativa pendente (BL130.6):** nenhum quadro de movimento foi comparado a olho na janela do editor nativo; as provas acima
  são hashes e métricas do quadro devolvido pelo runtime.
- Mais de 2 cenas: o orçamento (3 e 4 cenas) e o fallback exato estão provados no runtime (Chrome real, SwiftShader, nuvens pequenas), não
  medidos com 1,18 M de gaussians por cena nem na GPU do editor (BL130.2). O estágio compartilhado serve as camadas além do orçamento uma
  depois da outra, cada uma esperando a própria ordenação: com muitas cenas grandes o quadro volta ao custo anterior ao BL130 naquelas camadas.
- Câmera parada com a ordem ainda por refinar também conta como `stale_sort_layers` (o sinal é conservador). O cache de resultado já evita
  o caso comum (cena estática, mesma câmera, mesmo tamanho: o quadro vem do cache e nunca é velho); distinguir "câmera parada, geometria
  igual" no Spark exigiria comparar as matrizes de cada malha e não quebrar `spin`/transformação, e não foi feito: o refino extra é barato
  (um quadro exato depois de 200 ms de quietude) e fica registrado como limite.
- GPU compartilhada e load average alto: leia a dispersão, não um número. Não existe script de trava de GPU neste repositório; a
  serialização foi só de execução (uma sessão por vez) e `wait_quiet_gpu` (BL130.3).
- Não medido: GPUs/máquinas além da M3 Pro, áudio ouvido.

## Limites

Medido em SwiftShader (CPU); ordem de composição entre splats e planos transparentes segue as
limitações documentadas do Spark (splats ordenados entre si, planos pela ordem do plano).
Outros formatos do Spark (`.ksplat`, `.sog`) não têm prova aqui e não são aceitos.

Cor (BL019-F1): o desenho vai a um render target e, nele, o Spark 2.4.0 usa `ColorManagement.workingColorSpace`
(linear) para ligar `encodeLinear`, o que escurecia a cor guardada. `keepStoredColour`
(`runtime/src/splat-stage.js`) mantém o uniform em `false` só no material do Spark, depois do `onBeforeRender` do
próprio Spark. **Isto também muda a cor do caminho de projeto (`scene`)**, não só o da cena dirigida: um splat do
projeto agora mostra a cor guardada. Detalhes e prova em [scene-splat.md](scene-splat.md#cor-bl019-f1).

### Conferência independente R2

No head `7e2ebfc`, a revisão executou 72 testes focados de runtime em Chrome real, 58 testes do host e o spec Python (8/8). Em janela isolada, capturas dos quadros 60 (take) e 193 (crossfade) após refino coincidiram com a referência exata abaixo do cabeçalho. Isso não valida movimento real, playback ou scrub longo; BL130.6 continua pendente. O teste atual de orçamento conta estágios após o quadro, sem discriminar o pico durante a criação (`makeRoom`); essa cobertura permanece pendente. Após erro de sort, o próximo pedido reporta a causa e descarta o estágio; não há retry automático do refino e uma nova ação pode ser necessária.

O coordenador executou o build release e uma sessão final com três repetições por variante, sem inspect/PNG: nenhum timeout ou erro; os relatórios e limites estão no [fechamento](../viewport-performance-results/bl130-spark-settle/README.md#fechamento-do-coordenador-2026-11-07). Isso não substitui o aceite visual de movimento real.

Foi gravado playback real pela operação semântica `playback.start` do quadro 160, por 5,2 s, seguido de stop/refino (411 ms, qualidade original, `stale_sort_layers: 0`), em janela isolada capturada por ID. Vídeo local privado `/tmp/pr19-final/native-playback.mp4`, fora do git; metadados em [motion.json](../viewport-performance-results/bl130-spark-settle/final-gate/motion.json). A gravação está disponível para avaliação humana; não equivale a aprovação de movimento nem criativa.

Correção de prova após R2: as comparações históricas de hash do scrub feitas antes deste fechamento avaliavam a origem restaurada, não o último quadro do gesto. O subprocesso agora recebe `--no-restore` apenas na sessão isolada do harness; o script avulso continua restaurando por padrão. A nova prova de release usa o quadro final do gesto. O script de evidência também recusa sheet zero e encerra com erro quando o worker falha, fecha ou não responde.

Nova execução inspect de release (uma repetição por cenário, fora do benchmark): quadros 120 e 250 com `refined_equals_explicit.equal=true`, sem falhas. Spec Python 9/9 normal e -O; validação do script de evidência 5/5, incluindo worker indisponível e sheet zero. [Relatório](../viewport-performance-results/bl130-spark-settle/final-gate/release-frame-check.json).
