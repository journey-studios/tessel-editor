# F053 — Cabeçalho do projeto aberto e miniaturas reais de splat/cena (BL144)

## Objetivo e limites

Duas lacunas do filme SHARP (BL144):

1. Depois de `project.open` pela CLI o cabeçalho seguia mostrando `North / Master`. Agora ele lê o estado vivo:
   `Projeto / Sequência` do projeto que está aberto, criado, restaurado ou renomeado, por qualquer superfície.
2. A biblioteca mostrava só o glifo de **splat** e de **cena**. Agora cada um tem uma miniatura **desenhada** pelo
   renderizador real (o mesmo do monitor e do export): um quadro da fonte do ativo, em `240 px` de largura e altura par
   na proporção da sequência.

Apresentação e cache descartável: nada disto é edição (sem revisão, sem undo, sem `dirty`). **Fora do escopo:** imagem
parada e vídeo (BL144.1, a operação `media.thumbnail` do FFmpeg não está ligada à biblioteca), invalidação por
reconstrução do runtime (BL144.3). A miniatura de um splat do SHARP passa pela câmera e pelos eixos da captura (BL144.2, abaixo). O medidor e a
telemetria da PR #18 não foram tocados (`perf_meter.rs`, `status_bar.rs`, `perf.rs`); só a identidade/título do
cabeçalho mudou, sem alterar rótulos nem handlers de desempenho.

## Contrato do core / biblioteca

`core::header` (sem GPUI): `EditorState::header() -> Header { app, project, sequence, project_id, dirty, document }`,
`Header::label()` = `Projeto / Sequência` (só o projeto se a sequência for vazia) e `to_json()`.
`status.header` e `project.header` respondem o mesmo objeto; o nó semântico do título (`node:13`) e o texto pintado vêm de
`label()`. Nenhum estado paralelo: a janela lê o `EditorState` vivo a cada quadro.

`core::thumbnails` (sem GPUI nem renderer):

- **Quem recebe miniatura:** só `splat` e `scene` com arquivo e hash (`thumbnail_targets(retry)`); uma cena cujo splat
  falta é `failed` com `missing_dependency` (nomeia o ativo), sem iniciar renderizador.
- **Enquadramento do splat avulso** (`core::splat` + `core::splat_framing`, sem GPUI): o adaptador **mede** o arquivo
  (`.splat`, PLY binário e SPZ; amostra de posições, extremos robustos) e a câmera é calculada a partir dessa medida: a
  nuvem é normalizada para a esfera unitária (posição e escala do objeto de cena) e a câmera a enquadra com margem. Não há
  regra em JS; funciona para nuvem deslocada ou minúscula e para uma nuvem **a até ±65 504 unidades por eixo** da origem,
  dentro de near/far 0.05/1000. **Limite medido (BL144.16):** o Spark 2.4.0 guarda os centros em `float16` (`PackedSplats`,
  máximo 65 504); uma nuvem além disso vira só fundo no renderizador, e o adaptador a reporta como `empty_picture`, sem
  miniatura (o teste de runtime usa uma nuvem a 90 000 u para provar a recusa). Nuvem sem extensão, não finita ou grande demais é `splat_not_drawable`. Depois do quadro,
  25 sondas e a média decidem se saiu **só fundo**: então é `empty_picture`, nunca uma miniatura. A convenção de eixos do
  SPZ no Spark não foi verificada com um arquivo real; a checagem de imagem vazia é a rede de segurança.
- **Identidade** de uma miniatura (v2): `v2|<tipo>|<hash do arquivo>|<L>x<A>|deps=<id>=<tipo>:<hash>,...|framing=<enquadramento>`, onde o enquadramento é `own` (cena), `fit:default` (splat sem captura) ou `capture:cv1:<L>x<A>:f=<fx>,<fy>:c=<cx>,<cy>` (splat com captura do SHARP).
  Uma cena é desenhada **com os splats que ela nomeia**; por isso cada ativo de que ela depende (`meta.requires`,
  transitivo, por ID da biblioteca, ordenado) entra na identidade com tipo e hash, ou como `missing` quando a biblioteca não
  o tem (`unhashed`: sem identidade ainda, a miniatura espera). Religar/substituir o splat, importar a dependência que faltava
  depois de uma falha ou remover um ativo mudam a identidade: a imagem antiga nunca é mostrada e **uma falha lembrada deixa de
  valer quando a causa some**. Edições na linha do tempo não a invalidam. `framing=fit:default` é a vista usada no
  enquadramento do splat avulso sem captura; com captura o componente nomeia a imagem de origem, as focais e o ponto principal
  (o que muda a imagem), de modo que **outra captura, ou o mesmo arquivo lido sem ela (`capture: ignore`), é outra imagem** e o
  cache em disco nunca devolve a anterior. A cor e a versão da escrita não entram (não mudam o desenho). A miniatura só é mostrada
  (`EditorState::thumbnail`) enquanto a identidade guardada é a atual. Calculada do modelo, sem ler arquivo.
- **Splat do SHARP (BL144.2).** Um splat com `meta.capture` (validada na importação, `sharp::Capture`) **não é re-enquadrado**:
  o adaptador usa `splat_framing::fit_capture`, que mantém a nuvem onde o arquivo a põe (`position` 0, `scale` 1), gira-a **uma
  vez** por `sharp::ORIENTATION` (`rotation: [180,0,0]` no objeto `splat` da cena de apoio, os eixos y para baixo/z à frente do
  arquivo para y para cima/-z à frente do editor) e usa a câmera da captura (`Capture::camera`: na origem, olhando para -z, FOV
  **vertical** das intrínsecas). A miniatura é a imagem de origem, em pé, como a visualização do clipe. A cena de apoio é uma
  *cena*: a camada do plano não leva `orientation` (só um clipe de splat com captura leva), então o giro existe em um único
  lugar. Numa proporção diferente da imagem de origem a lente é a mesma e o corte muda (como no monitor). Sem captura vale o
  enquadramento comum, inalterado byte a byte (o objeto só ganha `rotation` quando há giro). Uma nuvem inteira atrás da câmera
  (`z <= 0` no arquivo) é `splat_not_drawable` com a causa, e nunca desenhada de outro lado; `empty_picture` segue valendo.
- **Carimbo (`Stamp`)** = ID do projeto + número da sessão (`ProjectSession`). Reabrir o mesmo documento é outra sessão.
  `set_thumbnail`/`set_thumbnail_failure` recusam com `stale_result` um resultado de outro carimbo ou de um ativo que mudou
  no meio do desenho, sem alterar nada. `prune_thumbnails` remove o que deixou de existir.
- **Falha** é lembrada por identidade (`code`, `message`); não é tentada de novo a cada tick, só com `retry: true`.
- `thumbnail_report(id)` → `{"state": "none|pending|ready|failed", ...}` (`path`, `width`, `height`, `identity` ou
  `code`/`message`). Aparece em `media:<id>` na árvore e em `media.status`.

`crates/media` (adaptador, injetado no `MediaHost`): `media.thumbnails {assets?, retry?}` e o agendamento automático da janela.

- Um job `thumbnail` (um por vez) desenha um quadro **por alvo** pelo renderizador do export, **fora da thread da UI**
  (`PreviewIntent::Source`, quadro 0), sobre um **snapshot mínimo** do projeto (só os alvos e o que eles nomeiam, no mesmo
  canvas; sem histórico de undo e sem o `state.clone()` do projeto inteiro). Falhar num alvo vira `Made::Failed` e não custa
  o quadro dos outros; só o cancelamento encerra o job e então **nada é aplicado**.
- O PNG vai para `<workspace>/.tessel/cache/<hash>/picture-<L>x<A>-<sha256(identidade)[..16]>/thumb.png`. Um cache só é
  **confiado** se o manifesto traz a identidade, o arquivo tem o tamanho em bytes e o CRC-32 gravados e é um PNG íntegro
  (assinatura, IHDR de 13 bytes primeiro, CRC de cada bloco, ao menos um IDAT, IEND por último e nada depois) **do tamanho
  pedido**. Truncado, alterado, de outro tamanho ou ausente: a entrada é removida e o quadro é desenhado de novo. Sem
  workspace o job espera (nada é escrito fora do projeto).
- **Splat avulso:** o runtime da `main` não desenha uma camada de splat sem cena. O job desenha-o como uma **cena de um
  objeto** num snapshot privado, com ID `thumb-<16 hex do ID do splat>` (curto, estável, sem estourar 64 bytes e nunca o ID
  de um ativo que o snapshot tenha: colisão vira `-2`, `-3`…). O projeto aberto não é alterado.
- **Identidade e cancelamento:** o carimbo é projeto + sessão: reabrir o **mesmo** documento é outra sessão. `poll` cancela o
  job de um carimbo que não é o do projeto aberto (`cancel_requested`; o job observa `ctx.check()` antes de cada alvo e após
  o início do renderizador) e só aplica resultado com carimbo atual; `finish` (CLIs) e `job.commit` também recusam (`stale`).
  **Nunca se devolve o job de outro projeto como `reused`:** com um job de outro carimbo ainda terminando,
  `media.thumbnails` responde `state: "waiting"`, `reused: false`, `waiting_for` e `deadline_seconds`, pede o cancelamento e
  não inicia um segundo renderizador; o tick inicia o job do projeto aberto quando o antigo termina. Um cancelamento
  **do usuário** pausa o agendamento automático daquele carimbo (sem laço de refila); `media.thumbnails` explícito a retoma.
- **Prazos declarados:** o cancelamento é observado **entre quadros**: no pior caso, um quadro em andamento
  (`FRAME_TIMEOUT` = 60 s) mais a partida do renderizador; o `Renderer` bloqueia e não é interrompível no meio do quadro.
  Não há retentativa infinita: uma falha é lembrada por identidade e só `retry: true` a refaz.
- **Recursos:** um renderer por vez entre exports e miniaturas, pela política única de [F056](renderer-ownership.md)
  (BL144.10): com um export (ou seu preflight) em andamento, ou o renderer reservado para um export recusado, o tick
  **não** inicia miniaturas que ninguém pediu (elas começam quando o export acaba) e `media.thumbnails` responde
  `renderer_busy`/`retryable`. O custo por tick é só montar as identidades (consultas ao modelo, sem clonar o
  estado nem ler arquivos); a cópia do projeto só acontece ao iniciar um job e é mínima. Não há número medido de
  custo por tick neste head.
- O resultado volta pelo `poll` do tick da janela; a UI não espera por job algum.

Erros: `unknown_element` para ativo inexistente em `assets`; `unknown_element`/motivo (`reason`) se nada precisa de
miniatura; falha de renderização aparece em `thumbnail.failed` com o `code` real (ex.: runtime ausente), nunca escondida.

## CLI headless

`./scripts/render` (processo `tessel-render`, runtime real, **sem janela**; `./scripts/headless` não tem `MediaHost`):

```sh
./scripts/fixtures /tmp/fx
printf '%s\n' \
 '{"op":"project.create","name":"Aurora","sequence":"cut-a","width":320,"height":180,"length":48,"dir":"/tmp/aurora"}' \
 '{"op":"project.header"}' \
 '{"op":"media.import","paths":["/tmp/fx/cloud.splat","/tmp/fx/scene.tessel-scene.json"]}' \
 '{"op":"media.thumbnails"}' \
 '{"op":"media.status"}' | ./scripts/render
```

`project.header` → `{"label":"Aurora / cut-a","project":"Aurora","sequence":"cut-a",...}`; `media.thumbnails` (aguarda e
aplica) → `{"action":"media.thumbnails","applied":["cloud","scene-tessel-scene"],"failed":[],"stale":[]}`; `media.status`
traz `thumbnail.state: "ready"` e o `path` de cada PNG. Com `"wait":false` o job fica pendente e o resultado só vale em
`job.commit` se o projeto ainda for o mesmo. `./scripts/headless` responde `project.header` (sem mídia).

## UI e CLI da janela

- **Cabeçalho:** `node:13` (título) tem `label` = `Projeto / Sequência`; segue `project.create`, `project.open` e o
  restante sem ação de UI. `./scripts/ui get node:13`, `./scripts/ui op '{"op":"project.header"}'`.
- **Biblioteca:** o tile de `splat`/`scene` pinta a miniatura (`img`, `object-fit: cover`) quando a identidade bate, e o
  glifo antes disso; `./scripts/ui get media:<id>` mostra `state.thumbnail`. A janela agenda os jobs sozinha a cada tick
  (`schedule_thumbnails`) e `media.thumbnails` também funciona (responde com o job). Barra de status: "Miniaturas
  concluída/cancelada/falhou".
- **Correção vizinha (identidade):** a árvore de pastas só comparava IDs; todo projeto tem a mesma pasta raiz, então depois
  de abrir outro projeto a árvore seguia com o nome do anterior (`Aurora` dentro de `Basalt`, visto na captura anterior à
  correção). A assinatura passou a incluir o rótulo.
- Sem novo controle clicável: nenhum botão, atalho ou diálogo foi criado (por isso nenhum `disabled_reason` nem
  `Workflow::has_dialog`). As operações novas são `project.header` e `media.thumbnails`.

## Evidências e status

**Estado: validado na base `6639976` (main depois das PRs #24 e #28) nas três superfícies, com os limites abaixo.**
Tudo desta seção foi **executado na PR de validação** (BL144.17), sobre dados sintéticos de `./scripts/fixtures` e dos
geradores da própria prova; as contagens antigas (workspace 1508/0/12, runtime 148/148, provas 53/56) não valem para este
head e não são citadas como evidência.

Gates (macOS, SwiftShader, `TESSEL_REQUIRE_RUNTIME=1`, runtime de `./scripts/runtime setup`
`runtime.js` sha256 `7b6b43381b55f4d9768164931783f943944608bada70240016d366eb803a2a98`,
`index.html` `707fd90fac224637b486d78a74babbd274e749e02e60aa95da8a314644c484a6`):

| Comando | Resultado |
|---|---|
| `cargo fmt --all -- --check` | limpo |
| `./scripts/cargo check --workspace --locked` | limpo |
| `./scripts/cargo clippy --workspace --all-targets --locked -- -D warnings` | limpo |
| `TESSEL_REQUIRE_RUNTIME=1 ./scripts/cargo test --workspace --locked --no-fail-fast` | **1785 passam, 0 falham, 12 ignorados** (101 binários; os 12 são `#[ignore]` explícitos de medição/disco/GPU, sem linha `SKIPPED`) |
| `./scripts/cargo build -p tessel-editor --locked` (+ `tessel-render`) | ok; sha256 `tessel-editor` `5c04cc26…584e`, `tessel-render` `18fba0d1…2079` (debug, esta máquina) |
| `./scripts/cargo tree -p tessel-editor-core --locked` | sem `gpui` nem `wry` |
| `scripts/header-thumbnails-proof.spec.py` (18), `proof-surface.spec.py` (29), `scene-lifecycle-proof.spec.py` (7) | passam, normal e `-O` |

| Superfície | Evidência executada | Estado |
|---|---|---|
| Lib (core) | `crates/core/tests/thumbnails.rs` (dependências, snapshot mínimo, stand-in curto/sem colisão, medida entre formatos), unitários de `splat`, `splat_framing` e `thumbnails`. Mutações: tirar a sessão do carimbo → 3 testes falham; tirar as dependências da identidade → 6 falham (4 core e 2 media) | V |
| Lib (media) | `crates/media/tests/thumbnails.rs` (21 testes): enquadramento por arquivo, cache íntegro e 7 danos, A→A′→B→A″ com o mesmo ID de documento, `stale` em `job.commit`, `waiting`, pausa do usuário, sem miniaturas durante export, e o novo `a_thumbnail_job_ended_by_commit_or_discard_frees_the_slot_for_the_next_one` (vermelho antes da correção do host, verde depois) | V |
| Runtime real (Chrome) | `crates/render/tests/thumbnail_frames.rs` com `TESSEL_REQUIRE_RUNTIME=1`: splat avulso, nuvens deslocadas (1 000 u, 0,01 u e uma captura a 40 000 u) no meio da imagem (centro > 40, cantos ≤ 12, limiares inalterados) e a nuvem a 90 000 u recusada como `empty_picture` (BL144.16) | V |
| UI (GPUI) | `workstation/identity_tests.rs` (cabeçalho segue o projeto aberto, o tile só mostra a imagem do seu ativo) | V (sem ponteiro físico: não há controle novo) |
| CLI headless | `scripts/header-thumbnails-proof.py --only headless` com `tessel-render` (runtime real, sem janela): **123 passam, 0 SKIP** | V |
| Janela nativa | `scripts/header-thumbnails-proof.py --only native`, `HOME` próprio, só `ui op/get/set`, captura do SO pelo id da janela (`screencapture -l`): **131 passam, 0 SKIP** | V (sem ponteiro físico) |

O que a prova mostra, nas superfícies indicadas abaixo, com identidades e pixels conferidos por um oráculo em Python puro (leitor de
PNG e projeção pinhole, sem o código do adaptador):

- cabeçalho vivo `Projeto / Sequência` depois de criar, abrir e reabrir projetos (a renomeação só tem teste de lib); miniaturas de splat e cena desenhadas
  (caixa laranja, esfera azul, nuvem no meio, cantos de fundo);
- **mudança de dependência:** `media.relink` do splat (com `accept_changed`) muda as identidades e os caminhos das miniaturas do splat e da cena; as medições de pixel continuam valendo.
  Não há comparação explícita de todo o conteúdo de cada imagem antes e depois. **Faltando→presente não tem caminho ao vivo**: `media.import` de
  uma cena cujo splat não está na biblioteca é recusado com `missing_dependency`; esse caso só tem teste de lib;
- **A→B→A no mesmo documento e sessão** sem aplicar resultado de outro projeto (`stale`);
- **cancelamento** visto em andamento (`running`, alguns quadros e não todos), depois `cancelled` com `done < total`,
  nada aplicado e job novo correto; resultado de outra sessão recusado (`stale`);
- **retomada e pausa do usuário (janela nativa):** `job.cancel` do usuário pausa o agendamento por 5 s sem novo job e sem imagens novas;
  `media.thumbnails` explícito retoma;
- **mesmo documento reaberto** (outra sessão): na janela nativa, o job da sessão antiga termina `cancelled` no meio e a nova desenha automaticamente.
  No headless, `job.commit` de outra sessão responde `stale`, sem tocar a biblioteca, e a prova solicita as miniaturas explicitamente;
- **janela com clipe real e monitor `ready`:** a captura (`screencapture -l`) mostra o cabeçalho `Chalk / cut-c`, as duas
  miniaturas reais e o monitor com a cena desenhada. Captura que o SO recusa é SKIP, nunca PASS (houve 0).

Rodar (os dois bloqueios de GPU, `/tmp/tessel-scene-gpu.lock` e `/tmp/tessel-video-bl140-bl144-review-gpu.lock`, se outro
agente os usa):

```sh
./scripts/runtime setup
./scripts/cargo build -p tessel-editor --locked && ./scripts/cargo build -p tessel-editor-render --bin tessel-render --locked
python3 scripts/header-thumbnails-proof.py --editor target/debug/tessel-editor --render target/debug/tessel-render \
  --only headless --report /tmp/proof-headless.json
python3 scripts/header-thumbnails-proof.py --editor target/debug/tessel-editor --render target/debug/tessel-render \
  --only native --report /tmp/proof-native.json --copy-screenshots /tmp/shots
```

Capturas vão para `--out`; só `--copy-screenshots DIR` as copia (nada é escrito em `docs/`). Código de saída: 0 completo,
1 falhou, 2 incompleto (SKIP). Cada superfície roda dentro de `run_surface` (`scripts/proof_surface.py`, o helper
canônico da `main`): binário ausente, processo que morre, resposta que não é JSON, EOF ou tempo esgotado viram falha
**dessa** superfície, a outra ainda roda e o relatório é escrito.

**Evidência do BL144.2 (PR #31, integrada com `main` `10683c5`, código de produção validado em `3c260a6`, macOS, SwiftShader).**

- `cargo fmt --check`, `./scripts/cargo check` e `clippy --all-targets -- -D warnings` limpos; `TESSEL_REQUIRE_RUNTIME=1 ./scripts/cargo test
  --workspace --locked --no-fail-fast`: **1834 passam, 0 falham, 12 ignorados**, incluindo
  `a_sharp_splat_thumbnail_is_the_source_picture_upright_and_a_plain_reading_is_not` (runtime real, `runtime.js` sha256 `7b6b4338…`, sem
  mudança nesta PR); `tessel-editor` e `tessel-render` compilam com `--locked` (debug `b289968e…` e `62fe287e…`); `cargo tree -p tessel-editor-core` sem `gpui` nem `wry`;
  `header-thumbnails-proof.spec.py` (31 testes) passa normal e `-O`. O clippy da primeira integração reprovou um tipo complexo no teste
  de `media` (`type_complexity`); corrigido com um alias, sem `allow`.
- `scripts/header-thumbnails-proof.py`, fase BL144.2, as duas superfícies: headless **152 passam**, janela nativa **167 passam**, 0 SKIP.
  Cada superfície importa `sharp.ply` (cinco gaussianas sobre pixels conhecidos da imagem 160×120), desenha a miniatura e confere, por um oráculo
  em Python puro, as cinco cores sobre os pixels da imagem de origem escalada à altura e centrada; a mesma importação com `capture: ignore`
  é outro caminho de cache e a imagem não coincide (1 de 5). Vistas: a miniatura em pé (vermelho/verde em cima, azul/amarelo embaixo,
  branco no centro), a leitura sem captura girada 180° e a janela com o tile de `sharp.ply`.
- Aprovação visual por um humano, GPU real e ponteiro físico continuam distintos e **não** foram executados (I).

**Defeitos reais achados e corrigidos na validação.**

- `MediaHost` ficava preso a um job de miniatura que já terminara quando `job.commit` ou `job.discard` o encerrava antes
  do `poll`: a janela nunca mais desenhava e `media.thumbnails` respondia `reused` com um job já encerrado. A pausa do
  usuário de um job cancelado também se perdia por esse caminho. `release_thumbnail_job` é agora o único ponto que libera
  o slot (`poll`, `job.commit`, `job.discard`).
- O Spark guarda os centros em `float16` (máximo 65 504): o contrato prometia "dezenas de milhares de unidades" para uma
  captura real. Nuvens além desse valor viram só fundo e são recusadas como `empty_picture` (BL144.16).
- Quatro testes já falhavam na `main` (BL156) por erros do próprio teste, não da lógica: ver o backlog.

**Limites declarados (não provados nesta validação; os IDs seguem abertos no backlog).**

- **BL144.1:** imagem parada e vídeo seguem com glifo.
- **BL144.2:** validado na PR #31 (ver "Evidência do BL144.2" acima); SwiftShader, sem GPU real, só o fixture sintético de cinco gaussianas (um PLY real do SHARP é BL143.8; eixos SPZ/PLY, BL144.13).
- **BL144.3 (impressão do runtime):** o cache só é invalidado por `VERSION`/`CACHE_VERSION`; reconstruir `runtime/dist` não
  refaz imagens guardadas. Plano: o runtime expõe o hash do bundle em `capabilities`, o adaptador o inclui na impressão do
  cache e `media.thumbnails {rebuild: true}` limpa só as miniaturas. Não implementado.
- **BL144.10:** validado em [F056](renderer-ownership.md) (um export iniciado atrás de um quadro de miniatura é
  recusado com `renderer_busy`/`retryable`, o job de miniaturas é mandado parar e o renderer fica reservado para o
  export; a pausa do usuário sobrevive a export e preflight, concluídos, cancelados ou falhos). Limites: BL144.18
  (a sonda do preflight cancelado) foi corrigida e BL144.19 (o diálogo) tem política e prova; ver F056.
- **BL144.13:** a convenção de eixos do SPZ e do PLY no Spark só é provada por testes unitários do core (medida entre
  formatos); a prova ponta a ponta e o teste de runtime usam `.splat`.
- **BL144.14:** o custo de montar identidades (BFS) por paint/tick não foi medido.
- **BL144.15:** uma falha ao montar o snapshot no agendamento pode ser engolida.
- SwiftShader, não GPU real; **sem ponteiro físico** (não há controle novo para clicar); faltando→presente só em lib.
- Mudou o runtime da `main` (PR #28, perf do viewport) em relação ao `ce71ce9`; as execuções acima são todas sobre o
  runtime novo (hashes acima).

## Política de requisições (R2, vigente; export: F056)

`media.thumbnails` valida os ativos antes de alterar a pausa por cancelamento. Durante cancelamento ou ao pedir
filtro/retry com um job atual, responde `thumbnail_busy` com `retryable: true`; o chamador repete depois do término. O
pedido não é aceito nem enfileirado nesses casos. Com um export (normal, dirigido ou seu preflight) em andamento, ou o
renderer reservado para um export que foi recusado atrás de um job de miniaturas, responde `renderer_busy`/`retryable`:
é um erro transitório, não uma fila. O agendamento automático espera o export acabar; uma miniatura explícita nunca
preempta o export nem desfaz a pausa do usuário. Inversamente, um export pedido com um job de miniaturas no renderer é
recusado (`renderer_busy`), o job é mandado parar no próximo quadro (sem pausar o desenho: não é cancelamento do
usuário) e o renderer fica reservado por 30 s para a nova tentativa ([F056](renderer-ownership.md)). O cancelamento é
conferido antes de enviar cada quadro ao renderer. O teto de
amostragem é 20 mil posições. Os lados da miniatura são pares entre 2 e `MAX_SIDE` mesmo em proporções extremas (aspecto
extremo é aproximado).
