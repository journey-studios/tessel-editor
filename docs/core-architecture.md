# Core independente dos adapters

## Contrato

O core monta e controla o estado do editor e executa as regras de domínio sem janela,
GPUI, socket ou event loop gráfico. UI e CLI são adapters: traduzem intenção em comandos,
chamam a mesma biblioteca e apresentam resultados. Um fluxo de edição deve poder ser
executado integralmente por um programa Rust ou processo headless.

```text
UI GPUI ───────────┐
CLI da janela ────┤
CLI headless ─────┼──> tessel-editor-core ──> estado / regras / resultados / erros
Programa Rust ────┘
```

`crates/core` é uma biblioteca pública local (`tessel_editor_core`) e tem somente
`serde_json` como dependência direta. `crates/desktop` depende dela, nunca o contrário.
`state::EditorState`, `Command`, `CommandError` e `Raw` são reexportados na raiz.
`commands` traduz intenções semânticas; `headless::execute` é a entrada JSON compartilhada
com o executável `tessel-headless`. As regras ficam em `EditorState::execute` e nos modelos.
O desktop mantém `use tessel_editor_core::state` como alias de compatibilidade, sem cópia do estado.

## Usar como biblioteca

```rust
use tessel_editor_core::{Command, EditorState, Raw};

let mut editor = EditorState::default(); // sessão demonstrativa em memória
editor.execute(Command::SelectClip { clip: "atlantic-coast".into() }).unwrap();
editor.execute(Command::SetPlayhead { frame: Raw::Number(120.0) }).unwrap();
editor.execute(Command::SetProperty {
    property: "zoom".into(), value: Raw::Number(150.0),
}).unwrap();
let snapshot = editor.summary();
assert_eq!(snapshot["playhead"]["frame"], 120);
```

O chamador é dono da sessão. Não existe singleton gráfico nem processo desktop obrigatório.
Para adicionar a lib em outro crate local, use `tessel-editor-core = { path = "../core" }`,
ajustando o caminho. A API ainda é 0.1 e não tem compromisso de estabilidade pública.

### Identidade da sessão de edição (BL113a)

`ProjectMeta::id` é persistido e `revision` volta a 0 a cada abertura, então `(id, revision)` não
distingue um projeto do documento reaberto. `ProjectMeta::session` (`ProjectSession`) é identidade
só de runtime: nasce quando um `EditorState` é *construído* (`demo`, `empty`, carregar ou recuperar
um documento, restaurar um pacote) e não muda ao salvar, editar, desfazer, refazer, consultar nem
clonar (snapshots de undo e planos de job continuam na mesma sessão). Não entra no documento,
não vale em outro processo e `==` a ignora (compare com `is_same`). Os jobs `scene.attach.start` e de restauração capturam `ProjectMeta::stamp()` (ID, sessão,
revisão) e aplicam só se `ProjectStamp::is_current` ainda vale. Essa guarda não cobre todos os
jobs: `media.prepare` confere ID do projeto e hash do asset, podendo aplicar após outra edição.
`history.status` expõe `project.session` para scripts e provas.

## Usar pela CLI sem UI

```sh
./scripts/headless <<'JSONL'
{"op":"invoke","target":"clip:atlantic-coast","action":"select"}
{"op":"set","target":"timeline.playhead","field":"frame","value":120}
{"op":"set","target":"property:zoom","field":"value","value":150}
{"op":"status"}
JSONL
```

Um JSON por linha, uma resposta por linha. As operações são `set`, `invoke`, `move` e
`status`. `set` recebe `target`, `field`, `value` escalar; `invoke` recebe `target`, `action`;
`move` recebe `target`, `args` objeto. IDs e regras são os mesmos da CLI desktop.
`status` retorna seleção, playhead, tracks e clipes; consultas detalhadas de propriedades
estão disponíveis pela API Rust. A CLI headless ainda não expõe `tree/get/actions`.

Sucesso: `{ "ok": true, "version": 1, "result": ... }`. Falha:
`{ "ok": false, "version": 1, "error": { "code": ..., "message": ... } }`.
O processo continua após um comando inválido e sai com código 1 se alguma linha falhar;
JSON inválido retorna `invalid_json`. A sessão dura até EOF e não é persistida. Não há
transação de lote: comandos anteriores válidos continuam aplicados após uma falha posterior.
Erros de I/O encerram o processo e são escritos em stderr. Não há paralelismo implícito.

`./scripts/ui` continua controlando a janela aberta por socket. Não é headless. Ambos
os acessos usam o parser em `core::commands` e as regras em `EditorState::execute`.

## Estado de apresentação

O contrato de render está em [F021 — Viewport multimídia](features/viewport.md). O core
resolve conteúdo, tempo racional e câmera num `RenderPlan` (`EditorState::plan`) e fala com
backends **injetados** (`RenderBackend`, `Transport`, bridge JSON v1, `ViewportSession` para
revisão/cancelamento/resposta velha). O crate `crates/render` fornece o transporte real:
worker Node + Chrome headless (CLI/lib, `./scripts/render`) e o servidor de assets da WebView
(`host.rs`). O desktop usa o mesmo plano e a mesma sessão com a WebView persistente do monitor.
Three.js roda em `runtime/` (JS, bundle local); o core não depende de GPUI, Wry nem DOM.
Áudio: `plan_audio` + `OfflineMixer` produzem o mix processado (clipes de tracks de áudio **e** o som embutido dos
clipes de vídeo, por `audio_muted`/`audio_stream`); o export grava esse mix e o desktop o entrega ao `crates/audio`
(`tessel-editor-audio`, `cpal`/CoreAudio), cujo **relógio de amostras** conduz o playhead ([F018](features/playback.md)).
`crates/audio` é o único crate com dependência de dispositivo e o core não o conhece. Áudio *ouvido* não é provado.

Mídia pesada nunca bloqueia a UI: `MediaHost` devolve um job e o desktop aplica o resultado no tick, uma transação por
job, descartando resultados obsoletos ([F025](features/jobs.md)); a CLI sem janela espera e aplica no mesmo caminho.

## Projeto, mídia e export

O core guarda o documento canônico (`document.rs`, `persist.rs`), as sequências, o histórico por snapshot
(`state/history.rs`), os presets (`preset.rs`), o merge entre projetos (`interchange.rs`) e o `JobRegistry`
(`jobs.rs`). Tudo que toca FFmpeg, hashes, ZIP64 e arquivos grandes é do crate `tessel-editor-media`
(`MediaHost`), que entra pelo hook `Extension` de `headless::run_session_ext` e serve a mesma operação a
`./scripts/media`, `./scripts/render` e `ui op`. O desktop usa o `MediaHost` em modo `interactive()`
(nada bloqueia a UI; `poll()` aplica os jobs concluídos). Contratos: F019 e F022–F030 em `features/`.

DockArea continua dono do layout vivo do desktop. O core contém tipos e validação pura
do layout, mas não simula uma janela headless. Tema, foco e operações de janela pertencem
ao desktop; solicitações headless a esses alvos retornam `desktop_required`. Seleções,
tabs e accordions atuais também são estado de sessão, sem processamento de mídia.

## Limites e evolução obrigatória

Entregue com a fundação da viewport: IDs próprios (`Id`, internado e `Copy`; o vazamento é
limitado pelo número de IDs *distintos* criados no processo), `EditorState::empty`, assets
tipados, sequência/FPS configuráveis em `state.timeline` e o demo como bootstrap explícito
(`EditorState::demo()`, `--demo`). Campos públicos mutáveis herdados do protótipo ainda
existem: consumidores devem mutar por `execute`; o compilador não impede todos os acessos
diretos. Ver [F015](features/project-creation.md).

Para funcionalidades futuras, o core deve montar projetos, tracks, fontes, efeitos e
sequências a partir de parâmetros do chamador. Seleção da UI não pode ser requisito de
operações de batch: criar comandos com IDs explícitos e manter seleção como conveniência
do adapter. Efeitos e split/corte interno permanecem pendentes; importação, playback, persistência e exportação
estão entregues (ver o [relatório](media-project-export-report.md) para os limites). Jobs futuros precisam de progresso,
cancelamento e erros expostos pela mesma API, sem depender de renderização da janela.

## Acesso com renderer

`./scripts/render` é `tessel-headless` com um `Renderer` real: aceita `render.frame` e
`render.audio` além do vocabulário de projeto (`ops::parse_op`). `Parsed::ReloadViewport`
(`monitor.reload`) é de janela e responde `desktop_required` fora dela. Sem Node/Chrome/bundle
a resposta é `backend_unavailable` com o comando de setup (`./scripts/runtime setup`).

## Evidências

`crates/core/tests/headless.rs` compara estado de execução direta e JSON, verifica rejeição
sem mutação, executa o binário como processo filho, confirma estado entre linhas e erros.
Testes de domínio anteriores foram movidos com o código. O workspace mantém testes de
input GPUI e Dock no desktop. Confira os resultados da entrega; não confunda teste da
plataforma GPUI com arraste manual na janela nativa.

## Cena dirigida: integração M/N/O/P

`SceneCommand::Plan` consulta o plano completo ou diagnóstico parcial, aplicando admissão estrutural/temporal e usando opcionalmente um candidato de gesto vivo. `scene_operation` (render) reutiliza esse comando e `headless::output_geometry`; não duplica domínio.
`SceneHost` reúne `MediaHost` e `SceneRenderer` na sessão JSONL de `tessel-render`: `scene.render` consome apenas recursos aprovados por F040. O desktop usa o mesmo SceneSlots/frame_render_message de forma assíncrona, confirma loads e coalesce seek, sem esperar GPU na thread gráfica. `monitor.show_scene` é escolha de apresentação local; documento e histórico continuam no core.
A cena dirigida ainda não entra no export comum da timeline; BL008 permanece pendente. Contrato [F041](features/scene-renderer.md).
