# F039-UI — Inspector e árvore de cena dirigida na janela (BL014/BL021)

## Status

| Superfície | Estado | Evidência |
|---|---|---|
| Lib / modelo (`workstation/scene_model.rs`, sem GPUI) | **V (lib)** — 13 testes unitários: árvore igual à de `scene.tree`, propriedades por tipo de entidade, indisponibilidade com motivo, reset, candidato sem tocar o documento, cache, janela do slider, Euler | `cargo test -p tessel-editor scene_model` (13/13) |
| Headless | **N/A** — o inspector é apresentação; o headless já prova os mesmos comandos em [F039](scene-interaction.md) (`scene.select`, `scene.gesture.*`, `scene.edit`) | — |
| Socket / `ui op` / `ui get|set|invoke` da janela | **V (socket, instância própria)** — mesmo executor da UI; ver "Prova de socket" | `/tmp/tso-proof.py` (receita abaixo), binário `target/debug/tessel-editor` com `HOME=/tmp/tso` |
| UI GPUI (entrada injetada) | **V (UI, input injetado)** — 16 testes com `simulate_mouse_*`, `simulate_keystrokes`, `simulate_event` e `deactivate_window` | `cargo test -p tessel-editor scene_tests` (16/16) |
| Ponteiro físico / pixels | **P — não provado.** Os testes injetam eventos pela janela de teste do GPUI; nada aqui prova mouse real (BL043) nem aparência. Nenhuma captura de tela foi tirada | — |
| Monitor da cena dirigida | **Parcial** — renderer integrado; recursos precisam estar carregados em F040. Botão e comando semântico usam o mesmo host | `scene_monitor.rs`, `scene_monitor_host.rs`, F041 |

Não confundir: um **comando** (`scene.gesture.update`) não é um **ponteiro** (arrastar o slider). O
slider é um adaptador de entrada que envia os mesmos comandos; o socket os envia diretamente. As duas
provas são distintas e nenhuma substitui a outra.

## Objetivo e limites

Na aba **Cena** do inspector da janela (ao lado de **Clipe**), editar uma cena dirigida já anexada
(F036): escolher a cena, ver a árvore de entidades, selecionar uma entidade por ID e editar as
**8 propriedades tipadas do catálogo** (`position`, `rotation`, `scale`, `width`, `height`,
`start/position`, `start/target`, `start/fov`) com valor vivo e candidato, reset quando o catálogo
define um padrão, erro tipado visível e controles indisponíveis com o motivo na tela.

Não acrescenta regra de domínio. O catálogo, a validação, o candidato do gesto, a entrada única de
undo e a invalidação tipada continuam no core ([F039](scene-interaction.md)). A janela não guarda
cópia da cena: tudo é lido do `EditorState` e toda mudança é um `Command::Scene`.

Fora, de propósito: overlays/handles sobre o monitor (BL015), export dirigido (BL008),
diálogo de anexar cena, edição de qualquer propriedade fora do catálogo.

## Contrato

### Leitura

`SnapshotCache::get(&EditorState, scene_id)` devolve um `Snapshot` imutável (`Rc`), refeito só
quando muda a assinatura (cena, documento, seleção, candidato, gesto). Ele usa as **mesmas funções**
que os comandos de leitura: `scene_interaction::tree_json` (a de `scene.tree`), `catalog()`
(`scene.catalog`), `read_property` (`scene.get`/`scene.properties`) e `Gesture::preview_scene`.
O teste `the_tree_is_the_one_scene_tree_answers` compara a árvore do painel com a resposta do comando.
Ler pela biblioteca (e não por `execute`) deixa o `render` ser `&self` e não poda a seleção.

Cada propriedade tem `live` (documento) e `candidate` (gesto aberto); o controle mostra o candidato.

### Escrita (sempre `Command::Scene` por `Workstation::execute`)

| Ação na janela | Comando |
|---|---|
| Clicar na linha da árvore / mover a seleção do Kit `Tree` por teclado | `scene.select` (`replace`, uma entidade, na cena ativa) |
| Pressionar / arrastar o slider | `scene.gesture.begin` na entidade **do slider** → `scene.gesture.update` com um `set` absoluto da propriedade a cada `Change` |
| Soltar o slider / soltar a tecla de passo / janela perde o foco | `scene.gesture.commit` — **uma** entrada "Edit scene" |
| Escape / botão "Cancelar gesto" | `scene.gesture.cancel` — documento e histórico intocados |
| Botão de reset da propriedade | `scene.edit` com `EditOp::reset` (uma entrada) |

Regras de entrada (`scene_gesture.rs`, mesmo contrato do BL062 para clipes):

- o gesto começa na **entidade do slider**, nunca na seleção atual; trocar a seleção durante o arraste
  não redireciona a edição (o core também nunca lê a seleção);
- teclado ≡ ponteiro: setas, Shift (passo ×10), PageUp/PageDown, Home/End; segurar a tecla é um
  gesto e uma entrada; cada toque novo é a sua;
- depois de Escape, o resto do clique/tecla é ignorado e soltar não grava nada;
- um valor recusado pelo core (`invalid_value`) deixa o gesto aberto e o erro tipado aparece no painel;
  um gesto que o core já descartou (`gesture_invalidated`, `no_gesture`) encerra o arraste sem repetir.

### Escala do slider (apresentação)

O catálogo limita só alguns números (`0 < width`, `0 < fov < 180`); um slider precisa de dois
extremos. `scene_model::scrub` dá uma **janela** em volta do valor (limites do catálogo aplicados,
extremo exclusivo recuado um passo; rotação em ±180°, eixo Y em ±90°). `SliderState` não muda a faixa
depois de criado, então o slider é reconstruído só quando o valor sai da janela por outra via (um `set`
pelo socket) **e** nenhum arraste é dono do controle. É só apresentação: quem decide o que é válido
continua sendo o core. A rotação é editada como ângulos de Euler (a conversão é a do core) e
permanece quaternion unitário.

### Nós semânticos (`ui tree` / `get` / `set` / `invoke`)

| Nó | Conteúdo |
|---|---|
| `inspector.mode` | `tabs`; `set inspector.mode value clip\|scene` |
| `scene.panel` | `state`: cena ativa, seleção, alvo (`none`/`multiple`/`entity` + `reason`), `gesture` (id, updates, `has_candidate`, `invalidated`), `notice` (erro tipado), `monitor`; ação `cancel_gesture`; `enabled:false` + `disabled_reason` sem cena anexada |
| `scene.picker` | só com mais de uma cena; `set scene.picker value <id>` |
| `scene.tree` | `state.truncated` quando passa de 256 linhas visíveis |
| `scene.entity:<path>` | `treeitem` com `path`, tipo, `selected`, `expanded`, `editable[]`; ações `select`, `expand`, `collapse` |
| `scene.property:<path>` | campos por componente (`value` \| `x,y,z` \| `euler_x,euler_y,euler_z`) com `min/max/step` **iguais aos do slider**; `state.live/candidate/default/modified`; ação `reset` com `enabled` e `disabled_reason`; `disabled_reason` quando a entidade não aceita a propriedade |
| `monitor.scene` | filho de `region:monitor`; `enabled:false` + `disabled_reason` enquanto o runtime ou os recursos estão indisponíveis; `invoke` devolve `disabled_element` tipado |

`set scene.property:<path> <campo> <n>` é begin → update → commit pelo **mesmo** caminho do slider
(`scene_set_component`): **uma** entrada de undo; se o update falha, o gesto é cancelado e nada fica
aberto.

## Monitor integrado (F041)

`SceneMonitor::Available` só habilita o botão quando o runtime está pronto e há mapa aprovado do projeto atual. Sem recursos, informa `scene.resources.resolve`; a edição funciona mesmo sem monitor.
`monitor.show_scene {scene}` e `invoke monitor.scene show` usam o mesmo host assíncrono. Pedidos de frame explícito entram por `scene.render`. Regras de plano e gesto permanecem no core; a escolha do monitor é apresentação local à janela.


## Prova de socket (instância própria)

Instância isolada, binário direto, `HOME=/tmp/tso`, lock `/tmp/tessel-scene-gpu.lock`, app do usuário
não tocado, encerrada e removida ao fim. Resultado (build com as fontes desta frente):

- `op scene.attach` (path da fixture), `set inspector.mode value scene`, `op scene.select` → `get scene.entity:…/figure`
  mostra `selected:true`; `get scene.property:…/width` traz `live:2`, `candidate:null`, limites e o
  motivo do reset indisponível (largura não tem padrão no catálogo);
- `set scene.property:…/width value 3.5` → `action: scene.gesture.commit`, `before:2`, `after:3.5`;
  `op history.undo` → rótulo "Edit scene", largura volta a `2`;
- `op scene.gesture.begin` + `update` (candidato 5) **sem** a janela tocar nada → `get scene.property`
  mostra `live:3.5`, `candidate:5`; `get scene.panel` mostra o gesto aberto; `invoke scene.panel cancel_gesture`
  → `scene.gesture.cancel`, valor segue `3.5`;
- `invoke scene.property:…/position reset` → `scene.edit`, `position` volta a `(0,0,0)`;
  `invoke scene.property:camera/cam-main/start/fov reset` → `disabled_element` ("não define um valor padrão");
- `set … width value -1` → `invalid_value` tipado e o mesmo erro em `scene.panel.state.notice`;
- `get monitor.scene` → `enabled:false` com `disabled_reason`; `invoke monitor.scene` → `disabled_element`.

Não houve captura de pixels. A prova de ponteiro físico segue em BL043.

## Cobertura de testes

`crates/desktop/src/workstation/scene_tests.rs` (GPUI, entrada injetada):

- arrastar → candidato ao vivo, documento e histórico intocados, soltar → **uma** entrada, undo volta;
- Escape cancela e ignora o resto do clique; foco perdido confirma uma vez; teclado: segurar é uma entrada;
- trocar a seleção durante o arraste não redireciona a edição;
- a seleção do Kit `Tree` (teclado/clique) vira `scene.select` e o painel edita o novo alvo;
- reset (uma entrada; sem padrão → erro tipado, sem entrada); valor recusado → erro na tela, nada aberto;
- `set` semântico ≡ slider (uma entrada; componente de vetor sozinho); gesto aberto pelo socket aparece como
  candidato e se cancela pelo painel;
- nós por ID (selecionar, recolher, filhos ocultos), propriedade de grupo desabilitada com motivo, monitor
  indisponível, sem cena anexada, seletor de cena, limites do nó = limites do slider.

`scene_model.rs`: 13 testes sem janela (listados no status).

## Limites conhecidos

- **Aparência não verificada visualmente**; só estrutura, estado e entrada injetada.
- Trocar de modo (Clipe | Cena) é estado da janela, não um `Command` do core (como o dock antes do
  `execute_layout`); está nos nós `inspector.mode`. Proposta no relatório O.
- Ainda não há diálogo "Anexar cena" na janela: o painel explica e aponta `scene.attach`.
- Increment/Decrement de acessibilidade do Kit não emitem evento (BL074): igual aos sliders de clipe.
- Salto do thumb ao cancelar (BL075): mesma mecânica do BL062; **não medido** nos sliders de cena.
- A árvore lista no máximo 256 linhas visíveis nos nós semânticos (`truncated`); o Kit `Tree` rola.
