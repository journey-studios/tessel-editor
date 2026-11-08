# F016 — Adicionar e processar efeitos (primeira fatia: vinheta, escopo de saída)

## Status

**Parcial.** Uma fatia completa de ponta a ponta: o efeito **vinheta** com escopo **saída** (a imagem
final da cena dirigida), criado, editado e removido pela biblioteca, pelo JSONL sem janela e pela janela;
avaliado por quadro no core; persistido; desfeito em uma entrada; desenhado pelo monitor e pelo export.
Não é a migração do motor de efeitos: o catálogo tem **um** efeito e o piloto tem 59 shaders. O restante
(catálogo, transições, edição de shots) continua pendente em BL028/BL036/BL082 e **não** há placeholder
habilitado para ele.

| Superfície | Estado | Evidência (executada neste head; ver "Evidências") |
|---|---|---|
| Lib | V | `crates/core/tests/scene_fx.rs` (16), `reference.rs` (5 unitários) |
| CLI headless (JSONL) | V | `scene.fx.*` pelo `tessel-headless` real, resposta idêntica à da lib (`scene_fx.rs`); export headless com FFmpeg real (`crates/render/tests/scene_fx_export.rs`) |
| UI / CLI da janela | V (semântico) · Parcial (ponteiro físico) | 7 testes GPUI (`scene_fx_tests.rs`) com arrasto, Escape e cliques injetados; `scripts/scene-fx-ui-proof.py` na janela real, 70/70 por `op`/`get`/`set`/`invoke`; captura nativa só de observação |
| Pixels no monitor | V no corpus | monitor == `scene.render` (hash) e sondas == vinheta independente em Python (pior 0,5/255) |
| Pixels no export | V no corpus | MP4 decodificado == referência sobre o baseline decodificado (pior 7/255 pela lib, 7 pela janela), fora do clipe == baseline |

## Decisão de domínio (antes de implementar)

O efeito vive **uma vez**, em `fx`, chave de primeiro nível do **documento do bundle dirigido**
(`DirectedScene`), não na timeline de sequências do editor e não numa cópia no plano:

- o bundle já é o dado canônico e sem perdas (`document()`); o undo, a persistência (o documento da cena
  viaja no projeto), o pacote e o intercâmbio já o carregam sem código novo;
- o `ScenePlan`/`PlanTimeline` já são a única avaliação por quadro que monitor e export consomem, então
  um `fx` resolvido por quadro dentro do plano dá **a mesma avaliação offline, de preview e de export**;
- duplicar na sequência criaria dois modelos a reconciliar. A fatia **não** põe nada na timeline do projeto.

Não é um bundle "do piloto": a chave `fx` é uma extensão **do editor** ao documento (campos desconhecidos
continuam preservados e avisados, `UNKNOWN_FIELD`; `fx` é lido, validado e normalizado). O piloto é uma
especificação a reabrir, não algo que se afirme portado.

### Formato (`catalogVersion` 1)

```json
"fx": {
  "catalogVersion": 1,
  "tracks": [{
    "id": "fx-track-1", "label": "FX 1", "hidden": false,
    "clips": [{
      "id": "fx-1", "effect": "vignette",
      "startFrame": 0, "durationFrames": 48,
      "params": { "intensity": 0.7, "radius": 0.75, "softness": 0.45, "roundness": 0.6,
                  "center": { "x": 0.5, "y": 0.5 }, "color": { "r": 0, "g": 0, "b": 0 } },
      "fadeInFrames": 8, "fadeOutFrames": 8, "hidden": false, "scope": { "kind": "output" }
    }]
  }],
  "transitions": []
}
```

- Frames de edição (inteiros); o fim é exclusivo. `params` é escrito como o catálogo tipa (omitir usa o
  padrão). Fades de 0, `hidden: false` e `scope` de saída são grafia canônica (omitidos); uma camada sem
  clipes sai do documento, então adicionar e remover devolve o documento byte a byte.
- Sem `fx` (ou vazia) o plano **não tem a chave `fx`** e os bytes do plano de antes não mudam (golden).
- Recusado com código tipado, nunca ignorado: `keys` (`FX_KEYS_NOT_YET`), escopo `shot`/`node`
  (`FX_SCOPE_NOT_YET`), qualquer entrada em `transitions` (`FX_TRANSITION_NOT_YET`), `catalogVersion > 1`
  (`CATALOG_TOO_NEW`), efeito fora do catálogo (`UNKNOWN_EFFECT`), parâmetro desconhecido, de outro tipo ou
  fora da faixa, sobreposição de clipes na mesma trilha (`FX_CLIP_OVERLAP`), clipe vazio ou fora da edição,
  fades que não cabem (`ENVELOPE_TOO_LONG`), IDs duplicados/reservados (`track`, `fx`, `transition`) e
  limites (64 trilhas, 1024 clipes por trilha, 64 bytes por ID).

### Catálogo (`directed_scene::fx::catalog`)

| Efeito | Parâmetro | Tipo | Faixa | Padrão |
|---|---|---|---|---|
| `vignette` | `intensity` | número | 0–1 | 0,7 |
| | `radius` | número | 0–1,5 | 0,75 |
| | `softness` | número | 0,05–1 | 0,45 |
| | `roundness` | número | 0–1 | 0,6 |
| | `center` | ponto (`x` à direita, `y` **para baixo**) | 0–1 | (0,5; 0,5) |
| | `color` | cor `{r,g,b}` em luz linear | 0–1 | preto |

`typical_duration_frames` = 96 (duração padrão de um clipe adicionado sem duração, cortada no fim da edição).

### Avaliação determinística por quadro

`Fx::resolve(frame)` devolve um `FxPass` por clipe visível que cobre o quadro, na ordem das trilhas e dos
clipes. O peso do envelope é amostrado no **meio** do quadro:
`min((f + 0,5)/fadeIn, (dur − f − 0,5)/fadeOut)`; `amount = clamp(intensity × weight)`. `amount ≤ 0`
é identidade exata (sem ida e volta sRGB, então intensidade 0 não move um pixel).

Vinheta (especificação do shader do piloto, transcrita em `reference.rs` e **independentemente** em Python na
prova da janela): distância `|(p − centro)·2·forma| / max(|forma|,1)` com `forma = (mix(1, w/h, roundness), 1)`,
`shade = smoothstep(radius, radius+softness, distância)`, mistura com `color` em **luz linear** (curvas
IEC 61966-2-1 exatas) por `shade × amount`.

### Plano e runtime

`ScenePlan.fx` (chave aditiva, só quando há passes): `trackId`, `clipId`, `effect`, `frameInClip`, `weight`,
`amount`, `params`, `scope: {kind: "output"}`. `Capability::OutputFx` (`outputFx`, a 10ª) entra em `requires`
quando há passe; um runtime sem ela recusa o plano pelo nome. O validador recalcula `amount` e recusa um plano
adulterado. `PlanTimeline::at_frame` re-resolve `fx` e `requires` por quadro (igual a `ScenePlan::build` em
todos os quadros, testado). O runtime só desenha (`runtime/src/output-fx.js`): um `ShaderMaterial` em tela cheia
por passe, ping-pong em alvos sem MSAA depois do MSAA da cena; efeito ou escopo desconhecido é `unsupported`
tipado, nunca pulado. `frame.sources.directed_scene_fx` conta os passes.

## Entrada compartilhada (lib · JSONL · janela)

Core: `scene_edit::fx` (`FxOp`, `apply_fx`/`apply_fx_with`/`dry_run_fx`) e `scene_project::fx`. O lote é
**atômico** (clona o JSON, valida cada op, reimporta o candidato com as regras do importador e dos gates
estrutural/temporal; recusa ⇒ nada muda e a op é nomeada). Um comando = uma entrada de undo ("Edit effects");
no-op e dry-run não gravam nada.

| Op JSONL | Efeito |
|---|---|
| `scene.fx.catalog` | catálogo, faixas, escopo e recusas como dado |
| `scene.fx.list {scene}` | trilhas e clipes |
| `scene.fx.frame {scene, frame\|time}` | passes resolvidos de um quadro |
| `scene.fx.edit {scene, ops[], dry_run?}` | lote atômico de `add_clip {effect, start_frame, duration_frames?, track?, id?, params?}`, `remove_clip {clip}`, `set_param {clip, param, value}`, `set_field {clip, field: start_frame\|duration_frames\|fade_in_frames\|fade_out_frames, value}`, `set_hidden {clip, hidden}` |

Sem trilha, `add_clip` usa a primeira trilha em que cabe, ou cria `fx-track-N` (rótulo `FX n`); sem ID, `fx-N`;
remover o último clipe de uma trilha remove a trilha. Códigos de erro do comando (`command_code`):
`unknown_effect`, `unknown_param`, `param_type`, `param_out_of_range`, `unknown_fx_track`, `unknown_fx_clip`,
`reserved_id`, `duplicate_id`, `fx_clip_overlap`, `fx_clip_empty`, `fx_clip_out_of_range`, `envelope_too_long`,
`catalog_too_new`, `fx_scope_not_yet`, `fx_keys_not_yet`, `fx_transition_not_yet`, `fx_limit`; um candidato
recusado pelo importador mantém `invalid_bundle`.

### Janela

Seção **Efeitos** no inspetor de cena (`workstation/scene_fx.rs`): botão `+` (adicionar vinheta) e, por clipe,
título, interruptor de visibilidade, remover e sliders do Kit (intensidade primeiro, raio, suavidade, cor R/G/B,
arredondamento, centro X/Y, início, duração, fade in/out). Cada gesto é **um** `scene.fx.edit` por
`Workstation::execute` (um slider guarda o valor no arrasto e envia **uma** edição ao soltar; Escape descarta;
perda de foco confirma). Recusas do core aparecem como o aviso do painel (`scene.panel.state.notice`).

Nós semânticos: `scene.fx` (ação `add:vignette`; `state.clip_count`, `effects`, `scope`) e
`scene.fx.clip:<id>` (campos numéricos `intensity`, `radius`, `softness`, `roundness`, `center_x/_y`, `color_r/_g/_b`,
`start_frame`, `duration_frames`, `fade_in_frames`, `fade_out_frames` e `hidden`; ações `remove`, `toggle_hidden`).
`set` de um componente de ponto/cor mantém os irmãos como o documento os tem; quadros exigem inteiros.

## Evidências

Todas executadas neste head (ver o relatório de entrega para SHA e comandos):

- **Lib/JSONL** — `cargo test -p tessel-editor-core --test scene_fx`: 16 testes (catálogo; plano sem `fx` idêntico ao
  anterior; criar/editar/remover com undo/redo; no-op e dry-run; trilhas automáticas; 13 recusas atômicas;
  sobreposição; recusas de import; envelope; salvar/reabrir; plano com passes e capacidade; igualdade
  `PlanTimeline` em todos os quadros; plano adulterado recusado; pixels de referência — baseline, intensidade 0,
  1, centro movido; `cargo tree` sem gpui/wry; processo `tessel-headless` real igual à lib, com a recusa
  `param_out_of_range` e saída 1).
- **Runtime** — `runtime/test/output-fx.test.mjs` (5, puro) e `directed-scene-fx-browser.test.mjs` (2, Chrome/SwiftShader:
  parede cor `[200,160,80]`; amount 0 == baseline; amount 1 == transcrição JS ±2; centro (1,1) discrimina; cor azulada;
  dois passes empilhados; efeito desconhecido ⇒ `frame.failed`/`unsupported`). Suíte completa `node --test`: 186/186.
- **Renderer (lib + processo Node/Chrome)** — `crates/render/tests/scene_fx.rs` (3): a cena do corpus recebe a vinheta
  pela edição do core; 54 sondas por quadro comparadas à referência do core (pior diferença 0); intensidade 0 ==
  baseline (sondas idênticas); fora do clipe == baseline; centro movido discrimina; mesmo quadro ⇒ mesmo hash depois de
  um seek para longe; fade (3º quadro de um fade-in de 10: peso 0,25); runtime sem `outputFx` recusado pela lib.
- **Export (FFmpeg real)** — `crates/render/tests/scene_fx_export.rs`: 1080p, quadros 44..52 (o clipe termina em 48):
  hash dentro ≠ baseline, fora == baseline, hash de cada quadro == `scene.render`; MP4 decodificado == referência
  sobre o baseline decodificado (pior 7/255, tolerância 16, maior mudança 69 — discrimina); `history.undo`
  devolve os hashes do baseline; o export não grava undo nem clipe de timeline.
- **Janela (GPUI)** — `scene_fx_tests.rs` (7): adicionar é uma entrada; nós descrevem o clipe; `set` por nó é uma entrada
  e componentes mantêm irmãos; recusas tipadas (`param_out_of_range`, `invalid_value`, `unknown_field`,
  `unknown_element`) sem sobra e com aviso no painel; ocultar/alternar/remover e undo; arrasto do slider confirma
  **uma** vez ao soltar; Escape descarta; cliques nos botões de adicionar e remover.
- **Janela real** — `scripts/scene-fx-ui-proof.py` (instância privada, binário direto, `HOME` curto, lock de GPU, só
  `op`/`get`/`set`/`invoke`): 70/70 — árvore viva, baseline monitor == render, adicionar, intensidade 1 (54 sondas ==
  vinheta em Python, pior 0,5), intensidade 0 == baseline, centro movido, `param_out_of_range` no painel sem mudar valor,
  histórico nem imagem, undo/redo, ocultar, **salvar/reabrir o projeto** com os parâmetros, e dois exports pelo diálogo
  (`flow.export_scene`) decodificados contra o modelo Python. A captura nativa
  (`--shot`, `screencapture -l`) é só observação: mostra as bordas do monitor escurecidas.

## Limites e gaps (todos em [backlog](../backlog.md) BL028/BL036/BL082)

- Só `vignette`. Os outros efeitos do piloto (59 shaders) **não** foram portados; cada efeito novo é uma entrada no
  catálogo, um shader no runtime e um teste de pixel.
- Só escopo `output`. `shot`/`node`, chaves (`keys`, animação de parâmetros) e `seed` (lido, sem uso) são recusados.
- Sem transições: `transitions` é recusado se não vazio. **Contrato de extensão** (sem controle habilitado): uma
  `crossDissolve` será `transitions[]` com `{id, type, fromShot, toShot, durationFrames}` resolvida no plano com
  **dois** quadros de origem (precisa que o plano e o runtime desenhem dois shots no mesmo quadro: capacidade nova
  `transitionBlend`), e o ripple/roll de shots será um comando de edição estrutural do core sobre `shots[]` que
  mantém o join/temporal gates; nenhum dos dois tem nó, comando nem botão ainda.
- Sem prévia ao vivo durante o arrasto do slider (a imagem muda ao soltar) e sem teclado nos sliders da seção
  (o gesto de teclado da seção de cena não foi ligado): a edição por nó/CLI é o caminho acessível.
- O botão adiciona no quadro 0 (não no playhead); uma segunda vinheta nos mesmos quadros vai para uma trilha nova.
- Ponteiro físico real continua não provado (BL043); a prova da janela é semântica e o GPUI injeta entrada.
- Não há dithering (o runtime escreve 8 bits como a cena); gradientes suaves podem mostrar bandas de 1/255.
- A janela pinta a seção só com uma cena anexada; sem cena, a edição é por `scene.fx.edit` (headless).
