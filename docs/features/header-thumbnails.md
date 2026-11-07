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
reconstrução do runtime (BL144.3) e miniatura orientada pela captura do SHARP (BL144.2, depende da PR #23). O medidor e a
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
  regra em JS; funciona para nuvem deslocada, minúscula ou na escala de uma captura real (dezenas de milhares de unidades,
  dentro de near/far 0.05/1000). Nuvem sem extensão, não finita ou grande demais é `splat_not_drawable`. Depois do quadro,
  25 sondas e a média decidem se saiu **só fundo**: então é `empty_picture`, nunca uma miniatura. A convenção de eixos do
  SPZ no Spark não foi verificada com um arquivo real; a checagem de imagem vazia é a rede de segurança.
- **Identidade** de uma miniatura (v2): `v2|<tipo>|<hash do arquivo>|<L>x<A>|deps=<id>=<tipo>:<hash>,...|framing=fit:default|own`.
  Uma cena é desenhada **com os splats que ela nomeia**; por isso cada ativo de que ela depende (`meta.requires`,
  transitivo, por ID da biblioteca, ordenado) entra na identidade com tipo e hash, ou como `missing` quando a biblioteca não
  o tem (`unhashed`: sem identidade ainda, a miniatura espera). Religar/substituir o splat, importar a dependência que faltava
  depois de uma falha ou remover um ativo mudam a identidade: a imagem antiga nunca é mostrada e **uma falha lembrada deixa de
  valer quando a causa some**. Edições na linha do tempo não a invalidam. `framing=fit:default` é a vista usada no
  enquadramento do splat avulso (a PR #23 passa a orientação da captura por `view`, ver BL144.2). A miniatura só é mostrada
  (`EditorState::thumbnail`) enquanto a identidade guardada é a atual. Calculada do modelo, sem ler arquivo.
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
- **Recursos:** com um export de vídeo em andamento (`export_running`) o tick **não** inicia miniaturas que ninguém pediu
  (elas começam quando o export acaba). O custo por tick é só montar as identidades (consultas ao modelo, sem clonar o
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

**Estado: corrigido, a validar.** Esta revisão (correção dos achados da revisão independente B da PR #24) foi **escrita
sem compilar nem executar** os gates de Rust, runtime e janela: o disco ficou abaixo do piso de 12 GiB e a regra da tarefa
proibia Cargo/build/GPU. As contagens e execuções abaixo, anteriores a esta revisão (workspace 1508/0/12, runtime 148/148,
provas 53/56), **não foram reexecutadas pelo revisor nem por mim** e não valem para este head. Só rodou o teste puro de Python
da prova (`scripts/header-thumbnails-proof.spec.py`, 18 testes, com e sem `-O`).

| Superfície | Evidência escrita neste head | Estado |
|---|---|---|
| Lib (core) | `crates/core/tests/thumbnails.rs`: dependências (religar/importar depois da falha/remover/transitivo/ciclo/outro projeto), snapshot mínimo, ID do stand-in curto e sem colisão, medida entre formatos; unitários de `splat`, `splat_framing` (projeção pinhole) e `thumbnails`; `crates/core/tests/headless.rs` agora compara o **resumo completo** de dois estados do mesmo projeto (clone), sem remover `project_id` | a validar (não compilado) |
| Lib (media) | `crates/media/tests/thumbnails.rs`: enquadramento de nuvens deslocadas/escaladas/várias na mesma sessão/ID reaproveitado com outro arquivo (câmera conferida por um pinhole do teste), nuvem não desenhável e imagem vazia, dependência ausente → importada → desenhada, hash de dependência muda o cache, cache íntegro/7 danos (truncado, ausente, IHDR, bloco de pixel, bytes após IEND, outro tamanho, vazio), `png_dimensions`, ID de 64 bytes e colisão, **A→A′→B→A″ com o mesmo ID de documento e renderizador retido** (job antigo `cancelled`, nunca aplicado), `job.commit` de outra abertura = `stale`, `waiting` em vez de `reused`, pausa após cancelamento do usuário, sem miniaturas durante export | a validar (não compilado) |
| Runtime real | `crates/render/tests/thumbnail_frames.rs`: splat avulso enquadrado (centro da imagem, cantos de fundo) e **nuvens deslocadas** (1000 u, 0,01 u, 90 000 u) no meio da miniatura; o espião devolve ao adaptador as sondas dele | a validar (não executado; exige runtime) |
| UI (GPUI) | `workstation/identity_tests.rs` e `library_panel::tests` (sem mudança de código nesta revisão) | execução anterior, não repetida |
| Headless | `scripts/header-thumbnails-proof.py` (`tessel-render`) refeito sobre `scripts/proof_surface.py`: job **observado em andamento** (`running`, alguns quadros e não todos) → `job.cancel` → `cancelled` no meio, resultado recusado, novo job correto; A→B→A com `stale`; splat deslocado | não executado |
| Janela nativa | mesmo script, `HOME` próprio, só `ui op/get/set`: job de B observado em andamento e substituído por A (precisa terminar `cancelled` antes de desenhar tudo; senão **falha**, não passa); monitor com **clipe** desenhando um quadro real (hash, fase `ready`); captura do SO da própria janela pelo id da janela (recusa vira SKIP, nunca check verdadeiro) | não executado |

Rodar: `python3 scripts/directed-scene-proof/with-gpu-lock.py python3 scripts/header-thumbnails-proof.py`. Capturas vão
para `--out`; só `--copy-screenshots DIR` as copia (nada é escrito em `docs/` por padrão). Código de saída: 0 completo,
1 falhou, 2 incompleto (SKIP).

`scripts/proof_surface.py` é o helper compartilhado (BL140–BL144); aqui é a cópia **idêntica byte a byte** do blob
`bb555f0983061374708a4ae6d97e38546fa4dc59` de `origin/feat/bl140-camera-keyframes` (último commit que o toca:
`420a0b160de8b88cd3f6871b715eb13a8b273c85`). Outro agente corrige a lógica dele nas PRs #20/#23; **não é editado aqui** e
a integração reconcilia as duas cópias. O script desta feature usa só a API do helper (`Headless`, `Native`, `run_surface`,
`Native.capture`) e cada superfície roda dentro de `run_surface`: binário ausente, processo que morre, resposta que não é
JSON, EOF ou tempo esgotado viram falha **dessa** superfície, a outra ainda roda e o relatório é escrito. A prova pura de
Python (`header-thumbnails-proof.spec.py`) cobre isso com processos duplos, o oráculo, a regra "terminou antes de poder ser
cancelado = falha" e a regra da captura.

**Limites declarados.**

- **Monitor preto (BL144.4):** as capturas antigas eram de uma sequência com **0 clipes**; um monitor sem clipe é preto por
  desenho. Isso **não** confirma um defeito da WebView nem o contrário: é prova **incompleta**. As capturas sintéticas
  históricas ficam só como evidência parcial do cabeçalho e da biblioteca. A prova nativa agora põe um clipe da cena na linha
  do tempo e exige um quadro real do monitor (hash e fase `ready`) antes da captura; essa execução está pendente.
- SwiftShader, não GPU real; sem ponteiro físico (não há controle novo para clicar).
- Imagem parada e vídeo seguem com glifo (BL144.1). A identidade não contém a captura do SHARP e o splat do SHARP seria
  desenhado sem a orientação dela (BL144.2: o enquadramento já aceita `view`, falta a PR #23 fornecer a orientação).
- **BL144.3 (impressão do runtime):** o cache só é invalidado por `VERSION`/`CACHE_VERSION`; reconstruir `runtime/dist` não
  refaz imagens guardadas. Plano: o fabricante do renderizador expõe uma impressão (hash do bundle) em `capabilities`, o
  adaptador a inclui na impressão do cache (não na identidade do core, que não conhece o runtime) e um comando
  `media.thumbnails {rebuild: true}` limpa só as miniaturas. Ainda não implementado.
- A prova nativa tem janela real, mas o enquadramento de formatos PLY/SPZ só é provado por testes unitários do core (medida
  entre formatos); a prova ponta a ponta usa `.splat`.
- Mutações dirigidas anteriores (aplicar sem conferir o carimbo, mostrar sem conferir a identidade, título fixo, assinatura da
  árvore sem rótulo) valiam para o código anterior; **as mutações desta revisão ainda não foram executadas** (por exemplo,
  um host que reutiliza a sessão, uma identidade sem dependências, um enquadramento fixo): os testes foram escritos para
  falhar com elas, sem prova de execução.

R2: tamanhos extremos agora cabem em MAX_SIDE com lados pares (mínimo2; aspecto extremo é aproximado). Chamadas explícitas de miniatura aguardam export ativo. Um export válido iniciado com miniatura ainda em curso cancela a miniatura e devolve renderer_busy/retryable até seu frame terminar, sem iniciar outro renderer; o chamador repete após concluir o cancelamento. A pausa automática é desfeita por uma nova solicitação explícita. Isso não é uma fila automática de export e deve ser tratado na UI/CLI como erro transitório. Testes escritos, não executados por disco. Spec do helper comum incluída nesta branch. SPZ/PLY e orientaçãoSHARP continuam a validar em runtime, custo de BFS por paint e falha de snapshot silenciosa seguem limites pendentes.
