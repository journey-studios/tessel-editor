# F044 — Aceite independente de cena dirigida: splat, câmera, áudio, pacote e export (BL019)

## Objetivo e limites

Um harness que julga o editor por **esperados calculados fora dele**, com **controles negativos** que precisam falhar e
um **livro de emendas** que registra toda mudança do oráculo, dos verificadores, dos controles, do corpus e das
tolerâncias. Cobre câmera, splat, áudio, pacote/restauração e export da cena dirigida. É uma frente de prova (BL019,
ampliação S10): **não muda nenhum comportamento do editor**, não adiciona comando, nó de árvore, handler ou dependência.

Superfícies (matriz F044): UI e CLI da janela são N/A (o harness dirige o processo `tessel-render`, não a janela; o
export pela janela é do F042, `scripts/directed-scene-export-proof`). CLI headless e Lib são **Parcial**: o harness
exercita as mesmas operações da lib (`scene.*`, `project.*`) pelo processo JSONL sem janela, mas não adiciona teste de
unidade à lib, e as fases dependentes de S7/S9 só têm veredito vivo quando essas frentes forem aprovadas e integradas.

Diretório: `scripts/directed-scene-acceptance/`.

| Arquivo | Papel |
|---|---|
| `tolerances.json` | Limites numéricos, controles negativos obrigatórios e política BLOCKED. Todo limiar que um verificador usa mora aqui (inclusive área mínima, folga de vídeo, janela e fração de borda do áudio). |
| `amendments.json` + `ledger.py` | O livro de emendas: baseline verificado no Git, rodadas com motivo, achado, emendas e SHA-256 (antes/depois) dos seis arquivos fixados. `tolerances_frozen` falha se um arquivo fixado difere do último estado registrado. |
| `oracle.py` | Modelo independente: projeção da câmera e máscara exata por ray-cast, centro/raio e composição `over` de gaussianas, tabela de tons, janela de áudio da origem e lei de canais. Não importa nada do editor. Escreve SPZ v2 e WAV. |
| `corpus.py` | Gera `corpus/` de forma determinística (PNG de cor sólida, SPZ de 45–62 bytes, WAV senoidal de 44,1 kHz de 13 s, mais o WAV de 8 s da emenda A1 como fixture negativa) com `provenance.json`. |
| `verify.py` | Verificadores: máscara/centroide/IoU/cor em imagem real, marcador ausente, splat (centroide, cor, r50, ordem e composição), áudio decodificado por ffmpeg (início, correlação, ganho pela lei de canais, silêncio, bordas), pacote lido por `zipfile`/diretório e hashes declarados. |
| `controls.py` | Entradas **certas e erradas** para cada verificador, em paisagem e retrato, em **todos** os quadros de splat; áudio sempre na forma que a lei de canais prescreve. Cada controle congelado em `tolerances.json` precisa ser exercitado e falhar. |
| `proof.py` | Driver: fases, relatório JSON, política de saída. Reusa só a infraestrutura (`harness.py`, trava de GPU, `JsonlSession`) de `scripts/directed-scene-proof`. |
| `proof.spec.py` | 50 testes do próprio harness (sem GPU/Chrome), incluindo a cola das fases dependentes contra *fakes* e mutações que o teste precisa pegar. |
| `history/A1-audio-window/` | Registro imutável do estado anterior à emenda A1 e do FAIL histórico que a motivou. |
| `results.json` | Resultado **histórico** da primeira execução (harness `1b5953e`, árvore com 1 arquivo sujo, oráculo anterior às emendas). Não é aceite deste head e não foi reescrito. |

## Contrato do harness

```text
python3 scripts/directed-scene-acceptance/proof.py --list
python3 scripts/directed-scene-acceptance/proof.py --phases tolerances_frozen,corpus,models,controls_camera
python3 scripts/directed-scene-proof/with-gpu-lock.py \
  python3 scripts/directed-scene-acceptance/proof.py --gpu-lock-held --report /tmp/s10.json
```

Pré-requisitos das fases com imagem: `./scripts/runtime setup`, `tessel-render` compilado
(`./scripts/cargo build -p tessel-editor-render --locked`), Chrome/SwiftShader e ffmpeg/ffprobe. Uma fase sem
pré-requisito sai BLOCKED, nunca pass. Um `tessel-render` **mais velho que as fontes** do checkout também é BLOCKED, a
menos que se passe `--allow-stale-binary` (o relatório registra a escolha).

| Saída | Significado |
|---|---|
| `0` | toda fase selecionada rodou e passou |
| `1` | pelo menos uma FAIL, ou o orçamento de 2 GiB estourou |
| `2` | erro de uso ou pré-requisito recusado (fase desconhecida, corpus ausente, **disco abaixo de 12 GiB**: na partida ou entre fases) sem FAIL |
| `3` | nenhuma FAIL, mas ao menos uma fase **BLOCKED**. BLOCKED nunca conta como pass **e nunca substitui uma FAIL** (uma fase com checagens reprovadas continua FAIL mesmo se um passo seguinte bloquear) |

`--work <dir>` aponta uma pasta **do chamador**: o harness cria e remove só o seu `s10-acc-*` dentro dela e nunca toca no
resto (teste com arquivo sentinela). Cada fase e cada pedido usam diretórios próprios (`fresh`), de modo que um destino
deixado por uma fase ou sonda nunca vira `destination_exists` em outra.

### Fases

| Grupo | Fases | O que prova |
|---|---|---|
| Dados | `tolerances_frozen`, `corpus`, `models` | ancestralidade Git das tolerâncias, livro de emendas íntegro (cadeia, baseline no Git, nenhum arquivo fixado editado fora do registro); corpus byte-idêntico ao gerador e com hashes pinados; modelos contra identidades analíticas e o oráculo da origem |
| Controles | `controls_camera`, `controls_splat`, `controls_audio`, `controls_package` | cada verificador **aceita** a entrada certa e **recusa** cada entrada errada congelada (câmera em paisagem e retrato; splat nos seis quadros; áudio pela lei de canais) |
| Câmera | `camera_plan`, `camera_render`, `camera_export` | pose/matriz do `scene.plan`; máscaras reais de `scene.render` contra o ray-cast, em 640×360 e 360×640 e FOV 30/50/75, incluindo marcador que o modelo põe fora do quadro; frames do MP4 de `scene.export` |
| Baseline | `baseline_splat_geometry`, `baseline_splat_colour`, `baseline_audio` | o splat da camada de projeto e o mixer offline que **existem hoje** (o clipe do baseline de áudio agora lê o arquivo em `editStart + t`, como a origem) |
| Áudio desligado e pacote | `audio_off`, `package_portability`, `package_tamper` | `audio:false` preservado; pacote lido por leitor que não é o do editor (manifesto sem `files` é falha), restauração com a origem apagada; pacote adulterado recusado sem criar pasta |
| Seams dependentes | `splat_plan`, `splat_render`, `splat_export`, `splat_package` (S7), `audio_export`, `audio_package_export`, `audio_short_source` (S9), `full_av` (S7+S9) | os mesmos verificadores contra splat dirigido e áudio mixado no export; `audio_short_source` é o controle negativo vivo (fonte menor que a janela deve ser recusada com `audio_source_short`) |

### Seams aguardados

| Frente | Sinal de ausência observado | Quando vira avaliação |
|---|---|---|
| S7 / BL025 | `scene.plan` de um bundle com nós `splat` responde `unsupported` com `details.domains` contendo `splat` | `ScenePlan` com referência aprovada, geração, matriz e câmera avaliadas; render/preview/export coincidentes |
| S9 / BL016 | `scene.export` com `audio` padrão responde `unsupported` com `details.domains` contendo `audio` | mix do WAV do bundle no arquivo, mapeamento racional, janela da origem, lei de canais; `audio:false` omite com aviso |

Os veredictos vivos de S7 e S9 só valem para **heads aprovados e integrados** (BL121). Rodar o harness contra um head
em correção, ou contra *fakes*, não é aceite.

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
`full` `[0, 2)`), com assertivas do gerador para as duas regras. O WAV de 8 s anterior fica **byte a byte** como
`mix-short.wav`, fixture do controle negativo vivo `audio_short_source`. O FAIL histórico (`audio_export` 75/92 contra a S9
original e o `destination_exists` da sonda) está preservado em `history/A1-audio-window/` com os hashes anteriores. A
causa era do oráculo, não do editor. O baseline de áudio 46/46 **nunca validou** esta regra (o clipe do baseline começava
em 24 com fonte 0, o que satisfazia a suposição antiga); agora o clipe cobre a edição e lê o arquivo a partir de
`editStartSeconds`. A emenda parcial preservada em `wave-resource-stop/S10` foi lida, conferida contra a origem e
aplicada; o que não estava nela (A2–A5) foi acrescentado aqui.

### A2 — composição na mistura do splat (F2, BL120)

No centro do splat de trás (`s-order`) a gaussiana da frente tem opacidade 0,80–0,92 conforme o quadro, então exigir ali a
cor da frente dentro de 40 reprovava um render correto nos quadros 36 e 47 (distâncias 43,5 e 52,3 pela composição do
próprio modelo; `proof.spec.py` reproduz isso). Agora: no centro da frente, a cor da frente dentro de 40 e ≥ 80 da cor de
trás (inalterado); no centro de trás, a **composição `over` do oráculo** (`oracle.composite_at`) dentro dos mesmos 40
(`overlapBackCompositionDelta`, **não alargado**) e ≥ 80 da cor própria de trás. Controles positivos e negativos (ordem
trocada incluída) rodam em **todos** os `SPLAT_FRAMES`. Esta é a única lei de mistura modelada; luz linear e tonemapping
continuam não modelados.

### A3 — lei de ganho de canais (F3, BL019-F3)

A lei esperada deixou de ser 0 dB. A origem força `aformat=...:channel_layouts=stereo` e soma com `amix normalize=0`
(`audio-graph.ts`), de modo que vale o *rematrix* padrão do FFmpeg: fonte mono entra em E e D a `1/√2` (−3,0103 dB), layout
igual é intacto (`oracle.channel_gain_db`; qualquer outro par levanta erro em vez de ser adivinhado). `proof.spec.py`
**rederiva o número executando o filtro da origem** no FFmpeg local, sem ler o renderizador. O fake "mix correto" e os
controles gravam o sinal pela lei; um novo controle negativo, "mono source written at 0 dB", precisa falhar, e o limiar de
borda passou a ser relativo ao nível esperado (já com a lei).

### A4 — limiares fora do verificador (F5)

`minJudgedAreaPx` (300), `videoBackgroundExtraDelta` (6), `onsetSearchWindowSeconds` (0,08) e `onsetThresholdFraction`
(0,35) saíram do código para `tolerances.json` com o mesmo valor. Novo: `absentMarkerMaxPixels` (20): um marcador que o modelo
põe fora do quadro precisa deixar no máximo 20 pixels da sua cor (antes só era "não julgado").

### A5 — proveniência e controles (F4, F5, baixos)

Reescrita da proveniência (abaixo), controles de retrato (`360x640@24`, clipe da própria tomada), controle da leitura
aposentada de A1, controle de hash declarado que agora **chama** `verify.judge_declared_hashes` em vez de reimplementar a
regra, e asserção do gerador de que todo quadro de retrato julga ao menos um marcador.

## Proveniência das tolerâncias (F4)

**Não afirmamos pré-registro.** O que o Git prova: `tolerances.json` foi commitado em `da0b593` (2026-11-05 20:20:57 -0300),
seis segundos **antes** de `1b5953e` (o commit do código do harness), e esse segundo commit já continha o `FINDING` do
`baseline_splat_colour`. Houve execuções de desenvolvimento antes do primeiro commit com um arquivo de resultado. Isso não
prova que algum número foi ajustado, e também não prova o contrário. O que o harness verifica hoje é ancestralidade e o
livro de emendas, e a fase diz isso no próprio nome dos checks.

Nenhuma emenda mudou um valor que já existia em `da0b593`; os valores novos trazem a justificativa `why…` no arquivo. O
autor **não** rederivou os valores antigos de forma independente. **Revisão independente de todos os números por quem
não é o autor é pré-requisito do próximo resultado vivo aceito**; ela não foi feita pelo autor e não é afirmada aqui.

## API lib, CLI headless e UI

- **Lib:** Parcial, ver acima. Nenhum tipo público novo; o harness consome `tessel-render`.
- **CLI headless:** o harness usa `tessel-render`, o processo JSONL sem janela. Os esperados não vêm do core.
- **UI / CLI da janela:** N/A justificado: nenhuma prova por janela, ponteiro físico ou socket. Nada de coordenadas de tela.

## Erros e invariantes

- Arquivo fixado editado fora do livro: `tolerances_frozen` FAIL, com os arquivos e hashes divergentes.
- Corpus adulterado (`--corpus <cópia>`): `corpus` FAIL, saída 1.
- Pixel e áudio nunca são substituídos por schema/ffprobe: o áudio é decodificado para float PCM e comparado com o sinal
  analítico, escalado pela lei de canais.
- Orçamento: `s10-acc-*` próprio, 2 GiB, recusa início com menos de 12 GiB livres e **para entre fases** se a reserva ou o
  orçamento forem violados (saída 2 sem FAIL; 1 com FAIL ou orçamento estourado).

## O que não é afirmado

Áudio não é *ouvido* (amostras decodificadas, nenhum dispositivo); fótons e seek-to-photon; ponteiro físico; janela
nativa; outras GPUs/plataformas/codecs (uma máquina, SwiftShader); opacidade parcial além da composição `over` do A2,
escalas anisotrópicas, harmônicos esféricos, luz/clima de splat e oclusão por planos; frames/amostras bit a bit; relógio do
editor e playback em dispositivo (BL046). O corpus de retrato julga um marcador em `@8`/`@15` e três ou quatro nos demais
(assertiva do gerador). Independência: os oráculos não leem o destino, mas a lei de áudio e a janela vêm da leitura da
**origem** (citada acima), e o oráculo de câmera reusa `scripts/directed-scene-proof/model.py`. A lista também sai no
`not_claimed` do relatório.

## Evidências e status

Resultado **histórico** (`results.json`, harness `1b5953e`, SwiftShader, macOS arm64, ffmpeg 7.1.1, oráculo anterior às
emendas, árvore com 1 arquivo sujo): saída 1, uma fase FAIL (cor do splat existente, abaixo) e sete BLOCKED. Ele **não
descreve este head** e foi mantido como estava.

Este head (o relatório de uma execução não é commitado porque mudaria o head que ela descreve; hashes e saídas ficam no
corpo do PR #8):

- **Executado:** `python3 scripts/directed-scene-acceptance/proof.spec.py`, 50 testes, OK, sem GPU. Mutações temporárias
  confirmadas e revertidas: BLOCKED sobrescrevendo FAIL, `Workspace.close` removendo a base, lei de ganho 0 dB, regra antiga
  de mistura, janela de áudio da suposição aposentada. Fases `tolerances_frozen`, `corpus`, `models`, `controls_camera`,
  `controls_splat`, `controls_audio` e `controls_package` por `proof.py` (PASS, em árvore de trabalho ainda sem commit).
- **NÃO executado nesta rodada:** nenhuma fase viva (`camera_*`, `baseline_*`, `audio_*`, `splat_*`, `full_av`, `package_*`)
  e nenhum gate Cargo (fmt/check/clippy/test/build), porque o disco ficou abaixo do piso de 12 GiB; o próprio harness
  recusou iniciar (saída 2, sem FAIL). O oráculo emendado **ainda não foi confrontado com um editor real**: as correções de
  A1–A3 estão sustentadas pela leitura da origem, pela aritmética e pelo FFmpeg local, não por um export vivo.
- O aceite vivo combinado (BL121) depende dos heads aprovados de S7 (PR #13) e S9 (PR #11); antes disso nada aqui é aceite.

O PR #8 saiu de draft por autorização do usuário **para receber feedback**; isso não é aprovação nem aceite.

### Achado BL019-F1 — cor do splat escurecida no caminho existente

O splat da camada de projeto (o único caminho com splat antes de S7) desenha o centro de um splat opaco com a cor
armazenada passada por `sRGB→linear` e **sem** recodificação: stored `(230,49,49)` aparece como `(203,7,7)`, igual ao
previsto por essa conversão (`diagnosis` do relatório histórico: distância ≤ 4 nas três cores). Causa observada no código: o
compositor define `renderer.outputColorSpace = LinearSRGBColorSpace` (`runtime/src/compositor.js`) e o Spark liga
`encodeLinear` quando `outputColorSpace !== SRGBColorSpace` (`@sparkjsdev/spark 2.4.0`). A geometria passa; o oráculo de cor
não foi relaxado, e a fase fica FAIL até a decisão registrada em BL019/BL025.
