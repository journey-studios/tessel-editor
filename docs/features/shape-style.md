# F068 — Formas com estilo e máscara de imagem (BL169.6, etapa 1B)

## Objetivo e limites

A etapa 1B evolui o clipe de forma de [F067](shape-clips.md) sem quebrá-lo: além do retângulo há
**elipse** e **caminho** (traçado SVG restrito), **preenchimento** sólido, **linear**, **radial** ou
nenhum, **contorno**, **sombra** e uma **máscara arredondada ou elíptica** para clipes de imagem e
de vídeo. Tudo vive no core (`ShapeSpec` e duas propriedades de clipe), passa pelo mesmo plano
(`RenderPlan`) e é desenhado pelo mesmo runtime no monitor, em `render.frame` e no export.

Campos da forma (`set clip_shape:<id> <campo> <valor>`; cada um é uma edição desfazível):

| Campo | Unidade | Limites | Padrão |
|---|---|---|---|
| `kind` | — | `rect`, `ellipse`, `path` | `rect` |
| `width`, `height` | % da largura / altura da tela | maior que 0, até 200 | 40, 20 |
| `radius` | % da altura da tela | 0 a 100 (só `rect`) | 0 |
| `path` | traçado em caixa 100×100 | só `M L H V C Q Z` absolutos; até 128 segmentos; coordenadas de -200 a 300 | `M 50 0 L 100 100 L 0 100 Z` |
| `fill_mode` | — | `solid`, `linear`, `radial`, `none` | `solid` |
| `fill` | cor | `#rrggbb` ou `#rrggbbaa` | `#4a90e2` |
| `fill_to` | cor | idem; cor final do gradiente | `#ffffff` |
| `stops` | `<pos> <cor> ...` | paradas intermediárias do gradiente ([F072](reveal-beam.md)): `pos` em (0,1) crescente, até 6, `#rrggbb[aa]`; vazio = só `fill` e `fill_to` | vazio |
| `gradient_angle` | graus | 0 a 360; 0 vai da esquerda para a direita, 90 de cima para baixo | 0 |
| `stroke_width` | % da altura da tela | 0 a 20; 0 desliga o contorno | 0 |
| `stroke` | cor | idem | `#ffffff` |
| `shadow` | cor | idem (o alfa é a opacidade da sombra) | `#00000080` |
| `shadow_blur` | % da altura | 0 a 20; 0 é sombra dura | 0 |
| `shadow_x`, `shadow_y` | % da largura / altura | -30 a 30 | 0, 0 |

Semântica de desenho (a mesma no preview e no export):

- O tamanho declarado (`width`×`height`) é o da **caixa**. O contorno é centrado na borda (metade
  fora da caixa) e a sombra é lançada pela silhueta inteira (preenchimento + contorno). A caixa
  continua centrada no clipe quando há contorno ou sombra: a margem é simétrica.
- O gradiente linear percorre a caixa no ângulo dado. O radial é uma elipse inscrita na caixa: `fill`
  no centro, `fill_to` na borda (num retângulo, os cantos ficam além da borda e têm `fill_to`).
- O contorno usa quina viva em `rect` e ponta redonda em `path`. Um `path` é desenhado na caixa
  100×100 esticada para a caixa da forma; o texto é canonicalizado ao gravar (espaços e `-0`
  normalizados), então o mesmo desenho tem o mesmo texto e o mesmo cache.
- `fill_mode: none` com `stroke_width: 0` não desenha nada: o plano omite a camada e avisa
  `shape_draws_nothing` em vez de deixar um clipe invisível sem explicação.
- Máscara (`mask_shape`, `mask_radius`, propriedades do clipe, seção **Máscara** da aba Vídeo,
  só para imagem e vídeo; em forma é `not_applicable`): `none`, `rounded` ou `ellipse`. O raio é
  `mask_radius`% do **lado menor do retângulo já cortado** (0 a 50), e a máscara segue o corte
  (`crop_*`), a escala e a rotação do clipe. Fora da máscara a camada é transparente (o fundo
  aparece), com borda suavizada em meio pixel. Combina com correção de cor e opacidade. Uma camada
  mascarada deixa de ocultar o que está abaixo (`covers` é falso).

## Compatibilidade

- `to_document_json` grava os cinco campos da etapa 1A sempre e os da 1B **somente quando diferem do
  padrão**; a leitura devolve os padrões. Um projeto que só usa o que a 1A oferecia continua
  byte a byte como a 1A gravava e abre em um editor da 1A. Um projeto que usa elipse, caminho ou
  qualquer campo novo **não** abre em um editor da 1A (`unknown_field`/`kind` desconhecido).
- O plano (`layer.shape`) carrega todos os campos, então o runtime nunca depende dos padrões do
  editor que gerou o plano; um plano antigo (só os cinco campos) é completado pelos padrões do
  runtime. `layer.mask` só existe quando a máscara está ligada.
- A máscara é uma propriedade comum de clipe: um projeto sem máscara não ganha chaves novas.

## CLI headless

```json
{"op":"clip.add","id":"cartao","track":"frente","start":0,"duration":6,"shape":{"kind":"rect","width":60,"height":50,"radius":10,"fill":"#ff0000","fill_mode":"linear","fill_to":"#0000ff","gradient_angle":0,"stroke":"#ffffff","stroke_width":5,"shadow":"#000000","shadow_x":5,"shadow_y":10}}
{"op":"clip.add","id":"halo","track":"frente","start":6,"duration":6,"shape":{"kind":"ellipse","width":40,"height":40,"fill":"#ffd166","fill_mode":"radial","fill_to":"#ffd16600"}}
{"op":"set","target":"clip_shape:cartao","field":"shadow_blur","value":4}
{"op":"set","target":"clip_shape:halo","field":"kind","value":"path"}
{"op":"set","target":"clip_shape:halo","field":"path","value":"M 0 0 L 100 0 L 50 100 Z"}
{"op":"clip.property","clip":"capa","property":"crop_left","value":25}
{"op":"clip.property","clip":"capa","property":"mask_shape","value":"rounded"}
{"op":"clip.property","clip":"capa","property":"mask_radius","value":20}
{"op":"plan","frame":3}
```

`clip.add` e `clip.shape` aceitam qualquer subconjunto dos campos (os ausentes ficam no padrão).
Valor inválido é recusado antes de qualquer mutação, com o campo no erro: `invalid_value`
(`kind`, `fill_mode`, cor, número fora do limite, caminho com comando relativo, comando
desconhecido, aridade errada, mais de 128 segmentos ou coordenada fora de -200..300) e
`unknown_field`. Documento, histórico e revisão ficam iguais.

## UI e CLI da janela

- Inspetor, aba **Vídeo**, seção **Forma** (`inspector.shape`): uma entrada de texto por campo,
  agrupadas em Forma, Preenchimento, Contorno e Sombra, com a unidade ao lado e uma amostra para
  cada cor (com alfa quando há oito dígitos). Enter ou sair do campo confirma **uma** edição
  (`SetClipShapeField`); um valor recusado volta ao valor do modelo e o erro vai para a barra de
  status; valor igual ao atual não é edição. O nó `clip_shape:<id>` expõe os 15 campos e
  `./scripts/ui set clip_shape:<id> <campo> <valor>` chega à mesma entrada.
- Inspetor, aba **Vídeo**, seção **Máscara** (`inspector.mask`, recolhida por padrão) em imagem e
  vídeo: `mask_shape` (seleção) e `mask_radius` (controle deslizante), pelo mesmo caminho das demais
  propriedades (`property:mask_shape`, `set property:mask_shape value rounded`). Some em formas.

## Evidências e status

Executado no Windows (worktree `tessel-editor-tessel-video`), `TESSEL_REQUIRE_RUNTIME=1`:

| Superfície | Teste | Resultado |
|---|---|---|
| Lib | `shape_path::tests` (3) e `shape::tests` (10): gramática e canonicalização, limites, ida e volta, campos antigos e novos | passam |
| Lib / CLI headless | `crates/core/tests/shape_style.rs` (10): elipse + radial no plano, caminho canônico e undo, cada campo uma edição desfazível, recusas sem mutação, salvar e reabrir, projeto da 1A lido igual e retângulo simples gravado só com 5 chaves, `shape_draws_nothing`, máscara no plano só quando ligada, `not_applicable` em forma, valor inválido e limite | passam |
| Plano | `plan::tests` (máscara tira `covers`) e `crates/core/tests/shape_clip.rs` (11, adaptado: `kind` desconhecido agora é `star`) | passam |
| Árvore da UI | `state::tree::tests::the_shape_is_a_node_only_for_a_selected_shape_and_the_toolbar_adds_one` (15 campos) | passa |
| GPUI | `workstation::shape_clip_tests` (10): uma entrada por campo mostrando o modelo, oito campos novos editados por Enter (um passo de undo cada, desfeitos até o retângulo), recusas que restauram a entrada, `set` semântico que chega à entrada, seção Máscara só em mídia, uma edição = um undo | passam |
| Runtime (Chromium) | `runtime/test/shape-browser.test.mjs` (8): elipse, linear 0/90/180°, radial, contorno (dentro/fora/sozinho/quina viva), sombra dura e suave com deslocamento, caminho (triângulo, traço, curva, cache), caixa centrada com sombra; `runtime/test/mask-browser.test.mjs` (4): pixels da máscara contra a referência de CPU (`maskCoverage`), máscara segue o corte, combina com correção de cor e opacidade, camada sem máscara idêntica byte a byte depois de quadros mascarados | passam |
| Regressão do runtime | `grading`, `occlusion`, `retention`, `worker`, `warm-draw`, `preupload`, `pure` (72 testes no total com os acima) | passam |
| Quadro e export reais | `crates/render/tests/shape_style_frames.rs` (4): um projeto com **cartão** (retângulo arredondado, gradiente vermelho→azul, contorno branco, sombra dura), **halo** (elipse, gradiente radial) e **capa recortada** (imagem de quatro quadrantes, corte de 25% por lado, máscara arredondada) sobre fundo chapado. Pixels conferidos por coordenada no PNG (±1 a 2 níveis) e no MP4 decodificado pelo FFmpeg (±14 a 16 níveis); edição de sombra, modo do preenchimento e forma da máscara mudam o quadro e 4 undos devolvem os três quadros byte a byte; reabrir o projeto desenha os mesmos bytes e o documento guarda os campos novos | passam |

Também rodados: `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`,
`cargo check --workspace --locked` e `cargo build -p tessel-editor --locked` limpos. O
`cargo test --workspace --locked --no-fail-fast` termina com 8 alvos vermelhos, todos fora desta
entrega e em código que ela não toca: `scene_gizmo_rotation` e `scene_gizmo_scale` (último dígito
de ponto flutuante), `media --lib` (travas de arquivo com limite de 20 ms),
`directed_resources_transport` (separador `\` do Windows), `media/scene_export`,
`scene_resource_integration`, `scene_resource_publication` e `render/scene_export` (programa
externo ou separador de caminho). Nenhum arquivo dessas áreas foi alterado.

### Limitações conhecidas

- Os campos são entradas de texto: não há seletor de cor, seletor para `kind`/`fill_mode`, editor de
  caminho nem alças de arrasto sobre a forma no monitor.
- Caminho: só comandos absolutos `M L H V C Q Z`, um subtraço por `M`, sem arcos (`A`), relativos nem
  `S`/`T`. O preenchimento usa a regra não-zero do Canvas.
- Contorno, sombra e desfoque aumentam a imagem gerada com uma margem simétrica em volta da caixa.
  A margem é limitada para a imagem não passar de 8192 px por lado (limite de textura da GPU): numa
  forma de até 200% da tela com sombra grande, o excesso da sombra é cortado.
- Sombra e gradiente têm um ângulo cada; o gradiente aceita até 6 paradas intermediárias (`stops`,
  F072) e a sombra uma cor; não há sombra interna, contorno
  tracejado nem contorno interno ou externo (o contorno é centrado).
- A máscara é por clipe de imagem ou vídeo, não por grupo, e não há máscara por forma livre, inversa
  nem animada. A borda tem meio pixel de suavização.
- O gradiente e a sombra são desenhados em um canvas 2D e carregados como textura: um clipe
  que muda de estilo a cada quadro (quadros-chave, etapa 2) pagará um novo desenho por quadro.
- Sem verificação com ponteiro físico nem captura de tela da janela aberta: a janela foi provada
  pelo teste GPUI sobre o mesmo `Workstation`. A janela do usuário não foi aberta nem fechada.
- Um editor anterior a esta etapa recusa projetos que usam qualquer campo novo (ver Compatibilidade).
