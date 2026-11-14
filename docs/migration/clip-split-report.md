# Dividir clipe (F017 / BL081) — relatório de entrega

Contrato e limites por superfície: [split.md](../features/split.md). Este relatório registra o que foi executado, onde e com
que resultado. Código integrado localmente em `c0f7e24`, validado Windows/Mac; sem push, PR ou merge.
As seções da entrega Windows original abaixo são históricas; o fechamento da revisão está ao final.

## Base e ambiente

- Windows 11 (10.0.26200), PowerShell, Rust 1.99.1, `./scripts/cargo` (wrapper). Base `ead9ca2` com os patches de preparação
  B1 (testes de `media` no Windows) e B2 (descoberta de FFmpeg/Node/Chrome no Windows) já aplicados; o BL081 não toca em
  nenhum arquivo deles (manifesto de 18 arquivos conferido: 0 divergências antes e depois).
- Logs duráveis (fora do repositório): `C:\Users\wally\tessel-readiness-logs\2026-11-13-bl081\` — cada `*.log` registra
  base, comando e `exit=`.
- Sem lock de GPU (nenhuma medição de desempenho foi feita nem declarada).

## O que mudou

| Camada | Arquivo | Mudança |
|---|---|---|
| core | `state/split.rs` (novo) | `plan_split`, `split_clip`, `split_availability`, `SplitBlock` |
| core | `state/mod.rs` | `Command::SplitClip { clip, frame, new_id }`, rótulo de histórico "Split clip" |
| core | `ops.rs` | op `clip.split` |
| core | `commands.rs` | `invoke timeline.split` (recusa com `disabled_element`, pois o headless não checa `enabled`) |
| core | `state/tree.rs` | nó `timeline.split` no lugar do placeholder `timeline.tool.razor` |
| core | `id.rs` | `Id::validate` (valida sem internar; `Id::new` o usa) |
| core | `state/inspector.rs` | `duplicate_overrides`, `assign_exact` (slider sem arredondar) |
| desktop | `workstation/timeline_panel.rs` | botão do Kit `timeline-split` (`IconName::Slice`) com `disabled_reason` |
| desktop | `workstation/mod.rs`, `split_tests.rs` | testes GPUI |
| docs | `features/split.md`, linha F017 de `features/README.md`, este relatório | |

Sem dependência nova e sem mudança em `Cargo.lock`. `cargo tree -p tessel-editor-core -e normal` não lista gpui nem wry
(`tree-core.log`).

## Evidências executadas

| Superfície | Prova | Resultado |
|---|---|---|
| Lib | `crates/core/tests/clip_split.rs`, 20 testes: corte no meio (cobertura exata, plano de render idêntico), metades de 1 frame, 30000/1001 com `source_start` e velocidade ≠ 1 exatos, texto/propriedades, câmera orbital e com chaves, Ken Burns, fades de imagem e de áudio (aceitos e recusados), IDs padrão e explícito, independência da seleção, recusas tipadas atômicas, estouro, undo/redo (uma entrada), salvar/reabrir | 20/20 (`test-split-2.log`) |
| Lib (unidade) | `id::validate_answers_like_new_without_interning`, `inspector::a_clip_can_be_duplicated…`, `tree::the_cut_control_is_enabled_exactly_where_invoke_would_split` | passam; lib do core 577/0 (`test-core-all.log`) |
| Headless | `the_headless_process_cuts_undoes_and_reports_refusals`: processo `tessel-headless` real, JSONL: corta, desfaz, recusa | passa |
| UI (GPUI) | `split_tests.rs`: botão, nó e `invoke` concordam; clique simulado corta como uma edição desfazível; `invoke` e op fazem o mesmo corte e a recusa não muda nada | 3/3 (`test-desktop-split-2.log`) |
| Janela real | binário de debug próprio (PID 60948), só `tessel-editor ui …`, sem coordenadas, fechado por `window.close` (`ui-1…ui-4*.log`) | ver abaixo |
| Mutação | quebrar a regra de `source_start` faz 6 testes do `clip_split` falharem (`mutation-1.log`) | testes não são vácuos |

### Janela real (`ui-3.log`)

Clipe `basalt-cliffs` (24 fps, 504..696). Com o playhead em 86 (fora do clipe selecionado): `timeline.split` `enabled:false` com
`disabled_reason`; `invoke` no frame 504 → `disabled_element` (exit 1). Playhead em 600: `enabled:true` com `target`/`frame`;
`invoke` → esquerda 504..600 (96), direita `clip:basalt-cliffs-split-1` 600..696 (96), `source_start` 4 s, direita selecionada,
Ken Burns ajustado (`end_zoom` 106→102,984…, `zoom` 100→103,0157…). `history.status`: uma entrada "Split clip", revisão 1.
`history.undo`: `basalt-cliffs` volta a 192 frames, o clipe da direita deixa de existir; `history.redo` o devolve.
`op clip.split` de `title-north` no frame 10 com `new_id` `title-north-b`: direita 10..120, `source_start` 5/12 s.
Recusas: `invalid_duration` (frame 504 na borda) e `unknown_element`; ao final `undo_depth 2`, `revision 4`, ou seja, as recusas
não consumiram revisão nem histórico. `window.close` encerrou o processo; `ui status` seguinte: conexão recusada; stderr vazio.

## Validações do workspace

| Comando | Resultado |
|---|---|
| `cargo fmt --all -- --check` | 0 |
| `./scripts/cargo clippy --workspace --all-targets --locked -- -D warnings` | 0 (`clippy-1.log`; B1 já aplicado) |
| `./scripts/cargo test -p tessel-editor-core --locked --no-fail-fast` | só falham as **2 falhas já existentes** de ponto flutuante no Windows: `scene_gizmo_rotation::the_rotate_overlay_has_three_closed_rings…` e `scene_gizmo_scale::four_parents_of_anisotropic_scale…`; 57 alvos `ok` (`test-core-all.log`) |
| `./scripts/cargo build -p tessel-editor --locked` | 0 |
| `./scripts/cargo tree -p tessel-editor-core --locked -e normal` | sem gpui/wry |
| `./scripts/cargo test --workspace` | **não executado** por inteiro (instrução: dois testes de ponto flutuante já falham no Windows e testes de mídia podem passar de 30 min). Rodados: core inteiro, `desktop split_tests`; os testes de `media`/`render` não são tocados pelo BL081 |

## Ambiente: o executável de debug trazia um caminho `W:\`

O primeiro `tessel-editor.exe` falhou ao abrir ("DirectXRenderer … caminho especificado", `win-err*.log`): em debug, o
`gpui-pre-windows` compila o HLSL em tempo de execução a partir de `CARGO_MANIFEST_DIR`, e o artefato reaproveitado do cache
compartilhado tinha `W:\www\tessel-editor\vendor\…` embutido (drive que não tem este checkout). Contornado sem mudar código: o
`LastWriteTime` de `vendor/gpui-pre-windows/build.rs` foi atualizado para recompilar a caixa com o caminho `E:\`; o
`git status` de `vendor/` permanece limpo. Registrado como achado (abaixo), não como parte do BL081.

## Limites

- Só a sequência ativa; sem corte em lote, atalho de teclado ou clique na timeline.
- Fade cruzando o corte é recusado (`split_unsupported`), não aproximado; Ken Burns e órbita usam `f64` (tempo de fonte é exato).
- Seleção não é restaurada por undo/redo (é estado de sessão, como nas demais edições).
- Sem ponteiro físico, sem áudio ouvido, sem pixels. Prova de janela é semântica.
- Sem prova em macOS/Linux; sem medição de desempenho.
- O WebView/monitor da janela não foi inspecionado visualmente neste corte.

## Achados registrados no backlog comum

Os achados da entrega original foram integrados a `docs/backlog.md` como BL081.1–.4;
a revisão acrescentou BL081.5/.6. O backlog é a autoridade de status/atribuição.
O vocabulário de `clip.split` em F015 já foi corrigido pela coordenação (BL081.3).

`docs/ui-automation.md:174` (`timeline.tool.*` desabilitados) continua correto: só o nó `timeline.tool.razor` foi substituído por
`timeline.split` (habilitado quando há corte possível); `timeline.tool.select` e `timeline.tool.delete` seguem desabilitados.

## Recebimento e revisão da coordenação (Mac, 2026-11-14)

Patch original96710bytes, SHA256
5aff608f5b8e56cb8ba2d5f7b03ca1142590d50af293dd097994ab7fbc87088d,
recuperado em três partes gzip/base64 e preservado no SSD. Aplicação sobre65ea9a4:
somente o contexto da matriz divergente (provas B2) exigiu integrar a linha F017
separadamente. Nenhuma linha F019/F021/F029 foi substituída pelo texto Windows.

Entrega original passou no Mac: fmt/check/clippy workspacealltargets/no-run/build,
core completo e desktop split3/3. Core continua sem GPUI/Wry. Provas finais do código
corrigido são distintas e ficam abaixo; a rodada original não valida alterações novas.

Dois controles de revisão falharam (20 antigos passam,2 casos novos falham):
- BL081.6: uma recusa encerrava o gesto de slider aberto. O comando agora valida
  o corte antes de liquidar o gesto; a recusa deixa também o gesto/histórico intactos.
- BL081.5: áudio a30000/1001fps, movido ao frame1 e cortado no2, muda o PCM em
  0.020370752 por reiniciar um segmento numa amostra inteira diferente. Tempo racional
  de fonte sozinho não preserva a fase do mixer. Até o modelo representar essa fase,
  áudio inclusive embutido em vídeo exige amostras inteiras por quadro na saída e na
  fonte (considerando velocidade). Caso contrário, split_unsupported/audio_sample_grid;
  o botão também informa o motivo. Esse suporte amplo permanece pendente no backlog.

Teste inicial com speed em clipe Audio foi uma tentativa inválida (not_applicable),
sem diagnóstico de produto; o controle válido usa o clipe deslocado, sem speed.
Logs de todas as tentativas estão preservados. Correções de produção limitadas ao
core de corte/execute e ao motivo do botão; nenhum adapter media/render/audio alterado.
Alvo clip_split corrigido22/22. Vocabulário de ops F015 atualizado (BL081.3).
Evidências locais: /Volumes/Extreme Pro/tessel-bl081-coordination/mac/ e mac-final/,
com comandos, exitcodes e hashes dos logs em results.jsonl.

### Resultado final Mac após revisão

| Check | Resultado |
|---|---|
| fmt/check/clippy workspacealltargets -D warnings | exit0 |
| core completo (inclui22 clip_split e processo headless real) |1435 passam;7 ignorados existentes;0 falhas,59 resultados de alvos incluindo doctests |
| desktop split_tests |3/3 |
| workspace test --no-run; build desktop |exit0 |
| cargo tree core |sem GPUI/Wry |
| spec do backlog |19/19 |
| janela Mac própria, HOME/socket isolados |11 requisições semânticas, corte/undo/redo/recusa passam; window.close e processo exit0 |

Janela final: PID92936; logs e hash do executável em
`mac-final/ui-proof-r3/native-proof-result.json` e `ui-requests-results.json`.
Duas tentativas anteriores do harness interromperam nas asserções de formato JSON
(`data` em vez de `result`, `history.undo_depth` aninhado); ambas fecharam somente
sua própria janela, logs preservados em `mac-final/` e `mac-final/ui-proof/`.
Não foram falhas de produto nem tentativas descartadas de desempenho. Não se
promove essa prova a ponteiro físico, imagem/pixel ou áudio ouvido.

Manifesto `mac-final/code-manifest.json` identifica as fontes BL081; todas18fontes
B1/B2 continuam intactas. Patch de revisão8327bytes, SHA256
bfc5ba52f361a4546305f71bfcdca463e2783eacd16aeed2f759501baca35cc6.
Uma nova [rechecagem Windows das correções](t3-thread://v2/33595e42-547d-42b7-92a7-cb4acb430dd7/mcp:launch:17b83891c96d66a070df0b763d6f78dc8532b75ce0657ccc492972d16d845f01)
foi iniciada por delegate_task, Cursor Sonnet5.5, somente4paths e gates/testes
dirigidos. O retorno concluído está na tabela de fechamento abaixo; os gates foram refeitos
no patch corrigido, sem herdar a prova Windows original. Sem push/PR/merge.

## Fechamento da revisão Windows/Mac (2026-11-14)

A rechecagem Windows final concluiu com os quatro hashes iguais ao manifesto da
correção integrada em `c0f7e24`. Patch8327bytes SHA256
bfc5ba52f361a4546305f71bfcdca463e2783eacd16aeed2f759501baca35cc6,
git apply --check/apply exit0. Revisão focada não encontrou objeção bloqueante.

| Check Windows no código corrigido | Resultado |
|---|---|
| fmt/check workspace/clippy alltargets -D warnings | exit0 |
| workspace test --no-run; build desktop | exit0 |
| clip_split (inclui processo headless e regressões novas) |22/22 |
| desktop split_tests |3/3 |
| janela própria, comandos semânticos |corte/undo/redo/recusa passaram; fechada por window.close |
| integridade |18fontes B1/B2 idênticas; readiness e índice real intactos; HEAD ead9ca2 inalterado |

Logs: `C:\Users\wally\tessel-readiness-logs\2026-11-14-bl081-review\`
(`patch4.diff`, `gates.log`, `clip-split.log`, `desktop-split.log`, `ui.log`,
`four-final.txt`, `manifest-*.log`, `index.before`). Sem execução do workspace
completo, core inteiro ou desempenho nesta rechecagem. As falhas Windows de
claims e de gizmos continuam nos respectivos IDs, sem promoção da suíte a verde.
O novo disabled_reason de áudio foi verificado pelo core, não pela janela Windows.

Limite conservador de BL081.5: a guarda olha a grade por quadro, não só a posição
de um corte. Em30000/1001 recusa também múltiplos de5. A100% de velocidade,
44,1kHz/24fps recusa (1837,5 amostras/quadro);44,1kHz/25,30,60fps passa.
Flags de mute e ganho não isentam um stream da guarda. Isso evita mudar conteúdo
caso o som seja reativado, mas restringe cortes que poderiam ser possíveis com
um modelo de fase fracionária. O contrato explicita a restrição; expansão segue
no backlog, sem ampliar o mixer nesta fatia.

BL081 validado localmente nas superfícies previstas dentro desses limites.
Código `c0f7e24` preservado; esta atualização altera apenas docs/status.
Sem push, PR, merge, ponteiro físico, pixels, áudio ouvido ou desempenho aceitos.
As duas tarefas Windows estão concluídas, sem executor continuando em segundo plano.
