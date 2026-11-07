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
- `PlaybackLoss`: quadros do relógio de reprodução que o monitor não apresentou
  (`perdidos de N (x%)`, `atraso`). Só existe durante a reprodução; fora dela é
  "só em reprodução".
- `Meter`: não registra nada enquanto desligado (`samples() == (0, 0)`,
  `next_wakeup() == None`).
- `Readout` (`rows()`, `strip()`, `to_json()`), `FrameContext::of(&FrameResult)`.
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

Repouso: sem movimento, a linha lê `parado`. Valor indisponível lê `indisponível` (por exemplo,
o monitor sem runtime).

### Não distorce a medição

- **Escondido: custo zero.** Nenhum elemento é adicionado à árvore, nenhum timer existe,
  nada é gravado e nenhuma leitura é calculada; cada feed volta num único teste de `bool`.
- **Visível:** o texto é recalculado **no máximo a cada 250 ms (4 Hz)**, só a partir de
  quadros que já estavam sendo pintados, nunca por pedido do medidor. Não há temporizador
  contínuo. O único timer é de disparo único, armado depois de um paint, que acorda uma vez
  para o redraw de "parado" e não rearma em repouso (a janela volta a dormir).
- Não pede quadros ao monitor, não dispara refino e não invalida cache: ele só **lê** o que o
  host já recebeu.
- O contador de UI vive no paint, não num timer: o teste
  `the_ui_rate_counts_frames_the_window_paints_and_not_timer_ticks` pinta 5 quadros, deixa o
  relógio e o timer correrem, e confere que continuam 5.

### Persistência

O app **não guarda preferências de interface** (nem o tema nem o layout do Dock sobrevivem ao
reinício), então o estado ligado/desligado também não persiste. Não foi criado um sistema de
preferências para isso.

## Evidências e status

Implementado e validado nas superfícies que se aplicam (UI e CLI da janela; lib com regras
testadas sem GPUI; headless N/A). Não validado: ver [limites](#limites).

- **Core:** 30 testes em `core::perf` (relógio falso: taxa, pior intervalo, repouso x parada,
  perdas, janelas, `place`), mais parser, nó de árvore e `desktop_required` no headless.
- **GPUI (geometria, não pixels):** `crates/desktop/src/workstation/perf_tests.rs` (12) e três
  testes de alimentação em `viewport_host_tests.rs`: atalho único e sem conflito, alterna com
  qualquer foco, o nó e o atalho compartilham estado, escondido não grava nada, a UI conta
  quadros pintados e não ticks, 4 Hz, três layouts, nenhum painel deslocado, clique atravessa
  o cartão, monitor conta só imagens aceitas, MSAA ausente fica omitido.
- **Janela real:** `scripts/perf-meter-proof.py` (HOME isolado, cópia do projeto em `/tmp`,
  só requisições semânticas, captura com `screencapture -l` do PID do próprio processo). Em
  build release: 15 checks, todos passaram. Em repouso, escondido ou visível, a janela fez
  **0 repaints em 4 s**. Posições: padrão = cartão flutuante em `y=8` (sobre o inspetor);
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
  | build com contador de paint, 4 rodadas | desligado 1,5 (6, 0, 0, 0) · ligado 0,5 (1, 0, 0, 1) | 190,5 · 191,5 | 187,5 · 188,5 |

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
- **Captura nativa:** `/Users/wallyssonnunes/www/tessel-video-splat/previews/perf-meter-bl145/`
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
- O contador de paints independente (build local) não faz parte do produto.
- A medição de custo usou uma máquina sob carga de outras compilações; repetir em máquina
  ociosa antes de afirmar mais que "sem diferença detectável".
