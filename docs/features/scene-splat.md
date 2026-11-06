# F048 — Splat Spark numa cena dirigida (S7, subconjunto de BL025)

> F048: o F044 já é do aceite independente do S10 (PR #8, `scene-acceptance.md`); esta frente usa F048 para
> não colidir. O coordenador renumera se outra frente da rodada S1–S10 usar o mesmo.

## Status

Lib, runtime e CLI headless **V no corpus sintético** (Chrome real, SwiftShader); pacote **V**
(diretório e arquivo, sem as origens); export **V** com cópia privada do `spz` aprovado (S3/BL109, integrado).
Monitor nativo (WKWebView) **V no corpus independente do S10** (`scripts/scene-splat-native-proof.py`, 119/119
checks, instância própria, só pedidos semânticos). Cor guardada do splat (BL019-F1) **corrigida** e provada
com controle negativo. Luz, clima, rigs e mapas de dados em nó `splat` continuam **recusados** por
`unsupported`, com o nome do domínio. Aceite independente do S10: ver [Aceite S10](#aceite-independente-s10-pr-8).

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
  `/splats/N/assetId`, `/splats/N/afterLayers` recalculado do próprio plano), recurso `spz` do próprio asset; recurso `spz` com `used_by` exato.
- Mudança de contrato: `Domain::Splat` deixou de existir (antes: todo nó `splat` tornava o plano
  parcial). `Domain::ALL` tem 13 domínios; `Capability::ALL`, 9.

## Ordem de pintura e limites do Spark

O **core** decide quem está na frente de quem (o runtime só executa): `after_layers` é o índice, na
ordem dos `layers`, do primeiro plano cuja origem de nó está **estritamente mais perto da câmera**
que a origem do nó do splat (profundidade = distância ao longo de `target − position`, na
translação da `world_matrix`); se nenhum está, `layers.len()`. `0` = atrás de todos os planos,
`layers.len()` = na frente de todos. Profundidade igual não é "mais perto": o splat é pintado depois.
O validador recalcula o valor só com o plano (câmera, matrizes, layers): outro valor é
`invalid_plan` em `/splats/N/afterLayers`.

Pintura (`drawDirectedScene`): limpa em preto opaco; para cada fatia de splats em ordem de `after`,
pinta os planos até ali (ordem do plano, F037) e depois os splats da fatia, ordenados pelo Spark
para a câmera exata do pedido (`settle`); por fim os planos restantes. Um plano **nunca** é
reordenado por profundidade: só o splat é encaixado. Entre splats da mesma fatia a ordem é a do
Spark por câmera (a ordem de `splats` não é ordem de pintura).

Limites assumidos: a decisão usa a **origem do nó** (um plano grande atravessado pela nuvem ou um
splat que cruza um plano não é partido; o splat inteiro fica de um lado do plano). Planos com alfa
mostram o splat pelo alfa; um plano opaco que está na frente o esconde. Não há conversão de
lateralidade (o mesmo `.spz` e a mesma câmera do runtime de Spark existente).

## Cor (BL019-F1)

O compositor escreve o espaço de trabalho como está (`outputColorSpace = LinearSRGBColorSpace`, para
que imagens cheguem intactas). O Spark 2.4.0 liga `encodeLinear` sempre que a saída não é sRGB e
decodifica a cor do splat com sRGB→linear, que ninguém recodifica: `(230,49,49)` saía `(203,7,7)`
(achado do S10 no caminho de projeto existente; o caminho dirigido usa o mesmo palco). Correção em
`runtime/src/splat-stage.js` (`keepStoredColour`): após o `onBeforeRender` do próprio Spark, o
uniform `encodeLinear` é mantido em `false` — só no material do Spark. Imagens não passam por esse
material. Efeito: a cor guardada é a mostrada, nos dois caminhos (projeto e cena dirigida).
Reprodução e controle negativo: `runtime/test/directed-scene-splats.test.mjs` ("BL019-F1"): sem a
correção falha com `rgb(202,7,7)` no centro; com ela, a diferença ao armazenado é ≤ 6 e o plano de
imagem na mesma cena fica a ≤ 1. O vermelho puro `(255,0,0)` não vê o defeito (0 e 1 são invariantes
da conversão), por isso os testes antigos passavam.

## Recursos e orçamento

- O host (`SceneSlots`) carrega um splat como `asset.load` `kind: "splat"`, `meta.format: "spz"`
  e a mesma extensão aditiva `scene` (identidade). Antes de gastar um slot, o host abre o
  cabeçalho (`core::splat::inspect`): um arquivo que não é SPZ é `resource_mismatch`.
- Orçamento: arquivo + 16 bytes por splat, no host e no runtime (`sceneEntryBytes`). Figuras
  continuam em `largura × altura × 4`.
- Cada quadro empresta uma malha ao splat (`splatLease`) e a devolve ao terminar ou ao ser
  abortado; o mesmo arquivo em dois nós dá duas malhas, liberadas com o quadro. Perda de contexto
  reconstrói das bytes guardadas.
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
| Core (lib) | `crates/core/tests/scene_plan_splat.rs` | plano/matriz/recurso/capacidade, JSON sem splat idêntico, canais de desempenho, recusas de luz/clima/rig, tampering |
| Runtime puro | `runtime/test/directed-scene-splats.test.mjs` (1–4, 11) | parser do plano, recusas com ponteiro, identidade do recurso, orçamento |
| Runtime em Chrome | mesmo arquivo (5–10) | posição por oráculo pinhole independente, splat entre planos por profundidade (atrás, na frente, entre dois), cor guardada vs. imagem (BL019-F1), ordem/repetição/frio, compartilhamento, recusas tipadas, perda de contexto |
| Host/render | `crates/render/tests/scene_splat_frames.rs` | frame headless 0 e 24 contra oráculo, mesmo hash em qualquer ordem, arquivo não-SPZ, export = `scene.render` quadro a quadro e MP4 verificado |
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

`full_av` precisa do mix de áudio do S9 (`scene.export` ainda recusa o bundle com áudio): fica BLOCKED e não conta
como pass. As fases `splat_*` devem ser reexecutadas (BL121) com o merge de S7 e S9 e a emenda BL120.

### Correções da revisão PR13

A timeline recalcula `afterLayers` após atualizar câmera e matrizes em cada frame; o slot do
frame molde não é reutilizado quando a câmera muda a ordem de profundidade. A regressão
`timeline_recomputes_splat_order_when_the_camera_orbits_past_the_planes` exige mudança de slot,
compara os planos em cache com `ScenePlan::build` e valida cada plano.
A derivação retorna imediatamente sem splats; com splats, indexa os nós uma vez para evitar
buscas lineares repetidas. Isso preserva o caminho de planos sem splats. O script nativo lê
o corpus uma só vez com UTF-8 explícito e grava seu relatório em UTF-8.
