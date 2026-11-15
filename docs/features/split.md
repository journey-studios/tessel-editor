# F017 — Dividir clipe / corte interno

## Status

**BL081.2 (cortar dentro de fades) validado tecnicamente no Mac e Windows** — ver [clip-split-fades-report.md](../migration/clip-split-fades-report.md);
não há mais recusa por fade. **BL081 e continuidade fracionária BL081.5 validados Windows/Mac, dentro dos limites abaixo.** Entregue em biblioteca, CLI headless (processo JSONL) e janela. Evidência e limites em
[clip-split-report.md](../migration/clip-split-report.md). Sem ponteiro físico, sem áudio ouvido e sem pixels: a prova da
janela é semântica (`ui get/set/invoke/op`).

## Contrato

Corta **um clipe da sequência ativa** num **frame absoluto** da sequência. A regra vive em `crates/core/src/state/split.rs`
(`EditorState::plan_split` valida tudo sem tocar o estado; `split_clip` aplica um plano que não pode falhar). Lib, headless e
janela chamam o mesmo `Command::SplitClip`.

| Entrada | Tipo | Regra |
|---|---|---|
| `clip` | ID do clipe | tem de existir na sequência ativa |
| `frame` | inteiro, frame **absoluto** | estritamente dentro do clipe: `start < frame < start + duration` |
| `new_id` | texto, opcional | ID do clipe da direita |

Op JSON: `{"op":"clip.split","clip":"title-north","frame":10,"new_id":"title-north-b"}`. Também
`invoke timeline.split invoke` (usa o clipe selecionado e o playhead; só a **UI** depende da seleção, a op não).

### Efeito

- Esquerda: mantém o ID, `start` e a faixa; `duration = frame − start`.
- Direita: novo ID, mesma faixa e mesmo asset, `start = frame`, `duration = end − frame`; entra logo depois da esquerda na ordem
  de `clips`. Duas durações positivas cobrem o clipe original exatamente (sem lacuna nem sobreposição).
- `source_start` da direita = `source_start + (frames_antes / fps) × velocidade`, em `Ratio` com aritmética checada (sem `f64`).
  Vale para taxa racional (30000/1001) e velocidade ≠ 1.
- Propriedades do inspetor (texto, zoom, opacidade…) são copiadas para a direita. Câmera: ver abaixo.
- A direita fica **selecionada** (`selected_media` e assunto = clipe). A seleção é estado de sessão: undo/redo **não** a restauram,
  apenas descartam a que deixou de existir.
- Desfazer/refazer: **uma** entrada de histórico ("Split clip"). Salvar e reabrir preserva os dois clipes.

### Política do ID

- `new_id` explícito: validado por `Id::validate` (as mesmas regras de `Id::new`, sem internar). Duplicado (clipe, track ou clipe
  de qualquer sequência) → `duplicate_id`; inválido → `invalid_id`.
- Sem `new_id`: `<clip>-split-<n>`, o menor `n` ≥ 1 livre em faixas, clipes e todas as sequências (determinístico). Se o nome
  padrão não cabe em `MAX_ID_LEN`, erro `invalid_id` (não trunca): passe `new_id`.
- Recusa não consome ID, revisão nem histórico.

### Continuidade

Cada frame da sequência lê, depois do corte, o mesmo tempo de fonte, câmera, zoom e opacidade que lia antes.

| Caso | Regra |
|---|---|
| Câmera orbital (sem chaves) | a direita começa onde a órbita já estava: `camera.at(shift)` |
| Câmera com chaves | ancorada no tempo absoluto da cena: nada a mudar |
| Ken Burns (só imagem estática, `ken_burns` ligado, `end_zoom ≠ zoom`) | esquerda termina em `z0 + (z1−z0)(k−1)/(D−1)`; direita começa em `z0 + (z1−z0)k/(D−1)` e mantém `end_zoom`. Valores gravados **sem arredondar** |
| Fade-in | rampa que não alcança a direita: a esquerda mantém o fade e a direita fica com 0. Rampa que **atravessa** o corte (BL081.2): as duas metades leem a mesma janela de fade (`fade_window`), e cada quadro mantém a mesma opacidade |
| Fade-out | simétrico: a direita mantém e a esquerda fica com 0 quando a rampa não a alcança; se atravessa, a janela é conservada nas duas |
| Fades sobrepostos / maiores que a metade (áudio) | cada rampa que a metade herda é fixada com a âncora e o vão em que o mixer a encurtou no clipe inteiro; editar o outro fade não a encurta |
| Áudio (BL081.5) | Mesmas amostras antes/depois do corte sob o relógio absoluto e fase racional, inclusive nas grades fracionárias; validado Windows/Mac, ver relatório |

### Erros tipados (atômicos)

Nenhum erro altera clipe, propriedade, seleção, revisão, histórico ou consumo de ID.

| `code` | Quando |
|---|---|
| `unknown_element` | clipe inexistente |
| `out_of_bounds` | `frame < start` ou `frame > end` |
| `invalid_duration` | `frame == start` ou `frame == end` (metade vazia) |
| `invalid_id` / `duplicate_id` | `new_id` inválido/duplicado, ou nome padrão que não cabe |
| `invalid_time` | estouro de aritmética checada (frames ou tempo de fonte) |
| `invalid_request` | parser: `frame` fracionário/não inteiro, tipos errados |
| `disabled_element` | `invoke timeline.split` com o controle desabilitado |

### Fades que atravessam o corte (BL081.2, BL081.2.2)

`Clip.fade_window` (`crates/core/src/state/envelope.rs`) guarda, por rampa (`lead` = fade-in, `tail` = fade-out), em quadros **relativos às
bordas do clipe**: a **âncora** (`frames`: onde começou a rampa de entrada / onde termina a de saída), a guarda do valor do fade para o
qual foi cortada (`fade`) e, para áudio, a **normalização** (`norm.far`, `norm.peer`: o vão em que o mixer encurtou a rampa e o pedido
da outra rampa nesse vão). Opacidade (`plan.rs`) mede cada fade a partir da sua âncora; o mix (`plan_audio`) ancora cada rampa e roda o
`resolve_fades` de uma rampa fixada no vão congelado dela, nunca no vão de uma janela comum. Sem fixação a conta é a de antes, bit a bit.

Uma rampa que não alcança a metade continua sendo zerada (sem dado novo). Estouro aritmético recusa o corte com `invalid_time`,
atomicamente. **Editar `fade_in`/`fade_out` numa metade redefine só aquela rampa** (medida da borda da metade); a outra rampa herdada
conserva forma, posição e normalização, também quando o clipe inteiro a encurtava por sobreposição. Aparar a borda além da origem da
rampa descarta a âncora daquele lado; aparar menos a mantém no lugar da sequência, e o vão de normalização também não muda. O campo é
salvo em `fade_window` do clipe e exige schema 3 (um clipe de áudio com rampa fixada sem `norm`, ou de imagem com `norm`, é
`invalid_document`); o resultado do split traz `fade_window.left/right`. Regras completas, operações e evidências:
[clip-split-fades-report.md](../migration/clip-split-fades-report.md).

### Áudio em grade fracionária (BL081.5 — validado Windows/Mac)

A recusa `audio_sample_grid` e `SplitBlock::AudioSampleGrid` foram removidos nesta
implementação. O relógio do core usa `S(f)=round(f·den·48000/num)` como fronteira
absoluta do quadro; cada amostra de saída lê posição racional da fonte a partir de
`source_start`, início do clipe, velocidade e offset do stream. O cursor mantém a
fase fracionária por somas inteiras, sem reiniciá-la quando um trecho é dividido.
BL081.5 não persiste a fase da fonte nem muda tempo visual/câmera; a expansão BL081.2 acrescenta o campo opcional fade_window para o envelope.

Mix em blocos e faixas parciais deve coincidir bit a bit com a mesma faixa do
inteiro. Fades externos usam diferenças no mesmo relógio; cortar dentro de rampa é aceito (BL081.2, `fade_window`). Mute não é um bypass: o modelo também deve manter o
som quando reativado. Em grades fracionárias o PCM anterior era arredondado por
segmento e pode mudar com a correção; preservar esse erro antigo não é o contrato.
Grades inteiras legadas têm controles bit a bit. `AudioSegment` ganha fase racional
e velocidade exata; o JSON conserva `speed` numérico e acrescenta `speed_exact`.

Contrato detalhado, controles e limites na [entrega BL081.5](../migration/clip-split-audio-continuity-report.md).

## Superfícies

- **Lib/headless:** `Command::SplitClip` / op `clip.split`; `./scripts/headless` executa a sessão JSONL sem janela.
- **UI:** botão do Kit "Cortar clipe no playhead" (ícone `Slice`) na barra da timeline, no lugar do antigo placeholder da navalha.
  Habilitado só quando `split_availability()` é `Ready`; desabilitado traz `disabled_reason` coerente.
- **CLI da janela:** nó `timeline.split` (`icon_button`, ação `invoke`), `state.playhead/clip/target/frame`; `enabled` e
  `disabled_reason` vêm da mesma regra que o botão e o `invoke`.

## Limites conhecidos

- Só a sequência ativa; clipes de outras sequências não são cortados por esta op.
- Cortes dentro de um fade conservam o envelope por `fade_window` (BL081.2); mais de um corte seguido conserva a âncora e a normalização do clipe original, e editar um fade da metade não move a outra rampa (BL081.2.2). Sem motor de envelopes, keyframes ou crossfade.
- O Ken Burns e a órbita são recalculados em `f64` (propriedades do inspetor são `f64`); o tempo de fonte e a divisão do tempo
  são exatos.
- A direita herda o nome do clipe (não é renomeada).
- Sem atalho de teclado, sem corte por clique na timeline e sem corte em lote (a op é por clipe).
- Selecionar de volta o clipe original depois de undo/redo não é automático (seleção é de sessão).

## Evidências

Ver [clip-split-report.md](../migration/clip-split-report.md): `crates/core/tests/clip_split.rs` (23 testes: lib e processo
`tessel-headless`), `crates/core/tests/clip_split_envelope.rs` e `clip_split_envelope/` (BL081.2 + BL081.2.2, 44 testes: planos e PCM bit a bit, controles, rampas herdadas por fade editado), testes de unidade (`id`, `inspector`, `tree`), `crates/desktop/src/workstation/split_tests.rs` (5 testes
GPUI com clique simulado, incluindo o corte dentro de um fade) e a prova na janela própria por `tessel-editor ui …`.

Revisão Mac: uma recusa preserva também o gesto de slider aberto e seu histórico;
a validação ocorre antes de liquidar o gesto. Evidências finais no relatório.

Revisão da [PR #57](https://github.com/journey-studios/tessel-editor/pull/57), BL081.13:
regressão de fade visual fracionário carregado pelo documento, recusa atômica e
controle de disponibilidade compartilhado com a UI; cortes aceitos conservam todos
os planos, e o controle de áudio mantém o PCM bit a bit. Validação dessa correção
no Mac; não foi repetida janela nem execução Windows para esse ajuste.

Gate Mac dessa correção: core completo **1456 passam, 0 falhas, 7 ignorados
existentes**; split23/fades8/fase14 e desktop split4 passam. fmt, check de todos
os targets, clippy, compilação de todos os testes e build desktop: exit0.

BL081.14 (revisão PR #57): áudio cujos fades somados excedem a duração recebe
`SplitBlock::FadesLongerThanClip` e motivo para encurtar o fade, sem sugerir mover
o playhead. O nó, invoke e tooltip da janela usam essa causa compartilhada. O
oráculo de PCM rejeita NaN/Inf nos dois canais e a prova desktop confere também
o número de amostras antes da identidade bit a bit.

BL081.15: `split_unsupported.details.fade_in`/`fade_out` mantêm JSON inteiro
quando o fade é inteiro (inclusive áudio); só fades visuais fracionários usam
JSON fracionário. Regressões distinguem `as_i64` e `as_f64` na resposta.

BL081.15 validado Mac: split23/23 e fmt/check/clippy/no-run/build exit0; sem
nova janela ou execução Windows. O teste compara o tipo JSON além do valor.

BL081.2 (validado Mac/Windows): `split_unsupported`, `SplitBlock::FadeCrossesCut` e `FadesLongerThanClip` foram removidos;
o JSON inteiro de BL081.15 não tem mais onde aparecer porque não há recusa por fade. Evidências no relatório do BL081.2.


## Ajustes finais da revisão BL081.2

Rampa herdada ancorada na própria borda (frames=0) acompanha essa borda ao aparar,
como uma rampa nativa; só âncoras que já ficam fora da metade permanecem no lugar
da sequência. A outra rampa herdada permanece independente. A prévia do trim e
a execução conferem o mesmo limite de janela. O nó clip:<id> expõe fade_window
(nulo sem janela), incluindo a normalização das rampas de áudio.

Fixação conservadora: um corte de áudio fora da rampa também pode criar janela
e elevar o documento a schema3 em metades curtas (ex.: clipe60, fade-in5, corte30).
Isso preserva a rampa se o outro fade for editado até48quadros, sem renormalizá-la;
metades suficientemente longas dispensam janela e mantêm schema1/2. É uma
limitação intencional da representação, não uma afirmação de compatibilidade
com leitores anteriores. A revisão da implementação e o aceite Windows são distintos.

Prova funcional reproduzível: [scripts/split-fades-proof.py](../../scripts/split-fades-proof.py),
com argumentos REPO e OUT, após build de tessel-headless e tessel-editor. Sem
coordenadas, desempenho, pixels ou áudio ouvido.


Validação final BL081.2: commit local8901553; Mac core1587 e desktop516,
prova headless12/12 e janela12/12. Windows envelope44/44 e GPUI split5/fades2;
headless16/16 e janela27/27, 14 WAVs iguais entre superfícies. Leitor antigo
real recusa schema3 (unsupported_schema found3/supported2). No Windows, o
harness foi o equivalente PowerShell preservado nos logs, sem executar o
Python versionado. Sem pixels, ponteiro, áudio ouvido ou desempenho. Ver
[fechamento técnico](../migration/clip-split-fades-report.md#10-fechamento-técnico-macwindows).
