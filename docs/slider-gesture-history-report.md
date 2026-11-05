# BL062 — Slider = um gesto = uma entrada de undo

Estado atual: **implementado e validado nos gates direcionados pelo coordenador**.
A frente L entregou sem rodar Cargo; o coordenador compilou, corrigiu o harness e executou
28 testes core + 13 testes de input GPUI e o executável headless real. O histórico abaixo
registra a entrega original; a evidência atual está na seção de integração ao final.

## Causa

`Workstation` assinava `SliderEvent::Change` do slider do Kit e despachava `Command::SetProperty` a cada valor.
`EditorState::execute` tira um snapshot e grava uma entrada por comando que muda algo, então um arrasto de N
valores deixava N entradas de undo (e apagava o redo no primeiro). O teclado (setas no wrapper do inspetor)
tinha o mesmo defeito, uma entrada por tecla/repetição.

## API real do Kit usada (gpui-base 0.9.0, `slider.rs`)

- `SliderEvent::Change(SliderValue)` a cada posição (clique na trilha, arrasto da trilha ou do thumb).
- `SliderEvent::Release(SliderValue)` no mouse-up dentro **ou fora** do slider, só depois de um Change real
  (`dragging`). Um thumb pressionado e solto sem mover não emite nada.
- **Não há** evento de início nem de cancelamento, **nem teclado** (o wrapper do inspetor faz o passo) e as ações
  de acessibilidade Aumentar/Diminuir chamam `set_value`, que **não emite evento**.
- O thumb engole o mouse-down (`stop_propagation`): só a fase de captura vê o press.

Sem timer, debounce ou vendor: o fim do gesto é o `Release` (ou key-up, Escape, perda de foco).

## Desenho

Agrupamento no **core** (`crates/core/src/state/property_gesture.rs`), não no desktop:
`Command::PropertyGesture(Begin|Update|Commit|Cancel)`, o mesmo para UI, CLI (`property.gesture.*`) e lib.
O estado aberto mora em `History` (`gesture`, `next_gesture`; `history.rs` é exclusivo deste trabalho).

| Situação | Política |
|---|---|
| Preview | `update` aplica na hora com a validação/clamp/erros de `SetProperty`; não grava, não apaga o redo; move a revisão do projeto (dirty/autosave/jobs veem a mudança) |
| Release | `commit`: **uma** entrada do original ao final. Sem entrada se não mudou ou voltou ao original |
| Escape | `cancel` restaura o original, sem entrada; o adapter ignora o resto do press (`Ended`) |
| Clique na trilha | begin + update + commit = uma entrada |
| Teclado | key-down novo começa o gesto; auto-repeat (`is_held`) continua; key-up da tecla de passo confirma. Soltar Shift não encerra |
| Perda de foco / MouseUp perdido (BL043) | desativação da janela confirma e ignora o resto do press; o próximo press confirma o que ficou pendente |
| Seleção muda no meio do arrasto | o gesto permanece no clipe do `begin`: o valor nunca vai para outro clipe |
| Outra edição / undo / redo / lote / troca de sequência / novo begin | **confirma** o gesto antes de rodar (o preview não atravessa outro commit) |
| Documento mudou por baixo (revisão, sequência, clipe) | `update`/`commit` rejeitam com `stale_gesture` (`document_changed`/`target_changed`) e revertem o próprio preview |
| Valores depois que o core encerrou o gesto | descartados (`no_gesture` → `Ended`); não iniciam gesto novo |
| `SetProperty` discreto (lib/CLI) | continua **uma entrada por chamada** |
| Dentro de `transaction` | o commit não grava; o lote é a entrada |
| `clear_history` / abrir projeto | encerra o gesto; ids continuam crescendo (sem colisão entre sessões) |
| Zoom da timeline | apresentação: nunca entra no histórico e não encerra o gesto |
| Gesto de cena (`scene.gesture.*`, F039) | ver abaixo |

### Coexistência com `Command::Scene(Gesture…)`

O preview de propriedade **está no documento**; o preview de cena **não** (é estado de sessão). Daí:

1. Qualquer comando com entrada de histórico, incluindo `scene.attach|detach|edit` e o `scene.gesture.commit`
   (rótulo "Edit scene"), **confirma** antes o gesto de propriedade. Begin/update/cancel/status de cena não têm
   rótulo e deixam o slider aberto (são leituras do documento).
2. Um `update` ou `commit` de propriedade move a revisão do projeto; o gesto de cena aberto vira
   `gesture_invalidated` (`project_revision`), conservador de propósito (contrato de F039). Nenhum preview antigo
   atravessa o commit do outro.

Escritos (não executados): `slider_gesture_a_property_preview_invalidates_an_open_scene_gesture` e
`slider_gesture_a_valid_scene_commit_never_swallows_or_reorders_the_property_entry`.

## Arquivos

Core: `state/property_gesture.rs` (novo), `state/history.rs`, `state/mod.rs` (enum `Command`, `history_label`
via `settles_property_gesture`, `execute`/`apply`; edições pontuais), `state/tree.rs` (`gesture_open` no nó do
slider), `ops.rs` (arm `property.gesture.*`, liberado pelo coordenador; `scene_command` intacto).
Desktop: `workstation/slider_gesture.rs` (adapter), `workstation/mod.rs` (assinatura, Escape, ativação da
janela), `workstation/inspector_panel.rs` (press em captura, teclas com gesto, key-up).
Docs: `docs/features/properties.md`, `docs/features/history.md`, `docs/features/README.md`, este relatório.
`core/lib.rs` **não** precisa de export: `state` já é público e `property_gesture` é `pub mod`.

## Testes: escritos × executados

**Nenhum foi executado** (sem Cargo). Escritos:

- `crates/core/tests/slider_gesture.rs`, 28 testes `slider_gesture_*`: baseline (N valores discretos → N
  entradas), N updates → 1 entrada, preview sem histórico, redo preservado até o commit, cancel, release sem
  mudança, voltar ao original, validação de `SetProperty` preservada, begin rejeitado, seleção que muda, outra
  edição/undo/redo/lote/sequência liquidam, novo begin, erros tipados, documento mudado (stale), alvo sumido,
  `clear_history` e ids, todos os sliders de `PROPERTIES`, sets discretos, zoom da timeline, JSON `property.gesture.*`
  pela mesma execução da lib, coexistência com cena. Mais 2 testes de `parse_op` em `property_gesture.rs`.
- `crates/desktop/src/workstation/slider_gesture_tests.rs`, 13 testes de **entrada GPUI real**
  (`simulate_mouse_*`, `simulate_keystrokes`, `simulate_event(KeyDown/KeyUp)`, `deactivate_window`): arrasto com
  vários updates = 1 entrada (+ undo/redo), clique na trilha, Escape, voltar ao original, MouseUp perdido, perda
  de foco, troca de seleção no meio, undo no meio, teclado (hold = 1 entrada, tap = 1 entrada, Shift não encerra),
  fluxo do adapter, `SetProperty` discreto, `slider_pressed` limpa o rastreio.
- Hipóteses não verificadas que o primeiro run pode derrubar: `deactivate_window` precisa disparar o observador
  de ativação na janela de teste; `simulate_keystrokes("right")` precisa mandar só o key-down (o key-up é
  injetado à mão); o clique na trilha precisa focar o wrapper para o teste de teclado; os valores de pixel mapeiam
  para o passo do slider (o teste "voltar ao original" assume zoom 100 = (100-10)/390 da largura).
- Mouse físico **não** é provado por teste (BL043 continua aberto); só a entrada simulada do GPUI.

Gate (pelo coordenador, serial): `python3 /Users/wallyssonnunes/.cache/tessel-editor/cargo-serial.py test -p
tessel-editor-core --locked` (e `-p tessel-editor-desktop`), clippy `-D warnings`, `fmt --check`.
Formatado com `rustfmt --edition 2024` apenas os arquivos novos e `tests/slider_gesture.rs`.

## Limites conhecidos

- Ações de acessibilidade Aumentar/Diminuir do Kit não emitem evento: nenhum comando é gerado e o próximo render
  ressincroniza o slider com o modelo (BL074).
- Escape só chega se o foco está dentro da árvore do `Workstation` (o press no slider foca o wrapper, então o
  caso normal funciona).
- Após cancelar, o thumb pode piscar de volta para o valor do modelo até o release.
- Um release sem mudança ou de volta ao original deixa a revisão (dirty) incrementada, de forma conservadora; o
  histórico fica intacto.
- O key-up só conta para teclas de passo; o gesto de teclado depende do key-up chegar (se a janela perder o foco,
  a desativação confirma).
- Um valor rejeitado (`invalid_value`) deixa o gesto aberto e aparece na barra de status, como um `SetProperty`.

## Linhas propostas

### BL062 (docs/backlog.md)

`| BL062 | F005/F024 | Sliders gravam cada microChange no histórico; agrupar por gesto | A validar | Gate Cargo do conjunto (nada foi compilado/rodado) | L / Cursor Sonnet 5.5 | Preview contínuo; um undo no release, cancel/no-op sem entrada; undo/redo início/fim; lib/CLI/UI compartilham regras; testes e limites explícitos. Evidência: [relatório](slider-gesture-history-report.md); core `slider_gesture.rs`, desktop `slider_gesture_tests.rs`, escritos e não executados |`

### Matriz (docs/features/README.md; aplicado)

F005: `... | I | I | I | V | SetProperty, property.gesture.begin|update|commit|cancel |` e F024 inalterada na matriz
(o gesto é descrito em `history.md`). Promoção de I para V só depois do gate e da prova na janela nativa.

### Novas pendências (para docs/backlog.md; IDs a partir de BL074)

| ID | Área | Pendência | Estado | Depende de | Critério de aceite |
|---|---|---|---|---|---|
| BL074 | F005/a11y | Ações Aumentar/Diminuir do slider do Kit não emitem evento | Pendente | BL062 | Mudança upstream do Kit ou ponte local sem fork; uma entrada por ação |
| BL075 | F005/UX | Thumb pisca ao cancelar até o release | Pendente | BL062 | Sem salto visual após Escape; teste de estado do slider |
| BL043 | F005/prova | Prova com mouse físico de arrasto, soltar fora da janela e Escape | Pendente | BL043, BL062 | Evidência na janela nativa sem `computer use`/coordenadas |
| BL076 | F005/CLI | Documentar `property.gesture.*` em `docs/features/project-creation.md` e no `ui-automation.md` (a janela já o alcança: `workstation/ops.rs` usa o mesmo `parse_op` e `Source::Cli`) | Pendente | BL062 | Doc cruzada; prova de `ui op` com preview visível na janela nativa |

## Gancho de integração

Nenhuma linha restante: o arm em `ops.rs::parse_op` já foi adicionado:

```rust
gesture if gesture.starts_with("property.gesture.") => {
    crate::state::property_gesture::parse_op(gesture, request)?
}
```

## Integração e validação pelo coordenador

- `test -p tessel-editor-core --test slider_gesture --locked`: 28/28; suíte unitária do core: 197/197.
- `test -p tessel-editor --locked slider_gesture_tests -- --nocapture`: 13/13.
- Executável `target/debug/tessel-headless`: begin → 2 updates → commit → undo → redo;
  profundidade undo 0 durante updates, 1 após commit, 0 após undo, 1 após redo.
- Clippy core (lib) e desktop (all-targets), com `-D warnings`: limpos.
- Correções do harness: separar empréstimos mutáveis aninhados; ativar janela antes de testar
  desativação; um press no thumb não emite Change, então novo gesto só abre no primeiro valor.
- O adapter limpa um drag GPUI anterior quando recebe novo press de slider, impedindo sua continuação.
- Testes usam input do GPUI, não mouse físico. O binário do app aberto ainda é o anterior à correção.
- BL074/BL075/BL076 são os IDs reconciliados; a prova física continua em BL043. Docs de automação e
  criação já receberam o ciclo JSONL; prova socket aguarda o build atualizado.

## Release e janela atualizada

Build release passou. `python3 scripts/slider-live-proof.py` executou 12 checagens em uma
instância nativa com HOME/socket próprios: valores expostos no controle, histórico vazio durante
updates, commit único, undo/redo, cancel e no-op. [Resultado e hashes](slider-gesture-live-proof.json).
A prova usa IPC semântico, não ponteiro físico nem comparação visual de pixels.

O app do usuário foi atualizado depois de salvar um snapshot local fora do Git, sem jobs ativos.
Nome, playhead e duração da timeline foram conferidos após reabrir. Os dados do projeto e o caminho
da cópia não entram nos artefatos versionados. BL062/BL076 concluídos; BL043/BL074/BL075 continuam abertos.

Regressão desktop completa após a correção: `test -p tessel-editor --locked`, 108/108 passaram (inclui os 13 novos de slider). Não equivale ao gate workspace da rodada, que ainda aguarda integração de recursos.
