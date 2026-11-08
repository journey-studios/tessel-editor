# F049 — Histórico de desempenho (60 s)

## Objetivo e limites

Uma janela móvel dos últimos **60 segundos** das métricas que o [medidor de debug](perf-meter.md)
mostra só no instante: o que a interface pintou, o que o monitor apresentou, quando a fluidez
caiu e **o que aconteceu perto** (play, pausa, seek, troca de projeto, recarga do viewport, falha
de pedido). Serve para responder "por que travou há 20 segundos?" sem repetir a reprodução.
Backlog: BL145.2 (subitem de [BL145](../backlog.md)); plano em
[perf-history-execution-plan.md](../migration/perf-history-execution-plan.md).

É uma ferramenta de **diagnóstico local**: nada vai ao projeto, ao desfazer, ao autosave, à rede
ou a qualquer serviço. **Desligada por padrão**: sem gravação não existe anel alocado, e cada
ponto de coleta volta num único teste de `bool`. **Gravar e mostrar são estados separados**:
fechar o painel não para a gravação e iniciar a gravação não abre o painel.

## Contrato do core (`tessel_editor_core::perf_history`, sem GPUI)

- **Anel de 240 baldes de 250 ms** (60 s) com `count/min/soma/max` por métrica (memória fixa,
  sem amostras brutas, sem percentis). A média é **ponderada pela contagem**. A janela é
  fotografada em `now`: baldes mais velhos que 60 s saem **pelo tempo**, não por contagem, e um
  intervalo sem amostra é `null`, nunca zero.
- **Anel de eventos** à parte (1024) que informa `dropped` quando descarta; segmentos de sessão
  (64) que isolam o que não pode ser comparado.
- **Relógio injetado**: toda a API recebe um `Duration` desde a origem monotônica; os testes não
  dormem.

| Métrica | Unidade | Fonte |
| --- | --- | --- |
| `ui_frames` / `ui_interval_ms` | quadros por balde / ms | pintura efetiva do GPUI (`Window::draw`), nunca um timer |
| `monitor_frames` / `monitor_interval_ms` | quadros por balde / ms | imagem **aceita pelo host** (`frame.done` da requisição em curso) |
| `render_ms` | ms | CPU do runtime até a submissão (telemetria da própria imagem) |
| `latency_ms` | ms | da **admissão do pedido** pelo host até a aceitação da imagem do **mesmo id**; sem id casado, sem latência |
| `lost_frames` | quadros | delta do contador de perdas **por corrida** de reprodução (um seek abre outra corrida) |
| `rss_bytes` | bytes | RSS do processo do editor (sem WebKit), no máximo 1 Hz |
| `texture_bytes` | bytes | estimativa do runtime a partir do tamanho das imagens, no máximo 1 Hz; **não é memória de GPU** |

Regras de honestidade (cada uma tem teste): repouso **não** é travamento (silêncio depois de
`REST_AFTER` = 1 s sem reprodução é "parado"; com o relógio de reprodução andando, silêncio é
travamento); falha ou perda de contexto **não carregam** o último valor; resposta de sessão
antiga ou de pedido superado é descartada e contada; trocar de projeto ou recarregar o viewport
**abre um segmento** (nenhum intervalo, delta ou latência cruza a borda); latência e `render_ms`
são métricas separadas (a primeira inclui fila e readback do runtime, a segunda é só CPU).

**Eventos**: gravação iniciada/parada/limpa, play, pausa/stop, seek (seeks a menos de 250 ms
viram um evento com `repeat`), projeto trocado, **cena trocada**, viewport recarregado, mudança
de formato do quadro, pedido que falhou, **carga de recurso** (cena dirigida), **exportação de
vídeo** (início, fim, falha com o código, cancelamento) e **perda/restauração do contexto
gráfico**. Só entra o que foi **realmente observado** (tabela de ganchos abaixo); o tempo é o do
relógio monotônico do host, nunca o relógio JS. O detalhe de um evento passa por `code()`: só letras, dígitos, `_-.` e
no máximo 48 caracteres; um caminho ou um título **nunca** entra.

**Snapshot e inspeção**: `freeze` fixa **um** snapshot imutável e limitado (240×9 agregados e no
máximo 1024 eventos); a gravação continua por baixo. `select` (um instante dentro da janela)
devolve os números do balde (contagem/mín/média/máx), os extremos e os **eventos correlacionados**
(os do balde mais os que ocorreram durante a maior espera que terminou nele). Fora da janela,
erro tipado `perf_history_outside_window`.

**Export** (`tessel.perf-history`, `schema_version` 1): build, origem monotônica, intervalo da
gravação, janela, contexto não sensível, métricas com unidade/fonte/amostragem, séries por balde,
resumo, eventos, segmentos, contadores e a lista de limites. O destino é **sempre escolhido
pelo usuário** (nada é exportado sozinho, nem para o temp) e a escrita é **sem sobrescrever**:
arquivo irmão temporário (`create_new`) + `sync` + `hard_link` ao destino, que falha se algo
apareceu ali (em volume sem hard link, uma checagem final e `rename`); o temporário sempre é
removido. Erros tipados: `perf_history_empty`, `perf_history_export_path_missing`,
`perf_history_export_relative` (o caminho precisa ser absoluto), `perf_history_export_is_directory`,
`perf_history_export_exists` (um arquivo, ou um link, já está lá: o projeto ou outro arquivo
nunca é substituído), `perf_history_export_parent_missing`, `perf_history_export_failed` (só o
tipo do erro do SO, nunca o caminho). Sem caminhos, textos do projeto, credenciais nem conteúdo
de mídia: o destino digitado vive só na janela (`get` o mostra) e não entra no documento.

## Superfícies

| Superfície | Estado | Como |
| --- | --- | --- |
| Biblioteca | Implementada | `Recorder`, `Snapshot`, `HistoryCommand`; 49 testes do módulo com relógio injetado |
| CLI headless | **N/A justificado** | a medição só existe com a janela e o monitor vivos; `tessel-headless` responde `desktop_required` ao alvo `perf.history` (teste em `crates/core/tests/headless.rs`). O core funciona com entradas sintéticas e sem GPUI/wry (`cargo tree`) |
| UI da janela | Implementada | botão `toolbar-perf-history` no cabeçalho e painel (diálogo Root do Kit) com gráficos nativos |
| CLI da janela | Implementada | `get perf.history`; `set perf.history recording\|visible\|frozen\|selected_ms\|export_to\|export_destination`; `invoke perf.history record_start\|record_stop\|show\|hide\|freeze\|resume\|clear\|snapshot\|clear_selection\|export` — mesmo handler do painel (`Workstation::execute_perf_history`). `export_to <caminho>` escreve direto; `export_destination <caminho>` só preenche o campo do painel (como digitar ou escolher) e `invoke export` é o botão |

### O painel

Um diálogo **Root do Kit** com corpo em `Entity` própria (`HistoryView`): o cartão do medidor
continua transparente ao ponteiro e não ganha controles. Contém o estado ("Gravando", "Congelado",
"Parado", "Desligado"), os botões Gravar/Parar, Congelar/Retomar, Limpar seleção, Limpar
histórico e Exportar JSON (desabilitados com dica do motivo quando não há dados ou destino), o
**campo "Destino do JSON"** (entrada do Kit; "Escolher…" abre o seletor nativo de arquivo novo, que
**só preenche o caminho**), nove linhas de
gráfico (uma por métrica, **cada uma com a sua escala**: unidades diferentes nunca dividem eixo),
a linha de **eventos e sessões**, o bloco do instante selecionado (clique num gráfico) e os
contadores do que o recorder recusou (respostas obsoletas, imagens sem pedido, pedidos esquecidos,
amostras tardias, leituras de memória indisponíveis, eventos descartados).

- Um balde sem amostra **não tem barra** (lacuna); um zero é uma barra de 1 px; a faixa gravada
  aparece como pista, o intervalo anterior à gravação fica vazio.
- Cada barra é o nível do balde: taxa para contadores de quadros, soma para perdas, média para
  medidores e o pior valor para esperas.
- `Workflow::has_dialog` inclui o painel: a WebView nativa é escondida como em qualquer diálogo.
  O monitor **continua produzindo** imagens e o histórico continua gravando-as; o contexto
  registra `monitor_on_screen`. Só um diálogo por vez: abrir o painel com outro aberto é
  `dialog_open`.
- **Custo da UI**: o único timer fica dentro do painel aberto, roda no máximo a 4 Hz e só
  enquanto a gravação anda (congelado, parado ou fechado não há timer; fechar derruba a entidade
  e a tarefa). O redraw que ele causa é marcado como **do próprio diagnóstico**
  (`PerfState::own_redraw`) e não conta como quadro de UI. Se um quadro real cai no mesmo draw
  do refresh, ele é subcontado em 1: limite aceito e documentado.
- **Captura leve**: gravando com o cartão escondido, a janela só tem uma sonda de pintura de
  tamanho zero (conta quadros e lê RSS a cada ≥ 1 s); sem overlay, sem timer de repouso, sem
  readout.

## Ganchos mínimos (ownership)

| Ponto | Arquivo | O que faz |
| --- | --- | --- |
| Pedido admitido, imagem aceita (id, `render_ms`, textura, contexto), resposta obsoleta, pedido que falhou | `workstation/viewport_host.rs` | alimenta o `Meter` já anexado; `enabled()` falso retorna antes de construir qualquer coisa |
| Troca de projeto, recarga do viewport | `viewport_host.rs` (`reset_project`, `begin_reload`) | abre segmento |
| **Troca de cena** dirigida | `scene_monitor_host.rs` (`render_scene`: o id da cena mudou ou entrou-se numa cena), `viewport_host.rs` (`leave_directed`, chamado por `execute` ao escolher um preview comum) | um segmento `scene_changed` e um evento por troca; um quadro novo da **mesma** cena não abre nada. O id é o número do segmento e o relógio do host: o nome da cena nunca entra |
| Telemetria do monitor dirigido | `scene_monitor_host.rs` (`dispatch_directed`, `finish_directed`) | admissão do pedido (id do adaptador, ≥ 2⁴⁸, nunca colide com o da sequência), imagem aceita com `render_ms`/textura/contexto, falha com id. Latência = do pedido admitido (inclui esperar recursos) até o `frame.done` do mesmo id |
| Carga de recurso (cena dirigida) | `scene_monitor_host.rs` (`prepare_directed`, `directed_asset_state`) | `resource_load_started` ao enviar cada `asset.load`; `resource_load_finished` com detalhe `ready` ou o código da falha; respostas obsoletas não contam |
| Perda e restauração do contexto | `core::perf_history` (`request_failed` com `context_lost`), `viewport_host.rs` (`drain`: `needs_redraw`) | um `context_lost` por perda (o aviso da página e o quadro que falhou são a mesma perda) e um `context_restored`; não é também um `request_failed` |
| Exportação de vídeo | `workstation/perf_meter.rs` (`observe_exports`, na sonda de pintura, ≤ 1 Hz) | transições de jobs `VideoExport` do registro: início, fim, falha (código), cancelamento. A primeira olhada da gravação é só a linha de base: o que já rodava não vira notícia; um job visto já terminado só tem o fim. O tempo é o da olhada (nunca anterior ao fato) |
| Taxa-alvo do contexto | `viewport_host.rs` (`target_fps`: `request_preview`, `render_scene`) | `sequence_target_fps` = taxa da sequência (preview de sequência) ou o `fps` do bundle da cena; `null` para preview de fonte e antes do primeiro pedido. `ui_target_fps` continua `null`: não há alvo conhecido e nunca se assume 60 |
| Seek intencional (`execute(SetPlayhead)`, `playback_seek`) | `workstation/mod.rs`, `playback.rs` | evento `seek`; os ticks do relógio de reprodução não são seek |
| `has_dialog` | `workstation/workflow.rs` | `history_open` |
| Botão do cabeçalho | `header.rs` | mesmo `execute_perf_history` |
| Alvo `perf.history` | `automation/server.rs`, `core::commands` | `Parsed::PerfHistory` |

Nenhum arquivo de FX, `scene_fx`, `scene_ui`, `scene_monitor` ou do viewport de outros agentes
foi alterado além de `viewport_host.rs` (feeds do host, diff mínimo) e de visibilidade `pub(super)`
nos helpers de `viewport_host_tests.rs`.

## Lacunas declaradas

- **Fontes do monitor da cena dirigida**: agora há admissão, imagem aceita (com `render_ms`,
  textura e contexto), falha e eventos de recurso. A latência inclui a espera dos recursos
  (a admissão acontece quando o adaptador aceita o pedido), diferente da sequência, onde os
  recursos já estão carregados; compare os dois com isso em mente. Nada disso é GPU concluída.
- `resource_load_*` só existe na cena dirigida: a sessão da sequência (`core::session`) não
  expõe o início/fim de carga de recurso ao host e **não foi alterada** para isso (é código
  compartilhado com o headless). Fica registrado, não inventado.
- O evento `export_*` vem de olhar o registro de jobs a cada ≤ 1 s numa pintura: uma janela sem
  nenhuma pintura durante a exportação só a vê na próxima pintura (o horário é o da olhada).
  Exportações de cena em processo separado que não passam pelo registro de jobs não aparecem.
- O RSS é amostrado nas pinturas: uma janela completamente parada deixa lacuna (`null`), que é a
  leitura honesta; **nunca** se preenche com o valor antigo.
- O painel é modal (diálogo Root): enquanto ele está aberto, a WebView está escondida; não dá
  para olhar o monitor e o histórico ao mesmo tempo.
- `ui_target_fps` é `null` (a interface não tem alvo conhecido; o histórico não assume 60).
- Overhead: 3 rodadas de 8 s sob carga externa não provam custo zero (veja a seção de custo).

## Evidências e status

Veja o estado vivo em [backlog.md](../backlog.md) (BL145.2). Resumo do que foi executado e do que
ainda é "a validar" está na seção de evidências abaixo, atualizada com a entrega.

### Testes

- `cargo test -p tessel-editor-core`: módulo `perf_history` (relógio injetado: anel e evicção por
  tempo, lacunas, repouso x travamento, média ponderada, latência por id, perdas por corrida,
  RSS ≤ 1 Hz, eventos e `dropped`, segmentos, freeze/select/resume, export e erros tipados,
  limite de memória após minutos simulados), `core::perf` (cartão e gravação independentes),
  `core::commands` (parser único de `set`/`invoke`) e `state::tree` (nó `perf.history`).
- `cargo test -p tessel-editor` (GPUI, relógio controlado):
  `workstation::perf_history_tests` — desligado por padrão e sem custo; gravar com o cartão
  escondido não arma timer; cartão e gravação independentes; painel esconde o monitor sem parar
  nem iniciar a gravação; snapshot congelado imutável com seleção dentro da janela; export local
  versionado sem caminhos e erros tipados; `set`/`invoke` no mesmo handler; seek como evento;
  refresh de no máximo 4 Hz que só existe enquanto a gravação anda e cujo redraw não conta; feeds
  do host (latência, resposta obsoleta, segmentos) e host sem gravação.
- `scripts/perf-history-proof.py` (+ `.spec.py`): prova na janela real; veja abaixo.

### Prova na janela real (executada na 1ª rodada; a 2ª está a validar)

**Escopo desta seção:** o que abaixo foi executado vale para o código de `2560bbf` (1ª rodada).
A 2ª rodada (destino de export explícito sem sobrescrita, `scene_changed`, telemetria do monitor
dirigido, eventos de recurso/exportação/contexto, taxa-alvo do contexto) foi **escrita, mas ainda
não compilada nem provada na janela real**: `perf-history-proof.py` já cobre o campo de destino
(`export_destination` + `invoke export`, recusas e uma captura), porém não rodou. A cena dirigida
e a perda de contexto reais também não foram provocadas numa janela; ficam cobertas só pelos
testes (quando rodarem) e **a validar**.

`TESSEL_RUNTIME_DIR=/tmp/phr python3 scripts/directed-scene-proof/with-gpu-lock.py python3
scripts/perf-history-proof.py --binary <release> --seconds 8 --rounds 3 --buffer-minutes 3`, em
instância própria (home temporária), só com requisições semânticas (`perf.history`, `playback.*`,
`theme.*`). Resultado: `ok: true`, 0 falhas, cobrindo: desligado por padrão e sem dado; gravar com
o cartão escondido (as 5 métricas com amostra e o monitor continua na tela); latência separada de
`render_ms`; o painel esconde a WebView como qualquer diálogo e abrir/fechar não muda a gravação;
congelar fixa um snapshot, selecionar fora da janela é erro tipado, retomar volta à janela móvel;
export `tessel.perf-history` v1 sem caminhos locais, nulos preservados e erros tipados (diretório,
pasta ausente); limpar esquece o histórico.

Capturas inspecionadas (painel congelado com seleção): tema
[escuro](assets/f049-history-frozen-inspect-dark.png) e
[claro](assets/f049-history-frozen-inspect-light.png). Os gráficos nativos usam os tokens do tema
(sem hex solto); as barras ocupam só o trecho já gravado e o resto da janela de 60 s fica vazio,
não preenchido.

### Custo e buffer (executado, com ressalva)

Média de 3 rodadas de 8 s de playback, ordem dos braços alternada, build release, mesmo clipe de
1280x720, CPU do processo em s/s: **desligado 0,302**, **gravando com painel escondido 0,290**,
**painel visível 0,353**. Os ticks, imagens aceitas e redraws foram equivalentes nos três braços
(sem quadros pulados). Leitura honesta: gravar escondido ficou dentro do ruído do desligado; o
painel visível custou cerca de 0,05 s/s a mais (nove gráficos a 4 Hz), custo de diagnóstico
aceito enquanto o painel está aberto. **Ressalva:** a máquina estava com load average 14–21
(builds e editores de outros agentes), então a diferença de ~17% do braço visível e a ausência de
diferença do escondido não são medida de laboratório; 3 rodadas não dão intervalo de confiança.

Buffer de 3 minutos com gravação contínua: 240 baldes (cheios) o tempo todo; eventos expiram por
tempo (5 → 0 retidos) e `dropped` ficou 0; RSS 101 MiB aos 20 s e 92 MiB aos 180 s (variação
−14,8 MiB após o aquecimento, sem crescimento). Não foi feita medição de 10+ minutos: **a
validar**.

### Gates

Executados com sucesso nesta entrega:

- `cargo fmt --all -- --check` (saída 0).
- `./scripts/cargo build -p tessel-editor --release --locked` do estado final do código
  (usado nas provas acima).
- `./scripts/cargo test -p tessel-editor-core perf_history` (51 testes) e o conjunto do core
  (421 testes) no estado anterior aos últimos ajustes de layout do painel; os 11 testes de
  `workstation::perf_history_tests` passaram no mesmo ponto.
- `python3 scripts/perf-history-proof.spec.py` (4 testes).
- `cargo tree -p tessel-editor-core`: nenhuma ocorrência de `gpui` ou `wry`.

**2ª rodada (sem build desde a 1ª):** todo o código e os testes novos foram só formatados
(`cargo fmt --all`); nenhum `cargo check/clippy/test/build` foi executado depois deles. Nenhum
teste novo foi rodado, e os números "51 testes do módulo" / "11 testes do desktop" acima descrevem
a 1ª rodada (a 2ª adiciona testes a ambos).

**Não executados com sucesso nesta rodada (a validar):** `./scripts/cargo check --workspace
--locked`, `./scripts/cargo clippy --workspace --all-targets --locked -- -D warnings`,
`./scripts/cargo test --workspace --locked` e `./scripts/cargo build -p tessel-editor --locked`
(debug) sobre o estado final. Os quatro foram **recusados pela guarda de disco**
(`REFUSED_DISK`: 11 GiB livres com piso de 12 GiB, ocupado por builds de outros agentes) e não
foram forçados nem houve limpeza de cache. Nenhum deles é declarado PASS; a CI da PR ou uma nova
rodada com disco livre deve cobri-los.
