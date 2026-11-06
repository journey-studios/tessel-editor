# F048 — Splat Spark numa cena dirigida (S7, subconjunto de BL025)

> F048: o F044 já é do aceite independente do S10 (PR #8, `scene-acceptance.md`); esta frente usa F048 para
> não colidir. O coordenador renumera se outra frente da rodada S1–S10 usar o mesmo.

## Status

Lib, runtime e CLI headless **V no corpus sintético** (Chrome real, SwiftShader); pacote **V**
(diretório e arquivo, sem as origens); export **V** com cópia privada do `spz` aprovado (S3/BL109, integrado).
Monitor nativo (WKWebView): a prova `scripts/scene-splat-native-proof.py` (instância própria, só pedidos semânticos) passou
**119/119** no head limpo `2c7cf7c` e **129/129** no head `586a671` (+10 checks das guardas novas), com runtime reconstruído;
proveniência em [Revalidação no head `586a671`](#revalidação-no-head-586a671-executado-com-proveniência). Cor guardada do splat
(BL019-F1) **corrigida** e provada com controle negativo.
Luz, clima, rigs e mapas de dados em nó `splat` continuam **recusados** por `unsupported`, com o nome do domínio;
uma matriz de splat que não é semelhança (escala não uniforme ou espelhada, cisalhamento) também (BL123). Aceite
independente do S10: ver [Aceite S10](#aceite-independente-s10-pr-8); **BL121 continua BLOCKED**.

## Objetivo e limites

Um nó `splat` de um bundle dirigido ([F031](directed-scene.md)) é desenhado pelo Spark já existente
([F030](spark.md), nada reimplementado) com a **matriz de mundo e a câmera que o avaliador do core
calculou** para o frame. O runtime só desenha o que o plano diz (D4).

Dentro: nó `splat` com asset `spz` aprovado (identidade `generation`, `assetId`, `revisionId`,
`contentHash`), a posição que o desempenho e os sockets dão ao nó, a profundidade entre os planos, o mesmo arquivo em dois nós,
preview (monitor), `scene.render` headless, pacote/restauração e export de vídeo.

Fora (recusado, não omitido): luz, clima, efeitos, rigs, `reveal`, `surface`, `normalMap`,
`sceneDepth`, `relief` e `rigBind` — em qualquer nó, inclusive `splat`. Formatos `.splat`/`.ply`
não entram numa cena dirigida: o bundle declara `spz` (F040 verifica o cabeçalho).

## Contrato do core / biblioteca

- `ScenePlan::splats: Vec<PlanSplat { node_id, asset_id, after_layers }>`: os nós `splat` do frame,
  em ordem de documento; `after_layers` é o lugar do splat na ordem de pintura dos planos (abaixo). A matriz está em `plan.node(id).world_matrix` e a câmera em `plan.camera`, as do
  avaliador (teste compara bit a bit).
- `plan.resources` lista o `spz` por `assetId`; `PlanResource::is_splat()`.
- `Capability::Splats` (`"splats"`): exigida só quando há splat (`requires`); o runtime a anuncia.
  Um runtime que não a tem recusa o plano com `unsupported` + `details.missing: ["splats"]`.
- Forma de fio: a chave `splats` só existe com splat. Plano sem splat mantém os bytes do golden
  de PR4 (teste); a leitura aceita a chave ausente e `[]`. `PLAN_VERSION` não mudou.
- Invariantes novos de `validate`: `splats` igual aos nós `splat` (ponteiro `/splats`,
  `/splats/N/assetId`, `/splats/N/afterLayers` recalculado do próprio plano), recurso `spz` do próprio asset; recurso `spz` com `used_by` exato;
  uma camada de imagem **nunca** lê um recurso `spz` (`/layers/N/assetId`, BL126); a matriz de mundo de um nó `splat` é uma semelhança
  (abaixo).
- Mudança de contrato: `Domain::Splat` deixou de existir (antes: todo nó `splat` tornava o plano
  parcial). `Domain::ALL` tem 13 domínios; `Capability::ALL`, 9.

## Matriz do splat: só semelhança (BL123)

O Spark 2.4.0 (`covSplats` padrão `false`) desenha um splat pela matriz decomposta com
`escala = (sx + sy + sz) / 3`, sem cisalhamento (`SplatTransformer.updateFromMatrix`; conferido no pacote
`@sparkjsdev/spark` travado). Um plano irmão recebe a matriz inteira. Por isso `{2,1,1}` no nó ou num ancestral seria
desenhado com escala isotrópica 1,33 (pai espelhado `{-1,1,1}`: média 1/3; pai não uniforme + filho rotacionado: cisalhamento
perdido), e o pixel divergiria do plano em preview, `scene.render`, pacote e export, sem erro. O contrato do runtime é "recusado
com erro tipado, nunca aproximado": o **core** recusa a matriz que não é rotação + uma escala uniforme positiva + translação.

- Erro: `PlanError::SplatTransform { node_id, reason }`, código `unsupported`, `details { unsupported: "splat_transform", node, reason }`.
  Razões: `the scale is not uniform`, `the transform is mirrored` (determinante negativo; dois espelhos são uma rotação e passam),
  `the axes are sheared`, `the scale is degenerate`, `the matrix is not affine`.
- Tolerância explícita: `SPLAT_SIMILARITY_TOLERANCE = 1e-6` (relativa: comprimento das colunas e ortogonalidade). Matrizes honestas
  erram ~1e-12; o runtime tem a mesma constante.
- Onde: `ScenePlan::build_with` (que `PlanTimeline` repete **por frame**: `at_frame` recalcula as matrizes e confere de novo, e o molde do
  tiro não carrega a recusa de um frame aos demais), `ScenePlan::validate` (logo `from_json`, `SceneSlots::prepare`) e uma guarda no
  runtime (`parseDirectedLayer`, mesmo código `unsupported`) para um plano que chegue por outro caminho. Um plano parcial
  (`allow_partial`) não contorna: não é domínio, é a matriz.
- A escala local é estática, mas rotações animadas sob escalas não uniformes na hierarquia podem mudar a similaridade da matriz composta.
  Um frame pode passar e outro ser recusado; a verificação por frame é necessária inclusive durante o export, que pode falhar no meio do
  job sem publicar o resultado. Não há suporte aproximado nem `covSplats` (exigiria `extSplats`).

## Ordem de pintura e limites do Spark

O **core** decide quem está na frente de quem (o runtime só executa): `after_layers` é o índice, na
ordem dos `layers`, do primeiro plano **visível** (profundidade `> 0`) cuja origem de nó está
**estritamente mais perto da câmera** que a origem do nó do splat (profundidade = produto escalar de
`posição do nó − câmera` com `target − position`, na translação da `world_matrix`); se nenhum está,
`layers.len()`. `0` = atrás de todos os planos, `layers.len()` = na frente de todos. Profundidade igual
não é "mais perto": o splat é pintado depois. **Um plano com profundidade `<= 0` (no plano da câmera ou
atrás dela) é invisível e nunca conta como mais perto** (BL124): câmera em z=9, plano A em z=12 (atrás),
plano B opaco em z=-2 e splat em z=0 dão `afterLayers = 2` (antes: `0`, e B cobria o splat). Um splat
que ele mesmo está atrás da câmera não tem plano visível à frente e fica em `layers.len()`.
O validador recalcula o valor só com o plano (câmera, matrizes, layers): outro valor é
`invalid_plan` em `/splats/N/afterLayers`.

Pintura (`drawDirectedScene`): limpa em preto opaco; para cada fatia de splats em ordem de `after`,
pinta os planos até ali (ordem do plano, F037) e depois os splats da fatia, ordenados pelo Spark
para a câmera exata do pedido (`settle`); por fim os planos restantes. Um plano **nunca** é
reordenado por profundidade: só o splat é encaixado. Entre splats da mesma fatia a ordem é a do
Spark por câmera (a ordem de `splats` não é ordem de pintura).

Ordem de pintura **não monotônica em profundidade** (ex.: `[A prof. 5, B prof. 20]`, splat em 10): a regra pega o
**primeiro** plano da lista mais perto que o splat (A, índice 0), então o slot é 0 e B, mais longe porém pintado depois
na lista, cobre o splat. É coerente com a regra escrita (o plano nunca é reordenado por profundidade), é um limite
assumido e está registrado em BL122.

Limites assumidos: a decisão usa a **origem do nó** (um plano grande atravessado pela nuvem ou um
splat que cruza um plano não é partido; o splat inteiro fica de um lado do plano). Planos com alfa
mostram o splat pelo alfa; um plano opaco que está na frente o esconde. Não há conversão de
lateralidade (o mesmo `.spz` e a mesma câmera do runtime de Spark existente).

## Cor (BL019-F1)

O compositor escreve o espaço de trabalho como está (`outputColorSpace = LinearSRGBColorSpace`, para
que imagens cheguem intactas). Todo quadro de splat é desenhado num **render target**, e para um render target o
Spark 2.4.0 **não** olha `renderer.outputColorSpace`: usa `THREE.ColorManagement.workingColorSpace` (linear;
`SparkRenderer.ts`, espaço de saída do desenho), então liga `encodeLinear` a cada desenho e decodifica a cor do splat com
sRGB→linear, que ninguém recodifica: `(230,49,49)` saía `(203,7,7)` (achado do S10 no caminho de projeto
existente; o caminho dirigido usa o mesmo palco). Correção em `runtime/src/splat-stage.js` (`keepStoredColour`): após o
`onBeforeRender` do próprio Spark, o uniform `encodeLinear` é mantido em `false` — só no material do Spark. Imagens não
passam por esse material. Efeito: a cor guardada é a mostrada, **nos dois caminhos** (projeto `scene` e cena dirigida); o
caminho de projeto também mudou de cor ([F030](spark.md)).
Reprodução e controle negativo: `runtime/test/directed-scene-splats.test.mjs` ("BL019-F1"): sem a
correção falha com `rgb(202,7,7)` no centro; com ela, a diferença ao armazenado é ≤ 6 e o plano de
imagem na mesma cena fica a ≤ 1. O vermelho puro `(255,0,0)` não vê o defeito (0 e 1 são invariantes
da conversão), por isso os testes antigos passavam. O teste de Chrome do caminho de projeto depende do harness S10
(`baseline_splat_colour`); no repositório só o caminho dirigido tem teste.

## Recursos e orçamento

- O host (`SceneSlots`) carrega um splat como `asset.load` `kind: "splat"`, `meta.format: "spz"`
  e a mesma extensão aditiva `scene` (identidade). Antes de gastar um slot, o host abre o
  cabeçalho (`core::splat::inspect`): um arquivo que não é SPZ é `resource_mismatch`.
- Orçamento de memória: arquivo + 16 bytes por splat, **uma vez por recurso**, no host e no runtime (`sceneEntryBytes`):
  é o que o arquivo decodificado ocupa. Figuras continuam em `largura × altura × 4`.
- Decodificação compartilhada (BL125): cada quadro empresta uma malha por nó (`splatLease`) e a devolve ao terminar ou ao
  ser abortado. A primeira malha é a do próprio recurso; as demais são `SplatMesh` sobre os **mesmos** `packedSplats`
  (`shareSplatMesh`): N nós de um arquivo decodificam 1 vez, não N−1 por quadro (antes: 40 nós de 1 M de splats = 39
  decodes por quadro, ~624 MB fora do orçamento). Ao devolver, a malha emprestada solta os dados sem descartá-los. Perda de
  contexto reconstrói das bytes guardadas.
- Orçamento de desenho por quadro (BL125): ordenar e desenhar custa por **nó**, não por arquivo. O quadro soma, sobre
  os nós `splat`, os splats do arquivo de cada um e é recusado com `resource_budget` acima de `8 Mi` splats (`LIMITS.frameSplats`
  no runtime, `DEFAULT_FRAME_SPLATS` no host, mesmo número; `details`: `frame_splats`, `limit`/`max_frame_splats`), antes
  de qualquer empréstimo ou slot. O valor é um teto de segurança, não uma medição de custo por quadro (não medida).
- Export (S3/BL109, integrado): o `spz` aprovado entra no conjunto de recursos exigidos do job
  (`required_assets` vem do plano), é copiado e conferido por SHA-256 para a cópia privada 0444 do próprio
  estágio e o renderer lê a cópia, não o original. Arquivos do diretório do bundle que o manifesto não aprovou
  não viajam (teste). Trocar o original (mesmo tamanho) depois do `resolve` é `resource_changed`; removê-lo é
  `preflight_failed`; reescrevê-lo **durante** o job não chega a nenhum quadro (hashes iguais aos do export de
  referência); cancelar remove só o estágio do próprio job.

## CLI headless

`scene.render` e `scene.export` (F041/F042) não mudaram de forma: um bundle com nó `splat` e o
mapa aprovado por `scene.resources.resolve` produzem os quadros. Sem splat no runtime (versão
antiga) o pedido responde `unsupported` com `details.missing`.

## UI e CLI da janela

Sem controle novo (nenhum `Command`/nó de árvore): o monitor da cena dirigida passa o mesmo plano
ao mesmo runtime. `./scripts/ui` não ganhou verbo. Apresentação N/A: não há painel de splat;
inspetor de cena é F034.

## Evidências

Ver a seção de resultados abaixo e `docs/backlog.md` (BL025).

| Superfície | Teste | Prova |
|---|---|---|
| Core (lib) | `crates/core/tests/scene_plan_splat.rs` (+ unitário em `scene_plan/timeline.rs`) | plano/matriz/recurso/capacidade, JSON sem splat idêntico, canais de desempenho, recusas de luz/clima/rig, tampering (inclui plano de imagem lendo `spz`, BL126), matriz que não é semelhança (BL123) no nó, em ancestral, espelhado, cisalhado, degenerado, no leitor e por frame, plano atrás da câmera (BL124) |
| Runtime puro | `runtime/test/directed-scene-splats.test.mjs` | parser do plano, recusas com ponteiro, identidade do recurso, orçamento, guarda de semelhança com a mesma tolerância do core |
| Runtime em Chrome | mesmo arquivo | posição por oráculo pinhole independente, splat entre planos por profundidade (atrás, na frente, entre dois), **dois splats de arquivos e contagens diferentes em slots distintos, cada um sobre os seus planos e sob o seguinte, em ordem trocada e repetida (T1/BL127)**, cor guardada vs. imagem (BL019-F1), ordem/repetição/frio, **40 nós de um arquivo = 1 decodificação e recusa por soma de splats (BL125)**, recusas tipadas, perda de contexto |
| Host/render | `crates/render/tests/scene_splat_frames.rs` | frame headless 0 e 24 contra oráculo, mesmo hash em qualquer ordem, arquivo não-SPZ, export = `scene.render` quadro a quadro e MP4 verificado, orçamento por soma de splats dos nós (40 nós, sem nuvem grande) |
| Pacote | `crates/media/tests/scene_splat_package.rs` | adoção, pacote (dir e arquivo), restauração sem origens, mesmo hash, replanejamento |

### Provas adicionadas na recuperação (S3/S4 integrados)

| Superfície | Teste | Prova |
|---|---|---|
| Export + snapshot (lib/JSONL, Chrome real) | `crates/render/tests/scene_splat_frames.rs` | só o recurso aprovado entra na cópia privada (arquivo não declarado ao lado não viaja; `snapshot.files == 3`); troca de mesmo tamanho → `resource_changed` (`asset:cloud`), outro tamanho/ausente recusados, nada fica; reescrita **durante** o job não chega a nenhum quadro (hashes dos 12 quadros iguais aos do export de referência); cancelar remove só o estágio próprio (pasta alheia `.tessel-*` sobrevive) |
| Janela nativa (WKWebView) | `scripts/scene-splat-native-proof.py --corpus <corpus do S10>` | 5 quadros (câmera se move): monitor convergiu, hash do monitor = hash do `scene.render` do mesmo quadro, centro de cada splat com a cor guardada (≤ 24) e fundo plano; controles: 40 px ao lado não é a cor, a cor escurecida pelo caminho antigo não é a cor; `asset.identity` negociado e todas as cargas carimbadas; runtime e livro de slots têm o mesmo número de recursos |

A prova nativa lê o corpus do S10 (esperados do oráculo independente) por `--corpus`, somente leitura; ele não é
copiado para este repositório (PR #8 é a dona).

### Aceite independente S10 (PR #8)

Harness e corpus do PR #8 (head `0d95be1`, `tolerances.json` sha256 `696c3cd1…1ed1c722`, **sem alterar nenhum
limite nem verificador**) copiados para um scratch somente leitura e executados contra o `tessel-render` desta
branch (debug, runtime real, SwiftShader, sob o lock da GPU):

| Fase | Resultado |
|---|---|
| `tolerances_frozen` | não executável no scratch (precisa do git do PR #8); o sha256 do `tolerances.json` copiado foi conferido idêntico ao de `origin/pr8` |
| `controls_*`, `camera_*`, `baseline_splat_geometry`, `baseline_audio`, `audio_off`, `package_*`, `corpus`, `models` | PASS (sem regressão) |
| `baseline_splat_colour` (BL019-F1) | **PASS 7/7** (antes: FAIL, `(230,49,49)` saía `(203,7,7)`) |
| `splat_plan` | PASS 78/78 |
| `splat_render` | **FAIL 165/167** |
| `splat_export` | **FAIL 186/187** |
| `splat_package` | **FAIL 59/60** |
| `audio_export`, `audio_package_export`, `full_av` | **BLOCKED** (S9/BL016 não integrado) |

As quatro falhas são a **mesma regra**: `s-order back centre: the front splat's colour wins within 40`, nos quadros
36 e 47 (em 0/12/23/24 passa). O corpus do oráculo coloca, nesses quadros, o centro do splat de trás a 10,6 e 11,6 px
do centro do da frente (sigma da frente ≈ 15,7 px); o splat da frente tem alfa gaussiana ≈ 0,80/0,76 ali, e a regra
exige que a cor da frente "vença" como se fosse opaca. Predição independente com o próprio `expected.json`
(`a·frente + (1−a)·trás`): `(197,161,69)` e `(190,163,76)`; medido `(198,161,68)` e `(190,163,75)`: o renderer
pinta o que a física da mistura pede, a frente por cima (a regra "longe da cor de trás ≥ 80" passa). A tolerância
não foi relaxada e o oráculo não foi tocado: é **BL120** (emenda do S10, fora do S7) e a fase continua FAIL neste
documento. Controle negativo da correção de cor: com `keepStoredColour` desligado o mesmo harness volta a medir
`(203,7,7)`, `(7,149,15)`, `(104,11,149)` e as fases de cor falham.

O PR #8 já não é draft (head `a848007`; o harness e `tolerances.json`, `amendments: []`, não mudaram desde `0d95be1`).
Uma recomputação independente com o `expected.json` do PR #8 (mistura gaussiana `a·frente + (1−a)·trás`) dá distâncias de canal à
cor da frente de 17, 21, 27, 17, **42, 50** nos frames 0, 12, 23, 24, 36, 47: a regra de 40 reprova os frames 36 e 47 com
qualquer renderer correto, o que sustenta a hipótese de BL120 (emenda do S10, que decide; este PR não toca em tolerância nem
harness). **Nada disso transforma FAIL em PASS**: `splat_render`, `splat_export` e `splat_package` seguem FAIL e `full_av`/áudio
BLOCKED.

`full_av` precisa do mix de áudio do S9 (`scene.export` ainda recusa o bundle com áudio): fica BLOCKED e não conta
como pass. As fases `splat_*` devem ser reexecutadas (BL121) com o merge de S7 e S9 e a emenda BL120.

### Correções da revisão PR13

Rodada 1: a timeline recalcula `afterLayers` após atualizar câmera e matrizes em cada frame; o slot do
frame molde não é reutilizado quando a câmera muda a ordem de profundidade. A regressão
`timeline_recomputes_splat_order_when_the_camera_orbits_past_the_planes` exige mudança de slot,
compara os planos em cache com `ScenePlan::build` e valida cada plano.
A derivação retorna imediatamente sem splats; com splats, indexa os nós uma vez para evitar
buscas lineares repetidas. Isso preserva o caminho de planos sem splats. O script nativo lê
o corpus uma só vez com UTF-8 explícito e grava seu relatório em UTF-8.

Rodada 2 (revisão independente, [review 5800820718](https://github.com/journey-studios/tessel-editor/pull/13#pullrequestreview-5800820718)):

| Finding | Reproduzido? | Correção | Prova |
|---|---|---|---|
| F1 escala não uniforme/espelhada/cisalhada aproximada em silêncio (BL123) | Confirmado na fonte do Spark 2.4.0 instalado (`SplatTransformer.updateFromMatrix`, média das escalas, sem cisalhamento) | Recusa tipada no core (`SplatTransform`), por frame na timeline, no leitor e guarda no runtime | `a_splat_whose_matrix_is_not_a_similarity_is_refused_never_approximated`, `the_reader_refuses...`, `a_frame_cut_from_a_template...`, `BL123` do runtime |
| F2 plano atrás da câmera tira o slot (BL124) | Reproduzido: câmera z=9, A z=12, B z=-2, splat z=0 dava `0` (esperado `2`); mutação da correção falha o teste | `derive_splat_slots` ignora camadas com profundidade `<= 0` | `a_plane_behind_the_camera_does_not_take_the_splats_slot`, `the_timeline_ignores_a_plane_the_camera_flies_past` |
| F3 N nós de um arquivo: N−1 decodificações/quadro fora do orçamento (BL125) | Confirmado por leitura de `splatLease` (decodificação por empréstimo extra); a mutação sem compartilhamento reprova o teste, que agora mede 40 nós ⇒ 1 `PackedSplats` | `packedSplats` compartilhados + teto de splats por quadro somando por nó, no host e no runtime | `BL125` (Chrome), `a_frame_is_budgeted_by_the_splats_every_node_draws...` (host) |
| F4 plano de imagem lendo `spz` passava `validate` (BL126) | Reproduzido pelo caso de adulteração; mutação falha o teste | `check_layers` recusa recurso `spz` | caso novo em `a_plan_that_lies_about_its_splats...` |
| F5 backlog/matriz/comentários contraditórios | Confirmado por leitura | Backlog, matriz, F030, comentários de `scene_plan/mod.rs`, `directed-scene.js` e `splat-stage.js` (explicação de `encodeLinear`) reconciliados | revisão |
| T1 falta de pixels com dois splats em slots distintos (BL127) | n/a (teste ausente) | teste de Chrome com arquivos e contagens diferentes, ordem trocada e repetida; mutação (todos os splats no slot 0) o reprova | `T1` do runtime |

Integração com main6f7a944: conflito apenas na matriz, preservando F043 e F048.

#### Estado de validação desta rodada (escrito vs executado)

Tudo o que a tabela acima diz de "mutação reprova", e os números abaixo, vêm da execução do autor da correção
numa árvore **suja** sobre o head `6a0bdd5`, antes do commit: são **atribuídos**, não repetidos no head commitado.

| Item | Estado |
|---|---|
| `cargo fmt` (conferido de novo ao commitar), `check`, `clippy -D warnings` | atribuído: passaram na árvore suja; fmt reconferido com `rustfmt --check` |
| `tessel-editor-core`: `scene_plan_splat` (15), unitário da timeline (1), `scene_splat_frames` do orçamento (1) | atribuído: passaram |
| Runtime em Chrome real (`directed-scene-splats.test.mjs`, 15) e mutações de F1/F2/F3/F4/T1 | atribuído: passaram; mutações reprovaram os testes novos |
| `cargo test --workspace` | **não concluiu**: abortou em `playback_follows_the_audio_clock_or_says_why_it_cannot` (a asserção do `sample_rate` do dispositivo de áudio esperava 48000 e o AirPods mediu 24000); falha do ambiente/teste, não do splat; o PR #12 corrige a asserção e não foi importado aqui |
| Restante de `scene_splat_frames`, `scene_splat_package`, `media`, `desktop` (exceto o teste acima) e prova nativa do monitor | **pendentes** neste head; a prova nativa de 119 checks é de outro head sujo e não vale |
| Harness S10 (`splat_*`, `full_av`) | **BLOCKED/FAIL** como antes (BL120/BL121); nada alterado |

#### Execução no head commitado `2c7cf7c` (executado, com proveniência)

A tabela acima fica como histórico da rodada anterior (árvore suja, head `6a0bdd5`); esta seção a **substitui** como
evidência. Tudo abaixo foi executado por um único runner enfileirado sob o lock da GPU (`with-gpu-lock.py`), com Cargo por
`cargo-serial.py`, árvore limpa no início, `TESSEL_REQUIRE_RUNTIME=1` e piso de 12 GiB livres vigiado durante cada passo
(mínimo observado 13,3 GiB). Head `2c7cf7cbf099e36080e59f52be0a3e340c433d46`, tree `e25374f8…`, UTC 2026-11-06 16:32:30–16:40:37.
Os logs, o sidecar de cada passo (head, tree, `status --porcelain`, `cmd`, `cwd`, env em lista branca, UTC, saída, livre
antes/mín./depois, sha256 do log) e o manifesto ficam fora do repositório em `/tmp/te-711b-final/`
(`manifest.json` sha256 `7917457e73bcd319…`, runner `runner.py` sha256 `6e64ad346401d7a2…`).

| Gate | Resultado |
|---|---|
| `cargo fmt --all -- --check` | PASS (saída 0) |
| `cargo check --workspace --locked` | PASS |
| `cargo clippy --workspace --all-targets --locked -- -D warnings` | PASS |
| `cargo test --workspace --locked --no-fail-fast` | PASS: **1239 passed, 0 failed, 9 ignored** (o teste de áudio do dispositivo passou neste ambiente; os mesmos totais do gate antecessor em `ed7827c`, cuja produção não mudou: a diferença até `2c7cf7c` é só `runtime/test/directed-scene-browser.test.mjs` e docs) |
| `cargo build -p tessel-editor --locked` | PASS |
| `cargo tree -p tessel-editor-core --locked` | PASS: sem `gpui` nem `wry` |
| `./scripts/runtime test` (108 testes) | PASS **atribuído ao coordenador** (mesmo head, `TESSEL_REQUIRE_RUNTIME=1`, saída 0, 108/0): não repetido aqui; ver [scene-renderer.md](scene-renderer.md) (BL102c) |
| `scripts/scene-splat-native-proof.py` (janela real, WKWebView, Apple GPU) | PASS **119/119** no commit limpo |

Prova nativa: binário direto `target/debug/tessel-editor` (sha256 `b69b11f2210f82a3…`), `runtime/dist/runtime.js` sha256
`bbe3bbfcae0b2fbc…`, script sha256 `9863cee21f09579f…`, corpus S10 somente leitura em
`t3-02272e59` no head `d435abea` (limpo antes e depois; 21 arquivos, sha agregado `2a2582be4816bd6b…`), relatório JSON
sha256 `4833bbd59dcad30d…`, sidecar `native-sidecar.json` sha256 `601f5921ee672443…`. A prova é por requisições semânticas
(`op`, `get`, `set`; HOME curto e socket próprios); o "pixel" é o hash que o WebView reportou comparado ao de `scene.render`,
e as sondas de cor leem o `scene.render`: **não** é captura da tela nem ponteiro físico.

Mutações (cada uma aplicada na árvore, o teste alvo tem de falhar, depois `git checkout` do arquivo, árvore limpa e a mesma
seleção volta a passar; os diffs e as listas de testes reprovados ficam em `/tmp/te-711b-final/mut-*`):

| Mutação | Teste que reprovou | Restaurado |
|---|---|---|
| F1 core: guarda de semelhança desligada (`check_splat_transforms`) | `a_splat_whose_matrix_is_not_a_similarity_is_refused_never_approximated`, `the_reader_refuses_a_splat_matrix_that_is_not_a_similarity_and_the_timeline_checks_each_frame`, `a_frame_cut_from_a_template_is_checked_for_its_own_splat_matrices` | PASS |
| F1 runtime: `splatMatrixFault` devolve `null` | `BL123: a splat whose matrix is not a similarity is refused, never approximated` | PASS |
| F2: camada com profundidade `<= 0` volta a contar | `a_plane_behind_the_camera_does_not_take_the_splats_slot`, `the_timeline_ignores_a_plane_the_camera_flies_past` | PASS |
| F4: recusa de `spz` em camada de imagem desligada | `a_plan_that_lies_about_its_splats_is_refused_with_the_pointer` | PASS |
| T1: todos os splats no slot 0 (runtime reconstruído com a mutação, Chrome) | `T1: two splats of different files in different slots are each over their own planes and under the next` | PASS; `runtime/dist/runtime.js` reconstruído tem o mesmo sha256 da linha de base |

A mutação F3 (decodificação compartilhada, BL125) não foi repetida nesta execução: continua só com a atribuição da rodada
anterior e a prova direta `BL125`, que passou na suíte completa.

Limites que esta execução **não** muda: o aceite independente S10 segue `FAIL`/`BLOCKED` (BL120/BL121); `full_av` e áudio
dependem do S9; a revisão independente final (Opus) ainda precisa aceitar o delta.

### Correções da revisão CodeRabbit em `2c7cf7c`

O parser do runtime recusa `nodeId` repetido em `/splats/N/nodeId`, como o core. A prova nativa exige ao menos uma sonda de splat isolado por frame e cardinalidade exata da resposta antes de comparar cores; uma resposta curta não pode passar pelo `zip`. Os gates de `2c7cf7c` são anteriores a essas correções; a revalidação está pendente.

Validação desta correção: `node --check`, `py_compile`, `git diff --check` e o teste de parser `hides, mislabels` passaram (1 teste). A execução ampla foi interrompida e não conta como gate.

Teste de regressão das guardas da prova nativa (`python3 scripts/scene-splat-native-proof.spec.py`, 10 testes, sem editor, GPU, Chrome nem bundle): dirige o `main()` real da prova por seus seams com um monitor falso. Corpus só `sp-order` e resposta de `scene.render` curta, longa, ausente ou não lista reprovam em todos os frames (e o primeiro caso nem renderiza); resposta completa passa, e cor errada com contagem certa ainda reprova pela regra de cor. Uma terceira classe remove cada guarda de uma cópia do script e confirma que as mesmas entradas passam vazias, logo os testes discriminam as guardas. Contra o script de `91bd915` 7 dos 10 testes reprovam. No runtime, a mutação sem `listed.has(nodeId)` faz `hides, mislabels` reprovar (`expected a refusal`) e o código atual passa; o conjunto puro `directed-scene`+`directed-scene-splats` dá 15 passam/9 pulados (navegador).

Limites (históricos, no head `0248f8b`): o disco livre (~9 GiB) estava abaixo do piso de 12 GiB, então nenhum Cargo, Chrome nem rebuild rodou. A revalidação foi feita depois, no head `586a671`: ver a seção seguinte, que **substitui** este parágrafo.

#### Revalidação no head `586a671` (executado, com proveniência)

Head `586a6717bcc8a6e26f3e7a6b2d43075be6abfc1c`, tree `9ad450e0…`, árvore limpa no início e no fim, branch da PR `t3/s7-directed-spark-scene-1` (`origin` = `journey-studios/tessel-editor`, fast-forward limpo a partir de `6084892`). Um runner enfileirado sob o lock da GPU (`with-gpu-lock.py`), UTC 2026-11-06T17:08:44–17:10:17, piso de 12 GiB vigiado em cada passo (mínimo observado 15,19 GiB; não houve aborto). Logs, sidecars e manifesto fora do repositório em `/tmp/te-bl127-final/` (`manifest.json` sha256 `71284dc3b95f9655…`, `runner.py` sha256 `8b69d084817f95d7…`). Runtime próprio: `./scripts/runtime setup` neste worktree, `runtime/dist/runtime.js` sha256 `0957c807…` (o `dist` do binário antigo, `bbe3bbfc…`, **não** foi usado: a prova roda com `TESSEL_RUNTIME_DIR` apontando para este `runtime/`).

**Rust não foi repetido, por equivalência demonstrada, não por suposição:** `git diff 2c7cf7c..586a671 -- crates Cargo.toml Cargo.lock` e `-- docs/migration/fixtures assets` (os únicos caminhos que o Rust embute por `include_str!`/`include_bytes!`) são vazios; só mudaram `runtime/`, `scripts/` e docs. O binário `target/debug/tessel-editor` usado (sha256 `b69b11f2210f82a3…`, mtime UTC 16:38:53) é o do worktree `t3-711b6cd3`, somente leitura (head `0248f8b`, limpo antes e depois; o diff de `crates` desse head ao `586a671` também é vazio) e tem o mesmo sha256 do binário da prova de `2c7cf7c`. Os gates Rust de `2c7cf7c` (fmt, check, clippy `-D warnings`, `cargo test --workspace` **1239/0/9**, build, árvore do core sem gpui/wry) continuam valendo para esse código inalterado.

| Gate | Resultado |
|---|---|
| `node --check runtime/src/directed-scene.js`, `py_compile`, `git diff --check 2c7cf7c HEAD` | PASS |
| `python3 scripts/scene-splat-native-proof.spec.py` | PASS (10 testes) |
| `./scripts/runtime setup` (`pnpm install --frozen-lockfile` + build) | PASS; árvore ainda limpa |
| `./scripts/runtime test`, `TESSEL_REQUIRE_RUNTIME=1`, Chrome real, lock da GPU | **duas execuções do runner de gates: uma 107/108, outra 108/108** (abaixo) |
| `scripts/scene-splat-native-proof.py` (janela real, WKWebView, Apple GPU) | PASS **129/129** (contagem real; ver abaixo) |
| Mutação do parser: sem `listed.has(nodeId)` | `hides, mislabels` FAIL (`expected a refusal`); restaurado, passa |
| Mutação T1: todos os splats no slot 0 (runtime reconstruído com a mutação) | `T1:` FAIL; restaurado, `dist` com o mesmo sha256 `0957c807…` |

Suíte runtime: a primeira execução do passo (UTC 17:08:45–17:10:05, log sha256 `0657240e…`) deu **107/108**: falhou `video B2: a frame superseded while it seeks is cancelled and the next one is exact` (`video.test.mjs:208`, esperava `frame.failed`, veio `frame.done`: o primeiro `frame.render` terminou antes de o segundo o substituir). Esse teste, `video.test.mjs` e o seu caminho de código não estão no diff desta PR; é uma corrida de tempo do teste, não do splat. Uma execução anterior do mesmo runner no mesmo head (processo encerrado antes de gravar o manifesto, logs preservados em `/tmp/te-bl127-final-attempt1-killed/`) passou 108/108, e a repetição completa e isolada (UTC 17:10:33–17:13:29, mesmos env e lock, log sha256 `ea5c9e54…`) também: `# tests 108`, `# pass 108`, `# fail 0`, `# skipped 0`. Nada foi relaxado nem pulado; a flutuação é registrada como BL128 e **não** é apresentada como 108/108 numa execução única.

Prova nativa: 129 checks, não 119. A contagem mudou porque as guardas novas (`the corpus has isolated colour probes` e `every requested probe was returned`) são 2 checks por quadro × 5 quadros, +10; nada foi forçado a 119 nem a 129. Relatório JSON sha256 `15b287c9…`, sidecar `native-sidecar.json` sha256 `513cc6e5…` (head, tree, status, binário e sha, `runtimeDir`/sha do `dist`, script sha256 `4b73bc1a…`, corpus). Corpus S10 somente leitura em `t3-02272e59` no head `d435abea` (limpo antes e depois; 21 arquivos, sha agregado `2a2582be…`, idêntico ao de `2c7cf7c`). Instância privada (HOME curto e socket próprios), binário direto, só `op`/`get`/`set` sem coordenadas; o "pixel" é o hash que o WebView reportou comparado ao de `scene.render`, e as sondas leem o `scene.render`: **não** é captura de tela nem ponteiro físico.

Diff de equivalência com o head de `2c7cf7c`: só o runtime (`directed-scene.js`, o `Set` do `nodeId`), um teste de runtime, a prova nativa e seu teste Python, e docs. Os commits de documentação posteriores a `586a671` não tocam `crates/`, `runtime/` nem `scripts/` (verificável por `git diff 586a671 HEAD --stat`); o head da PR após eles prova o mesmo código.

Limites que esta execução **não** muda: aceite independente S10 `FAIL`/`BLOCKED` (BL120/BL121, externos); `full_av`/áudio dependem do S9; a mutação F3 (BL125) segue só com a atribuição antiga e a prova direta `BL125`, que passou na suíte runtime; a revisão independente final (Opus) ainda precisa aceitar o delta. Dois nitpicks de `4494744803`/`5803260055` (limites host×runtime em `crates/render`) são Rust e escopo novo, não acionados aqui.
