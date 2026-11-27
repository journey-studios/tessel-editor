# BL081.1.1 — atalho Cmd+B / Ctrl+B para cortar o clipe no playhead

Fatia de BL081.1 (F017, [split.md](../features/split.md)). **Validado tecnicamente no dispatcher Mac/Windows.** Não é aceite de teclado físico,
ponteiro, pixels, áudio ouvido nem desempenho. Sem corte em lote/seleção múltipla e sem mudança do envelope.

As contagens14, os limites de Mac e as propostas de status anteriores ao fechamento
abaixo registram a entrega inicial; a seção final consolida os16testes atuais e o aceite.

## Base e dependência

- Worktree novo, HEAD `e3c20cb7c8d3e32692719a9c71adb1b65eb3c392`, branch `feat/bl0811-split-shortcut`, status limpo ao começar.
- Pacote do core de envelopes aplicado com `git apply` (zip `cf9d4b35…5f9f`, 75357 bytes; `bl0812.patch` sha256 `3d10d987…c9c0`, os 36 hashes do
  `manifest.json` conferidos byte a byte depois do apply, sem normalização CRLF/LF). Sem staging, commit, push ou PR.
- Baseline do envelope congelado com índice Git **temporário fora da árvore** (`GIT_INDEX_FILE`); o diff desta entrega é incremental a ele.

## Contrato

| Item | Decisão |
|---|---|
| Comando | `Command::SplitClip` via `Workstation::dispatch` → `execute`, o mesmo do botão, de `ui invoke timeline.split` e de `clip.split`. `split_shortcut::split_command` é o único construtor no desktop (botão e tecla). Nenhuma regra de edição nova |
| Disponibilidade | `EditorState::split_availability()` lida **na hora da tecla**, nunca capturada no render |
| Undo | Um corte = uma entrada ("Split clip"); seleção do lado direito, undo/redo, envelope e PCM são os do fluxo atual |
| Plataforma | macOS `cmd-b` (⌘B); demais `ctrl-b` (Ctrl+B). Uma linha `shortcut(Platform)` define o que o listener casa **e** o texto do tooltip/nó |
| Casa | Exata: tecla `b` + só o modificador da plataforma. Shift/Alt/AltGr, a tecla do outro sistema e `b` sozinho não cortam |
| Mecanismo | Listener `on_key_down` do root `Workstation` (contexto `Workstation`). Sem `KeyBinding` global |
| Repetição | `KeyDownEvent::is_held` → consumida, não corta (nem se o playhead entrar no clipe com a tecla mantida). Novo toque = novo corte |
| Escopo de foco | Foco dentro do `Workstation`. Fora: cabeçalho, WebView nativa (BL044), outra janela, teclas fora do editor |
| Bloqueios | Sem seleção; playhead na borda/fora; foco em campo do Kit (`Input`, `NumberInput`, `TimeField`, `SearchPanel`, `Select`, `Combobox`, popover, diálogo, sheet); `Workflow::has_dialog`; `prefer_character_input` |
| Headless | **N/A**: entrada de apresentação. A edição pela lib/processo JSONL continua `clip.split`. Sem op `key.press` |
| Estado vivo | Nó `timeline.split` ganha `shortcut {keystroke,label,scope,repeat,blocked_in}` com o binding real; `enabled`/`disabled_reason` seguem do core |

### Por que listener e não action/keybinding

O GPUI despacha a *action* de um `KeyBinding` antes de qualquer listener (`dispatch_key_event`) e a action não carrega `is_held`:
uma tecla mantida cortaria a cada repetição, e de novo se o playback movesse o playhead para dentro do clipe. O listener do root vê o
evento. Consequência boa: não há registro global (nada a duplicar com duas janelas) e o `main.rs` não precisa de init.

### Colisões de keymap

Inspecionado `gpui-base` 0.9.0 / `gpui-component` 0.9.0 e o app: `Input` (cmd/ctrl a,c,x,v,z,y,f,h, setas, delete…), `Root` (c, tab), `Dialog`,
`Select`, `Popover`, inspetor (`cmd-alt-i`/`ctrl-shift-i`), medidor (`ctrl-shift-d`). **Nenhum `cmd-b` nem `ctrl-b`: sem conflito real, tecla mantida.**
Um teste (`the_keymap_has_no_binding_…`) consulta `all_bindings_for_input` para as duas teclas.

## Arquivos (ownership respeitado)

| Arquivo | Mudança |
|---|---|
| `crates/desktop/src/workstation/split_shortcut.rs` | novo: política de plataforma, casa exata, `decide`, listener, metadado do nó |
| `crates/desktop/src/workstation/split_shortcut_tests.rs` | novo: 14 testes |
| `crates/desktop/src/workstation/mod.rs` | `mod split_shortcut`, módulo de teste, `key_context(split_shortcut::SCOPE)`, uma chamada no `on_key_down` do root, duas chamadas de `adopt_split_shortcut` |
| `crates/desktop/src/workstation/timeline_panel.rs` | botão usa `split_command` e o tooltip ganha o atalho da plataforma |
| `docs/features/split.md`, este relatório | contrato e limites |

`main.rs`, `split_tests.rs`, core, áudio/envelope, media, `Cargo.lock`, dependências, runtime JS, backlog, matriz e arquitetura: **não editados**.

## Testes (`split_shortcut_tests.rs`, 14)

1. **Dispatcher real** (`simulate_keystrokes`): a tecla corta o clipe selecionado no playhead, spans e seleção corretos, uma entrada de undo; undo/redo restauram; a tecla lê seleção e playhead do estado vivo (alterados sem repaint antes da tecla); dois toques = duas entradas.
2. **Equivalência**: botão (clique), `invoke`, tecla e a biblioteca (`clip.split`) a partir do mesmo projeto NTSC 30000/1001, áudio (fades 20/20) e título (fades 12/24) cortados dentro da rampa no quadro 10: spans, plano de 96 quadros (JSON), PCM (comprimento e bits) e documento salvo idênticos; o PCM também é idêntico ao anterior ao corte. Sem tolerância.
3. **Negativos** (documento, revisão, histórico, seleção e playhead idênticos): sem seleção; playhead em `start`, `end`, antes e depois; `Input` do Kit com foco (texto `ab` preservado, `cmd/ctrl-a` + `z` ainda substitui o texto, tecla mantida também); diálogo aberto (e volta a cortar ao fechar); outras teclas/modificadores e `b` digitado; tecla mantida (inclusive com o playhead movido, e foco chegando com a tecla já pressionada).
4. **Escopo**: duas janelas/`Workstation`; a tecla na primeira corta só a primeira; a segunda sem foco no workstation não corta; com foco corta uma vez. Não há registro global (nada a duplicar).
5. **Plataforma**: política parametrizada (`decide`, `is_split_keystroke`, `shortcut`, `with_shortcut`) para Mac e demais; Windows executa a linha `ctrl-b` no dispatcher. **Não foi executado em Mac.**
6. **Nó vivo**: `timeline.split` publica o binding da plataforma; só ele tem o campo.

Mutações (restauradas; fonte reconferida): sem `is_held`, sem checagem de campo, sem checagem de diálogo, casa sem modificadores. Cada uma derruba
2–3 testes (política e dispatcher).

## Comandos e exit codes (Git Bash, `./scripts/cargo`, target padrão/mbx; logs em `E:\www\backups\bl08111-shortcut-20261114\logs\`)

| Comando | Exit |
|---|---|
| `cargo fmt --all -- --check` | 0 (a primeira rodada acusou um diff em teste meu, corrigido) |
| `./scripts/cargo check --workspace --locked` | 0 |
| `./scripts/cargo clippy --workspace --all-targets --locked -- -D warnings` | 0 (a primeira rodada acusou `large_enum_variant` em `Verdict`, corrigido: `Cut { clip, frame }`) |
| `./scripts/cargo test --workspace --locked --no-run` | 0 |
| `./scripts/cargo test -p tessel-editor --locked split_shortcut` | 0 — 14 passam |
| `./scripts/cargo test -p tessel-editor --locked split_tests` | 0 — 5 passam |
| `./scripts/cargo test -p tessel-editor --locked audio_fades` | 0 — 2 passam |
| `./scripts/cargo build -p tessel-editor --locked` | 0 |

Não repetidos: core inteiro, workspace executado (falhas/hang Windows pré-existentes), runtime JS, export, leitores antigos (BL081.2.5), desempenho. Sem lock de GPU.

Observação: um `cargo fmt --all` sem `--check` rodou uma vez no worktree; o manifest do pacote conferido depois não mudou nenhum arquivo do envelope.

## Prova nativa semântica (opcional, janela própria)

Uma janela de `target\debug\tessel-editor.exe` (debug, desta árvore), com `LOCALAPPDATA` privado (socket próprio), encerrada pelo PID depois.
`ui get timeline.split` devolveu `shortcut.keystroke: "ctrl-b"`, `label: "Ctrl+B"`, `scope: "Workstation"` e `disabled_reason` do core; `ui invoke timeline.split invoke`
no estado inicial (playhead 86 fora do clipe) foi recusado `disabled_element`. **Isso não prova a tecla física**: `invoke` e o GPUI dispatcher não são teclado.
O limite BL081.4 (shader com caminho antigo) **não** apareceu: o binário foi compilado nesta árvore; nenhum cache global foi limpo.

## Objeções e limites

- A tecla física (Windows/Mac), a WebView nativa (BL044: pode não encaminhar a tecla ao GPUI) e o comportamento de `is_held` no macOS **não** têm prova. O atalho não é global e não captura teclas fora do editor.
- Um `Select` do Kit focado (por exemplo, o seletor de sequência logo depois de escolher) também bloqueia a tecla até o foco sair dele: decisão conservadora para não roubar teclas de campo.
- Sem `KeyBinding`, o atalho não aparece em menus/`bindings_for_action` do GPUI; o rótulo vem da linha `shortcut(Platform)`. Se um menu de aplicação for criado, deve ler a mesma linha.
- Corte em lote/seleção múltipla segue fora (BL081.1 continua pendente nessa parte).

## Propostas para o coordenador (arquivos comuns não editados)

**`docs/backlog.md`** — nova linha, e BL081.1 passa a referenciar a fatia:

| ID | Origem | Item | Status | Dependências | Responsável | Aceite/Evidência |
|---|---|---|---|---|---|---|
| BL081.1.1 | Corte / atalho | Cmd+B (macOS) / Ctrl+B cortam o único clipe selecionado no playhead, no foco do editor | Implementado, a validar — Windows: fmt/check/clippy/no-run/build exit 0, 14 testes do atalho + split 5 + audio_fades 2 passam; sem Mac, sem tecla física | BL081, BL081.2 (envelope), BL044 (WebView/teclado) | Cursor Sonnet 5.5; revisão e integração coordenador | Mesmo `Command::SplitClip`, uma entrada de undo, repetição ignorada, bloqueio em campos do Kit/diálogo, equivalência botão/invoke/tecla/lib em fade atravessado e grade fracionária. [Relatório](migration/clip-split-shortcut-report.md) |
| BL081.1.1.a | Atalho / Mac | Execução em Mac: dispatcher + política `cmd-b` e `is_held` | Pendente | BL081.1.1 | — | Rodar `split_shortcut` no Mac; sem alegar tecla física |
| BL081.1.1.b | Atalho / WebView | Tecla física com o foco na WebView e fora do `Workstation` | Pendente (reutiliza BL044) | BL044 | — | Observação nativa; hoje o escopo termina no `Workstation` |

BL081.1 (lote/seleção múltipla) permanece **Pendente** (o atalho de teclado sai dessa linha).

**`docs/features/README.md`** — linha F017: acrescentar "atalho Cmd/Ctrl+B (BL081.1.1, a validar; headless N/A)" em UI; lib/headless inalterados.
Arquitetura (`docs/core-architecture.md`): sem mudança; o atalho é adapter de apresentação.


## Revisão independente e ajustes da coordenação

A revisão por fonte de GPUI/Kit e do adapter não encontrou bloqueios. Ela não
rodou Cargo nem janela: os gates Mac são execução separada da coordenação.
Cmd+B passa por performKeyEquivalent e o keydown normal do backend usa
NSEvent.isARepeat; prefer_character_input é false no backend macOS.
Não houve observação física de repeat, beep ou encaminhamento da WebView.

Achados pequenos tratados: foco real em sequence_select com corte Ready;
Root/EditorShell (incluindo header) e foco num WorkPanel descendente; colisões
de prefixos verificadas em todos os bindings, com controle cmd-b x que o
lookup de casamento completo não detecta. O listener retorna cedo para teclas
irrelevantes, antes de alocar contexto ou executar split_availability/plan_split.
Corpus atual16testes passou no Mac (14 anteriores + Select e cadeia de produção).
O primeiro compile do teste Select acusou E0502, corrigido usando Entity::update;
nenhum comportamento de produto foi alterado para fazer o teste passar.

Patch de revisão só nos dois arquivos split_shortcut.rs e seus testes:
5025bytes, SHA25614b24cd16dbe717653c9cc178a486f8d5da50bbb26a25949931893427a785715.
As tabelas e contagens14 anteriores descrevem a entrega Windows original;
fontes finais e evidências Mac/Windows serão registradas separadamente.
Gates Mac finais e rechecagem Windows em execução; ainda sem fechamento.


## Gates Mac finais após a revisão

fmt/check workspace alltargets/clippy alltargets -Dwarnings/test workspace
--no-run/build desktop: todos exit0. Dispatcher do atalho16/16, split5/5 e
fades2/2, sem falhas ou ignorados nesses alvos. Os16 incluem Select focado
e Root/EditorShell com foco num painel descendente; Cmd+B é a linha executada,
não apenas a política Mac parametrizada. Sem tecla física ou janela real nova.

Logs duráveis no SSD de coordenação, bl08111-coordination/mac-final/:
summary.json, source-manifest.json, snapshot.json e logs por comando. Mesmas
fontes enviadas para a rechecagem Windows dos dois arquivos; ainda pendente.
Não foi repetido core inteiro, workspace executado ou desempenho, pois o
contrato de edição do core não mudou. O gate no-run só prova compilação.


## Fechamento técnico Mac/Windows

Coordenação conferiu a [rechecagem Windows final](t3-thread://v2/33595e42-547d-42b7-92a7-cb4acb430dd7/mcp:launch:dfb2880582c80b137d7a8a05ebf2960e4e0e5b610a56a42c7ed83777abf083e2)
e declara BL081.1.1 **validado no dispatcher GPUI Mac/Windows**, após revisão
independente sem bloqueios e os ajustes pequenos. Lib e headless são N/A
para entrada de teclado; o comando de edição permanece clip.split no core.
Não há regra de edição nova ou mudança de áudio/documento nesta entrega.

### Snapshot e integridade

Base Mac12ac339; base Windows e3c20cb + pacote do envelope e patch incremental
46a8cd6c36d83d013bfb6532d7bb736383a495973e6f4461a1af2835b5042779.
Patch final de revisão5025bytes SHA256
14b24cd16dbe717653c9cc178a486f8d5da50bbb26a25949931893427a785715,
check/apply0 no Windows. Dois hashes finais byte a byte iguais no Mac/Windows:

| Arquivo | SHA256 |
|---|---|
| split_shortcut.rs | c714da40a1b5876f673f5f3b4ab590da5a9c05ece3891166f25fa47bf3846a10 |
| split_shortcut_tests.rs | c7afb1c16012d90aafb43ceab075b4a671858a6c7a8d7dd290ef910cff03b9a1 |

mod.rs e timeline_panel.rs mantêm os hashes da entrega inicial; as quatro
fontes estão no source-manifest.json dos gates Mac finais. Os29outros paths
verificados pelo worker Windows mantêm baseline. HEAD Windows permaneceE3,
sem mudanças staged; raiz e outros worktrees não foram usados na rechecagem.
Documentação final foi reconciliada pela coordenação depois desses gates.

### Gates do snapshot final

| Gate | Mac | Windows |
|---|---|---|
| fmt --all -- --check | 0 | 0 |
| check --workspace --all-targets --locked | 0 | 0 |
| clippy --workspace --all-targets --locked -- -D warnings | 0 | 0 |
| test --workspace --locked --no-run | 0 | 0 |
| test -p tessel-editor --locked split_shortcut | 16 passam | 16 passam |
| test -p tessel-editor --locked split_tests | 5 passam | 5 passam |
| test -p tessel-editor --locked audio_fades | 2 passam | 2 passam |
| build -p tessel-editor --locked | 0 | 0 |

Sem falhas ou ignorados nos três alvos executados. Testes de tecla usam o
dispatcher real, incluindo Select, foco no painel na estrutura de produção,
repeat, contexto, modifiers, undo/redo e equivalência de documento/plano/PCM
com botão/invoke/lib em NTSC e fades atravessados. Os quatro controles de
mutação pertencem à entrega inicial; não se alega repetição deles no final.

### Evidência e limites restantes

Mac: bl08111-coordination/mac-final no SSD de coordenação; summary.json,
snapshot.json, source-manifest.json e log por comando. Windows:
bl08111-shortcut-final-recheck-20261114 no backup externo do Computador;
evidence.log, gates-summary.log, 1-fmt a8-build.log, review-fix.patch,
patch.b64 e gates.sh. Não houve commit/push/PR pelo executor Windows;
a coordenação integra a entrega localmente, sem publicação autorizada aqui.

Nenhuma nova janela real no snapshot final, tecla física, pixels, áudio
ouvido, foco de WebView ou desempenho. O backend macOS e o mapeamento isARepeat
foram inspecionados na fonte; os eventos de repeat nos testes são sintéticos.
Um eventual beep quando Cmd+B está bloqueado continua não observado, como
parte de BL081.1.1.b. Campo Select bloqueia conservadoramente enquanto focado.
Lib/core completo, workspace executado e runtime JS não foram repetidos;
no-run prova compilação. BL081.1.2 (lote) permanece pendente, sem implementação.
