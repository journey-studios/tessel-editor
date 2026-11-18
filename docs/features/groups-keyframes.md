# F069 — Grupos reutilizáveis, estilos de texto e quadros-chave (BL169.6, etapa 2)

## Objetivo e limites

A etapa 2 junta as formas da [1A](shape-clips.md)/[1B](shape-style.md) e o texto em **grupos**:
uma definição pequena (filhos editáveis desenhados em torno de uma origem) guardada **uma vez** no
projeto e mostrada por quantos **clipes de grupo** a timeline tiver. Editar um filho, um estilo ou
uma chave muda todas as instâncias. O clipe só leva o que é local: qual definição mostra e,
opcionalmente, outro texto para um filho de texto. Posição, zoom, rotação, opacidade, fades,
mistura e correção de cor do clipe (as propriedades comuns do inspetor) movem o grupo inteiro.

O grupo **não é um nó do compositor**: o plano (`RenderPlan`) expande cada instância em camadas
comuns de forma e de texto no quadro pedido (ids `layer:<clipe>:<filho>`), então o monitor, o PNG
de `render.frame` e o export desenham as mesmas imagens com o runtime que já existia (nenhum
arquivo do runtime mudou).

### Definição de grupo

| Campo | Unidade | Limites |
|---|---|---|
| `name` | texto | até 80 caracteres |
| `stagger_ms` | ms | 0 a 10 000; o tempo próprio do filho de índice *i* é `tempo do clipe − i × stagger_ms` |
| `children` | lista | até 32 filhos; a ordem é de baixo para cima |

Até 64 grupos e 64 estilos de texto por projeto. Um filho tem `id`, `kind` (`shape` ou `text`),
os valores base `x`, `y` (% da tela), `scale` (1 a 1000 %), `rotation` (graus) e `opacity` (0 a
100 %), o conteúdo (`shape`, o mesmo `ShapeSpec` da 1B, ou `content` + `style` no texto) e as
trilhas de quadros-chave (`tracks`).

### Estilos de texto

Um estilo (`id`, `size` % da altura, `color`, `font`, `align`) é do **projeto**: todo grupo que o usa
muda junto. Remover um estilo em uso ou apontar um filho para um estilo inexistente é recusado.

### Quadros-chave, easing, escalonamento e repetição

- Uma trilha por filho e propriedade: `x`, `y`, `scale`, `rotation`, `opacity` e, só em forma,
  `width`, `height`, `radius`, `stroke_width` (faixas: x/y ±200, escala 1..1000, rotação ±3600,
  opacidade 0..100, largura/altura (0, 200], raio 0..100, contorno 0..20). Até 64 chaves por
  trilha, tempo de 0 a 600 000 ms. Com chaves, elas **valem mais** que o valor base.
- Entre duas chaves o valor interpola linearmente, com o easing da chave **anterior** (`linear`,
  `easeInOutCubic`, `easeOutCubic` e `smoothStep`, este último entrou na [F072](reveal-beam.md));
  antes da primeira mantém o primeiro valor e depois da última, o
  último.
- Os tempos são do relógio do **clipe** (ms desde o primeiro quadro): mover ou duplicar o clipe leva
  a animação junto.
- `repeat` é o número de execuções (`0` é para sempre, até 1000); o ciclo é o intervalo entre a
  primeira e a última chave. `restart` volta à primeira chave a cada ciclo; `pingpong` toca os
  ciclos alternados de trás para frente, sem salto.
- Rotação é horária, o eixo y desce e o deslocamento do filho gira com o grupo.

## Compatibilidade

- O documento ganha o bloco de topo `"groups"` (`defs` e `styles`) **somente quando há algo nele**.
  Um projeto sem grupos continua byte a byte como a 1B gravava e abre em editores anteriores.
- Um projeto com grupos grava `schema: 2`; um documento `schema: 1` com `groups` é recusado, e uma
  referência pendente (clipe para grupo, texto para filho, filho para estilo) recusa a abertura
  citando o clipe. Um editor anterior recusa o esquema 2.
- Todo comando valida um livro candidato **antes** de gravar: um valor recusado deixa documento,
  histórico e revisão iguais. Apagar um filho remove as trocas de texto dos clipes que o citavam;
  qualquer outra edição que deixaria uma instância pendente é recusada (`in_use`, `unknown_element`).

## CLI headless

```json
{"op":"text_style.set","style":{"id":"title","size":8,"color":"#ffffff","font":"sans"}}
{"op":"group.define","definition":{"id":"card","name":"Card","stagger_ms":200,"children":[
  {"id":"panel","kind":"shape","shape":{"kind":"rect","width":60,"height":30,"fill":"#102030","radius":4},
   "tracks":[{"prop":"opacity","keys":[{"time_ms":0,"value":0,"easing":"easeOutCubic"},{"time_ms":400,"value":100}]}]},
  {"id":"title","kind":"text","content":"Hello","style":"title"}]}}
{"op":"clip.add","id":"card1","track":"overlay","start":0,"duration":90,"group":{"def":"card"}}
{"op":"group.set","group":"card","field":"stagger_ms","value":350}
{"op":"group.child","group":"card","child":{"id":"dot","kind":"shape","shape":{}}}
{"op":"group.child.set","group":"card","child":"panel","field":"x","value":5}
{"op":"group.key","group":"card","child":"panel","prop":"scale","time_ms":300,"value":100,"easing":"easeInOutCubic"}
{"op":"group.key.set","group":"card","child":"panel","prop":"scale","index":1,"field":"value","value":120}
{"op":"group.repeat","group":"card","child":"panel","prop":"scale","repeat":0,"mode":"pingpong"}
{"op":"clip.group_text","clip":"card1","child":"title","text":"Only this clip"}
{"op":"text_style.field","style":"title","field":"color","value":"#ffcc00"}
{"op":"set","target":"group_key:card/panel/scale/1","field":"value","value":120}
{"op":"invoke","target":"group_track:card/panel/scale","action":"remove"}
{"op":"plan","frame":12}
```

Alvos para `set`/`invoke`: `group_def:<g>` (`name`, `stagger_ms`), `group_child:<g>/<filho>` (`x`, `y`,
`scale`, `rotation`, `opacity`, `content`, `style` e os campos da forma), `group_key:<g>/<filho>/<prop>/<i>`
(`time_ms`, `value`, `easing`), `group_track:<g>/<filho>/<prop>` (`repeat`, `mode`), `text_style:<id>`,
`group_text:<clipe>/<filho>` (`text`; `invoke reset` volta ao texto do grupo), `clip_group:<clipe>`
(`group`) e `timeline.add_group`. O `/` separa as partes porque não é permitido num id.

Erros: `invalid_value` (faixa, tempo negativo, propriedade desconhecida), `unknown_field`,
`unknown_element` (grupo, filho, estilo, chave ou clipe), `not_applicable` (largura/raio/contorno
num filho de texto), `in_use` (remover grupo ou estilo em uso), `invalid_request`.

## UI da janela

- Botão **Adicionar grupo** (`timeline-add-group`) na barra da timeline: insere o clipe e um grupo
  inicial (painel + título, escalonamento de 200 ms, opacidade e deslocamento com `easeOutCubic`) e
  o estilo `title` quando falta, em **um** passo de undo.
- Aba **Vídeo** do clipe de grupo, seção **Grupo** (`inspector.group`): grupo do clipe, nome e
  escalonamento; um bloco por filho (campos base, forma ou texto), com suas trilhas, chaves (tempo,
  valor, easing), repetição e modo; botões para gravar uma chave no cursor com o valor atual da
  propriedade (`GroupDef::value_at`), trocar a propriedade, remover chave, trilha e filho; texto
  deste clipe com botão de reverter; estilos de texto do projeto. Enter ou sair do campo confirma
  **uma** edição pelos mesmos comandos de `set`; um valor recusado volta ao valor do modelo e o erro
  vai para a barra de status; valor igual ao atual não é edição.
- Clipes de grupo têm cor e ícone próprios na timeline e o monitor os trata como camadas visuais.

## Evidências e status

Executado no Windows (worktree `tessel-editor-tessel-video`), `TESSEL_REQUIRE_RUNTIME=1` nos testes
de quadro e export:

| Superfície | Teste | Resultado |
|---|---|---|
| Lib | `group::tests` (8): interpolação e easing, estilo, escalonamento, repetição `restart`/`pingpong`/infinita, limites, ida e volta do JSON | passam |
| Árvore da UI | `state::tree::tests` (um teste novo): o botão, os nós de grupo, filho, chave, trilha, estilo e texto | passa |
| Core / CLI headless | `crates/core/tests/group_clips.rs` (13): camadas do plano de baixo para cima, chaves com easing e escalonamento, repetição, propriedades do clipe sobre o grupo, troca de texto por clipe, edição de filho e estilo refletida, salvar e reabrir, undo e redo de uma sequência de edições, recusas sem mutação, esquema 1 + grupos recusado | passam |
| GPUI | `workstation::group_clip_tests` (5): o botão insere clipe e definição e um undo remove os dois, uma entrada por campo mostrando o modelo, Enter confirma uma edição desfazível, valor recusado volta ao modelo, troca de texto sem tocar a definição, evento atrasado não escreve em outro grupo | passam |
| Quadro e export reais | `crates/render/tests/group_frames.rs` (5): **cartão** (painel e título, escalonados em 200 ms): quadro 0 vazio, quadro 6 com o painel a 62,5 % `[125,63,31]` e sem título, quadro 60 com o painel `[200,100,50]` nas coordenadas exatas e título branco; troca de texto, cor do estilo e largura do filho mudam o quadro e cada undo devolve os bytes do PNG idênticos. **Sequência de etapas** (três quadrados em x −30 %, 0, +30 %, 500 ms entre si, opacidade em 100 ms e a primeira cresce de 50 % a 100 %): centros pretos no quadro 0, só a primeira no quadro 6 (com 75 % de tamanho), a segunda no 18 e as três nos corretos `[200,100,50]`, `[50,200,100]`, `[50,100,200]` no 36; `stagger_ms` 500 → 250 muda o quadro 12 e o undo o devolve. **Repetição**: barra com rotação 0→90° em 500 ms, `pingpong` infinito: deitada nos quadros 0 e 24 (igual byte a byte), de pé nos quadros 12 e 36, diagonal no 6. **Reabertura**: três quadros idênticos antes e depois de salvar e abrir, com o bloco `groups` no documento. **MP4**: a sequência exportada (H.264) lida pelo FFmpeg: preto no quadro 0, só a segunda etapa verde no 18, as três nos centros no 36 (±8 a 10 níveis) | passam |

Também rodados: `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`
e `cargo build -p tessel-editor --locked` limpos; `cargo tree -p tessel-editor-core` não traz GPUI.
O `cargo test --workspace --locked --no-fail-fast` termina com 6 alvos vermelhos, todos em código
que a etapa não toca. Cinco deles falham **igual no HEAD sem as mudanças** (clone limpo do commit
`70e61c6`, mesma contagem de testes vermelhos): `scene_gizmo_rotation` (1), `scene_gizmo_scale` (1),
`media/directed_resources_transport` (3), `media/scene_resource_integration` (1) e
`media/scene_resource_publication` (2). O sexto, `render/scene_export` (2 testes, `the_headless_binary_mixes_the_declared_wav_at_the_requested_range`
e `declared_audio_is_mixed_video_only_is_explicit_and_unevaluated_domains_are_refused`), falha com
`program not found` ao chamar `shasum`, que não existe neste Windows; no clone limpo o alvo "passa" em 0,01 s
porque o runtime não está instalado lá, então **não há comparação com a base** para esse alvo. Nenhuma
regressão é atribuída à base nem a esta entrega sem essa ressalva.

### Limitações conhecidas

- Os campos são entradas de texto: não há seletor de cor, de easing nem editor visual de curvas
  (o easing e o modo trocam num botão), nem arrasto das chaves ou das formas no monitor. O botão
  de chave grava no cursor do clipe; não há marcadores de chaves na timeline.
- Há um easing por chave, entre as três do `camera_keys` (`linear`, `easeInOutCubic`,
  `easeOutCubic`) mais `smoothStep` (`KeyEasing`, só grupos e revelação; câmera, FX e transição
  seguem com as três); sem curvas Bézier livres nem chaves de cor.
- As propriedades animáveis são as listadas: nenhuma cor, caminho, contorno colorido, sombra ou
  texto é animável. `width`, `height`, `radius` e `stroke_width` redesenham a imagem da forma a
  cada quadro em que mudam (o cache do runtime guarda as imagens recentes); uma animação longa
  desses campos custa um desenho por quadro, ao contrário de posição, escala, rotação e opacidade,
  que reaproveitam a mesma imagem.
- Grupos não se aninham; um filho é forma ou texto. O texto de um filho é de uma linha de estilo único (as
  animações de título da [F066](animated-titles.md) não se aplicam aos filhos).
- A troca de texto é por clipe e por filho; não há outros campos locais. Digitação, revelação por linha,
  agenda por fter/delay_ms e caixa de texto vieram na [F070](chat-typing.md).
- Os tempos são em ms contados do primeiro quadro do clipe: as chaves não se reescrevem sozinhas
  quando o início do clipe muda (não há deslocamento de chaves ao cortar).
- Um editor anterior recusa o esquema 2. Projeto sem grupos continua com o esquema 1.
- Sem verificação com ponteiro físico nem captura de tela da janela aberta: a janela foi provada
  pelo teste GPUI sobre o mesmo `Workstation`. A janela do usuário não foi aberta nem fechada e o
  socket dela não foi usado.
