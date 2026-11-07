# F049 — Medidor de desempenho de debug

## Objetivo e limites

Um cartão flutuante, ligado e desligado por atalho, que mostra o que importa para depurar
fluidez: quantos quadros a **janela** pinta, quantas imagens novas o **monitor** apresenta,
quadros perdidos na reprodução, `render_ms`, o contexto do quadro e a memória do processo.
Backlog: BL145.

É uma ferramenta de **apresentação**: não edita o projeto, não entra no undo e não é gravada
no projeto. Não há GPU, compositor nem hardware medidos aqui; o que cada número é, e o que não
é, está na seção [O que cada número é](#o-que-cada-número-é).

## Atalho

| Plataforma | Atalho |
|---|---|
| macOS | `⌘⇧D` (`cmd-shift-d`) |
| Outras | `Ctrl+Shift+D` |

- Escolhido por não colidir: nenhum outro binding do app ou do Kit usa essa tecla (o teste
  `the_shortcut_is_bound_once_and_no_other_binding_uses_it` varre o mapa de teclas vivo) e o
  macOS não o reserva para o sistema (Captura de tela é `⌘⇧3/4/5`, Spotlight é `⌘Espaço`,
  Mission Control é `⌃`+setas; `⌘⇧D` é de apps individuais, como Safari).
- É uma ação global do GPUI (`TogglePerfMeter`, contexto `None`): funciona com o foco em
  qualquer painel, no cabeçalho ou em lugar nenhum. Combinações com `⌘`/`Ctrl` não digitam
  texto, então um campo de texto não é afetado e o atalho também funciona dentro dele (o app
  não tem padrão de suprimir atalhos globais ao digitar).
- Descoberta: o botão de gauge do cabeçalho (ao lado de tema e reset do layout) mostra o
  atalho na dica; o cartão mostra `depuração · ⌘⇧D fecha`; o nó `toolbar.perf_meter`
  publica `shortcut` e `shortcut_label`; a barra de status confirma "Medidor de desempenho
  ligado/desligado (⌘⇧D)".

## Contrato do core / biblioteca

`tessel_editor_core::perf` (sem GPUI, testado com relógio injetado):

- `PerfMeterCommand::{Toggle, Show, Hide}` e `apply(visible)`; o parser de `commands`
  devolve `Parsed::PerfMeter` para `invoke toolbar.perf_meter toggle` e `set toolbar.perf_meter
  visible true|false`.
- `FrameClock`: taxa nos últimos 1 s (`RATE_WINDOW`) e **pior intervalo** dos últimos 5 s
  (`WORST_WINDOW`). Sem movimento esperado, 1 s sem quadros é **repouso** (`parado`), não
  "0 fps". Com movimento esperado (reprodução), o silêncio é uma **parada** e é lido como tal.
  O intervalo de uma imagem corre desde a última imagem **ou** desde o início do movimento (o
  que for mais recente); sem imagem anterior (medidor ligado durante a reprodução) a âncora do
  movimento é a origem, então uma parada de 3 s antes da primeira imagem continua no pior
  intervalo depois que as imagens chegam.
- `PlaybackLoss`: quadros do relógio de reprodução que o monitor não apresentou
  (`perdidos de N (x%)`, `atraso`). Só existe durante a reprodução; fora dela é
  "só em reprodução". `start(frame, on_screen)`: o quadro inicial só conta como apresentado
  se o monitor **estava mostrando esse quadro** (`on_screen == Some(frame)`); senão a própria
  imagem dele precisa chegar antes de qualquer mais nova (se uma mais nova chega primeiro, ele
  é um quadro perdido). Início de reprodução, seek e ligar o medidor no meio da reprodução
  usam a mesma regra; o adaptador informa o quadro de sequência da última imagem aceita
  (`ViewportHost::presented_frame`). Sem imagem ainda, não há extensão para perder: `atraso`
  conta os quadros devidos.
- `Meter`: não registra nada enquanto desligado (`samples() == (0, 0)`,
  `next_wakeup() == None`). `enable(now, playing_from, on_screen)` recebe o quadro do relógio
  **só quando o relógio está rodando**: pausa não tem relógio (o silêncio é repouso, não
  parada). `playback_started` de uma reprodução que já está rodando (um `playback.resume` sem
  pausa) **não** reinicia a execução: as contagens de perdas sobrevivem. Os contadores vivem
  em `core::perf`, compartilhados por biblioteca, headless (N/A, sem janela) e UI.
- `Readout` (`rows()`, `strip()`, `to_json()`), `FrameContext::of(&FrameResult)`. O rótulo de
  qualidade acrescenta `+ proxy` e `+ sort` (`source.stale_sort_layers`, BL130): uma imagem
  com a ordem de splats de outra câmera nunca é apresentada como a final, e o refino que a
  substitui conta como uma imagem nova do monitor, mas **não** como um segundo quadro
  apresentado nas perdas.
- `place(area, blocked)`: onde o cartão cabe longe do retângulo da WebView.

O que não se sabe aparece como **indisponível**, nunca como zero. O MSAA e as camadas
reduzidas (`frame.done.source.scene_samples|reduced_scene_layers`) só aparecem quando o
runtime os informa; sem esses campos a linha omite o MSAA.

## CLI headless

**N/A.** O medidor mede a janela e o monitor vivo; não há janela nem WebView no
`tessel-headless`. O alvo `toolbar.perf_meter` responde `desktop_required` ali (testado em
`crates/core/tests/headless.rs`). As regras (janela de amostragem, FPS, pior intervalo,
perdas, posicionamento) são da biblioteca e têm teste sem GPUI.

## UI e CLI da janela

- Atalho, botão do cabeçalho e CLI chamam o mesmo `Workstation::execute_perf`.
- `./scripts/ui get toolbar.perf_meter`: `visible`, `shortcut`, `shortcut_label`, `placement`
  (`floating` com `y`, ou `status_bar_strip`) e, **só quando visível**, `readout` com os
  números abaixo. Escondido, `readout` é `null`: nada é coletado.
- `./scripts/ui invoke toolbar.perf_meter toggle` e `./scripts/ui set toolbar.perf_meter visible true|false`.

### Onde fica

- Flutuante no canto direito, **fora do Dock** e sem deslocar painéis (o teste
  `the_meter_displaces_no_panel` compara a geometria de todas as regiões com e sem o medidor).
- Não captura o mouse: o cartão não tem id, hitbox nem listeners
  (`a_click_on_the_card_reaches_what_is_underneath` falha se alguém adicionar `occlude`).
- **A WebView nativa pinta acima de qualquer elemento GPUI.** O cartão não pode ficar sobre o
  monitor. A solução é geométrica, calculada a cada quadro a partir do retângulo real da
  moldura da imagem (`viewport.frame`, a mesma caixa que a WebView segue):
  1. canto superior direito, se estiver livre (layout padrão: sobre o inspetor);
  2. senão o canto inferior direito, ou logo acima/abaixo da WebView;
  3. senão (monitor maximizado, nenhum canto livre) o medidor vira **uma linha monoespaçada
     na barra de status**, que nenhuma WebView cobre.

  Quando o layout muda por baixo do cartão, o `prepaint` percebe e pede **um** quadro extra
  para ele cair no lugar certo. Casos: padrão (cartão sobre o inspetor), inspetor fechado
  (cartão desce para a ponta direita da timeline, abaixo da imagem), monitor maximizado (faixa
  na barra de status). Um diálogo aberto já esconde a WebView (`Workflow::has_dialog`) e não
  precisa de tratamento próprio.

### O que cada número é

| Linha | Fonte | O que é | O que **não** é |
|---|---|---|---|
| `interface` | quadros que a janela GPUI **pintou** (um `canvas` sem tamanho cujo paint roda uma vez por `Window::draw`), taxa de 1 s e pior intervalo de 5 s | fluidez da interface | não conta ticks de timer; o redraw que o próprio medidor pede para escrever "parado" não conta |
| `monitor` | imagens novas que o host **aceitou** (`frame.done` aceito; descartes de resposta velha não contam; `frame.presented` não é uma imagem nova), em reprodução e em scrub | o número que mais importa: a cadência real de imagens novas | não é a apresentação do compositor, que o host não observa |
| `perdidos` | quadros do relógio de reprodução menos imagens apresentadas desde o início da reprodução (e do último seek) | quadros pulados na reprodução em curso | só existe tocando |
| `render_ms` | `frame.done.timings.render_ms` do último quadro | **tempo de CPU até o envio do desenho** | **não inclui a rasterização na GPU**: pode marcar 16 ms com a imagem travada; nunca é lido como FPS ou fluidez |
| `quadro` | largura × altura, qualidade (`preview`, `original`, `proxy`) e MSAA se informado | contexto da medição | MSAA ausente fica omitido |
| `RSS` | `memory-stats` do processo do editor | memória residente do editor | **não inclui** os processos WebContent/GPU do WebKit; não é memória de GPU |
| `texturas` | `frame.done.cache.texture_bytes` do runtime | estimativa dos tamanhos RGBA das imagens retidas | **não é leitura de GPU** |

Repouso: uma fonte sem quadros por 1 s e sem movimento esperado lê `parado` (o `monitor`
parado, inclusive em pausa). A `interface` só lê `parado` se a janela realmente não pintar:
hoje o app pinta ≈4 quadros/s sozinho em repouso (ver o achado abaixo). Valor indisponível lê `indisponível` (por exemplo,
o monitor sem runtime).

### Não distorce a medição

- **Escondido: custo zero.** Nenhum elemento é adicionado à árvore, nenhum timer existe,
  nada é gravado e nenhuma leitura é calculada; cada feed volta num único teste de `bool`.
- **Visível:** o texto é recalculado **no máximo a cada 250 ms (4 Hz)**, só a partir de
  quadros que já estavam sendo pintados, nunca por pedido do medidor. Não há temporizador
  contínuo. O único timer é de disparo único, armado depois de um paint, que acorda uma vez
  para o redraw de "parado" e não rearma em repouso (a janela volta a dormir). **Esconder o
  medidor descarta a tarefa do timer** (cancela-o): nenhum despertar acontece escondido.
- **Relógio:** o medidor lê o relógio do executor (`BackgroundExecutor::now`): o relógio real
  no app e o controlado nos testes GPUI (`advance_clock`). Nenhum teste dorme.
- Não pede quadros ao monitor, não dispara refino e não invalida cache: ele só **lê** o que o
  host já recebeu.
- O contador de UI vive no paint, não num timer: o teste
  `the_ui_rate_counts_frames_the_window_paints_and_not_timer_ticks` pinta 5 quadros, avança o
  relógio controlado até o repouso (o timer dispara), e confere que continuam 5; o redraw de
  "parado" não conta e o quadro seguinte conta (controle).
- O limite de 4 Hz é afirmado **contando recálculos** (`refreshes`), com o relógio parado
  durante os paints: oito quadros no mesmo instante não recalculam; 249 ms depois, também
  não; 250 ms, exatamente um (`the_numbers_are_recomputed_at_four_hertz_at_most`). Recalcular
  a cada paint falha o teste.
- Barra de status a 960 px com o monitor maximizado: a faixa do medidor **encolhe e trunca**
  (`flex_shrink(1)`, `min_w(0)`, `truncate`); tarefas e atividade ficam inteiras e numa linha
  e o rótulo salvo não sai da barra (`at_960_px_with_the_monitor_maximized_...`).

### Persistência

O app **não guarda preferências de interface** (nem o tema nem o layout do Dock sobrevivem ao
reinício), então o estado ligado/desligado também não persiste. Não foi criado um sistema de
preferências para isso.

## Evidências e status

Implementado e validado nas superfícies que se aplicam (UI e CLI da janela; lib com regras
testadas sem GPUI; headless N/A). Não validado: ver [limites](#limites).

- **Core:** testes em `core::perf` (relógio falso: taxa, pior intervalo, repouso x parada,
  perdas, janelas, `place`, quadro inicial só se apresentado, parada anterior à primeira imagem,
  pausa, resume sem pausa, rótulo `+ sort`), mais parser, nó de árvore e `desktop_required`
  no headless. No head integrado `17d73f0` (PR #18 + `main` 52bf9ba): 38 testes `core::perf::tests`.
- **GPUI (geometria, não pixels, relógio controlado):** `crates/desktop/src/workstation/perf_tests.rs`
  e testes de alimentação em `viewport_host_tests.rs`: atalho único e sem conflito, alterna com
  qualquer foco, o nó e o atalho compartilham estado, escondido não grava nada, a UI conta
  quadros pintados e não ticks, 4 Hz por contagem de recálculos, esconder cancela o timer,
  ligar o medidor com a reprodução pausada lê repouso, resume sem pausa mantém as perdas,
  quadro inicial só se o monitor o mostrava, faixa da barra de status a 960 px, três layouts,
  nenhum painel deslocado, clique atravessa o cartão, monitor conta só imagens aceitas, MSAA
  ausente fica omitido e a interação com o preview de ordem velha + refino (`+ sort`, duas
  imagens do monitor, um quadro apresentado). No head `17d73f0`: 17 testes em `perf_tests.rs` e os dois
  de interação com o refino/sort em `viewport_host_tests.rs`; `cargo test --workspace --locked` com
  `TESSEL_REQUIRE_RUNTIME=1`: 1475 passados, 0 falhas, 12 ignorados (soma das linhas `test result`;
  desktop 238, core 284), e o runtime em Chrome 148/148 sem skip.
- **Janela real, head integrado `17d73f0`** (build **debug**, `--seconds 20 --rounds 3`,
  fixture H.264 de 768 quadros importada com `media.import`): 33 checks, todos passaram. Cobre
  medidor escondido (nó sem leitura, 0 pedidos de mudança de UI em 4 s), visível em repouso,
  pausado (`parado`; ligar o medidor pausado lê repouso, sem perdas), retomada sem a pausa no pior
  intervalo (75,7 ms), `playback.resume` de reprodução rodando que mantém as contagens, três
  posições (cartão `y=8`; inspetor fechado `y=659`; faixa da barra de status), uma execução
  longa de 20 s por rodada (6 rodadas) e projeto copiado por caminho relativo: o negativo (só o
  JSON copiado) tem a mídia `missing` em `media.status`; a cópia privada resolve a mídia pela pasta
  do documento e `render.frame` desenha. Capturas próprias (`screencapture -l` do PID do processo):
  4 PNGs conferidos a olho (cartão no inspetor durante a reprodução; faixa na barra com o monitor
  maximizado). A taxa de paints da janela **não foi medida** (sem build com `--paint-probe`).
  Execuções anteriores do mesmo head que **falharam** e foram corrigidas no driver, não no
  produto (logs preservados): `media_preparing` para sempre porque o fixture usava `asset.add`
  (asset sem hash, a preparação falha com "never hashed"); e um controle negativo que passava
  pelo motivo errado (`media_preparing`) e depois desenhava por ordem de execução (cache em
  memória), agora trocado por `media.status`. Spec do driver: 27 testes (normal e `-O`).
- **Janela real, execução anterior** (mesmo script, HOME isolado, cópia do projeto em `/tmp`,
  só requisições semânticas, captura com `screencapture -l` do PID do próprio processo). Em
  build release, **head 06afa80, anterior às correções da revisão** (histórico, não é
  evidência do head atual): 17 checks, todos passaram. Em repouso, escondido ou visível,
  `ui.repaints` do `monitor.viewport` não andou em 4 s. Esse contador conta só mensagens IPC
  que mudam a assinatura da interface, **não paints da janela** (o app pinta ≈3,8 quadros/s
  em repouso): não prova que o medidor não pede quadros, só que nada pediu mudança de
  interface. Pausar é
  repouso (`parado`, sem perdas) e, ao retomar, a pausa não vira intervalo (pior intervalo
  do monitor 84 ms). Posições: padrão = cartão flutuante em `y=8` (sobre o inspetor);
  inspetor fechado = cartão na ponta direita da timeline, abaixo da imagem; monitor
  maximizado = `status_bar_strip`. O projeto real de splats (1,18 M splats por cena)
  desenhou o quadro 0 com o medidor ligado (captura `-real-project.png`). Numa execução
  anterior, em que a troca para o projeto sintético falhou por um parâmetro do próprio script
  (já corrigido), a reprodução rodou sobre o projeto real: 192 imagens aceitas em 8 s, 0
  perdidos, com o medidor desligado e ligado (monitor 23,6–24,1 fps).
- **Custo, medido ligado x desligado na reprodução** (8 s de reprodução por rodada, rodadas
  alternadas desligado/ligado, H.264 1280×720 a 24 fps, preview 960×540). O que o app já contava sem o
  medidor (relatório do `playback.stop` e `monitor.viewport`):

  | Execução | Perdidos (`skipped_frames`), média | Imagens aceitas em 8 s, média | Repaints da janela (`ui.repaints`) |
  |---|---|---|---|
  | build final, 3 rodadas | desligado 0 · ligado 0 | 190,7 · 192,0 | 182,7 · 182,0 |
  | build final, outra execução | desligado 0 · ligado 0 | 192,3 · 192,3 | 190,7 · 189,3 |
  | build final, com a correção da revisão | desligado 0 · ligado 0 | 192,3 · 192,0 | 188,0 · 187,0 |
  | build com contador de paint, 4 rodadas | desligado 1,5 (6, 0, 0, 0) · ligado 0,5 (1, 0, 0, 1) | 190,5 · 191,5 | 187,5 · 188,5 |
  | **head `17d73f0`, build debug, 3 rodadas de 20 s** (por braço, 3 rodadas) | desligado 0,33 (0, 0, 1) · ligado 0 | 479,7 · 480,3 | 449,3 · 449,0 |

  As quatro primeiras linhas são de heads anteriores (8 s, build release) e valem como histórico.
  Na linha do head `17d73f0` (debug, 20 s, 3 rodadas, `ticks` do relógio de reprodução 656,7
  desligado e 626,0 ligado, 603 a 668 entre rodadas) não há diferença em perdas, imagens aceitas
  nem pedidos de interface; a variação dos `ticks` entre rodadas é do tamanho da diferença entre os
  braços, então nada se conclui sobre custo além de "não detectado nesta amostra". O medidor leu
  perdas de 1, 0 e 0 em 482 a 484 quadros e o monitor entre 23,8 e 24,6 fps.

  A **taxa de quadros pintados da janela** com o medidor desligado só se mede com um contador
  externo: uma build local, **não commitada**, registrava cada paint da janela nos dois braços
  (`PAINT <ms>` no log). Resultado, em paints por segundo: desligado 28,6 · 49,2 · 48,6 ·
  30,0 (média 39,1); ligado 68,0 · 30,0 · 40,8 · 57,1 (média 49,0). A taxa é bimodal
  (≈30 ou ≈50–70/s) **nos dois braços**, sem relação com o medidor, e a leitura do próprio
  medidor ("interface") concordou com o contador externo (por exemplo 70,4 contra 68,0).
  **Conclusão:** nenhuma diferença detectável em quadros perdidos, imagens aceitas ou paints
  por esta amostra pequena; a variação entre rodadas, numa máquina compartilhada com outras
  compilações (carga 24–44), é maior que qualquer efeito do medidor. Não prova custo nulo
  abaixo desse ruído.
- **Achado, não causado pelo medidor:** em repouso o app pinta ≈3,8 quadros/s sozinho
  (23 paints em 6 s com o medidor escondido e 23 com ele ligado, medidos com o contador
  externo), com intervalo de 267 ms. Por isso a linha `interface` em repouso lê uma taxa
  pequena (`3,9 fps · pior 267 ms`) e não "parado". A origem desse repaint não foi
  investigada aqui.
- **Captura nativa, head `17d73f0`:** `/tmp/pr18/logs/native/shots/` (fora do repositório, efêmera).
- **Captura nativa anterior (histórico):** `/Users/wallyssonnunes/www/tessel-video-splat/previews/perf-meter-bl145/`
  (`perf-meter-playing.png`, `-default.png`, `-inspector-closed.png`,
  `-monitor-maximized.png`, `-real-project.png`), fora do repositório.

### Limites

- O GPUI de teste não pinta pixels; cores e glifos só têm a captura nativa.
- O tempo de rasterização na GPU e a apresentação real do compositor não são medidos:
  "monitor" é o `frame.done` aceito pelo host.
- Memória de GPU e o RSS dos processos WebContent/GPU do WebKit não são medidos.
- O estado ligado/desligado não persiste (o app não guarda preferências de interface).
- O atalho foi provado pelo despachante de teclas do GPUI (`simulate_keystrokes`), não por
  teclado físico; o mouse físico não foi exercitado (o botão do cabeçalho usa o mesmo handler).
- O contador de paints independente (build local) não faz parte do produto. Sem ele
  (`--paint-probe`), `perf-meter-proof.py` registra a taxa de paints como "not measured", nunca
  como zero, e a ausência de pedidos de quadro do medidor não é provada, só a ausência de
  pedidos de mudança de interface.
- A medição de custo usou uma máquina sob carga de outras compilações; repetir em máquina
  ociosa antes de afirmar mais que "sem diferença detectável".
