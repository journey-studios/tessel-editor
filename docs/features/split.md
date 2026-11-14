# F017 — Dividir clipe / corte interno

## Status

**BL081 e continuidade fracionária BL081.5 validados Windows/Mac, dentro dos limites abaixo.** Entregue em biblioteca, CLI headless (processo JSONL) e janela. Evidência e limites em
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
| Fade-in | a esquerda mantém o fade; a direita fica com 0 |
| Fade-out | a direita mantém o fade; a esquerda fica com 0 |
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
| `split_unsupported` | `details.reason`: `fade_in_crosses_cut`, `fade_out_crosses_cut`, `fades_longer_than_clip` |
| `disabled_element` | `invoke timeline.split` com o controle desabilitado |

Fade que cruza o corte não é aproximado: uma rampa não pode ser dividida em duas sem mudar o ganho. Imagem recusa fade-in se
`frames_antes < fade_in` e fade-out se `frames_depois + 1 < fade_out`; áudio, fade-out se `frames_depois < fade_out`.
Fades visuais carregados do documento conservam o valor fracionário nessas comparações,
como no renderer; só os fades de áudio são arredondados a frames, como no mixer (BL081.13).
Dois fades de áudio cuja soma excede o clipe (o mixer os encurta) são recusados.

### Áudio em grade fracionária (BL081.5 — validado Windows/Mac)

A recusa `audio_sample_grid` e `SplitBlock::AudioSampleGrid` foram removidos nesta
implementação. O relógio do core usa `S(f)=round(f·den·48000/num)` como fronteira
absoluta do quadro; cada amostra de saída lê posição racional da fonte a partir de
`source_start`, início do clipe, velocidade e offset do stream. O cursor mantém a
fase fracionária por somas inteiras, sem reiniciá-la quando um trecho é dividido.
Não há campo persistido novo no clipe nem mudança no tempo visual/câmera.

Mix em blocos e faixas parciais deve coincidir bit a bit com a mesma faixa do
inteiro. Fades externos usam diferenças no mesmo relógio; cortar dentro de rampa
continua recusado (BL081.2). Mute não é um bypass: o modelo também deve manter o
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
- Cortes dentro de um fade são recusados (nunca aproximados).
- O Ken Burns e a órbita são recalculados em `f64` (propriedades do inspetor são `f64`); o tempo de fonte e a divisão do tempo
  são exatos.
- A direita herda o nome do clipe (não é renomeada).
- Sem atalho de teclado, sem corte por clique na timeline e sem corte em lote (a op é por clipe).
- Selecionar de volta o clipe original depois de undo/redo não é automático (seleção é de sessão).

## Evidências

Ver [clip-split-report.md](../migration/clip-split-report.md): `crates/core/tests/clip_split.rs` (22 testes: lib e processo
`tessel-headless`), testes de unidade (`id`, `inspector`, `tree`), `crates/desktop/src/workstation/split_tests.rs` (3 testes
GPUI com clique simulado) e a prova na janela própria por `tessel-editor ui …`.

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
