# F039-UI — Inspector e árvore de cena dirigida na janela (BL014/BL021)

## Status

| Superfície | Estado | Evidência |
|---|---|---|
| Lib / modelo (`workstation/scene_model.rs`, sem GPUI) | **V (lib)** — 13 testes unitários: árvore igual à de `scene.tree`, propriedades por tipo de entidade, indisponibilidade com motivo, reset, candidato sem tocar o documento, cache, janela do slider, Euler | `cargo test -p tessel-editor scene_model` (13/13) |
| Headless | **N/A** — o inspector é apresentação; o headless já prova os mesmos comandos em [F039](scene-interaction.md) (`scene.select`, `scene.gesture.*`, `scene.edit`) | — |
| Socket / `ui op` / `ui get\|set\|invoke` da janela | **V (socket, instância própria)** — mesmo executor da UI; ver "Prova de socket" | `/tmp/tso-proof.py` (receita abaixo), binário `target/debug/tessel-editor` com `HOME=/tmp/tso` |
| UI GPUI (entrada injetada) | **V (UI, input injetado)** — testes com `simulate_mouse_*`, `simulate_keystrokes`, `simulate_event` (clique, arrastar, roda) e `deactivate_window`; os de layout medem as sondas de geometria em 100…628 px | `cargo test -p tessel-editor scene_tests` (inclui `scene_tests::visual`, 11 testes) |
| Aparência (pixels) | **V (captura nativa da instância própria, inspecionada)** — 50 capturas do SO em janela larga e 960x640, tema escuro e claro, docks de 100/180/272/628 px; defeitos achados e corrigidos (BL087). Ver "Inspeção visual nativa" | `scripts/scene-visual-proof.py` sob `with-gpu-lock.py`; relatório [next-r9-report.md](../migration/next-r9-report.md) |
| Ponteiro físico | **P — não provado.** Os testes injetam eventos pela janela de teste do GPUI e a captura é de estado preparado por comandos semânticos; nada aqui prova mouse real nem a roda de um trackpad (BL043) | — |
| Monitor da cena dirigida | **Parcial** — renderer integrado; recursos precisam estar carregados em F040. Botão e comando semântico usam o mesmo host | `scene_monitor.rs`, `scene_monitor_host.rs`, F041 |

Não confundir: um **comando** (`scene.gesture.update`) não é um **ponteiro** (arrastar o slider). O
slider é um adaptador de entrada que envia os mesmos comandos; o socket os envia diretamente. As duas
provas são distintas e nenhuma substitui a outra.

## Objetivo e limites

Na aba **Cena** do inspector da janela (ao lado de **Clipe**), editar uma cena dirigida já anexada
(F036): escolher a cena, ver a árvore de entidades, selecionar uma entidade por ID e editar as
**propriedades tipadas do catálogo** (`position`, `rotation`, `scale`, `width`, `height`,
`renderOrder` e `start/position`, `start/target`, `start/fov`; os componentes `position/x` ... não são
linhas próprias: cada eixo já é um slider, ver [F034](scene-edit.md)) com valor vivo e candidato, reset quando o catálogo
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

A prova do socket não captura pixels; a aparência está em "Inspeção visual nativa". A prova de ponteiro físico segue em BL043.

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

`scene_visual_tests.rs` (módulo `scene_tests::visual`, BL087; layout em janela de teste 960x640):

- a árvore mostra linhas inteiras (altura múltipla de 28 px) e cresce com a janela;
- em todo largura de dock (100, 150, 180, 231, 232, 272, 400, 628) cada slider tem trilha ≥ 64 px e
  fica dentro do inspector;
- um valor longo (`12345.678`) nunca cobre a trilha, nas duas disposições (uma linha / empilhada);
- a disposição troca no limite documentado (232 px); a seleção por ID de uma entidade escondida abre
  os ancestrais e rola a árvore até a linha, e o passo real das linhas é `TREE_ROW_HEIGHT`;
- a revelação acontece **uma vez** por seleção: editar a entidade selecionada não puxa a árvore de volta;
- a roda rola a árvore sobre a árvore e o inspector fora dela, inclusive sobre um slider;
- o motivo de um controle desabilitado está na tela (monitor e "Anexar"), dentro do inspector, e o botão
  "Anexar" cabe no dock de 100 px;
- `attach_availability`: sem diálogo ligado, desabilitado com motivo; com ele, o mesmo ponteiro;
- arrastar um slider **empilhado** (inspector de 120 px, depois de rolar até ele) é um gesto e **uma**
  entrada de undo, e o undo volta ao valor original.

`scene_model.rs`: 19 testes sem janela (13 anteriores + `ancestors`, `PanelFit`, altura da árvore, revelar uma vez e
revelação que sobrevive a rebuild).

## Limites conhecidos

- **Aparência**: verificada por captura nativa só nos estados e tamanhos da seção "Inspeção visual nativa".
  Não há captura do estado com a roda rolando (sem comando semântico de rolagem) nem de escala de
  tela diferente; o scroll é provado por evento injetado, não por dispositivo.
- Trocar de modo (Clipe | Cena) é estado da janela, não um `Command` do core (como o dock antes do
  `execute_layout`); está nos nós `inspector.mode`. Proposta no relatório O.
- O botão "Anexar" do painel abre o diálogo F043, integrado pelo coordenador após a R9.
- Increment/Decrement de acessibilidade do Kit não emitem evento (BL074): igual aos sliders de clipe.
- Salto do thumb ao cancelar (BL075): mesma mecânica do BL062; **não medido** nos sliders de cena.
- Cabeçalho em 960 px: a integração mostra só ícones de ação abaixo de 1040 px e permite truncar o título; nova captura pendente em BL104.
- A árvore lista no máximo 256 linhas visíveis nos nós semânticos (`truncated`); o Kit `Tree` rola.

## Inspeção visual nativa (BL087)

`scripts/scene-visual-proof.py` (sob `scripts/directed-scene-proof/with-gpu-lock.py`) sobe o binário direto com `HOME` curto
próprio, prepara cada estado só por pedidos semânticos (sem coordenadas, sem ponteiro) e captura a **janela dessa instância**
com `screencapture -l <id da janela do PID>`. Estados: árvore expandida de uma cena funda e larga, entidade selecionada,
candidato de gesto, erro tipado, vetor, valor extremo, última linha. Combinações: janela larga e 960x640 × escuro/claro, e
no mínimo docks de 100/180/628 px. Relatório com `captured`/`limit` por captura; `visual_validated` só vira verdadeiro depois
de uma revisão humana ou de agente das imagens.

Defeitos vistos nas capturas **antes** e corrigidos (todos com regressão em `scene_visual_tests.rs` ou `scene_model.rs`):

| # | Defeito | Correção |
|---|---|---|
| D1 | árvore fixa em 240 px: linhas cortadas pela metade e, em janela alta, espaço desperdiçado | altura = linhas inteiras (`PanelFit::tree_height`, 5…16 linhas, ≤ 42 % do painel); altura de linha fixa em `TREE_ROW_HEIGHT` (28 px), porque a do texto dependia da fonte |
| D2 | seleção por ID (socket/headless) de entidade fora da janela da árvore ou sob ancestral recolhido não aparecia | revela uma vez: abre os ancestrais (mesmo `expand`) e rola até a linha (`Nearest`) |
| D3 | valor impresso por cima da trilha do slider | coluna de valor com teto de 64 px; abaixo de 232 px a linha empilha (rótulo+valor, trilha embaixo) |
| D4 | trilha do slider espremida a quase zero em dock estreito | trilha ≥ 64 px em 100…628 px |
| D5 | barra de modos cortada em 100 px | abas de largura igual (sublinhado) no dock estreito |
| D6 | motivos só em tooltip (invisíveis sem hover) | motivo impresso sob o monitor e sob "Anexar" |
| D7 | texto do gesto truncado | quebra de linha; "Cancelar gesto" desce de linha |
| D8 | sem separação entre árvore e propriedades | título `scene-target` com `border_t_1` |
| D9 | coluna de rótulo de 76 px desperdiçada | rótulo ocupa o que precisa; slider por linha |
| D10 | ícone da árvore: câmera/take/trilha caíam no genérico | mapeamento `camera_clip`/`shot`/`performance_track` |
| D11 | (2ª rodada) botão "Anexar cena" passava da borda em 100 px; recuo de 12 px/nível deixava só ícones | rótulo curto "Anexar" (tooltip e rótulo de acessibilidade completos) e recuo de 6 px/nível no layout compacto |

Probes novas (leitura por `geometry`): `scene:tree`, `scene:monitor-reason`, `scene:attach`, `scene:attach-reason`,
`value:<elemento>`, além das `control:<elemento>` que já existiam.

## Integração do botão "Anexar cena"

O abridor `ATTACH_SCENE_OPENER` chama `open_flow(FlowKind::AttachScene, ...)`, o mesmo diálogo F043 do cabeçalho. O botão está habilitado e conserva nome acessível e tooltip completos no layout compacto. O texto abaixo orienta a escolher o bundle e a pasta de recursos. `NO_SCENE_REASON` também aponta para o diálogo disponível.

Na integração o coordenador reconferiu os 11 testes visuais GPUI depois de ligar o diálogo e compactar o cabeçalho. As capturas R9 precedem esses dois ajustes; a nova captura do cabeçalho fica em BL104. Eventos injetados não provam ponteiro físico.

## Correção de identidade do cache (revisão PR #3)

A assinatura retém Arc do documento e candidato e compara Arc::ptr_eq. Isso impede reutilização de endereço após limpar/limitar o histórico, sem mudar as regras de edição. Regressão mantém o documento vivo enquanto a assinatura existe e o libera ao descartar o cache.

A captura final BL104 foi concluída: 50/50 capturas pelo SO, com selected-min-dark/light e selected-panel100-light-inspector abertas pelo coordenador. Cabeçalho sem sobreposição após compactar ações e Anexar habilitado dentro do dock de 100 px; [prova final](../migration/round-r1-r10-results/visual.json). Não é prova de ponteiro físico.
