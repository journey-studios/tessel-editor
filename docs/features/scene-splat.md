# F044 — Splat Spark numa cena dirigida (S7, subconjunto de BL025)

> O número F044 foi escolhido por esta frente; o coordenador renumera se outra frente da rodada S1–S10
> usar o mesmo.

## Status

Lib, runtime e CLI headless **V no corpus sintético** (Chrome real, SwiftShader); pacote **V**
(diretório e arquivo, sem as origens). Monitor nativo (WKWebView) **a validar**: usa o mesmo
`SceneSlots`/runtime, mas esta frente não rodou a prova nativa. Luz, clima, rigs e mapas de dados
em nó `splat` continuam **recusados** por `unsupported`, com o nome do domínio.

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
- Imutabilidade das bytes durante export é do S3 (BL109); o seam é o mesmo mapa aprovado.

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
