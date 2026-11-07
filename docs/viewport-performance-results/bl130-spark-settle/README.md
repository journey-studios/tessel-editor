# BL130 — ordem dos splats sem bloquear o movimento (2026-11-06)

Contrato em [spark.md](../../features/spark.md#ordem-dos-splats-em-movimento-bl130) e
[viewport.md](../../features/viewport.md#ordem-dos-splats-por-movimento-bl130). Esta pasta guarda só números e logs; **nenhuma imagem
do projeto de teste** (as PNG da ferramenta de evidência ficam fora do repositório).

## Condições

- Editor **release** (`./scripts/cargo build --release -p tessel-editor --locked`), M3 Pro, GPU **compartilhada** com outros agentes,
  load average 14–44 durante as corridas (o desvio disso aparece nas faixas).
- O mesmo projeto, a mesma cópia (`tessel-splat-h`, 1280×720/24 fps, 4 cenas de 1.179.648 gaussians), sem gerar mídia nova.
- "Antes" = o binário e o runtime de `main` com BL129 e BL131 (`09ad62d`); "latest" = este PR; "exact" = este PR com
  `TESSEL_VIEWPORT_PREVIEW_SORT=exact` (reversão, mesmo binário e runtime do "latest").
- 3 sessões de editor por variante, intercaladas (antes, latest, exact, antes, ...), cada uma com 3 repetições de playback
  (`40:60`, `181:30`) e de scrub (`take:20:100:60`, `xfade:150:100:60`):

```
python3 scripts/preview-quality-proof.py --binary BIN --runtime-dir RT --project COPIA.tessel.json \
    --label V-sN --reps 3 [--env TESSEL_VIEWPORT_PREVIEW_SORT=exact]
```

- Relógio do próprio editor (`TESSEL_VIEWPORT_TRACE=1`, aceito, não exibido). `--inspect` muda o tempo e **não** conta como
  benchmark (só os hashes abaixo). Nenhum rastro truncado foi contado como perda (o harness recusa a janela truncada).

## Resultados (média das repetições; faixa entre parênteses)

Playback é por região; `n` = repetições × sessões. `runs/*-sN.log` têm cada repetição.

| Medida | Antes (n) | exact (n) | latest (n) |
|---|---|---|---|
| Desenho no runtime, take, p50 | 16,3 ms (18) | 17,7 ms (18) | **1,0 ms** (18) |
| Desenho no runtime, crossfade, p50 | 43,1 ms (8) | 48,1 ms (8) | **1,0 ms** (9) |
| Take, quadros sem imagem | 3,4 % (0–39,1) | 3,3 % (0–42,9) | **0 %** (0–0) |
| Crossfade, quadros sem imagem | 8,0 % (0–21,4) | 17,9 % (7,1–35,7) | **0 %** (0–0) |
| Intervalo entre imagens, take, p50 | 43,3 ms | 42,9 ms | 44,0 ms |
| Intervalo entre imagens, crossfade, p50 | 46,1 ms | 50,4 ms | 42,8 ms |
| Scrub take, imagens/s | 42,1 (38,0–46,5) | 37,8 (19,5–43,5) | **49,9** (49,5–50,0) |
| Scrub crossfade, imagens/s | 38,9 (35,5–42,5) | 33,6 (19,5–36,5) | **49,8** (49,0–50,0) |
| Atraso do scrub (mediana, quadros) | 1,0 | 1,0–1,1 | **0** |
| Parou o playback → quadro final | 297 ms (179–430) | 327 ms (216–431) | 304 ms (202–403) |
| Soltou o scrub → quadro final, take | 272 ms (223–347) | 299 ms (226–357) | 349 ms (337–365) |
| Soltou o scrub → quadro final, crossfade | 253 ms (222–302) | 304 ms (229–396) | 361 ms (343–387) |

Leitura honesta: o desenho deixou de ser o gargalo (1 ms) e o gesto fica no ritmo do relógio (≈ 24 imagens/s do playback, 50 do
scrub, o teto do sondador de 60 Hz com 2 s). **O custo** é o quadro final depois de soltar o scrub, ≈ 90 ms mais tarde (a ordenação
exata passa a ser feita só então); depois de parar o playback o tempo é o mesmo. O `exact` do mesmo binário reproduz o antes dentro do
ruído (a faixa maior do `exact` coincide com as sessões de load 36–44): a diferença não é da máquina nem do binário.

## Invariantes (executados; `--inspect`, não é benchmark)

- Hash do `render.frame` dos quadros 5, 60, 100, 193, 300 e 450 **igual** antes e depois:
  `d64ea0612003aac5 4632654380285dee 2f7fea6510a0df39 65bad546acc28c04 2fc61bb00b19972d 9f806dd28d080ad5`
  (`runs/inspect2-base.log`, `runs/inspect2-new.log`).
- **O hash é o do quadro que o runtime devolveu (`frame.done.hash` do último quadro aceito pelo monitor), não uma captura dos pixels da
  tela.** A apresentação na janela nativa não foi verificada por estas provas (BL130.6, revisão visual nativa pendente).
- Último quadro devolvido depois de parar o playback = `render.frame` do mesmo quadro (`refined_equals_explicit.equal: true`) nas 4 paradas
  (`40:60` e `181:30`, 2 repetições, antes e depois), com `final_source.stale_sort_layers` 0.
- Último quadro devolvido depois de soltar o scrub = `render.frame` do mesmo quadro: take (quadro 20, `adc70df7ad9c472d`) e crossfade (quadro 150,
  `d9c627b2d86f1134`), 2 repetições cada, **o mesmo hash antes e depois** (`runs/inspect3-*.log`; a comparação no scrub é do harness
  novo, `refined_comparison`).
- Export de 6 quadros de crossfade (190–195, `fullhd`, `export.start` pela janela real): `framemd5` idêntico por quadro e **arquivo
  idêntico** (436.972 bytes, mesmo sha256) antes e depois (`runs/export-*.log`).

## Diferença entre o quadro de movimento e o exato (câmera animada; `evidence/*.json`)

`runtime/scripts/sort-order-evidence.mjs` com `TESSEL_GL=gpu` (worker real, 960×540), planos reais de quadros consecutivos (câmera
animada) do take (`take`, passos `s4/s8/s16`) e do crossfade: cada quadro de movimento (ordem de outra câmera, `stale_sort_layers` 1) é
comparado com o quadro exato do mesmo quadro.

**O que isto não é.** A ferramenta lê cada quadro como PNG, o que espera a GPU e a ordenação: o atraso de ordenação que o monitor
tem aqui é **menor**, então os números **não são um limite superior** do que um playback mostra. Mudar o ritmo não o estressa: os
ritmos 41,67, 16 e 10 ms deram **exatamente os mesmos dados** (só o `render_ms` muda, executado de novo nesta revisão, `evidence/local-*.json`).
Os passos `s4/s8/s16` (4, 8 e 16 quadros entre planos) **aproximam** um movimento de câmera maior; não são um playback medido. Não há
afirmação de "sem popping" nem critério de aceite novo.

### Dado histórico (primeira versão da ferramenta; `evidence/take*.json`, `evidence/xfade.json`)

Só médias do quadro inteiro. Mantido como histórico, sem aceite: a média é diluída pelos blocos que os splats nunca alcançam.

| Execução | Quadros | Pior diferença média por canal | Pior % de pixels > 8/255 | PSNR mínimo | "Popping" máximo (média do quadro) | Final = exato |
|---|---|---|---|---|---|---|
| `take` (passo 1) | 23 | 0,0028 | 0,006 | 66,3 dB | 0,0001 | sim |
| `take-s4` | 22 | 0,0109 | 0,027 | 56,5 dB | 0,0015 | sim |
| `take-s8` | 18 | 0,0211 | 0,051 | 53,1 dB | 0,0028 | sim |
| `take-s16` | 10 | 0,0384 | 0,088 | 50,2 dB | 0,0027 | sim |
| `xfade` | 21 | 0,0061 | 0,015 | 63,5 dB | 0,0004 | sim |

### Diferença local por bloco (esta revisão; `evidence/local-*.json`, mesmos planos e assets, ritmos 41,67/16/10)

A ferramenta agora mede por bloco de 32×32 (510 blocos por quadro): a média do pior bloco de cada quadro e, para a mudança de um quadro
ao seguinte, o quanto o pior bloco do quadro de movimento excede a mudança do exato (`worst_block_excess`). Valores 0–255.

| Execução | Quadros | Pior bloco (média) | Blocos > 2/255 (máx. por quadro) | Canal máximo isolado | Pior excesso de mudança por bloco | Média do quadro inteiro (pior) | Final = exato |
|---|---|---|---|---|---|---|---|
| `take` (passo 1) | 23 | 0,241 | 0 | 49 | 0,094 | 0,0028 | sim |
| `take-s4` | 22 | 0,751 | 0 | 115 | 0,345 | 0,0109 | sim |
| `take-s8` | 18 | 1,283 | 0 | 149 | 0,631 | 0,0211 | sim |
| `take-s16` | 10 | 1,632 | 0 | 145 | 0,627 | 0,0384 | sim |
| `xfade` | 21 | 0,420 | 0 | 47 | 0,104 | 0,0061 | sim |

Leitura: as médias do quadro inteiro se reproduzem (mesmo dado de antes) e no pior bloco a diferença cresce com o passo, de 0,24 a 1,63
de 255; nenhum bloco passa de 2/255 em média, e o canal isolado chega a 149/255 em poucos pixels. Isto descreve **este harness**
(planos de quadros consecutivos, GPU do M3 Pro, o mesmo projeto); não prova que o movimento não tem salto visível numa janela e **não
substitui a revisão visual nativa, que continua pendente** (BL130.6). As mesmas imagens exatas são iguais visitadas em ordem inversa
(`same_pictures_in_reverse_order: true` nas 5 execuções) e o quadro final refinado é idêntico ao exato em todas (`final_equals_exact`).
Esta repetição só serve para imagem (e o `render_ms` do harness, abaixo): `draw` e imagens/s do editor release não foram repetidos.

## Provas desta revisão (correções pós-revisão independente)

Medições acima = o que o PR já tinha mais a repetição local; o que mudou abaixo é de **correção**, provado por testes de Chrome real
(não por benchmark), cada um com mutação que o reprova: ver [spark.md](../../features/spark.md#evidência).

- Ordenação que rejeita (BL130.5): erro observável no pedido seguinte, estágio descartado e substituído, exato sem esperar 60 s,
  sem `unhandledrejection` (`splat-failures.test.mjs`, 6 testes).
- Orçamento de estágios vivos e fallback exato com 3 e 4 cenas (BL130.2, `splat-lineages.test.mjs`).
- Empréstimos sem dono e dono novo não roubam a malha de outra linhagem (BL130.7).
- Tempo esgotado do refino do scrub reprova (BL130.8, `scripts/preview-quality-proof.spec.py`) e a ferramenta de evidência recusa < 2 planos
  (status 2, `sort-order-evidence.test.mjs`).
- **Caminho quente (quadro de movimento), medição focada, não o benchmark completo:** o quadro de movimento com estágio vivo e ordem
  reaproveitável ainda retorna antes de qualquer espera nova (o que mudou nele: uma checagem de falha, o `catch` que registra em vez de
  engolir e a reserva de donos de malha); a espera do quadro exato passou a acordar pelo fim da ordenação e a conferir cancelamento e
  orçamento a cada 25 ms. O que se mediu foi a mediana de `render_ms` de cada quadro de movimento da ferramenta de evidência (inclui a
  leitura do PNG; GPU do M3 Pro, máquina carregada, `clippy` do workspace rodando em paralelo): `take` 74,5 ms (`e5447cb`) → 70,9 ms,
  `xfade` 112,3 → 102,3 ms (ritmo 41,67). Isto mostra que não houve regressão **nessa leitura**; **não** repete a tabela de `draw` p50 /
  imagens por segundo do editor release (1 ms, 50/s), que continua sendo a do `e5447cb` (não repetida: nenhuma volta do editor release
  foi feita nesta revisão).

## Limites e pendências

Ver [spark.md](../../features/spark.md#ordem-dos-splats-em-movimento-bl130): corrida de 2,4 s não reproduzida (BL130.1), leitura de PNG que
encurta o atraso, `done` é submissão e não exibição, **revisão visual nativa pendente (BL130.6)**, GPU compartilhada sem trava de GPU no
repositório (BL130.3), mais de 2 cenas sem medição de memória/latência com cenas grandes (BL130.2), áudio não ouvido.
