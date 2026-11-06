# F043 — Anexar uma cena dirigida pela interface (BL086, BL100, BL101, BL100a, BL113a)

## Status

Lib **V** (core: `AdmittedScene`; host: `scene.attach.start` e carga sem arquivos, testados com
arquivos sintéticos). CLI headless **V** (`scene.attach.start` pela sessão JSONL com host de mídia).
UI **Parcial**: a janela abre o diálogo, inicia o job de attach, aplica os jobs no tick e mostra a
cena (provado com host real em testes GPUI); o **painel de arquivos do macOS não é provável por
script**, a aparência não foi inspecionada e a prova viva (`scripts/scene-import-proof.py`) está
descrita em "Evidências e status".

BL100: a leitura, o parse e a validação do bundle saem da thread da UI. BL100a: a medição do bundle
(tamanho para o orçamento, contagens, avisos, famílias adiadas) também sai: o *apply* na UI é O(1) no
tamanho do bundle. BL113a: o resultado só é aplicado na mesma **sessão de edição** do projeto. BL101: uma cena cujo
manifesto não nomeia arquivo carrega sem pasta de recursos (e sem workspace).

Esta feature **não cria regra de domínio**. O importador e os gates do bundle (`scene_project`),
`scene.resources.adopt` e `scene.resources.resolve` (F040) continuam sendo a única implementação de
formato, contenção, hash, limite, cancelamento e instalação. O que existe aqui é a cadeia que as
encadeia a partir de um diálogo, e a fiação semântica que torna cada campo e ação alcançável sem
ponteiro.

## Objetivo e limites

Escolher um *render-bundle*, opcionalmente a pasta com seus recursos, e ver a cena no monitor:

1. **Anexar cena** (cabeçalho, botão "Anexar cena", `node:26`; ou `flow.attach_scene`). Campos:
   `bundle` (arquivo, obrigatório), `id`, `name` (opcionais; vazio = o do bundle), `source` (pasta de
   recursos, opcional), `show` (`yes`/`no`, padrão `yes`).
2. **Recursos da cena** (`flow.scene_resources`). Campos `scene` (pré-preenchido com a cena ativa),
   `source` (opcional), `show`. É o caminho de **reparo**: recarrega uma cena já anexada depois que
   arquivos faltantes/alterados foram corrigidos, sem um segundo `scene.attach`.

Cadeia (um estado por passo, visível no nó `scene_import`):

```
attach (job; 1 entrada de desfazer ao aplicar) → adopt (job, só se há pasta) → resolve (job) → select → show
```

- **Nada de domínio no clique.** O botão só monta o pedido (`flows::build_request`) e chama a
  operação. Um campo obrigatório vazio e um pedido que o host recusa antes de existir job (ID
  malformado/duplicado, nome em branco: `invalid_id`, `duplicate_id`, `invalid_value`) voltam
  **com o diálogo aberto**; nada é criado.
- **Leitura do bundle fora da UI (BL100).** `scene.attach.start` lê o arquivo (em blocos, com
  cancelamento entre eles), faz o parse, importa e roda os gates estrutural/temporal num **job**
  (`AdmittedScene::prepare`); o tick (`host.poll`) aplica o resultado com o **mesmo comando
  `scene.attach` do core** (ID, sala e orçamento de bytes), numa entrada de desfazer. Bundle
  inválido, JSON ilegível, arquivo inexistente ou grande demais (`invalid_bundle`, `invalid_json`,
  `bundle_unreadable`, `bundle_too_large`) **fecham o diálogo** e ficam no nó `scene_import` como
  falha no passo `attach` com o código tipado; nada é anexado, nenhuma entrada de histórico e
  nenhuma pasta é criada. Cancelar o job (`job.cancel`, painel de tarefas) termina em `cancelled`
  no passo `attach`. O resultado só é aplicado se o projeto aberto é exatamente o da partida: mesmo ID,
  **mesma sessão de edição** e mesma revisão (`ProjectStamp`); senão falha `stale_result` e nada é
  anexado, sem entrada de undo (anexe de novo). A sessão (BL113a) é identidade só de runtime criada
  quando um estado é construído: reabrir o mesmo documento tem o mesmo ID e revisão 0, mas é outra
  sessão. Salvar, consultar, desfazer/refazer antes de iniciar o job e clones internos não a mudam.
  O job não é cancelado na troca de projeto: termina (trabalho limitado) e é recusado na aplicação;
  a garantia é a guarda, não um cancelamento de melhor esforço.
- **Carga de recursos fora da UI.** `adopt`/`resolve` são jobs do host; a UI só inicia e, no tick de
  250 ms (`workflow_tick`), aplica o que terminou (`host.poll`). Hash e cópia ficam no job.
- **Sem pasta (BL101).** A resposta do attach traz `resources: {files, needs_folder}`, calculado
  pelo manifesto do core. Se `needs_folder == false` (nenhum arquivo a transportar), o diálogo
  **carrega mesmo sem pasta**: a cadeia segue direto para `resolve`, que instala o mapa sem exigir
  `scenes/<id>` nem workspace. Se a cena nomeia arquivos e o diálogo não tem pasta, a cadeia termina
  em `attached` (como antes) e o reparo é `flow.scene_resources`.
- **Uma cadeia por vez.** Outra submissão enquanto uma roda é recusada com `scene_import_busy`.
- **Falha não desfaz o que já valeu.** Se `attach` passou e `adopt` falha, a cena **continua
  anexada** (é uma entrada de desfazer válida), nada foi copiado e nada foi carregado; o diálogo já
  fechou, então o erro vive no nó `scene_import` e na barra de estado. O reparo é `flow.scene_resources`.
- **O monitor só mostra o que carregou.** `show` só roda com o mapa instalado
  (`applied.installed == true`); sem isso o passo falha com `not_installed`.
- O painel nativo de arquivos é **somente um seletor de caminho** (`Picker::File`/`Picker::Directory`);
  digitar o caminho no campo é equivalente.

Limites declarados:

- O limite de 32 MiB por texto de bundle e o orçamento agregado de 128 MiB de cenas continuam os do
  core. Não há attach+carga atômico: entre os dois a cena existe sem recursos.
- O cancelamento (BL114, decisão: continua explícito, sem parser JSON cancelável) é verificado entre
  blocos de leitura e entre as fases (leitura, parse+importação, gates, medição dos fatos), não
  *dentro* do parse de um JSON já em memória: um cancelamento durante essa fase tem efeito ao
  terminar a fase. Nenhuma cena é anexada nesse caso.
- A guarda de revisão é estrita **por decisão do coordenador (BL113)** (sem reaplicar oportunisticamente): qualquer edição registrada no projeto entre o início do job e a
  aplicação torna o resultado `stale_result` (dois `scene.attach.start` com `wait:false` sobre o
  mesmo estado: só o primeiro aplica; o segundo precisa ser repetido). O fluxo da janela tem uma
  cadeia por vez.
- O custo O(bundle) não diminuiu, mudou de thread: `scene.attach` síncrono (CLI, scripts, `ui op`)
  passa pelo mesmo `AdmittedScene::prepare` e continua levando ~0,6 s (debug, 24 MiB) na thread de
  quem chama. Também `project.restore` usa a mesma guarda por sessão; `project.import_scenes`
  compara ID e workspace (BL113b registra a decisão pendente sobre mapas de recursos).
- `scene.attach` síncrono continua existindo (CLI, scripts, `ui op`) e lê na thread chamadora; o
  diálogo da janela é que não o usa mais.
- Cena que **nomeia arquivos** sem pasta: `scene.resources.resolve` devolve `root_unavailable` (ou
  `no_workspace` sem workspace) como antes; só o manifesto sem arquivos dispensa a pasta. O formato
  exige ao menos um asset declarado, então uma cena "sem arquivos" é uma cena com assets não
  usados (ou usados só por seções preservadas): vão para `skipped`, o mapa fica `complete: false`.
- Uma falha de `resolve` mantém instalado o mapa anterior da cena (comportamento do host).
- `scene.export` (contrato R1) **não** está ligado: não há ação para ele; nada foi habilitado.

## Contrato do core / biblioteca

Módulo `workstation::scene_import` (crate `tessel-editor`, sem GPUI no contrato):

| API | Contrato |
|---|---|
| `ImportPlan {scene, source, load, show}` | o que o diálogo pediu; `load = SceneResources || source.is_some()` |
| `flows::import_plan(kind, values, scene)` | valores do diálogo → plano (puro) |
| `SceneImport::{new, request, bind, receive, fail, to_json}` | máquina de estados pura: não toca janela nem host |
| `State::{Running, Attached, Loaded, Failed(Failure)}` | `Failure {step, code, message, details}` |
| `Failure` | `code` é o do job (`resources_invalid`…): `details.job_code` vence o `job_failed` genérico; a mensagem ganha o primeiro problema por arquivo; `details` é guardado inteiro |
| `Workstation::scene_import_submit` | monta o pedido, chama `scene.attach` e inicia a cadeia |
| `Workstation::scene_import_receive` | chamado pelo tick para cada evento de job; ignora jobs que não são da cadeia |
| `SceneImport::attaching(plan, request)` / `Step::Attach` | cadeia que começa pelo job `scene.attach.start`; o ID da cena é o digitado ou o que o core gerou ao aplicar (`applied.scene`); `resources.needs_folder == false` implica a carga mesmo sem pasta |
| `Received::Attached` | o attach foi aplicado e nada mais foi pedido nem é necessário: a cena é só selecionada |

API pública que a cadeia consome (sem GPUI):

| API | Contrato |
|---|---|
| `scene_project::AdmittedScene::prepare(source, &CancelToken)` (core) | lê (em blocos), importa e roda `admit`; mesmos erros de `scene.attach`; `cancelled` sem cena |
| `SceneSource::Admitted(AdmittedScene)` (core) | `scene.attach` sem reimportar nem medir: só ID, sala, nome e orçamento, com o tamanho/contagens/avisos/famílias adiadas medidos por `prepare` (BL100a; `serialized_len()` somente leitura; nada vem do cliente) |
| `ProjectMeta::{session, stamp}`, `ProjectStamp::is_current` (core) | sessão de edição de runtime e o carimbo `(ID, sessão, revisão)` que um job guarda para aplicar depois (BL113a); `history.status` mostra `project.session` |
| `SceneBook::check_attach(id?, name?)` (core) | o portão de ID/sala/nome do attach, para recusar antes de ler o arquivo |
| `ops::scene_source(request)` (core) | `bundle`/`text`/`path`, o mesmo parser de `scene.attach` |
| `scene.attach.start {path\|bundle\|text, id?, name?, wait?}` (host) | job `Import`; aplica como `scene.attach` só se projeto, sessão e revisão forem os da partida; resposta traz `resources: {files, needs_folder}` |
| `scene_resources::needs_files(scene)` / `ResourceBase::unbound()` (host) | `false` só para manifesto válido sem arquivo; `resolve`/`preflight` dispensam raiz e workspace nesse caso |

Fluxo sem UI (o que um script faz; é exatamente o que a cadeia envia):

```
{"op":"scene.attach.start","path":"bundle.json","id":"street"}            -> {job, resources}
{"op":"scene.resources.adopt","scene":"street","source":"/dir/da/cena"}   -> {job}
{"op":"scene.resources.resolve","scene":"street"}                         -> {job}
```

Num host que pode bloquear (CLI) `wait: true` (padrão) espera o job e aplica; com `wait:false` o job
é aplicado por `job.commit`. Num host interativo (a janela) a resposta é o job e `poll` aplica.
`scene.attach.start` aceita os mesmos `path`/`bundle`/`text`, `id`, `name` de `scene.attach`.

Erros relevantes: `invalid_bundle`, `invalid_json`, `bundle_unreadable`, `bundle_too_large` (job
de attach), `invalid_id`, `duplicate_id`, `invalid_value` (recusa antes do job), `stale_result`
(projeto, sessão ou revisão mudou), `cancelled`;
`resources_invalid` (com `details.problems[]`: `resources`, `path`, `code`, `detail`),
`root_unavailable`, `cancelled` (jobs); `unknown_element` (cena não anexada); `scene_import_busy`, `not_installed` (cadeia).

Em falha de qualquer passo: nenhum arquivo é publicado parcialmente (garantia do host), a cena
anexada permanece, e o mapa instalado só muda quando o resolve conclui e é aplicado.

## CLI headless

Os três pedidos acima rodam por `tessel-media`/`headless::execute_ext` (F040) sem processo desktop;
o teste `crates/media/tests/scene_attach_job.rs` os executa contra o host real. **Gap:** não existe
um subcomando que encadeie os três; a cadeia (e o estado `scene_import`) é da janela.

## UI e CLI da janela

| Gesto | Equivalente semântico |
|---|---|
| Cabeçalho → "Anexar cena" | `./scripts/ui invoke node:26` ou `invoke flow.attach_scene` |
| Preencher um campo | `./scripts/ui set dialog.field.<bundle\|id\|name\|source\|show> value <v>` |
| "Anexar" / "Carregar" | `./scripts/ui invoke dialog.submit` |
| "Cancelar" | `./scripts/ui invoke dialog.cancel` |
| Reparar recursos | `invoke flow.scene_resources`, `set dialog.field.scene/source`, `invoke dialog.submit` |
| Ver andamento/resultado | `./scripts/ui get scene_import` (`state`: `idle`, `running`, `attached`, `loaded`, `failed`), `get jobs` |

Enquanto o diálogo está aberto, `Workflow::has_dialog` tira o WebView do monitor da tela (o WebView
pinta acima do GPUI); `monitor.viewport.visible` reflete isso. O pedido que o diálogo envia é o
mesmo JSON do script (`submitted.action == "scene.import"`, com o `attach` na resposta).

Apresentação: o botão fica no cabeçalho; em janelas estreitas pode apertar os vizinhos (não
inspecionado). Falta um ponto de entrada no estado vazio do `scene_panel` (patch sugerido no relatório).

## Evidências e status

| Superfície | Evidência | Resultado |
|---|---|---|
| Lib (core, BL100a/BL113a) | `./scripts/cargo test -p tessel-editor-core --test scene_attach_facts --test project_session` (5 + 6 testes: fatos do attach = cálculo independente do JSON cru, todas as fontes com a mesma resposta/documento/1 undo, recusas idênticas e atômicas, orçamento exato (4 de ~30 MB cabem, o 5º é `scenes_too_large` com o total exato, estado intacto, contabilidade correta depois), attach de cena preparada < 1/10 do prepare (24 MiB); sessão: construir = nova sessão, clone = mesma, reabrir = mesmo ID/revisão 0 e sessão diferente, sessão fora do documento, estável em edição/undo/redo/salvar/consulta, `project.open`/`create` a trocam, `open` falho a mantém). Medição: `./scripts/cargo test -p tessel-editor-core --test scene_attach_cost -- --ignored --nocapture` (harness, imprime tempos e fingerprints) | 11 passaram; harness: apply 528–537 → 0,0–0,1 ms (debug, 24 MiB), fingerprints idênticos antes/depois (ver BL100a) |
| Lib (core) | `./scripts/cargo test -p tessel-editor-core --test scene_attach_prepared` (5 testes: cena admitida = mesmo attach do caminho, mesmos códigos de recusa pelo job, arquivo acima do limite recusado antes de ler, cancelamento sem cena, regras de ID/sala/nome preservadas) | 5 passaram |
| Lib + headless (host) | `./scripts/cargo test -p tessel-editor-media --test scene_attach_job` (20 testes, +5 de BL113a: reabrir o documento com o job pronto e retido (`job.wait`+`project.open`+`job.commit`) → `stale_result`, `poll` de job que leu depois da reabertura (slots ocupados), projeto de outra sessão com mesmo ID/revisão, fluxo normal com salvar/consultas/undo-redo/`package_preflight`, attach interativo atravessando um save; e em `async_host` o restore pela mesma guarda. Os 3 primeiros falham com a guarda antiga (anexavam ao projeto reaberto): attach como 1 entrada de desfazer, paridade com `scene.attach`, fontes inline/texto, recusas tipadas sem mutação nem job, host interativo só muda no `poll`, falha reportada uma vez, `stale_result` por edição e por outro projeto, cancelamento de job enfileirado, bundle de 8 MiB sem bloquear quem chama, cena sem arquivos resolve sem pasta/sem workspace, cena com arquivos mantém `root_unavailable`/`resources_invalid`/`no_workspace`) | 20 passaram (+1 em `async_host`) |
| **Processo real** (`tessel-media`, JSONL, BL113a) | `./scripts/cargo test -p tessel-editor-media --test scene_attach_process` (executável `CARGO_BIN_EXE_tessel-media --empty`, uma linha por vez, sem sleep: `scene.attach.start wait:false` → `job.wait` → `project.open` do mesmo documento → `job.commit`; 3 testes: `stale_result` com ID e revisão iguais e `project.session` diferente, nenhuma cena nem undo (`nothing_to_undo`), controle positivo na sessão reaberta e segunda reabertura; salvar/consultar/undo-redo sem reabrir aplicam com 1 undo e saída 0; edição continua `stale_result`) | 3 passaram; com a guarda antiga o primeiro falha (`job.commit` respondeu `ok:true`) |
| Desktop (host real) | `cargo test -p tessel-editor scene_import` (`scene_import_tests.rs`, 23 testes: plano, nós, cadeia pura incluindo o passo `attach` (id gerado, `needs_folder`, falhas tipadas), e cenários contra o host real com cópia privada do corpus: arquivo válido, bundle inválido/ilegível pelo job, recurso faltante, fonte alterada, arquivo gerenciado alterado, só anexar, cena sem arquivos carrega sem pasta (com e sem workspace), ocupado/cena desconhecida, sem workspace, nós da árvore e do cabeçalho) | 23 passaram |
| Socket / janela | `python3 scripts/directed-scene-proof/with-gpu-lock.py python3 scripts/scene-import-proof.py` (instância própria, HOME curto, só pedidos semânticos; atualizado para o attach por job) | 35 checks passaram (S5 sessão 1, `scripts/runtime setup` prévio no worktree): diálogo fecha porque o bundle é lido por job, bundle inválido/ausente falha no passo `attach` com código tipado sem cena/histórico/pasta, ID recusado antes do job mantém o diálogo, bundle válido + pasta anexa (1 undo), adota e carrega por jobs, monitor desenha o quadro e o hash coincide com o render explícito, recurso faltante/alterado falha com o código do host, reparo carrega sem novo attach. Relatório anterior em `docs/migration/next-r2-report.md` |
| Socket / janela — manifesto vazio (BL101, S5 aceite independente) | `python3 scripts/directed-scene-proof/with-gpu-lock.py python3 scripts/scene-import-proof.py --scenario empty-manifest [--shots DIR]` (instância própria, HOME curto, só pedidos semânticos; cenário `all` roda `dialog` e `empty-manifest`, cada um com a sua instância) | 24 checks passaram no head `39979cc` (binário `target/debug/tessel-editor`, sha256 `489ca7ce…59f9`): projeto novo **sem pasta nem workspace**, bundle só com grupos (0 arquivos contados do JSON cru) anexado pelo diálogo sem pasta: cadeia `attach → resolve → select` (nunca `adopt`), `resources == {files: 0, needs_folder: false}`, 1 entrada de desfazer, mapa carregado sem `base`, nada criado no HOME; monitor `ready` na cena, quadro desenhado (320×180) com hash igual ao `scene.render` explícito do mesmo quadro; captura do SO da janela própria (observação). Controles negativos com as mesmas guardas: bundle com 3 arquivos sem pasta fica `attached`, `scene.resources.resolve` e o diálogo de reparo recusam `no_workspace` (diálogo permanece aberto), e num projeto salvo sem `scenes/street` o job falha `root_unavailable` sem carregar nem criar pasta. Bundle de 24 MiB: `dialog.submit` volta em ~1 ms e a janela continua respondendo (ver BL100a) |
| Socket / janela — head `f9898a0` (S5 sessão 2, BL100a) | `python3 scripts/directed-scene-proof/with-gpu-lock.py python3 scripts/scene-import-proof.py --binary target/debug/tessel-editor --scenario all` (binário debug sha256 `1a9f9e1e…d8`, instância própria, HOME curto, só pedidos semânticos; relatório fora do repositório) | **59 checks passaram** (35 `dialog` + 24 `empty-manifest`, repetidos): caso vazio (projeto nunca salvo, 0 arquivos, sem workspace/pasta: attach/resolve/select sem adopt, `ready` 320×180, hash igual ao `scene.render`) e negativos (3 arquivos sem pasta `no_workspace`; salvo sem `scenes/street` `root_unavailable`, sem criar raiz). Bundle de 24 MiB: `submit_ms` 0,8, 125 polls, `max_poll_ms` 29,2, `stalls_over_100ms` **vazio** (o check agora exige `submit_ms < 100` e nenhuma parada > 100 ms; antes `< 1000` escondia a parada de ~0,5 s); referência `scene.attach` síncrono 660 ms |

Não provado: o painel nativo de arquivos (precisa de humano), ponteiro físico e aparência. O monitor real
carregando uma cena **sem arquivos** foi provado no cenário `empty-manifest`; a cena só tem grupos, então
o quadro é o fundo vazio (a prova é monitor `ready` + hash igual ao render explícito, não uma imagem).
`stale_result` e cancelamento por `job.cancel` não têm prova na janela real (só host/lib; a corrida não é
determinística por socket); o cancelamento durante o parse de um bundle de 32 MiB não foi medido (BL114).
BL100a (resolvido, a validar): antes, o *apply* deixava um travamento único de ~0,5 s na janela com
24 MiB (binário debug); o perfil mostrou que ~98% era serializar o documento para contar bytes. Agora
`prepare` mede tudo no job e o apply é 0,0–0,1 ms no core; a janela real não tem parada > 100 ms.
Não provado na janela: bundle com dezenas de milhares de nós (só medido no core: debug 3,4 s de
`prepare`, apply 0) nem a reabertura do documento com um attach em voo (provada na lib, no host e
no processo `tessel-media`; a janela usa o mesmo host e o mesmo core).
Diálogos Kit exigem `Root`, que a janela de teste não tem: o corpo do diálogo é provado pelo script
vivo, não pelo teste unitário.
