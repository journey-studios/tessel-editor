# BL131 — malhas compartilhadas de assets de efeito entre camadas de crossfade (2026-11-06)

Contrato da feature: [F030 Spark](../../features/spark.md). Origem do item: seção 9, linha 3, de
`viewport-performance-investigation-report.md` (PR #14, ainda não integrada).

## O que a medição achou na base nova

A base é `origin/main` em `5ebb855`, 82 commits à frente da base do relatório. Nela o ponto principal do item **já estava
resolvido por BL125** (`7db7277`, PR #13): `splatLease` agora empresta, ao segundo uso do mesmo asset, uma `SplatMesh` sobre os
`packedSplats` já decodificados em vez de decodificar o PLY de novo. Medido no binário release, sem a mudança deste commit:

- **0 decodificações** de arquivo splat em 300 quadros de crossfade (4 execuções × 75 quadros) depois da carga inicial;
  na base do relatório o segundo empréstimo chamava `loadSplatMesh` por quadro (`git show 6084892:runtime/src/assets.js`).
- `lease_total` ≈ 0,08 ms e `rig_build` ≈ 0,23 ms por quadro de crossfade, não os ≈ 7 ms do relatório.

O que sobrava, e é o que esta entrega muda: cada quadro ainda construía **uma `SplatMesh` nova por uso além do primeiro**, e o
Spark cria um gerador e um `DynoProgram` por malha nova (`SplatAccumulator.prepareProgramMaterial`, uma entrada de
`WeakMap` por gerador). No crossfade do projeto de exemplo isso eram **1,87 programas Spark novos por quadro, em média** (`vfx-ring` e
`vfx-sparks` da segunda cena; um quadro cuja cena o cache de resultado serve não empresta malhas). Agora o asset guarda as malhas devolvidas (até `LIMITS.splatMeshes` = 16
sobressalentes por asset) e o quadro seguinte as empresta de novo: 0 programas novos por quadro.

## Método

- Projeto: cópia em `/tmp` de `tessel-splat-h` (privado, 1280×720, 24 fps, 25 s; quatro cenas de 1.179.648 gaussians e
  `vfx-ring`/`vfx-sparks` em três delas). Os assets foram clonados com `cp -c` (APFS, sem espaço novo) e o JSON reescrito para
  o clone; nada foi copiado para o repositório, só tempos e hashes.
- Binário release do editor (`cargo build -p tessel-editor --release --locked`, sem alteração de Rust), `HOME` isolado e socket
  próprio por execução; só requisições semânticas (`project.open`, `render.frame`, `set timeline.playhead`, `get monitor.viewport`).
  O editor do usuário (debug) não foi tocado.
- O runtime vem de `TESSEL_RUNTIME_DIR` apontando para dois `dist/` montados dos fontes (antes = `origin/main`, depois = esta
  mudança), cada um com a sonda local [`probe/perf-hooks.js`](probe/perf-hooks.js), que embrulha os protótipos de fora (o mesmo
  arquivo instrumenta os dois) e devolve os tempos em `source.perf` do `frame.done`. A sonda não faz parte do produto.
- Caminho medido: **interativo** (`seek` do playhead, o quadro de preview 960×540 do monitor, sem leitura de pixels), quadros
  183–203, com a GPU forçada a terminar após cada quadro (`drain`, leitura 1×1) para que um quadro não absorva a cauda do anterior.
  Crossfade = quadros 186–200 (15 por passe). Cada execução: 1 passe de aquecimento (cargas frias) + 5 passes medidos de 21 quadros;
  execuções intercaladas antes/depois (A B, B A, …), 4 execuções por variante a MSAA 0 e 2 a MSAA 4.
- O relógio do WebKit tem resolução de 1 ms e os trechos medidos são sub-ms: por isso as colunas são **médias** (viés zero
  com fase aleatória), não p50 (que dá 0 ou 1).
- Ruído: a GPU é compartilhada com outra frente (utilização média por execução 38–44 % a MSAA 0, 73–83 % a MSAA 4; carga
  média de 1 min no início das execuções 3,8–4,6).

## Resultado (crossfade, quadros 186–200; média por execução, faixa entre execuções)

MSAA 0 (4 execuções × 75 quadros por variante):

| Métrica por quadro | antes | depois |
|---|---|---|
| decodificações de arquivo splat | 0 | 0 |
| programas Spark novos | 1,87 | 0 |
| `spark_generate_cpu` (CPU de `generate`, inclui criar programas) | 0,66 ms (0,59–0,71) | 0,20 ms (0,15–0,24) |
| `rig_build` | 0,23 ms (0,17–0,28) | 0,12 ms (0,11–0,16) |
| `lease_total` | 0,08 ms | 0,01 ms |
| `settle_total` (espera da ordenação) | 30,84 ms (30,59–31,21) | 30,77 ms (30,31–31,09) |
| `render_ms` (do cabeçalho, com `drain`) | 39,44 ms (39,17–39,93) | 39,45 ms (38,85–39,72) |
| malhas criadas / reaproveitadas (contador `splat_mesh_*` do quadro) | n/d (contador novo) | 0 / 435 empréstimos por execução (um quadro cuja cena o cache de resultado serve não empresta nada) |

MSAA 4 (padrão; 2 execuções × 45 quadros por variante):

| Métrica por quadro | antes | depois |
|---|---|---|
| programas Spark novos | 1,87 | 0 |
| `spark_generate_cpu` | 0,74 ms (0,67–0,82) | 0,32 ms (0,31–0,33) |
| `rig_build` | 0,17 ms | 0,13 ms |
| `render_ms` | 86,84 ms (86,33–87,36) | 87,79 ms (87,11–88,47) |

Quadros de uma cena só (183–185 e 201–203): `render_ms` 23,29 → 23,32 ms (MSAA 0) e 46,72 → 46,44 ms (MSAA 4); nenhuma mudança.

**Leitura.** A mudança elimina a criação de gerador/programa por quadro (1,87 → 0) e poupa ≈ 0,5–0,6 ms de CPU por quadro de
crossfade a MSAA 0 (≈ 1,5 % de 39 ms), uma faixa que não se sobrepõe entre execuções em `spark_generate_cpu`. **O `render_ms`
não muda de forma mensurável** (as faixas se sobrepõem; a diferença a MSAA 4 está dentro do ruído de GPU compartilhada). Os
≈ 7 ms do relatório já tinham saído com BL125; esta entrega não os devolve nem os explica de novo. O custo dominante do crossfade
continua sendo a ordenação serializada das duas cenas (≈ 31 ms de 39), fora do escopo (BL130) e do raster (BL129).

## Quadros idênticos (hash)

Hash do quadro (FNV do runtime, `inspect`) antes e depois, e entre o passe frio e o quente de cada variante, para os quadros dos
três crossfades do projeto (186–200, 268–282, 410–424) e vizinhos:

| Caminho | Quadros | Resultado |
|---|---|---|
| `render.frame` explícito 1280×720, MSAA 4, quadros 1–8, 180–205, 262–288, 300–303, 405–430, 500–503 | 95, todos distintos | idênticos antes/depois |
| interativo 960×540, MSAA 0 (`TESSEL_VIEWPORT_INSPECT=1`), 180–205, 262–288, 405–430 | 79 | idênticos |
| interativo 960×540, MSAA 4, os mesmos quadros | 79 | idênticos |

Os hashes por quadro estão em [`summary.json`](summary.json). A prova de runtime (`runtime/test/splat-shared.test.mjs`)
compara bit a bit uma página quente com uma fria, em duas ordens de visita.

## O que não foi provado

- Nenhum ganho de `render_ms` ou de intervalo entre quadros: só a CPU de criar geradores, ≈ 0,5 ms, abaixo da precisão do relógio
  da página por quadro (a média sobre 300 quadros por variante a MSAA 0 é consistente entre as 4 execuções, mas é média).
- Reprodução em tempo real (`playback.start`) e o intervalo entre quadros entregues não foram medidos; o caminho medido é o
  `seek`, que faz o mesmo trabalho por quadro mas não passa pelo relógio do `playback`.
- O vertical (`tessel-splat-v`) e outras máquinas/monitores não foram medidos.
- O caso de N nós de um arquivo da cena dirigida (BL125) herda a reutilização por usar o mesmo `splatLease`, mas não foi medido
  aqui; acima de 16 usos simultâneos de um arquivo o excedente é criado e liberado por quadro, como antes.

## Reprodução

`probe/` guarda a sonda e os drivers usados (caminhos de `/tmp/bl131` e do worktree fixos: são descartáveis, não parte do produto):
`build.mjs <src> <out> instr` monta um `dist/` instrumentado, `drv.py <binário> <runtime> <rótulo>` abre o editor isolado e mede
(`MODE=seek|frame`, `SAMPLES=0|4`, `PASSES`, `RANGES`), `matrix.sh` intercala antes/depois, `analyze.py <prefixo>` agrega.
