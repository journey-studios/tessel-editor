# F044 — Aceite independente de cena dirigida: splat, câmera, áudio, pacote e export (BL019)

## Objetivo e limites

Um harness que julga o editor por **esperados calculados fora dele** e por **tolerâncias fixadas antes de qualquer
resultado**, com **controles negativos** que precisam falhar. Cobre câmera, splat, áudio, pacote/restauração e export
da cena dirigida. É uma frente de prova (BL019, ampliação S10): **não muda nenhum comportamento do editor**, não
adiciona comando, nó de árvore, handler ou dependência. Por isso a linha da matriz tem N/A justificado para UI e
CLI da janela (o harness dirige o processo `tessel-render`, não a janela) e Lib/CLI headless descrevem a superfície
realmente exercida.

Diretório: `scripts/directed-scene-acceptance/`.

| Arquivo | Papel |
|---|---|
| `tolerances.json` | Limites numéricos, controles negativos obrigatórios e política BLOCKED. Commitado **antes** do harness e de qualquer execução (`git log --diff-filter=A`); o relatório imprime o commit e o hash. Mudar depois de um resultado é uma *emenda* registrada em `amendments`. |
| `oracle.py` | Modelo independente: projeção da câmera e máscara exata de cada marcador por ray-cast, centro/raio de gaussianas, tabela de tons e mapeamento racional quadro→amostra. Não importa nada do editor. Escreve SPZ v2 e WAV. |
| `corpus.py` | Gera `corpus/` de forma determinística (PNG de cor sólida, SPZ de 45–62 bytes, WAV senoidal de 44,1 kHz) com `provenance.json` (hashes pinados, gerador, sem mídia privada). |
| `verify.py` | Verificadores: máscara/centroide/IoU/cor interior em imagem real, splat (centroide, cor, r50, ordem), áudio decodificado por ffmpeg (início, correlação com lag, ganho, silêncio, estreitamento espectral), pacote lido por `zipfile`/diretório (hash exato). |
| `controls.py` | Entradas **certas e erradas** para cada verificador (FOV +5°, dolly invertido, espelhado, um quadro atrasado, canais trocados, ordem trocada, áudio deslocado 5 ms/1 quadro, taxa 44,1 lida como 48, ganho −6 dB, NTSC tratado como 30 fps, byte virado no pacote …). Cada controle congelado em `tolerances.json` precisa ser exercitado e falhar. |
| `proof.py` | Driver: fases, relatório JSON, política de saída. Reusa só a infraestrutura (`harness.py`, trava de GPU, `JsonlSession`) de `scripts/directed-scene-proof`. |
| `proof.spec.py` | 16 testes do próprio harness (sem GPU/Chrome), incluindo a cola das fases dependentes contra *fakes* que respondem como o seam especificado. |

## Contrato do harness

```text
python3 scripts/directed-scene-acceptance/proof.py --list
python3 scripts/directed-scene-acceptance/proof.py --phases tolerances_frozen,corpus,models,controls_camera
python3 scripts/directed-scene-proof/with-gpu-lock.py \
  python3 scripts/directed-scene-acceptance/proof.py --gpu-lock-held --report /tmp/s10.json
```

Pré-requisitos das fases com imagem: `./scripts/runtime setup`, `tessel-render` compilado
(`python3 /Users/wallyssonnunes/.cache/tessel-editor/cargo-serial.py build -p tessel-editor-render --locked`),
Chrome/SwiftShader e ffmpeg/ffprobe. Uma fase sem pré-requisito sai BLOCKED, nunca pass.

| Saída | Significado |
|---|---|
| `0` | toda fase selecionada rodou e passou |
| `1` | pelo menos uma FAIL (também: corpus adulterado, orçamento de 2 GiB estourado) |
| `2` | erro de uso do harness (fase desconhecida, corpus ausente) |
| `3` | nenhuma FAIL, mas ao menos uma fase **BLOCKED** (dependência ausente). BLOCKED nunca conta como pass |

### Fases

| Grupo | Fases | O que prova |
|---|---|---|
| Dados | `tolerances_frozen`, `corpus`, `models` | tolerâncias commitadas antes dos resultados (hash, nenhuma edição fora de commit, emendas registradas); corpus byte-idêntico ao gerador e com hashes pinados; modelos contra identidades analíticas e o oráculo da origem |
| Controles | `controls_camera`, `controls_splat`, `controls_audio`, `controls_package` | cada verificador **aceita** a entrada certa e **recusa** cada entrada errada congelada |
| Câmera | `camera_plan`, `camera_render`, `camera_export` | pose/matriz do `scene.plan`; máscaras reais de `scene.render` contra o ray-cast, em 640×360 e 360×640 e FOV 30/50/75; frames do MP4 de `scene.export` decodificado |
| Baseline | `baseline_splat_geometry`, `baseline_splat_colour`, `baseline_audio` | o splat da camada de projeto e o mixer offline que **existem hoje**, para confiar nos verificadores antes das frentes dependentes |
| Áudio desligado e pacote | `audio_off`, `package_portability`, `package_tamper` | `audio:false` preservado (sem stream, aviso); pacote lido por leitor que não é o do editor, origem apagada, restauração, render e export; pacote adulterado/incompleto recusado sem criar pasta |
| Seams dependentes | `splat_plan`, `splat_render`, `splat_export`, `splat_package` (S7), `audio_export`, `audio_package_export` (S9), `full_av` (S7+S9) | os mesmos verificadores contra splat na cena dirigida e áudio mixado no export. **BLOCKED** enquanto a frente não existir |

### Seams aguardados (nada foi inventado)

| Frente | Sinal de ausência observado | Quando vira avaliação |
|---|---|---|
| S7 / BL025 | `scene.plan` de um bundle com nós `splat` responde `unsupported` com `details.domains` contendo `splat` | `ScenePlan` com referência aprovada, geração, matriz e câmera avaliadas; render/preview/export coincidentes |
| S9 / BL016 | `scene.export` com `audio` padrão responde `unsupported` com `details.domains` contendo `audio` | mix do WAV do bundle no arquivo, mapeamento racional, offset, conversão de taxa; `audio:false` omite com aviso |

Quando o seam existir, as fases rodam **sem mudança de verificador ou tolerância** (a cola foi exercitada com fakes
em `proof.spec.py`). Se o seam responder por outro motivo, a fase é FAIL, não BLOCKED.

## API lib, CLI headless e UI

- **Lib:** N/A. Nenhum tipo público novo; o harness consome `tessel-render` (os mesmos `scene.*`/`project.*` da lib).
- **CLI headless:** o harness usa `tessel-render`, o processo JSONL sem janela. Os esperados não vêm do core.
- **UI / CLI da janela:** N/A justificado — nenhuma prova por janela, ponteiro físico ou socket aqui; o export pela janela
  é do F042 (`scripts/directed-scene-export-proof`). Nada de coordenadas de tela.

## Erros e invariantes

- Tolerância alterada após resultado: `tolerances.json` muda de hash, `amendments` deixa de ser vazio e o relatório
  imprime os dois; `tolerances_frozen` falha se houver edição não commitada ou commits sem emenda registrada.
- Corpus adulterado (`--corpus <cópia>`): `corpus` FAIL, saída 1 (verificado com um byte virado em `mk-red.png`).
- Pixel e áudio nunca são substituídos por schema/ffprobe: ffprobe só confere forma (fps, duração, streams); a imagem
  é decodificada (BT.709 limitado) e comparada com a máscara exata; o áudio é decodificado para float PCM e comparado
  com o sinal analítico.
- Orçamento: diretório `s10-acc-*` próprio, 2 GiB, recusa início com menos de 10 GiB livres, remove só a si mesmo.

## O que não é afirmado

Áudio não é *ouvido* (amostras decodificadas, nenhum dispositivo); fótons e seek-to-photon; ponteiro físico; janela
nativa; outras GPUs/plataformas/codecs (uma máquina, SwiftShader); opacidade parcial, escalas anisotrópicas,
harmônicos esféricos, luz/clima de splat e oclusão por planos; frames/amostras bit a bit; relógio do editor e playback
em dispositivo (BL046). A lista também sai no `not_claimed` do relatório.

## Evidências e status

Execução completa (`proof.py --gpu-lock-held`, SwiftShader, macOS arm64, ffmpeg 7.1.1, `tessel-render` debug construído
no worktree da frente, harness no commit `1b5953e`): relatório em [`results.json`](../../scripts/directed-scene-acceptance/results.json)
(hashes dos arquivos do harness, do corpus, do binário e do bundle do runtime; tolerâncias introduzidas em `da0b593`,
`amendments: []`). **Saída 1**: uma fase FAIL (achado abaixo) e sete BLOCKED. Nenhum limite foi alterado depois do primeiro resultado.

| Fase | Resultado | Checks |
|---|---|---|
| `tolerances_frozen`, `corpus`, `models` | PASS | 5, 32, 16 |
| `controls_camera`, `controls_splat`, `controls_audio`, `controls_package` | PASS (cada controle negativo congelado é exercitado e falha) | 11, 13, 26, 8 |
| `camera_plan` / `camera_render` / `camera_export` | PASS | 45 / 252 / 292 |
| `baseline_splat_geometry` | PASS | 13 |
| `baseline_splat_colour` | **FAIL** (achado BL019-F1) | 2/7 |
| `baseline_audio` | PASS | 46 |
| `audio_off` / `package_portability` / `package_tamper` | PASS | 70 / 95 / 13 |
| `splat_plan`, `splat_render`, `splat_export`, `splat_package`, `full_av` | **BLOCKED** (S7 / BL025) | — |
| `audio_export`, `audio_package_export` | **BLOCKED** (S9 / BL016) | — |

Outras evidências reproduzíveis:

- `python3 scripts/directed-scene-acceptance/proof.spec.py` — 16 testes, OK (sem GPU).
- Corpus adulterado: `proof.py --phases corpus --corpus <cópia com 1 byte virado em mk-red.png>` → 6 checks FAIL, saída 1.
- Fases de dados e controles sem GPU: `proof.py --phases tolerances_frozen,corpus,models,controls_camera,controls_splat,controls_audio,controls_package` → saída 0.
- Orçamento: 12,3 MB de artefatos temporários na execução completa (limite 2 GiB), removidos ao final.

### Achado BL019-F1 — cor do splat escurecida no caminho existente

O splat da camada de projeto (o único caminho com splat hoje) desenha o centro de um splat opaco com a cor armazenada
passada por `sRGB→linear` e **sem** recodificação: stored `(230,49,49)` aparece como `(203,7,7)`, igual ao previsto por
essa conversão (`diagnosis` do relatório: distância ≤ 4 nas três cores). Causa observada no código: o compositor define
`renderer.outputColorSpace = LinearSRGBColorSpace` (`runtime/src/compositor.js`, para que imagens cheguem intactas) e o
Spark liga `encodeLinear` quando `outputColorSpace !== SRGBColorSpace` (`@sparkjsdev/spark 2.4.0`). A geometria (centroide e
raio de meia intensidade) passa; o oráculo de cor não foi relaxado, e a fase fica FAIL até a decisão registrada em
BL019/BL025: S7 precisa fazer o splat dirigido mostrar a cor armazenada (por exemplo, desligando a codificação linear só
para o splat) **ou** o contrato de cor muda por decisão explícita (emenda nas tolerâncias, com este resultado como motivo).

### Aceite vivo (BLOCKED)

As fases do S7 e do S9 saem **BLOCKED** (código 3 se forem as únicas pendentes). O PR permanece **draft** enquanto
`splat_*`, `audio_*` e `full_av` não rodarem e passarem com o merge de S7/S9. Nenhuma cena com splat ou áudio mixado foi
julgada por este harness: a cobertura dessas duas famílias hoje é o baseline do caminho existente e a cola testada com fakes.
