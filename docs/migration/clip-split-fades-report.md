# BL081.2 — cortar dentro de fades preservando o envelope

Base: `main` e3c20cb7c8d3e32692719a9c71adb1b65eb3c392 (PR #57 + P4b de outra frente). Executor único,
worktree T3 limpo na base. Este relatório começa pelo **contrato** (escrito antes do código) e termina
com as evidências; o backlog e a matriz comuns são atualizados só pelo coordenador (ver "Propostas").

> **Atualização BL081.2.2:** a seção 1 descreve o modelo inicial (janela `[start − lead, end + tail)` medida por um único
> `resolve_fades`). Para **áudio** ele foi substituído pelo modelo de rampas independentes da seção 6; imagem/vídeo seguem como na seção 1.

> As seções1–5 registram a proposta e as provas iniciais; a seção6 substitui o modelo de áudio.
> As correções finais de trim/compatibilidade estão registradas na seção8. Os gates antigos não validam os ajustes posteriores.

## 1. Contrato mínimo do envelope após o corte

### Problema

`plan_split` copiava o fade externo para a metade que o mantém e zerava o interno, e **recusava** o corte
quando uma rampa atravessava o ponto (`fade_in_crosses_cut`, `fade_out_crosses_cut`) ou quando dois fades
de áudio se sobrepunham (`fades_longer_than_clip`). Nenhuma propriedade por clipe consegue dizer "esta
rampa começou 30 quadros antes de mim": o fade é medido nas bordas do próprio clipe.

### Representação (persistível, opcional, sem dado derivado obsoleto)

Cada `Clip` ganha `fade_window: FadeWindow` (vazio por padrão = **comportamento atual, bit a bit**):

```text
FadeWindow { lead: Option<Reach>, tail: Option<Reach> }
Reach      { frames: i64 (> 0), fade: f64 (>= 0) }
```

- `lead.frames`: quadros da sequência entre a **origem da rampa de entrada** (onde o ganho/opacidade é 0)
  e o primeiro quadro do clipe. `tail.frames`: quadros entre o fim do clipe e o **fim da rampa de saída**.
  Os dois são **relativos às bordas do clipe**, nunca absolutos: mover não os envelhece.
- Janela de fades do clipe = `[start − lead, end + tail)`. Opacidade (`plan.rs`) e mix (`plan_audio`)
  medem **ambos os fades nessa janela** em vez de nas bordas do clipe. Sem janela, a janela é o próprio
  clipe e a conta é a de antes.
- `Reach.fade` é a **guarda**: o valor da propriedade `fade_in` (lead) / `fade_out` (tail) para o qual a
  janela foi cortada. Um lado só vale enquanto a propriedade for exatamente a guarda; se divergir, o lado é
  ignorado na leitura e **removido** no próximo comando (varredura), de modo que editar o fade e voltar ao
  valor antigo cria uma rampa nova, não ressuscita a herdada.
- Persistência: `"fade_window": {"lead": {"frames": n, "fade": x}, "tail": {...}}` no clipe, só quando
  não vazio. Documento com janela exige schema 3 (um leitor v1 a descartaria e mudaria o som/imagem).
  Documentos antigos não têm o campo e abrem como antes.

### Divisão (`clip.split`)

Sejam `before`/`after` os quadros de cada metade, `lead0`/`tail0` a janela viva do clipe (0 sem janela),
`W = lead0 + before + after + tail0`, `fi`/`fo` os fades (áudio: arredondados a quadros, como o mixer).

1. **Rampa que não alcança a metade ainda é zerada** (comportamento já aceito, sem janela): a direita zera
   `fade_in` se `lead0 + before ≥ fi`; a esquerda zera `fade_out` se `after + tail0 + 1 ≥ fo` (imagem) /
   `after + tail0 ≥ fo` (som). Igual ao critério antigo de aceitação.
2. **Rampa que alcança a metade fica com a janela** em vez de ser recusada: esquerda
   `lead = lead0, tail = tail0 + after`; direita `lead = lead0 + before, tail = tail0`. As duas metades
   leem exatamente a mesma janela do clipe inteiro; cada quadro/amostra reproduz o mesmo valor.
3. **Áudio com fades sobrepostos** (`fi + fo > W`, o mixer os encurta proporcionalmente) e **áudio cujo fade
   excede a metade** (o mixer limitaria a rampa ao clipe): as metades **nunca** zeram nada e ambas ficam
   com a janela completa; o `resolve_fades` roda sobre a janela, como no inteiro.
4. Cada metade só recebe janela se ela for necessária (não é "equivalente a medir nas bordas"); caso
   contrário o resultado é o de hoje (campo vazio, sem dados novos).
5. Transbordo aritmético recusa o corte inteiro e atomicamente (`invalid_time`, sem alterar nada). Não há
   mais `split_unsupported` por fade: não resta caso de fade não representável. `SplitBlock::FadeCrossesCut`
   e `FadesLongerThanClip` deixam de existir.

### Operações depois do corte (uma regra por operação)

| Operação | Regra |
|---|---|
| mover / trocar de faixa | a janela é relativa: nada a recalcular; a forma do envelope viaja com a metade |
| trim da borda | a rampa herdada **fica no lugar da sequência**: `lead += novo_start − start`, `tail += end − novo_end`; quando o lado chega a ≤ 0 a borda passou da origem da rampa e o lado some (o fade volta a ser medido na borda, como em qualquer clipe). Um clipe sem janela continua tendo o fade nas bordas |
| trocar a velocidade | sem efeito: o envelope é em quadros da sequência, a velocidade só muda a posição na fonte (teste prova imagem e som) |
| editar o fade na metade (`fade_in`/`fade_out`, slider, reset) | o fade passa a ser **da metade**: a rampa herdada daquele lado é descartada e o valor novo é medido na borda da metade. O outro fade e o outro lado ficam como estão |
| zerar o fade (`0`) | sem rampa naquele lado; o lado herdado é descartado |
| editar de volta ao valor da guarda | **não** ressuscita a rampa herdada (a varredura já a descartou ao primeiro valor diferente); dentro de um mesmo gesto de slider, cancelar restaura a rampa porque a varredura só roda ao concluir |
| novo corte numa metade | usa a janela viva dessa metade (`lead0`, `tail0`): cortes sucessivos conservam o envelope do clipe original |
| undo / redo / salvar / reabrir | o campo vive no `Clip` (snapshot e documento); a varredura roda ao final de todo comando e ao carregar |
| carga de documento | `frames` inteiro `> 0`, `fade` finito `≥ 0`, `lead + duration + tail ≤ 1 000 000`; fora disso `invalid_document` (nada é aberto) |

Não há motor de envelopes, keyframes, crossfade entre clipes, transição P4b, batch, seleção múltipla ou atalho.

## 2. Implementação

Base `e3c20cb7c8d3e32692719a9c71adb1b65eb3c392`; resultado é um diff **não commitado** (sem staging, commit, push, PR ou
merge: só o coordenador integra). SHA-256 do diff de `crates` + `docs` (exceto este relatório), na árvore entregue:
`d1806e58ebe526398a50d09cc5c7f97348a56e2d709540dffac20b46373615c8`; `envelope.rs` `21b7f5d2…fb129`,
`clip_split_envelope.rs` `5f165162…9ab2` (arquivos novos, fora do `git diff`).

### Arquivos

| Arquivo | Mudança |
|---|---|
| `crates/core/src/state/envelope.rs` (novo) | `FadeWindow`/`Reach`/`Extent`, `Clip::fade_bounds`, `split_fades` (decisão pura do corte), JSON, `trimmed`, varredura `sweep_fade_windows`, 7 testes de unidade |
| `crates/core/src/state/split.rs` | `plan_split` usa `split_fades`; sem `split_unsupported` por fade; plano com `left_window`/`right_window`; resultado `fade_window.{left,right}` |
| `crates/core/src/state/timeline.rs` | campo `Clip.fade_window`; `trim_clip` desloca a janela (erro `invalid_time` em estouro) |
| `crates/core/src/plan.rs` | opacidade e `plan_audio` medem os fades em `fade_bounds` |
| `crates/core/src/document.rs` | `fade_window` no JSON do clipe, schema 3, rejeição em documento v1/v2, validação ao carregar |
| `crates/core/src/state/mod.rs` | **fora da lista de ownership, explicado:** `pub mod envelope`, campo no literal de `add_clip` e a varredura ao fim de `apply()` (sem gesto aberto). Sem isso uma edição de fade deixaria uma janela obsoleta |
| `crates/core/src/state/property_gesture.rs` | **fora da lista, explicado:** a varredura roda também ao concluir um gesto de slider (durante o gesto ela é pulada para que cancelar restaure a rampa) |
| `crates/desktop/src/workstation/timeline_panel.rs` | removidos os motivos `FadeCrossesCut`/`FadesLongerThanClip` |
| `crates/desktop/src/workstation/split_tests.rs` | novo teste GPUI do corte dentro de fade (clique, `invoke`, undo/redo, PCM bit a bit) |
| `crates/core/tests/clip_split_envelope.rs` (novo) | 22 testes de aceitação (lib + processo `tessel-headless`) |
| `crates/core/tests/clip_split.rs`, `clip_split_audio_phase.rs`, `clip_split_audio_phase/history.rs` | os três testes de recusa viraram testes de aceitação (planos/PCM iguais) |
| `docs/features/split.md`, `docs/features/audio-fades.md` | contrato atualizado (sem recusa por fade; janela; operações) |

`crates/media/src/export.rs` **não** precisou mudar: o export consome `plan_audio`, que já carrega a janela.

### Invariantes (cada um com teste)

1. Sem janela, plano e PCM são os de antes, bit a bit (`a_cut_outside_the_ramps_is_what_it_always_was`, documentos antigos).
2. Duas metades leem, para cada quadro/amostra, o valor do clipe inteiro: planos iguais (opacidade/ganho/offset/clip_frames) e PCM
   WAV igual por bits, nas grades 24/25/30/60, 24000/1001, 30000/1001, fontes 44,1/48 kHz, mono/estéreo, com `source_start`,
   velocidade e offsets (`an_audio_cut_in_any_part_of_the_ramps_mixes_the_same_bits_on_every_grid`, `…visual…keeps_every_frame`).
3. Controles que **perdem ou reiniciam** o envelope (janela descartada) falham como devem
   (`a_cut_that_loses_the_ramp_of_the_original_clip_is_seen`, `…audio…`).
4. Cortes sucessivos (dentro e entre rampas), undo/redo, salvar/reabrir conservam o envelope.
5. Dado inválido ou estouro recusa a operação inteira, sem alterar estado (`invalid_document`, `invalid_time`).

## 3. Evidências (comandos e exit codes)

Logs duráveis, sem conteúdo privado, em `../bl081-2-evidence/` (fora do worktree). Só `./scripts/cargo`, alvo padrão/mbx.

| Comando | Exit | Resultado |
|---|---|---|
| `cargo fmt --all -- --check` (final) | 0 | — |
| `./scripts/cargo check --workspace --locked` | 0 | — |
| `./scripts/cargo clippy --workspace --all-targets --locked -- -D warnings` (final, após o último teste) | 0 | — |
| `./scripts/cargo test --workspace --locked --no-run` (final) | 0 | compila todos os alvos |
| `./scripts/cargo test -p tessel-editor-core --locked --no-fail-fast` | 0 | 1559 passam, 0 falhas, 7 ignorados |
| `./scripts/cargo test --workspace --locked --no-fail-fast` (antes do último teste desktop) | 0 | 2748 passam, 0 falhas, 17 ignorados (inclui export FFmpeg e `audio_fades_tests`) |
| `./scripts/cargo test -p tessel-editor --locked --no-fail-fast` (final) | 0 | 516 passam, 0 falhas |
| `./scripts/cargo test -p tessel-editor --locked split_tests` | 0 | 5 passam (um novo, BL081.2) |
| `./scripts/cargo build -p tessel-editor --locked` | 0 | — |
| `./scripts/cargo tree -p tessel-editor-core --locked` + busca `gpui\|wry` | 0 / 0 ocorrências | core sem UI |

Primeira rodada do core: 1 falha de **teste de unidade** com expectativa errada (corte com `after ≥ fade_out` zera a
metade, corretamente); a expectativa foi corrigida, a lógica não. Não houve outra falha.

### Discriminação contra o modelo anterior

Com `crates/core/src` e `crates/desktop/src` da base (stash, depois restaurado; diff idêntico ao backup `10-full-diff-backup.patch`) e
os testes novos, `clip_split` falha em 3 (`a_cut_inside_a_visual_fade_is_accepted…`,
`an_audio_cut_with_a_fade_across_it_is_accepted…`, `loaded_fractional_visual_fades…`) e a suíte de fase falha em
`a_cut_inside_an_audio_fade_keeps_the_sound_bit_for_bit` (log `11-old-model-vs-new-tests.log`). `clip_split_envelope` nem compila contra a
base (o campo `fade_window` não existe): isso prova a dependência do novo modelo, não comportamento; a discriminação
comportamental vem dos três testes acima e dos controles internos.

### Provas por superfície

| Superfície | Evidência | Estado |
|---|---|---|
| Biblioteca | `clip_split_envelope.rs` (planos iguais em todos os quadros, PCM bit a bit, fracionário, rampa no limite, duas rampas sobrepostas, vários cortes, undo/redo/salvar, edição/trim/mover/reset, gesto de slider, documento inválido) + 7 unitários | passa |
| CLI headless | `the_headless_process_cuts_inside_the_ramps_keeps_picture_and_sound` (processo real `tessel-headless`, JSONL) e, com WAV decodificado do `render.audio`, o script de prova em `../bl081-2-evidence/proof/split-fades-proof.py` (8/8 checks: corte dentro do fade-in, segundo corte dentro do fade-out, WAV idêntico byte a byte, 2 undo/2 redo, frame inválido recusado sem mudança) | passa |
| UI (GPUI) | `split_tests::a_cut_inside_a_fade_is_offered_and_keeps_the_envelope_sample_for_sample`: nó `timeline.split` habilitado, clique simulado, `invoke`, 1 entrada de histórico por corte, PCM bit a bit, undo/redo | passa |
| Janela real | mesmo script, janela própria (binário direto, `HOME` e socket próprios, uma instância, encerrada ao fim; só `ui op/get/set/invoke`): 12/12 checks, incluindo `timeline.playhead`=6, nó `timeline.split` habilitado sem `disabled_reason`, `invoke timeline.split` e o WAV do `render.audio` idêntico ao inteiro. Log `14-surface-proof.log` | passa |

Observação: o primeiro run do script falhou no headless porque `target/debug/tessel-headless` era o binário da base (reconstruído
pela execução de discriminação); após `cargo build -p tessel-editor-core --bins` o script passou nas duas superfícies.

## 4. Limites e o que não é afirmado

- Nenhum ponteiro físico, pixel de janela nem áudio ouvido foi observado; a prova da janela é semântica e o som é o mix
  offline lido de WAV.
- A prova da janela real é um script ad hoc fora do repositório (`../bl081-2-evidence/proof/`), não um script versionado
  em `scripts/` (fora do ownership); sem gate Windows nem corpus de equivalência de imagem por pixels (a imagem é provada
  por plano/opacidade por quadro, não por render). Sem execução de desempenho nem de lock de GPU (não era escopo).
- Critério de áudio conservador: em fades sobrepostos ou maiores que a metade as duas metades **sempre** guardam a janela
  completa, mesmo quando um lado poderia ser zerado; é correto, mas escreve dado que um critério mais fino dispensaria.
- Trim: a rampa herdada fica no lugar da sequência e some quando a borda passa da origem; a alternativa (rampa que acompanha a
  borda) seria uma escolha de produto diferente. Editar o fade da metade descarta a rampa herdada daquele lado.
- Schema 3 para documentos com janela: leitores v1/v2 recusam em vez de perder a rampa. Sem motor geral de envelopes,
  keyframes, crossfade, P4b, batch, seleção múltipla ou atalhos.
- BL081.2 **não** é declarado concluído para fechamento: está **implementado e validado nesta máquina (Mac)**, a validar pelo
  coordenador/Windows. Nada ficou não representável (não há guarda residual de fade), por isso `split_unsupported` por fade
  deixou de existir.

## 5. Propostas ao coordenador (não editadas nos arquivos comuns)

**`docs/backlog.md`, linha BL081.2** (status/responsável/aceite):

> | BL081.2 | Corte / envelope | Fades atravessando o ponto do corte (visual e áudio) | Implementado, a validar — diff não commitado | BL081, BL142, BL081.5 | Executor T3 (Cursor Sonnet5.5); revisão e integração coordenador | `Clip.fade_window` (lead/tail relativos, guarda do valor do fade, varredura); fim de `fade_*_crosses_cut`/`fades_longer_than_clip`. Core1559/7 ignorados, workspace 2748/0 falhas, desktop516, planos e PCM bit a bit em 24/25/30/60, 24000/1001, 30000/1001, 44,1/48 kHz, mono/estéreo; controles que perdem o envelope falham; headless real, UI GPUI e janela real (12/12) semânticos; gates fmt/check/clippy/no-run/build exit0. Sem ponteiro, pixels nem áudio ouvido. [Relatório](migration/clip-split-fades-report.md) |

Também atualizar as linhas BL081.13/BL081.14/BL081.15 (nota: "guarda de fade recusada" substituída por aceitação) e a menção "Não se
amplia corte dentro de rampa" de BL081.13 e a de BL081.5 (linha 1610), que dizem que fades atravessados são pendentes.

**`docs/features/README.md`**

- F017: trocar "corte dentro de fade segue recusado (BL081.2)" por "corte dentro de fade (visual e áudio) conserva o envelope por
  `Clip.fade_window` (BL081.2, a validar): planos e PCM iguais bit a bit, sem recusa por fade; ver
  [relatório](../migration/clip-split-fades-report.md)".
- F051: acrescentar "cortar o clipe dentro de uma rampa conserva o envelope (`fade_window`, BL081.2); trim mantém a rampa herdada
  no lugar até a borda passar da origem".

**`docs/core-architecture.md` / `docs/features/project-document.md`** (este último também fora do meu ownership): o clipe ganha o campo
opcional `fade_window` (`{lead:{frames,fade}, tail:{frames,fade}}`), que exige `schema_version 3`; v1 com o campo é
`invalid_document`; documentos sem o campo abrem como antes. Frases sugeridas: "Um clipe pode carregar `fade_window` quando uma
rampa foi dividida por um corte (F017/BL081.2); o schema é 3 nesse caso." e a lista de campos do clipe em `project-document.md`
com `fade_window` opcional.



## Revisão da coordenação (posterior à entrega inicial)

A revisão independente confirmou três defeitos: schema2 já era aceito pelo leitor
anterior; editar um lado do áudio podia encurtar a outra rampa herdada; trim no
limite podia produzir uma janela que o carregador recusava. As provas da seção3
são da entrega inicial, não dos ajustes abaixo. O hash d1806e58 identifica o
recebimento preservado, não o diff após a revisão.

Coordenação: schema3 somente quando há janela, preservando schema1/2 para
conteúdo anterior; v1/v2 com janela recusados. A recusa do leitor da base para
v3 foi conferida pelo código do migrate (limite2), não por uma execução dele.
Um helper checked_frames é comum ao carregador e ao trim; trim extremo agora
recusa atomicamente e o documento restante reabre. Testes reorganizados em cinco
módulos com fixtures compartilhadas:22originais preservados e2regressões novas,
24/24 passam no Mac. Logs: SSD de coordenação, coordinator-fixes.log.
A correção do acoplamento entre rampas de áudio é a seção 6 (BL081.2.2); não se declara aceite
nem se herdam os gates da entrega inicial.

## 6. BL081.2.2 — editar o fade de uma metade não move a outra rampa (áudio)

### Defeito
Áudio a 24 fps/48 kHz, clipe 3..63, `fade_out` 30, `fade_in` 0, corte em 40. A metade direita tinha janela com `lead` 37 e guarda 0, só para
preservar o comprimento que `resolve_fades` usava. Editar o `fade_in` dela para 5 descartava o `lead`, o vão caía para 23 quadros e
`resolve_fades` encurtava o `fade_out` não editado de 30 para ~20 quadros (PCM alterado perto do fim). O caso simétrico: metade esquerda
com `fade_in` 30 e corte em 20; editar `fade_out` alterava o `fade_in`.

### Modelo (menor que mantém o contrato)
Contrato: **editar um fade redefine só aquela rampa da metade; a outra rampa herdada mantém forma, posição e normalização**, mesmo que o
clipe inteiro a tenha encurtado por sobreposição. Sem motor geral de envelope nem keyframes; `AudioSegment` não mudou.

- **Âncora** ≠ **normalização**. A âncora diz onde a rampa começa (fade-in) ou termina (fade-out). A normalização (`norm.far`,
  `norm.peer`) é o vão em que `audio::resolve_fades` encurtou a rampa e o pedido (em quadros arredondados) da outra rampa nesse vão.
- `Clip.fade_window.{lead,tail}` passa a ser, por rampa, `Reach{frames, fade, norm}`; `frames` é a âncora (≥ 0 em áudio, relativa à borda
  do clipe), `fade` a guarda do valor da propriedade, `norm` obrigatório em áudio e proibido em imagem/vídeo. Persistido em
  `fade_window.{lead,tail}.norm{far,peer}`, schema 3; janelas de áudio sem `norm` (build intermediário) são `invalid_document`.
- `plan_audio` (`crates/core/src/plan.rs`): `clip_offset` conta da âncora do fade-in, `clip_frames` vai da âncora do fade-in à do
  fade-out; cada rampa roda `resolve_fades` no próprio vão: a fixada no vão congelado e com o par congelado; a livre na borda, com o par
  dela se o outro lado também é livre, ou sozinha se o outro está fixado. Sem fixações, o plano e o PCM são os legados bit a bit
  (`resolve_fades` proporcional intacto). Som embutido de vídeo: sem fixação nem fade.
- **Corte** (`state/envelope.rs::split_audio`): cada grupo é deslocado por metade (âncora pela borda próxima, vão pela oposta). R.in é
  zerado se `âncora ≥ rampa` e a rampa não se sobrepõe; L.out idem; L.in e R.out nunca são zerados; sobreposição nunca zera. Uma rampa
  sobrevivente só ganha fixação se medi-la na borda mudaria o resultado agora ou após qualquer edição do outro fade (âncora ≠ 0,
  `rampa + par > vão`, ou `rampa + max(par, edit_limit) > duração`, `edit_limit` = máximo do slider lido de `property_def`, 48).
  Metades longas continuam sem janela (sem dado novo).
- **Trim**: a âncora acompanha a borda próxima (some se < 0 em áudio, ≤ 0 em imagem); `far` acompanha a borda oposta; o vão é invariante.
  Overflow ou vão inválido recusa atomicamente (`checked_frames`, também usada pelo carregador, limite 1 000 000).
- **Editar um fade / reset / gesto**: a guarda só vale enquanto o valor do fade é o da guarda; `sweep_fade_windows` descarta só aquele lado
  ao fim do comando e na confirmação do gesto (não durante a prévia, então cancelar restaura a fixação). Undo/redo e reabrir reaplicam a
  mesma regra (a guarda é salva).

### Invariantes (cada um com teste)
1. Inteiro = metades bit a bit, em corte simples e múltiplo, grades fracionárias, blocos, offset, velocidade e streams.
2. Editar um fade da metade não altera plano nem PCM fora da rampa editada (os dois casos simétricos), nem zera os dois lados.
3. O fade novo tem efeito real na borda correta; o inteiro e o legado (sem janela) não mudam.
4. Mover, aparar, editar/resetar cada fade, gesto (cancelar/confirmar), undo/redo e salvar/reabrir não ressuscitam âncoras velhas.
5. Dado inválido ou estouro: recusa atômica; todo documento que o editor salva reabre. Fade de vídeo não altera o som embutido.

### Evidências (Mac, `./scripts/cargo` com o target padrão; logs em SSD de coordenação, diretório `audio-ramps-fix/final/`)
| Prova | Resultado |
|---|---|
| `cargo fmt --all -- --check` | exit 0 |
| `check --workspace --locked` | exit 0 |
| `clippy --workspace --all-targets --locked -- -D warnings` | exit 0 |
| `test --workspace --locked --no-run` | exit 0 |
| `-p tessel-editor-core` completo | 1585 passam, 0 falhas, 7 ignorados |
| `clip_split_envelope` | 42/42: 24 anteriores + `ramps` (7: repro direita/esquerda, matrizes de taxas, sobreposição original, nunca zerar o outro lado, legado bit a bit), `ramp_edits` (6), `ramp_documents` (4), `ramp_surfaces` (1, processo `tessel-headless` real: WAV do `render.audio`, plano, undo) |
| `state::envelope::tests` | 15 testes de unidade (cortes visual/áudio, grupo repetido, rampa nativa ao lado de fixada, validação, trim, guardas, overflow) |
| desktop `split_tests` / `audio_fades` | 5/5 e 2/2 |
| `crates/media` `export` (FFmpeg real) | 11/11 |

Taxas e formatos cobertos nos testes de áudio: 24/25/30/60, 24000/1001, 30000/1001; 44,1 e 48 kHz; mono e estéreo; valor fracionário vindo
do documento; gesto de slider em áudio.

### Controles discriminantes
- `without_windows(state)` (limpa `fade_window` dos clipes na memória) é o equivalente do modelo anterior depois da edição; os testes
  falham com ele (o PCM da rampa não editada muda), e a aritmética do modelo antigo é afirmada explicitamente com `resolve_fades`:
  `(5·2000, 30·2000, 23·2000) = (6571, 39429)` e, no caso esquerdo, `(30·2000, 5·2000, 17·2000)`. Nenhuma mutação foi feita no código
  compartilhado nem stash; os controles são oráculos em memória.
- Um oráculo independente confirma o PCM legado bit a bit para clipes inteiros com fades, inclusive 40/40 (sobreposição proporcional).

### Limites e o que não é afirmado
- Não houve execução no Windows, janela real, ponteiro físico nem áudio ouvido; a UI (GPUI) usa o mesmo comando/estado e só os testes
  existentes de split/fades foram executados nela. O novo comportamento é provado na lib e no processo headless.
- Fronteira de produto: uma rampa fixada fica congelada até **o próprio fade** ser editado; reeditar para o valor antigo não ressuscita a
  rampa herdada. Um fade novo ao lado de uma rampa herdada é resolvido sozinho e multiplica com ela, então um vale é possível (não
  existe o triângulo de `resolve_fades` entre uma rampa nova e uma herdada).
- O teste de processo grava PCM16: valores acima de 1,0 saturam; as asserções de bit a bit em float vivem nos testes de lib.
- Documentos de áudio com janela de uma build intermediária (sem `norm`) são recusados; o schema 3 não foi publicado.
- BL081.2 **não** é declarado aceito; falta revisão independente e as provas de Windows/janela.

### Propostas ao coordenador (arquivos comuns não editados)
- **`docs/backlog.md`, BL081.2.2:** "Implementado, a validar — diff não commitado. Modelo âncora ≠ normalização por rampa
  (`fade_window.{lead,tail}.norm{far,peer}`, schema 3): editar um fade da metade só redefine aquela rampa. Core 1585/0 (7 ignorados),
  `clip_split_envelope` 42/42, unitários de envelope 15, desktop split 5/5 e audio_fades 2/2, media export 11/11, fmt/check/clippy
  exit 0. Sem Windows, janela real, ponteiro nem áudio ouvido. [Relatório](migration/clip-split-fades-report.md)".
- **`docs/features/README.md`:** F017/F051 — "editar o fade de uma metade cortada redefine só aquela rampa; a outra mantém forma, posição e
  normalização (BL081.2.2, a validar)".
- **`docs/core-architecture.md` / `docs/features/project-document.md`:** o campo `fade_window.{lead,tail}` de um clipe de áudio carrega
  `norm{far,peer}` (obrigatório em áudio, proibido em imagem/vídeo), schema 3; âncoras ≥ 0, `peer` em [0, 1 000 000], vão `frames +
  duração + far` em [1, 1 000 000]; janelas de áudio sem `norm` são `invalid_document`.


## 7. Prova atual da coordenação (Mac)

Após a correção de âncora/normalização, os executáveis desktop e headless foram
recompilados juntos com scripts/cargo build -p tessel-editor -p tessel-editor-core
--bins --locked (exit0). Script durável independent-ramp-proof.py no SSD de
coordenação, proof-latest/, executou12/12checks headless e12/12na janela própria,
0skips. Os dois casos simétricos cortam via core e, na janela, invoke timeline.split.
WAV da região da rampa herdada não editada conserva bytes; a nova rampa altera a
borda esperada; undo/redo, salvar schema3 e reabrir conservam ambos os resultados.
As provas de bits float e seis taxas são dos testes de lib, não desses WAVs PCM16.
Janela fechada pelo runner; sem pixels, ponteiro, áudio ouvido ou Windows.
O novo modelo permanece em revisão independente e não está aceito.


## 8. Revisão independente do modelo corrigido e ajustes da coordenação

Sem bug bloqueante no áudio ou no visual após a correção das rampas. Pontos
tratados: rampa de borda frames=0 agora segue a borda aparada, com regressão
que falha no código anterior; testes de undo/redo/reabrir e seis taxas. O ramo
redundante do segundo olhar em needs_pins foi removido. Fixação conservadora
para futuras edições é mantida e explicitada: cortes fora da rampa em metades
curtas podem elevar a schema3; metades longas dispensam metadados. Não há
aprovação humana de produto alegada. São escolhas locais dentro do contrato
de preservar rampa não editada, verificáveis nos testes.

Prévia de trim e commit validam o mesmo limite; fade_window aparece no tree
vivo do clipe; o teste GPUI de split agora edita fade-in da metade, confere
a rampa de saída e undo/redo. A prova funcional foi incluída no repositório em
scripts/split-fades-proof.py. As tabelas anteriores continuam sendo evidências
dos respectivos snapshots, sem atribuí-las ao código ajustado. Novos gates
estão em execução; Windows ainda pendente.


## 9. Gates Mac finais e validação Windows iniciada

Após ajustes da revisão: fmt/checkworkspacealltargets/clippyalltargets-Dwarnings/
workspace--no-run/builddesktop+headless/higiene exit0. Core1587/0falhas/7ignorados
existentes, desktop516/0, exportFFmpeg11/0. Envelope44/44 inclui os dois novos
casos de trim em âncora0 e promoção conservadora de schema. O teste GPUI de
split inclui edição de rampa, estado vivo e undo/redo. Script versionado
split-fades-proof.py: headless12/12 e janela12/12,0skips nos binários atuais;
prova semântica/WAV, não pixels, ponteiro nem áudio ouvido. Não se afirma
workspace completo executado nesta versão; no-run prova compilação.

Logs duráveis no SSD de coordenação, final-mac/. As36fontes congeladas para o
Windows conferem SHA256 contra os gates Mac; ZIP75357bytes SHA256
cf9d4b358591ec3dc6cb7042a80f833aebc8dda7dc0c2b2cb4b4b47cef765f9f.
A delegação remota não suporta worktree; a validação Windows foi iniciada
em thread com worktree próprio, baseE3, sem usar a raiz paralela. Gates
Windows ainda pendentes; nenhuma prova Mac os substitui. As atualizações
documentais desta seção são posteriores ao pacote Windows, sem mudar código.
