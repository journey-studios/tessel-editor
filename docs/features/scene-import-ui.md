# F043 — Anexar uma cena dirigida pela interface (BL086)

## Status

Lib **V no corpus** (regras do núcleo e do host, testadas com arquivos sintéticos). CLI headless
**V** (a cadeia usa as mesmas operações que um script envia; nada novo no `MediaHost` nem no core).
UI **Parcial**: a janela abre o diálogo, valida só quando o usuário confirma, aplica os jobs e mostra
a cena; o **painel de arquivos do macOS não é provável por script** e a aparência não foi inspecionada
(ver "Evidências e status").

Esta feature **não cria regra de domínio**. `scene.attach` (F036/F039), `scene.resources.adopt` e
`scene.resources.resolve` (F040) continuam sendo a única implementação de formato, contenção, hash,
limite, cancelamento e instalação. O que existe aqui é a cadeia que as encadeia a partir de um
diálogo, e a fiação semântica que torna cada campo e ação alcançável sem ponteiro.

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
attach (na hora, 1 entrada de desfazer) → adopt (job, só se há pasta) → resolve (job) → select → show
```

- **Nada de domínio no clique.** O botão só monta o pedido (`flows::build_request`) e chama a
  operação. Bundle inválido, JSON ilegível ou arquivo inexistente voltam como o erro tipado do core
  (`invalid_bundle`, `invalid_json`, `bundle_unreadable`) **com o diálogo aberto**; nada é criado.
- **Carga de recursos fora da UI.** `adopt`/`resolve` são jobs do host; leitura/parse do attach continuam síncronos (BL100); a UI só inicia e, no tick de
  250 ms (`workflow_tick`), aplica o que terminou (`host.poll`). Hash e cópia ficam no job.
- **Uma cadeia por vez.** Outra submissão enquanto uma roda é recusada com `scene_import_busy`.
- **Falha não desfaz o que já valeu.** Se `attach` passou e `adopt` falha, a cena **continua
  anexada** (é uma entrada de desfazer válida), nada foi copiado e nada foi carregado; o diálogo já
  fechou, então o erro vive no nó `scene_import` e na barra de estado. O reparo é `flow.scene_resources`.
- **O monitor só mostra o que carregou.** `show` só roda com o mapa instalado
  (`applied.installed == true`); sem isso o passo falha com `not_installed`.
- O painel nativo de arquivos é **somente um seletor de caminho** (`Picker::File`/`Picker::Directory`);
  digitar o caminho no campo é equivalente.

Limites declarados:

- `scene.attach` lê o arquivo do bundle na thread que chama (limite de 32 MiB por texto de bundle; 128 MiB é o orçamento agregado de cenas). Não há
  attach+carga atômico: entre os dois a cena existe sem recursos. Um attach assíncrono pede mudança
  no core (fora desta frente).
- Uma cena só com dados (sem pasta) não carrega sozinha: `scene.resources.resolve` sem base devolve
  `root_unavailable` (nada gerenciado ainda). Sem pasta, o diálogo só anexa.
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

Fluxo sem UI (o que um script faz; é exatamente o que a cadeia envia):

```
{"op":"scene.attach","path":"bundle.json","id":"street"}
{"op":"scene.resources.adopt","scene":"street","source":"/dir/da/cena"}   -> {job}
{"op":"scene.resources.resolve","scene":"street"}                         -> {job}
```

Erros relevantes: `invalid_bundle`, `invalid_json`, `bundle_unreadable` (attach, síncronos);
`resources_invalid` (com `details.problems[]`: `resources`, `path`, `code`, `detail`),
`root_unavailable`, `cancelled` (jobs); `unknown_element` (cena não anexada); `scene_import_busy`, `not_installed` (cadeia).

Em falha de qualquer passo: nenhum arquivo é publicado parcialmente (garantia do host), a cena
anexada permanece, e o mapa instalado só muda quando o resolve conclui e é aplicado.

## CLI headless

Os três pedidos acima rodam por `tessel-media`/`headless::execute_ext` (F040) sem processo desktop.
**Gap:** não existe um subcomando que encadeie os três; a cadeia (e o estado `scene_import`) é da
janela.

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
| Lib | `cargo test -p tessel-editor scene_import` (`scene_import_tests.rs`, 18 testes: plano, nós, cadeia pura, falha/cancelamento/`not_installed`, e cenários contra o host real com cópia privada do corpus: arquivo válido, bundle inválido, recurso faltante, fonte alterada, arquivo gerenciado alterado, só anexar, ocupado/cena desconhecida, sem workspace, nós da árvore e do cabeçalho) | 18 passaram |
| Socket / janela | `python3 scripts/directed-scene-proof/with-gpu-lock.py python3 scripts/scene-import-proof.py` (instância própria, HOME curto, só pedidos semânticos) | ver relatório `docs/migration/next-r2-report.md` |

Não provado: o painel nativo de arquivos (precisa de humano), ponteiro físico, aparência.
Diálogos Kit exigem `Root`, que a janela de teste não tem: o corpo do diálogo é provado pelo script
vivo, não pelo teste unitário.
