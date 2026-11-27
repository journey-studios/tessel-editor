# F067 — Retângulo editável na timeline (BL169.6, etapa 1A)

## Objetivo e limites

O retângulo é um clipe visual **sem arquivo**: o core guarda uma `ShapeSpec` no próprio clipe
(`clip.shape`), a timeline o avalia como qualquer camada visual e o runtime o desenha. O monitor, o
quadro (`render.frame`) e o export usam o mesmo plano e o mesmo desenho. Posição, escala, rotação,
corte, opacidade, fades, mistura e correção de cor são as propriedades comuns de clipe visual
(as mesmas do vídeo e do título): nenhum controle foi duplicado para a forma.

| Campo | Unidade | Limites | Padrão |
|---|---|---|---|
| `kind` | — | `rect` nesta etapa; `ellipse` e `path` vêm da [etapa 1B (F068)](shape-style.md) | `rect` |
| `width` | % da largura da tela | maior que 0, até 200 | 40 |
| `height` | % da altura da tela | maior que 0, até 200 | 20 |
| `fill` | cor | `#rrggbb` | `#4a90e2` |
| `radius` | % da altura da tela | 0 a 100 | 0 |

O tamanho em pixels é `round(width/100 × largura)` por `round(height/100 × altura)`, então a forma
ocupa a mesma fração da tela no preview e no export. O raio em pixels é `radius/100 × altura`,
limitado à metade do lado menor (a forma vira uma pílula, nunca se deforma). A posição é a do clipe,
com origem no centro da tela (`position_x`, `position_y`, em % como nos demais clipes).

Fora desta etapa: animações de entrada (a etapa seguinte reutiliza os presets de
[F066](animated-titles.md)) e cantos individuais. Elipse, caminho, contorno, gradientes, sombra e a
máscara de imagem entraram na etapa 1B: veja [F068](shape-style.md), que mantém tudo o que está
aqui (os cinco campos desta etapa são gravados como antes).

## Contrato do core / biblioteca

- `tessel_editor_core::shape::{ShapeSpec, ShapeKind}`: `ShapeSpec::default()`, `validate`,
  `with_field`, `to_json`, `from_json` (chave desconhecida é `unknown_field`).
- `Command::AddShape` (atalho da barra, um único passo de histórico, cria a trilha se preciso),
  `Command::AddShapeClip` (clipe completo, via `clip.add` com `shape`), `Command::SetClipShape`
  (troca a especificação inteira) e `Command::SetClipShapeField` (um campo). Os nomes de histórico
  são "Add shape" e "Edit shape".
- `ClipKind::Shape` cabe em trilhas de vídeo. Uma `shape` em clipe de outro tipo é recusada
  (`not_applicable`) e junto de `asset`, `text` ou `camera` é `invalid_request`.
- O plano ganha `Content::Shape` (`layer.kind = "shape"`, `layer.shape` com os campos da forma (na 1A cinco; desde a [1B](shape-style.md) todos),
  `asset: null`). A capacidade `shape` entra na lista `layers` do handshake do runtime.
- Persistência: `clip.shape` no documento do projeto, obrigatório em clipes `shape` e recusado nos
  demais; o erro de leitura cita o clipe. Não houve mudança de versão do esquema: documentos sem
  formas são lidos e gravados como antes. **Limitação:** um editor anterior a esta entrega recusa um
  projeto que contenha formas ("unknown kind").
- Um valor inválido é recusado antes de qualquer mutação: documento, histórico e revisão ficam
  iguais. Dividir um clipe (`clip.split`) copia a forma para as duas metades.

## CLI headless

```json
{"op":"track.add","id":"overlay","kind":"video","name":"Sobreposição"}
{"op":"clip.add","id":"faixa","track":"overlay","start":10,"duration":40,"properties":{"position_y":30,"opacity":80},"shape":{"kind":"rect","width":50,"height":12.5,"fill":"#102030","radius":20}}
{"op":"set","target":"clip_shape:faixa","field":"fill","value":"#ffcc00"}
{"op":"clip.shape","clip":"faixa","shape":{"kind":"rect","width":10,"height":10,"fill":"#ffffff","radius":50}}
{"op":"shape.add"}
{"op":"plan","frame":20}
```

`shape.add` e `invoke timeline.add_shape invoke` inserem até 3 s no cursor, na trilha visual superior
quando ela comporta a forma, ou criam uma trilha, e selecionam o clipe. `render.frame`,
`export.start` e `project.save`/`project.open` funcionam como com qualquer clipe.

## UI e CLI da janela

- Barra da timeline: **Adicionar retângulo** (`control:timeline-add-shape`) chama `Command::AddShape`;
  equivalente `./scripts/ui invoke timeline.add_shape invoke`. O clipe aparece na timeline com
  ícone e cor próprios e fica selecionado.
- Inspetor, aba **Vídeo**, seção **Forma** (`inspector.shape`): campos Largura, Altura,
  Preenchimento (com amostra da cor) e Raio (a 1B acrescentou os demais; ver [F068](shape-style.md)). Enter ou sair do campo confirma **uma** edição
  (`SetClipShapeField`, um passo de desfazer); um valor recusado volta ao valor do modelo e o erro
  aparece na barra de status. O valor igual ao atual não é uma edição. A seção não tem "Redefinir".
  Equivalente: `./scripts/ui set clip_shape:<id> width 55`. O nó `clip_shape:<id>` expõe todos
  os campos da forma (na 1A: `kind`, `width`, `height`, `fill` e `radius`; os da 1B estão em
  [F068](shape-style.md)).
- Posição, escala, opacidade, fades e demais ajustes ficam nas seções comuns do inspetor, as mesmas
  para vídeo, título e forma.

## Evidências e status

Testes executados no Windows (worktree `tessel-editor-tessel-video`). As contagens são as da entrega
1A; a 1B ampliou `shape::tests`, `shape_clip_tests` e `shape-browser.test.mjs` (números atuais em
[F068](shape-style.md)):

| Superfície | Teste | Resultado |
|---|---|---|
| Lib | `shape::tests` (4) em `crates/core/src/shape.rs`: ida e volta JSON, valores inválidos, edição de um campo sem tocar nos outros, chaves e tipos desconhecidos | passam |
| Lib / CLI headless | `crates/core/tests/shape_clip.rs` (11): plano sem arquivo, padrões, edição por campo com undo/redo, `clip.shape`, recusas sem mutação, mistura com mídia, salvar e reabrir, documento quebrado cita o clipe, divisão, `timeline.add_shape` em um passo, `shape.add` | passam |
| Árvore da UI | `state::tree::tests::the_shape_is_a_node_only_for_a_selected_shape_and_the_toolbar_adds_one` | passa |
| GPUI | `workstation::shape_clip_tests` (5): botão da timeline, campos do inspetor, uma edição por Enter, recusa que restaura o campo, `set` semântico que chega ao campo, evento atrasado não edita outra forma | passam |
| Runtime (Chromium) | `runtime/test/shape-browser.test.mjs`: pixels de dentro, fora, canto reto e arredondado, pílula, opacidade, posição, reuso do cache | passa |
| Quadro e export reais | `crates/render/tests/shape_frames.rs` (3, também com `TESSEL_REQUIRE_RUNTIME=1`): PNG do `render.frame` com pixels conferidos por coordenada, edição e undo byte a byte, reabertura com os mesmos pixels, MP4 exportado decodificado pelo FFmpeg | passam |

No runtime, o desenho usa o cache limitado de imagens geradas (os
contadores `text_*`): uma forma com a mesma especificação e o mesmo tamanho de tela não é
redesenhada.

### Limitações conhecidas

- Quando o tamanho da forma e o da tela têm paridades diferentes, as bordas ficam suavizadas em meio
  pixel. Os testes usam tamanhos pares.
- A cor é digitada como `#rrggbb`; não há seletor de cores nem paleta na janela.
- Não houve verificação com ponteiro físico nem captura de tela da janela aberta: a prova da janela é
  o teste GPUI sobre o mesmo `Workstation`.
- Esta etapa não inclui animações (etapas seguintes do [handoff](../migration/native-video-authoring-handoff.md));
  elipse, contorno, gradiente, sombra e máscara estão em [F068](shape-style.md).
