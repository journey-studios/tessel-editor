# F044 — Aceite independente de cena dirigida: splat, câmera, áudio, pacote e export (BL019)

## Objetivo e limites

Um harness que julga o editor por **esperados calculados fora dele**, com **controles negativos** que precisam falhar pelo
motivo certo e um **livro de emendas** que registra toda mudança do oráculo, dos verificadores, dos controles, do corpus,
do driver e das tolerâncias. Cobre câmera, splat, áudio, pacote/restauração e export da cena dirigida. É uma frente de
prova (BL019, ampliação S10): **não muda nenhum comportamento do editor**, não adiciona comando, nó de árvore, handler ou
dependência.

Superfícies (matriz F044): UI e CLI da janela são N/A (o harness dirige o processo `tessel-render`, não a janela; o
export pela janela é do F042, `scripts/directed-scene-export-proof`). CLI headless e Lib são **Parcial**: o harness
exercita as mesmas operações da lib (`scene.*`, `project.*`) pelo processo JSONL sem janela, mas não adiciona teste de
unidade à lib.

Diretório: `scripts/directed-scene-acceptance/`.

| Arquivo | Papel |
|---|---|
| `tolerances.json` | Limites numéricos, controles negativos obrigatórios, política BLOCKED e, para **cada número**, uma justificativa com base (`analytic`, `inherited` ou `choice`) e texto. Todo limiar que um verificador usa mora aqui. |
| `amendments.json` + `ledger.py` | O livro de emendas: baseline verificado no Git (`ANCHOR`), rodadas R1–R4 com motivo, achado, emendas e SHA-256 (antes/depois) dos 14 arquivos fixados. `tolerances_frozen` falha se um arquivo fixado difere do último estado registrado, se o livro não só cresce em relação ao **histórico completo** do Git (`git log --full-history`, inclusive o lado de um merge que a simplificação padrão do `git log` omite) ou se um número fica sem justificativa. |
| `oracle.py` | Modelo independente: projeção da câmera e máscara exata por ray-cast, centro/raio e composição `over` de gaussianas (inclusive a sonda do splat de trás), tabela de tons, janela de áudio da origem e lei de canais. Não importa nada do editor. Escreve SPZ v2 e WAV. |
| `corpus.py` | Gera `corpus/` (versão 3) de forma determinística (reescrever remove só arquivos **da própria geração anterior**, com os bytes inalterados e nomeados no `provenance.json` da pasta; sobras de outra origem ficam e `--check` as reporta como `stray`) (PNG de cor sólida, SPZ de 45–62 bytes, WAV senoidal mono de 44,1 kHz de 13 s, mais o WAV de 8 s da emenda A1 como fixture negativa) com `provenance.json` e os ativos declarados por bundle. Recusa um quadro que não separe o splat de trás e uma janela de áudio sem borda. |
| `verify.py` | Verificadores: máscara/centroide/IoU/cor em imagem real, marcador ausente, splat (centroide, cor, r50, ordem, composição e sonda do splat de trás), áudio decodificado por ffmpeg (início, correlação, ganho pela lei de canais, canais de saída, silêncio, bordas, janela com borda), pacote lido por `zipfile`/diretório e hashes declarados (nomes e hashes exatos). |
| `controls.py` | Entradas **certas e erradas** para cada verificador, em paisagem e retrato, em **todos** os quadros de splat, no regime PNG e **depois da cadeia FFmpeg do preset** (libx264 medium/high/crf18/yuv420p/BT.709 limitado). Controle negativo só passa se um check **nomeado** falhar; exceção do verificador reprova o controle. |
| `proof.py` | Driver: fases, relatório JSON, bloco `acceptance`, política de saída (0–4). Reusa só a infraestrutura (`harness.py`, trava de GPU, `JsonlSession`) de `scripts/directed-scene-proof`. |
| `link.py` | Atestação do binário: SHA-256 do `tessel-render`, hashes do `runtime/dist`, ids de árvore Git de `crates/`, `runtime/`, `scripts/`, `Cargo.*` e do toolchain. Prova **qual head** gerou o binário usado; **não** é prova de build reproduzível. |
| `sensitivity.py` | Calcula **só pelo modelo** o que cada regra enxerga e não enxerga (fator de tamanho de splat por regime, separação do splat de trás, menor erro de FOV, menor deslocamento de áudio) e quanto de cada tolerância a própria imagem correta do modelo consome. É a base das justificativas; não é aprovação dos números. |
| `proof.spec.py` | 102 testes do próprio harness (sem GPU/Chrome), incluindo a cola das fases dependentes contra *fakes*, o ledger contra o Git e mutações que o teste precisa pegar. |
| `history/A1-audio-window/` | Registro imutável do estado anterior à emenda A1 e do FAIL histórico que a motivou. |
| `results.json` | Resultado **histórico** da primeira execução (harness `1b5953e`, árvore com 1 arquivo sujo, oráculo anterior às emendas). Não é aceite deste head e não foi reescrito. |

## Contrato do harness

```text
python3 scripts/directed-scene-acceptance/proof.py --list
python3 scripts/directed-scene-acceptance/proof.py --phases tolerances_frozen,corpus,models,controls_camera
./scripts/runtime setup
./scripts/cargo build -p tessel-editor-render --locked
python3 scripts/directed-scene-acceptance/link.py create --render target/debug/tessel-render   # logo após o build, árvore limpa
python3 scripts/directed-scene-proof/with-gpu-lock.py \
  python3 scripts/directed-scene-acceptance/proof.py --gpu-lock-held --report /tmp/s10.json
```

Pré-requisitos das fases com imagem: `./scripts/runtime setup`, `tessel-render` compilado **e atestado para este head**
(`link.py create`), Chrome/SwiftShader e ffmpeg/ffprobe. Uma fase sem pré-requisito sai BLOCKED, nunca pass. Um binário
mais velho que as fontes, **sem atestação válida** ou atestado para outro head também é BLOCKED, a menos que se passe
`--allow-stale-binary`; nesse caso (e com árvore suja) o resultado é só informativo (saída 4) e o relatório diz por quê.
`--bin-render` aponta para uma cópia do binário (útil quando o cache de alvos do Cargo pode ser descartado no meio da
execução; a atestação vale para o arquivo apontado).

| Saída | Significado |
|---|---|
| `0` | toda fase selecionada rodou e passou. `acceptance.acceptable` só é verdadeiro com **as 24 fases**, árvore limpa, binário atestado e sem `--allow-stale-binary`; um subconjunto sai 0 mas o relatório diz "NOT AN ACCEPTANCE" |
| `1` | pelo menos uma FAIL, ou o orçamento de 2 GiB estourou |
| `2` | erro de uso ou pré-requisito recusado (fase desconhecida, `--phases` vazio, `--work` que não é pasta, corpus ausente, **disco abaixo de 12 GiB**: na partida ou entre fases, execução que não rodou nada) sem FAIL |
| `3` | nenhuma FAIL, mas ao menos uma fase **BLOCKED**. BLOCKED nunca conta como pass **e nunca substitui uma FAIL** (uma fase com checagens reprovadas continua FAIL mesmo se um passo seguinte bloquear) |
| `4` | nenhuma FAIL nem BLOCKED, mas a execução é **informativa** (árvore suja, binário velho ou sem atestação, `--allow-stale-binary`): não é aceite |

FAIL tem prioridade no código de saída. `--work <dir>` aponta uma pasta **do chamador**: o harness cria e remove só o seu
`s10-acc-*` dentro dela e nunca toca no resto (teste com arquivo sentinela); a reserva de disco é lida no volume de
`--work`. Cada fase e cada pedido usam diretórios próprios (`fresh`), de modo que um destino deixado por uma fase ou sonda
nunca vira `destination_exists` em outra.

### Fases

| Grupo | Fases | O que prova |
|---|---|---|
| Dados | `tolerances_frozen`, `corpus`, `models` | ancestralidade Git das tolerâncias, livro de emendas íntegro (âncora, cadeia, baseline no Git, histórico completo só cresce, nenhum arquivo fixado editado fora do registro, toda tolerância justificada); corpus byte-idêntico ao gerador, com hashes pinados e ativos declarados; modelos contra identidades analíticas e o oráculo da origem |
| Controles | `controls_camera`, `controls_splat`, `controls_audio`, `controls_package` | cada verificador **aceita** a entrada certa e **recusa pelo check nomeado** cada entrada errada congelada (câmera em paisagem e retrato; splat nos seis quadros, nos regimes PNG e vídeo, com cegueiras declaradas; áudio pela lei de canais, saída mono e janela sem borda) |
| Câmera | `camera_plan`, `camera_render`, `camera_export` | pose/matriz do `scene.plan`; máscaras reais de `scene.render` contra o ray-cast, em 640×360 e 360×640 e FOV 30/50/75, incluindo marcador que o modelo põe fora do quadro; frames do MP4 de `scene.export` |
| Baseline | `baseline_splat_geometry`, `baseline_splat_colour`, `baseline_audio` | o splat da camada de projeto e o mixer offline que **existem hoje** (o clipe do baseline de áudio lê o arquivo em `editStart + t`, como a origem) |
| Áudio desligado e pacote | `audio_off`, `package_portability`, `package_tamper` | `audio:false` preservado; pacote lido por leitor que não é o do editor (manifesto sem `files` é falha), restauração com a origem apagada; pacote adulterado recusado sem criar pasta |
| Splat e áudio integrados (S7/S9) | `splat_plan`, `splat_render`, `splat_export`, `splat_package`, `audio_export`, `audio_package_export`, `audio_short_source`, `full_av` | os mesmos verificadores contra splat dirigido e áudio mixado no export; `audio_export` cobre 24 fps (duas janelas) e 30000/1001 (`audio_ntsc_window`); `audio_short_source` recusa **qualquer** faixa de uma fonte menor que a janela com `audio_source_short` (`details.rule == "window"`); `full_av` prova **splat e áudio na mesma cena e no mesmo export**, e de novo no projeto restaurado |

S7 (PR #13) e S9 (PR #11) estão integrados na main que este branch contém (`6a0bdd5` e `836d8cb` são ancestrais), de modo que
as fases vivas deixaram de ser "seams aguardados" e passaram a ter veredito próprio. O veredito só vale para o binário
atestado do relatório (veja Evidências).

## Emendas — rodada R1 (livro em `amendments.json`)

Aplicadas juntas, em um commit, **antes de qualquer execução viva do harness emendado**; os estados intermediários entre
A1 e A5 nunca existiram isolados. Hashes antes/depois de cada arquivo fixado estão no livro; o baseline é o head do PR
revisado (`a848007`), conferido contra o Git por `tolerances_frozen`.

### A1 — janela de áudio da origem (F1, BL016.5)

A suposição anterior do oráculo (`mix[0]` ↔ `editStartSeconds`) contradizia a origem, pinada em `d6adac15` (somente leitura):

| Arquivo | Linhas | Regra |
|---|---|---|
| `packages/scene-engine/src/transport/audio-window.ts` | 10–15 | `editSecondsToSourceTime(t) = editStartSeconds + t`: o WAV é a fonte inteira; o instante de edição `t` toca o arquivo em `editStartSeconds + t` |
| `packages/scene-engine/src/node/render.ts` | 246–248 | recusa `AUDIO_SHORT` quando `duração(fonte) + 0,001 < editEndSecondsExclusive` |
| idem | 250–252 | recusa `AUDIO_VIDEO_DURATION` quando `editEnd − editStart` difere da duração da edição por mais de 1 ms |

Corpus v2: mix de 13 s em tempo de arquivo, janelas derivadas da edição (`audio24` `[1, 13)`, NTSC `[0, 10,01)`,
`full` `[0, 2)`; a janela de `full` mudou na rodada R3), com assertivas do gerador para as duas regras. O WAV de 8 s
anterior fica **byte a byte** como `mix-short.wav`, fixture do controle negativo vivo `audio_short_source`. O FAIL
histórico (`audio_export` 75/92 contra a S9 original e o `destination_exists` da sonda) está preservado em
`history/A1-audio-window/` com os hashes anteriores. A causa era do oráculo, não do editor. O baseline de áudio 46/46
**nunca validou** esta regra (o clipe do baseline começava em 24 com fonte 0, o que satisfazia a suposição antiga); agora o
clipe cobre a edição e lê o arquivo a partir de `editStartSeconds`. A emenda parcial preservada em
`wave-resource-stop/S10` foi lida, conferida contra a origem e aplicada; o que não estava nela (A2–A5) foi acrescentado aqui.

A divergência **declarada** do editor com a origem na regra `AUDIO_VIDEO_DURATION` (a origem exige duração igual à da edição
em 1 ms; o editor mantém o contrato de uma amostra exata e exporta o trecho dentro da janela) é a BL016.11, decisão do
coordenador; o harness segue a regra estrita na janela do corpus e não mede a divergência.

### A2 — composição na mistura do splat (F2, BL120)

No centro do splat de trás (`s-order`) a gaussiana da frente tem opacidade **0,761 a 0,920** conforme o quadro (0,750 a
0,914 no pixel que o verificador lê), e não "0,80–0,92" como o texto da R1 dizia (o texto foi corrigido na R2; os quadros 36 e
47 ficam abaixo de 0,80). Exigir ali a cor da frente dentro de 40 reprovava um render correto nos quadros 36 e 47
(distâncias 43,5 e 52,3 pela composição do próprio modelo; `proof.spec.py` reproduz isso). Agora: no centro da frente, a cor
da frente dentro de 40 e ≥ 80 da cor de trás (inalterado); no centro de trás, a **composição `over` do oráculo**
(`oracle.composite_at`) dentro dos mesmos 40 (`overlapBackCompositionDelta`, **não alargado**) e ≥ 80 da cor própria de
trás. Composições no pixel julgado: `(196,162,70)` e `(187,164,78)` nos quadros 36 e 47. Controles positivos e negativos
(ordem trocada incluída) rodam em **todos** os `SPLAT_FRAMES`. Esta é a única lei de mistura modelada; luz linear e
tonemapping continuam não modelados.

### A3 — lei de ganho de canais (F3, BL019-F3)

A lei esperada deixou de ser 0 dB. A origem força `aformat=...:channel_layouts=stereo` e soma com `amix normalize=0`
(`audio-graph.ts`), de modo que vale o *rematrix* padrão do FFmpeg: fonte mono entra em E e D a `1/√2` (−3,0103 dB), layout
igual é intacto (`oracle.channel_gain_db`; qualquer outro par levanta erro em vez de ser adivinhado). `proof.spec.py`
**rederiva o número executando o filtro da origem** no FFmpeg local, sem ler o renderizador. O fake "mix correto" e os
controles gravam o sinal pela lei; um controle negativo, "mono source written at 0 dB", precisa falhar, e o limiar de
borda é relativo ao nível esperado (já com a lei).

### A4 — limiares fora do verificador (F5)

`minJudgedAreaPx` (300), `videoBackgroundExtraDelta` (6), `onsetSearchWindowSeconds` (0,08) e `onsetThresholdFraction`
(0,35) saíram do código para `tolerances.json` com o mesmo valor. Novo: `absentMarkerMaxPixels` (20): um marcador que o modelo
põe fora do quadro precisa deixar no máximo 20 pixels da sua cor (antes só era "não julgado"). Na R2 a medição virou
verificável: 6,7% de 300 e 3,6% de 561 pixels na imagem correta do modelo.

### A5 — proveniência e controles (F4, F5, baixos)

Reescrita da proveniência (abaixo), controles de retrato (`360x640@24`, clipe da própria tomada), controle da leitura
aposentada de A1, controle de hash declarado que agora **chama** `verify.judge_declared_hashes` em vez de reimplementar a
regra, e asserção do gerador de que todo quadro de retrato julga ao menos um marcador.

## Emendas — rodada R2 (segunda revisão independente do PR #8)

Registrada **antes de qualquer execução viva do harness R2**, em um commit (`6073f1e`). O que o Git prova é ancestralidade:
o registro diz o que mudou e por quê, não prova que nada o antecedeu. A revisão R2 **não aprovou nenhum número**; a rodada
justifica e mede cada número (A12), não o aprova.

| Emenda | Achado | Mudança |
|---|---|---|
| A6 | splat de trás do par de sobreposição escapava (BL120) | no centro de trás, com e sem o splat diferem só 16 a 46, e um disco opaco de meia intensidade não se distingue da gaussiana; o verificador lê, dentro de 2 sigmas de trás, o pixel em que as três hipóteses (gaussiana, ausente, disco opaco) mais diferem, escolhido **só pelo modelo** (`oracle.back_separable_point`); o corpus recusa quadro com separação < 24 (o modelo dá 30 a 49). Controles negativos: splat de trás ausente e desenhado como disco opaco |
| A7 | controles que não falhavam pelo motivo certo; sem regime de vídeo | controle negativo passa só se um check **nomeado** falhar; exceção do verificador reprova o controle; controles rodam também **depois do libx264** do preset; erros de tamanho aplicados a um splat de cada vez (×1,5 e ×2). O regime de vídeo é **cego** a ×1,5 em 15 de 30 medições (aritmética da regra do raio): declarado por splat como `blind` (precisa NÃO ser detectado; declaração velha reprova), `borderline` (a 0,5 px do limite) como `either`, e ×2 precisa ser detectado sempre. Nenhuma tolerância foi mudada para remover a cegueira |
| A8 | saída mono não podia passar | a origem sempre escreve estéreo; `judge_audio` confere `outputChannels = 2` |
| A9 | `audio_short_source` aceitava faixa coberta pelo arquivo (R2-1) | a origem decide `AUDIO_SHORT` pela janela **antes** de olhar a faixa: toda faixa da fonte curta é recusada (`audio_source_short`, `details.rule == "window"`, 352800 quadros do arquivo e 13,000 s do fim da janela na mensagem), sem criar arquivo; a sonda roda antes num bundle válido |
| A10 | hashes declarados "ao vivo" eram tautológicos | `judge_declared_hashes` exige exatamente os nomes e hashes do corpus (`declaredAssets`); controles de nome ausente e desconhecido |
| A11 | alegações de proveniência, livro reiniciável, binário/árvore sem identidade | proveniência sem pré-registro nem aprovação; livro ancorado ao Git (`ANCHOR`, ancestral, baseline no Git, toda versão commitada de `amendments.json` é prefixo da atual); 14 arquivos fixados, incluindo driver, `link.py`, `sensitivity.py`, o livro e os auxiliares somente leitura; `link.py`; árvore suja ou binário velho torna a execução informativa (saída 4); `--phases` vazio, `--work` que não é pasta e execução que não rodou nada saem 2 |
| A12 | justificativa e sensibilidade de cada número | cada número de `tolerances.json` tem justificativa com base e texto (número sem justificativa reprova); `sensitivity.py` mede; textos não verificáveis (`<0,1 px`, `SPZ quantisation <= 0.9`, `BT.601 versus BT.709 não é detectado`) foram trocados por valores medidos |

### Sensibilidade (`sensitivity.py`, calculada só pelo modelo)

| Regra | O que enxerga | O que não enxerga |
|---|---|---|
| tamanho de splat, PNG | fator ≥ 1,365 a 1,452 (30 medições) | nada: ×1,5 e ×2 são detectados em todas as medições |
| tamanho de splat, vídeo (libx264) | fator ≥ 1,437 a 1,552 | ×1,5 em 15 de 30 medições (declarado); ×2 é detectado em todas |
| splat de trás | separação 30 a 49 entre as três hipóteses; margem por hipótese 15 a 24,5 | — |
| FOV da câmera | a partir de 0,25° (varredura 0,1 / 0,25 / 0,5 / 1 / 5°) | 0,1° |
| deslocamento do áudio | a partir de 3 ms (varredura 1 a 6 ms) | 1 e 2 ms |
| consumo da tolerância pela imagem **correta** | centroide 0,032 px (PNG) e 0,154 px (vídeo); IoU ≥ 0,986 (vídeo); o erro de raio usa no máximo 6,4% da folga permitida | — |

Estes números **justificam** os limiares; não os aprovam. Uma tolerância "apertada" aqui é uma escolha do autor, argumentada.

## Emendas — rodada R3 (depois do primeiro resultado vivo)

Esta rodada foi feita **sabendo** o resultado de `full_av`; o livro diz isso (`applied`). Não toca nenhuma tolerância e não
pode fazer passar um editor que falha.

### A13 — `full_av` com janela sem borda (achado do primeiro run vivo)

No primeiro run vivo do harness R2, contra binários do head integrado, 23 das 24 fases passaram. `full_av` reprovou 4 checks
de 422 (canal 0 e 1, export e projeto restaurado): `envelope lag +4800 samples`, um valor arbitrário (4800 amostras são 0,1 s a 48 kHz, **dentro** da busca de 0,15 s e não no limite dela: o texto anterior errava), enquanto a
correlação amostra a amostra com o sinal analítico era **1,0000**, o ganho −3,01 dB e as janelas de tom certas. Causa **no
corpus, não no editor**: a janela do bundle `full` era o início do arquivo, um único tom de 440 Hz, cujo envelope é constante
(o lag é arbitrário) e para o qual 0,1 s são 44 ciclos inteiros, de modo que nem a correlação enxerga esse deslocamento.

Mudança: o bundle `full` declara a janela `[2, 4)` (mesma edição de 48 quadros), que contém o fim do tom de 440 Hz, uma
pausa e o início do de 880 Hz (bordas em 1,0 s e 1,5 s do export); o gerador do corpus recusa qualquer caso de áudio sem
borda tom/silêncio; `judge_audio` reprova um caso sem borda (`lag observable`); novo controle negativo congelado
("constant-envelope window") e controle positivo de `full_av`. Três testes novos (99 no total), incluindo a aritmética do
deslocamento de 0,1 s: o tom puro deslocado continua igual, a janela com borda deslocada reprova `edge at` e `envelope lag`.
O editor e os limiares não mudaram. O run vivo foi **repetido do zero** em binário reconstruído e reatestado para este head.

## Emendas — rodada R4 (revisão Sonnet 5.5 do head `7a26569`, 5810370582)

A revisão independente não achou finding Médio ou Alto e quatro Baixos, todos *fail-closed* (nenhum produz PASS indevido
sobre o editor). A R4 fecha os dois que tocam arquivos fixados; L3 e L4 são de documentação. **R1, R2, R3 e o baseline não
foram editados**, e nenhuma regra do oráculo, verificador, controle, byte do corpus ou tolerância mudou. Os hashes antes/depois
dos 14 arquivos fixados estão em `amendments.json`; só `ledger.py` e `corpus.py` mudam. A rodada foi escrita **depois** do
run vivo de `ad9e182` e diz isso (`applied`); o aceite foi repetido do zero para o head da R4.

| Emenda | Achado | Mudança |
|---|---|---|
| A14 | L2 / OpenCodeReview `4500301244` | `committed_versions()` lê `git log --full-history`: a simplificação padrão do log omitia o commit de um livro reescrito quando um merge com conflito mantinha o outro lado. Teste `Ledger.test_a_conflict_merge_cannot_hide_a_rewritten_book` constrói um repositório Git temporário real (branches divergentes, merge com conflito resolvido com o livro da primeira parent) e exige que a versão reescrita apareça |
| A15 | L1 / OpenCodeReview `4500301238` | `corpus.write()` remove só arquivos que o `provenance.json` da própria pasta nomeia, que `build()` já não gera, cujos bytes ainda batem com o hash registrado e cujo caminho fica dentro da pasta (e as pastas que ficaram vazias). Arquivo do chamador, arquivo gerado e editado depois, nome que sai da pasta e pasta sem `provenance.json` nunca são apagados; `--check` continua reportando `stray`. `build()` e `corpus/` não mudam. Testes `CorpusWriter` (2, com arquivos sentinela) |

Errata sem editar o livro (só-acréscimo): o texto da emenda A13 em `amendments.json` e uma versão anterior deste documento diziam
que `+4800 samples` estava "no limite da busca de 0,15 s". Está errado: 4800 amostras são 0,1 s a 48 kHz, dentro da busca (o
limite seria 7200). O argumento correto é o da **identificabilidade**: numa janela sem borda o lag do envelope é um empate
arbitrário (reproduzido sem mutação: sinal idêntico dá lags de −150 a +150 conforme o ruído, e atrasos de 5, 20 e 41,7 ms dão 55, 70 e 61),
e 0,1 s de um tom puro de 440 Hz são 44 ciclos inteiros. O FAIL do run 3 **não** vem do bytecode do mutante: um lag arbitrário num sinal sem bordas
não é explicado por bytecode velho. O bytecode velho é uma dívida **real e separada** (BL019-F11).

**Merge commit obrigatório (L4).** A âncora `a848007` e as versões do livro precisam estar no histórico alcançável do `HEAD`.
Com *squash* ou *rebase* da PR esse histórico some da `main` e `tolerances_frozen` **falha** ali (fail-closed, mas o aceite
deixa de ser obtível). Integre o PR #8 com **merge commit** (a prática do repositório: `Merge pull request #N`), nunca
*squash* nem *rebase*; se isso não for possível, abra um BL para reancorar `ANCHOR` com uma rodada.

## Proveniência das tolerâncias (F4)

**Não afirmamos pré-registro nem aprovação.** O que o Git prova: `tolerances.json` foi commitado em `da0b593` (2026-11-05
20:20:57 -0300), seis segundos **antes** de `1b5953e` (o commit do código do harness), e esse segundo commit já continha o
`FINDING` do `baseline_splat_colour`. Houve execuções de desenvolvimento antes do primeiro commit com um arquivo de
resultado. Isso não prova que algum número foi ajustado, e também não prova o contrário. O harness verifica ancestralidade e
o livro de emendas, e a fase diz isso no próprio nome dos checks.

Nenhuma emenda mudou um valor que já existia em `da0b593`; os valores novos trazem justificativa no arquivo, e `sensitivity.py`
mede o que cada regra vê. O autor **não** rederivou independentemente os valores antigos. A revisão independente final
(Sonnet 5.5, 5810370582, sobre `7a26569`) julgou as tolerâncias **adequadas, com cegueiras declaradas** (raio do splat em vídeo
cego a ×1,5 em 15 de 30 medições e, em PNG, 0 de 30; azul em 4:2:0; áudio não ouvido), sem finding Médio ou Alto. Isso é um
parecer de um revisor que não é o autor e fecha o item como "parecer independente concluído, limites declarados" (BL019-F4); **não
é** aprovação numérica do usuário, **não** é pré-registro (a revisão também não viu os números antes dos resultados) e nenhum
parâmetro foi relaxado para passar: o aceite é "o harness, como escrito e justificado, passou".

## API lib, CLI headless e UI

- **Lib:** Parcial, ver acima. Nenhum tipo público novo; o harness consome `tessel-render`.
- **CLI headless:** o harness usa `tessel-render`, o processo JSONL sem janela. Os esperados não vêm do core.
- **UI / CLI da janela:** N/A justificado: nenhuma prova por janela, ponteiro físico ou socket. Nada de coordenadas de tela.

## Erros e invariantes

- Arquivo fixado editado fora do livro: `tolerances_frozen` FAIL, com os arquivos e hashes divergentes; livro reiniciado ou
  reescrito em commits publicados: FAIL pelo histórico do Git.
- Número de `tolerances.json` sem justificativa (ou justificativa sem número): `tolerances_frozen` FAIL.
- Corpus adulterado (`--corpus <cópia>`): `corpus` FAIL, saída 1.
- Pixel e áudio nunca são substituídos por schema/ffprobe: o áudio é decodificado para float PCM e comparado com o sinal
  analítico, escalado pela lei de canais, com saída estéreo obrigatória e janela com borda.
- Binário sem atestação válida para o head, árvore suja, subconjunto de fases: nunca "ACCEPTABLE".
- Orçamento: `s10-acc-*` próprio, 2 GiB, recusa início com menos de 12 GiB livres e **para entre fases** se a reserva ou o
  orçamento forem violados (saída 2 sem FAIL; 1 com FAIL ou orçamento estourado).

## O que não é afirmado

Áudio não é *ouvido* (amostras decodificadas, nenhum dispositivo); fótons e seek-to-photon; ponteiro físico; janela
nativa; outras GPUs/plataformas/codecs (uma máquina, SwiftShader); opacidade parcial além da composição `over` do A2,
escalas anisotrópicas, harmônicos esféricos, luz/clima de splat e oclusão por planos; frames/amostras bit a bit; relógio do
editor e playback em dispositivo (BL046); build reproduzível (a atestação identifica o binário e as entradas, não prova que
outro build dá os mesmos bytes: dois builds das mesmas entradas de compilação nesta máquina deram hashes diferentes); aprovação numérica pelo usuário e pré-registro dos
números (o parecer independente da BL019-F4 não os substitui); a divergência `AUDIO_VIDEO_DURATION` do editor (BL016.11). O corpus de retrato julga um marcador em
`@8`/`@15` e três ou quatro nos demais (assertiva do gerador). Independência: os oráculos não leem o destino, mas a lei de
áudio e a janela vêm da leitura da **origem** (citada acima), e o oráculo de câmera reusa `scripts/directed-scene-proof/model.py`.
A lista também sai no `not_claimed` do relatório.

## Evidências e status

Resultado **histórico** (`results.json`, harness `1b5953e`, SwiftShader, macOS arm64, ffmpeg 7.1.1, oráculo anterior às
emendas, árvore com 1 arquivo sujo): saída 1, uma fase FAIL (cor do splat existente, abaixo) e sete BLOCKED. Ele **não
descreve este head** e foi mantido como estava.

Head do harness avaliado: **`fd4bc8c`** (rodada R4), que contém a `main` `09ad62d` (PR #17) por **merge commit** (`aee6091`),
sobre `7a26569`/`4c61de2`. Os commits seguintes só mudam `docs/`: as árvores Git de `crates/`, `runtime/`, `Cargo.toml`,
`Cargo.lock` e `rust-toolchain.toml` do head avaliado são **idênticas às da `main` `09ad62d`** (`2565c250…`, `6ce41fe3…`,
`692fdd1f…`, `f08c67e0…`, `58eef2cc…`), de modo que os gates de produção abaixo são os da `main` integrada; só `scripts/` e
`docs/` diferem dela. O relatório de uma execução não é commitado porque mudaria o head que ela descreve; hashes e saídas ficam
no PR #8 e em `/tmp/s10-evidence-r4/` (manifesto com o SHA-256 no comentário do PR). O piso de disco foi de 12 GiB e a entrada
de cada passo pesado de 16 GiB; a menor reserva medida foi 16,03 GiB.

| Etapa | Resultado | Head |
|---|---|---|
| `proof.spec.py` (`python3 -B`, sem bytecode, árvore limpa) | **102 testes OK** (99 + o teste de merge do coordenador + 2 do `CorpusWriter`) | `fd4bc8c` |
| Mutações relevantes da R4 (temporárias, revertidas, árvore limpa e sem `__pycache__` depois de cada) | **9/9 pegas**: M1 sem `--full-history`, M2 `--first-parent`, M3 R4 removida, M4 `rounds = []`, M5 rodada antiga reescrita, M6 texto da R4 reescrito, M7–M9 `write()` apagando arquivo do chamador, editado ou fora da pasta; controle sem mutação passa | `fd4bc8c` |
| `cargo fmt --all -- --check`, `check --workspace --locked`, `clippy --workspace --all-targets --locked -D warnings` | verdes | `fd4bc8c` |
| `cargo test --workspace --no-fail-fast --locked` | **1406 passados, 0 falhos, 12 ignorados** (87 binários) | `fd4bc8c` |
| `./scripts/runtime setup` + `./scripts/runtime test` com `TESSEL_REQUIRE_RUNTIME=1` e Chrome | **120/120**, 0 pulados (113 + 7 do PR #17) | `fd4bc8c` |
| `cargo build -p tessel-editor -p tessel-editor-render -p tessel-editor-core -p tessel-editor-media --locked` (inclui `tessel-render`, `tessel-headless`, `tessel-media`) | verde | `fd4bc8c` |
| `cargo tree -p tessel-editor-core` | sem `gpui`/`wry`/webview | `fd4bc8c` |
| Diferença JS contra a base (`b8598c7`) | `runtime/src/compositor.js`, `runtime/src/index.js` e `runtime/test/preview-samples.test.mjs` (PR #17), idênticos aos da `main` | `fd4bc8c` |
| **Harness completo vivo**, sem `--allow-stale-binary`, 24 fases | **24/24 PASS**, saída 0, `acceptance.acceptable = true`, 316 s, orçamento de trabalho respeitado (30 MB) | `fd4bc8c` |

Binário `tessel-render` atestado por `link.py` **para este head** (SHA-256 `d976f61b…cab97881`; `runtime.js` `8ee1c496…a0e9e27`; `index.html`
`707fd90f…c484a6`; `link.py check` mantém o vínculo ao `HEAD` depois dos commits só de docs porque nenhuma das entradas
`crates`, `runtime`, `scripts`, `Cargo.*` e toolchain muda neles). Binário velho **nunca** foi aceito.

Checks por fase no run vivo aceito: `tolerances_frozen` 29, `corpus` 44, `models` 25, `controls_camera` 30, `controls_splat` 230,
`controls_audio` 21, `controls_package` 10, `camera_plan` 45, `camera_render` 261, `camera_export` 301,
`baseline_splat_geometry` 13, `baseline_splat_colour` 6, `baseline_audio` 46, `audio_off` 70, `package_portability` 96,
`package_tamper` 13, `splat_plan` 78, `splat_render` 173, `splat_export` 193, `splat_package` 177, `audio_export` 270
(24 fps e `audio_ntsc_window`), `audio_package_export` 94, `audio_short_source` 12, `full_av` 432 (splat e áudio na mesma
cena e no mesmo export, e de novo no projeto restaurado).

Ambiente: macOS arm64, SwiftShader, ffmpeg/ffprobe 7.1.1, Node v22.23.3, rustc 1.99.1, Python 3.14.3.

Histórico (head anterior `ad9e182`, antes da R4 e da `main` com o PR #17): 99 testes, mutações M01–M15 pegas, gates em `6073f1e`
(fmt/check/clippy/build verdes, `cargo test --workspace` 1397 passados, 0 falhos, **12 ignorados**; runtime 113/113) e 24/24 vivas
(`full_av` 432, `audio_export` 270, 380 s). Ficam como histórico, não são evidência do head atual.

### Tentativas preservadas (nunca contam como PASS)

1. **Run vivo 1** (`6073f1e`): fases até `package_portability` passaram; `package_tamper` e as seguintes saíram BLOCKED
   ("binary not built") porque o diretório de alvos compartilhado do Cargo deste worktree foi descartado pelo cache durante a
   execução (não por esta tarefa). Saída 3. Repetido com uma cópia privada do binário (`--bin-render`).
2. **Run vivo 2**: abortado por mim antes de qualquer fase (a trava de GPU foi pedida duas vezes, uma por fora do `step.py` e
   outra por dentro). Nenhuma fase rodou.
3. **Run vivo 3** (`6073f1e`): 23 PASS e `full_av` FAIL 418/422 (A13 acima). Mantido como FAIL. Depois dele descobri que o
   `__pycache__` do harness guardava bytecode velho de uma mutação revertida (mesmo tamanho, mesmo segundo de mtime); o `proof.py`
   roda como `__main__` (sem pyc), mas os módulos auxiliares podem ter vindo do cache, **então este run não é aceito como
   evidência de harness limpo**; o aceite acima rodou com `PYTHONDONTWRITEBYTECODE=1`, cache apagado, `proof.spec.py -B`, e as
   mutações foram refeitas com `-B`. O bytecode velho **não é a causa** do FAIL de `full_av` (a causa é a janela sem borda da A13,
   reproduzida com o código inalterado e sem mutação; a revisão independente confirmou): é uma dívida real e separada, a BL019-F11.
   Nesta rodada (R4) os módulos rodaram com `-B`/`PYTHONDONTWRITEBYTECODE=1` e nenhum `__pycache__` existia antes nem depois.
4. **Passo `gate-core-tree`, primeira tentativa (R4)**: recusado ao iniciar (`refused to start: 15.91 GiB free, need 16.0`, saída 77),
   porque a reserva de disco caiu 0,09 GiB abaixo da entrada de 16 GiB durante os gates; o registro fica como está e o passo foi
   repetido (reserva 16,1 GiB) com saída 0. Nada foi limpo nem apagado para isso.

### Achado BL019-F1 — cor do splat escurecida no caminho existente (resolvido no S7)

O splat da camada de projeto desenhava o centro de um splat opaco com a cor armazenada passada por `sRGB→linear` e **sem**
recodificação: stored `(230,49,49)` aparecia como `(203,7,7)` (`diagnosis` do relatório histórico). Causa observada: o
compositor define `renderer.outputColorSpace = LinearSRGBColorSpace` (`runtime/src/compositor.js`) e o Spark liga `encodeLinear`
quando `outputColorSpace !== SRGBColorSpace` (`@sparkjsdev/spark 2.4.0`). O S7 corrigiu o caminho (`keepStoredColour` em
`runtime/src/splat-stage.js`, BL019-F1); o oráculo de cor **não foi relaxado**, e `baseline_splat_colour` passa 6/6 neste head.

O PR #8 saiu de draft por autorização do usuário **para receber feedback**; isso não é aprovação nem aceite de revisor.
