# F072 — Revelação diagonal e feixe de luz entre tomadas (BL169.6, etapa 5)

## Objetivo e limites

Reproduzir a passagem dos 23 s da apresentação: a tomada seguinte aparece por uma aresta diagonal
que varre o quadro, e um feixe de luz acompanha a aresta. Nada disso é um tipo de clipe novo, um
editor dedicado ou um painel especial. São duas peças genéricas, as duas editáveis pela timeline,
pelo inspetor comum e pela CLI:

1. **Revelação** (`reveal_*`): propriedades de clipe, na seção **Revelação** do inspetor, para
   qualquer clipe visual (vídeo, imagem, título, forma, grupo, player, legendas). A aresta cruza o
   quadro, inclinada, e o clipe só aparece de um lado dela.
2. **Paradas de gradiente** (`stops`): a forma passa a aceitar até seis paradas de cor no
   gradiente, além de `fill` e `fill_to`.

O feixe é a composição dessas peças com o que já existia: um **grupo** (F069) de dois retângulos
girados, um com o gradiente de paradas (a faixa suave) e outro branco e fino (o traço), com chaves
de posição `x` no mesmo intervalo e com o mesmo easing da revelação. Não há ligação automática entre
a revelação e o feixe: os dois são chaveados pelos mesmos números (à mão, por receita ou por
preset futuro). `smoothStep` entra como quarto easing dos grupos e da revelação.

Limites desta etapa: a aresta só viaja na horizontal e só se inclina (sem modo vertical, circular
ou de formato livre); um clipe tem uma revelação; sem edição da aresta no monitor; os valores são
campos de texto ou controles comuns do inspetor.

## Contrato do core / biblioteca

- `crates/core/src/reveal.rs`: `RevealSettings { from, to, angle, duration, offset, easing }` e
  `at(local, largura, altura) -> Option<Reveal>`. `Reveal { edge, angle, rightward }` tem
  `direction()`, `edge_x()`, `coverage()` e `uncovers_everything()`. É geometria pura (sem GPUI).
- Geometria (canvas em px, y para baixo, `W`×`H`): a aresta cruza `(edge% de W, H/2)` e se inclina
  `angle` graus da vertical. Ângulo positivo leva a ponta de cima na direção do deslocamento.
  `direction` é +1 (mostra a esquerda da aresta, que anda para a direita) ou −1. Na linha `y`:
  `edge_x = edge/100·W + dir·tan(angle)·(H/2 − y)`; cobertura de um pixel `x`:
  `clamp(0,5 + (edge_x − x)·dir·cos(angle), 0, 1)` (borda de meio pixel suave).
- Tempo (quadros do clipe): `progresso = clamp((local − offset)/duration, 0, 1)`;
  `edge = from + (to − from)·ease(progresso)`. Depois que o quadro inteiro fica atrás da aresta a
  revelação some do plano (`None`), e o clipe desenha exatamente como um clipe sem revelação.
- `crates/core/src/easing.rs`: `KeyEasing { Linear, EaseInOutCubic, EaseOutCubic, SmoothStep }`,
  compartilhado por grupos e revelação (`smoothStep = t²(3 − 2t)`). Câmera, FX e transição seguem
  com os três easings de origem (`camera_keys::Easing`).
- Propriedades de clipe (só gravadas se diferentes do padrão, então projetos antigos não mudam):

  | Propriedade | Tipo | Faixa | Padrão |
  |---|---|---|---|
  | `reveal_mode` | `none`/`wipe` | | `none` |
  | `reveal_from`, `reveal_to` | % da largura | −100 a 200 (passo 0,5; 3 casas) | −20 e 120 |
  | `reveal_angle` | graus | −80 a 80 (2 casas) | 0 |
  | `reveal_duration` | quadros | 1 a 240 | 9 |
  | `reveal_offset` | quadros | −240 a 240 | 0 |
  | `reveal_easing` | `linear`/`easeInOutCubic`/`easeOutCubic`/`smoothStep` | | `smoothStep` |

  `from` > `to` inverte o sentido (a aresta anda para a esquerda e mostra a direita).
- `crates/core/src/plan.rs`: a camada do plano ganha `reveal {edge, angle, direction}` (só no JSON
  quando ativa). Um grupo dá a mesma aresta a todas as camadas em que se expande. Camada revelada
  não conta como "cobre o quadro" (não oclui o que está abaixo).
- Divisão (`clip.split`): a metade esquerda mantém a varredura; a direita recebe
  `reveal_offset − (quadros da metade esquerda)`, então cada quadro fica onde o clipe inteiro o
  teria. O piso de −240 só corta quando a varredura já terminou.
- `ShapeSpec.stops` (`shape.rs`): texto `"<pos> <#rrggbb[aa]> ..."`, `pos` em (0,1) estritamente
  crescente, até 6 paradas; a forma canônica é minúscula e separada por espaço. Vazio mantém o
  gradiente de duas cores de antes; o documento só grava `stops` se não vazio.
- Erros: valor fora da faixa é recusado (ou limitado) como as outras propriedades; um documento
  editado à mão com valor estrangeiro em `reveal_*` é recusado ao abrir; `reveal_*` em clipe de
  áudio é recusado. Em falha o projeto fica intacto.

## CLI headless

```json
{"op":"clip.add","id":"proxima","track":"meio","start":15,"duration":9,"asset":"tomada"}
{"op":"clip.property","clip":"proxima","property":"reveal_mode","value":"wipe"}
{"op":"clip.property","clip":"proxima","property":"reveal_from","value":-15.625}
{"op":"clip.property","clip":"proxima","property":"reveal_to","value":116.094}
{"op":"clip.property","clip":"proxima","property":"reveal_angle","value":12.33}
{"op":"group.define","definition":{"id":"beam","name":"Feixe","children":[
  {"id":"glow","kind":"shape","rotation":12.326,"tracks":[{"prop":"x","keys":[
     {"time_ms":0,"value":-37.0,"easing":"smoothStep"},{"time_ms":300,"value":95.0}]}],
   "shape":{"kind":"rect","width":10.7,"height":130,"radius":0,"fill":"#c6a7ff00",
            "fill_to":"#c6a7ff00","fill_mode":"linear","gradient_angle":0,
            "stops":"0.58 #c6a7ff60 0.92 #c6a7fffa"}},
  {"id":"line","kind":"shape","rotation":12.326,"shape":{"kind":"rect","width":0.15,"height":130,"fill":"#ffffff"}}]}}
{"op":"clip.add","id":"feixe","track":"topo","start":15,"duration":10,"group":{"def":"beam"}}
```

`scripts/render` (ou `target/debug/tessel-render`) com esse JSONL e `render.frame`/`export.start`
dá o PNG e o MP4. Nesta etapa não há receita de um comando só (ver dependências da etapa 6). Não há
gap de CLI: todo controle tem comando.

## UI e CLI da janela

- Inspetor, aba Vídeo, seção recolhida **Revelação** (antes de Tempo) para clipes visuais:
  seletor de modo, `De`, `Até` (%), `Ângulo`, `Duração`, `Atraso` (quadros) e `Curva`. Cada edição
  é um passo de undo. A forma ganha o campo `Paradas` na seção Forma.
- Equivalente: `./scripts/ui set property:reveal_mode value wipe`, `set property:reveal_angle value
  12.33`, `set clip_shape:<id> stops "0.58 #c6a7ff60 0.92 #c6a7fffa"`.
- Os campos aparecem na árvore semântica (`state/tree.rs`); o comando é o mesmo da CLI headless.

## Evidências e status

| Superfície | Teste | Resultado |
|---|---|---|
| Lib | `reveal::tests` (7): geometria, cobertura, tempo, direção, descarte; `easing::tests` (2); `shape::tests` (paradas) | passam |
| Core / CLI headless | `crates/core/tests/reveal.rs` (10): sem revelação = plano de antes, aresta escondida/antes, andando/durante, sumida/depois, varredura curta mantém a aresta, atraso/curva/direção, um undo por propriedade até o clipe liso, salvar e reabrir com os mesmos planos, documento com valor estrangeiro recusado, vale para título/forma/grupo e é recusado em áudio, grupo dá a mesma aresta a todas as camadas, corte no meio da varredura mantém todos os quadros | passam |
| GPUI | `workstation::shape_clip_tests` (12 no módulo; 2 novos): a seção Revelação é o inspetor comum em vídeo e forma; `stops` é entrada da seção Forma | passam |
| Runtime (Chromium) | `runtime/test/reveal-browser.test.mjs` (7): cobertura contra a referência de CPU, linha de aresta, ângulo, direção, camada sem revelação idêntica byte a byte depois de camadas reveladas, paradas do gradiente e **equivalência com a referência**: 208 pontos de prova contra o desenho Canvas 2D de `tools/motion-revisao.js` (máscara + faixa + traço), pior canal com diferença de 1/255 | passam |
| Quadro e export reais | `crates/render/tests/reveal_frames.rs` (4), canvas 320×180 a 24 fps: antes (só a tomada anterior), durante (aresta e feixe no lugar certo), depois (idêntico a um clipe sem revelação, feixe fora), PNG de `render.frame` e MP4 decodificado pelo FFmpeg; undo byte a byte; projeto salvo e reaberto desenha os mesmos bytes; easing `smoothStep` ≠ `linear` no quadro do meio | passam |
| Demonstração privada | `projects/tessel-apresentacao/tools/build-transition-23s-native.py` → `out/transicao-23s-nativa.{mp4,jsonl}` e `transicao-23s-nativa.tessel.json` (ignorados pelo git) | ver abaixo |

Também rodados: `cargo fmt --all -- --check`, `cargo check --workspace --locked`,
`cargo clippy --workspace --all-targets --locked -- -D warnings` e `cargo build -p tessel-editor
--locked` limpos; `cargo tree -p tessel-editor-core` sem `gpui` nem `wry`. O
`cargo test --workspace --locked --no-fail-fast` (com `TESSEL_REQUIRE_RUNTIME=1`) termina com sete
alvos vermelhos, todos fora desta entrega e já vermelhos antes dela: `scene_gizmo_rotation` e
`scene_gizmo_scale` (último dígito de ponto flutuante), `directed_resources_transport`,
`media/scene_export`, `scene_resource_integration`, `scene_resource_publication` e
`render/scene_export` (separador de caminho do Windows, travas, `shasum` ausente). Os testes do
runtime (`node --test test/*.test.mjs`, 512) passam, menos cinco `orchestrator-*` que não carregam
`InterVariable.woff2` no Node (já registrado na etapa 3).

### Demonstração privada dos 23 s

Um projeto separado de 60 quadros a 30 fps (1920×1080, quadros 675–734 da apresentação; o corte é o
quadro 690). A montagem principal (`tessel-apresentacao*.tessel.json`) não foi aberta nem alterada
(datas de gravação conferidas). A tomada anterior é a sequência de quadros de referência até o
corte, depois o último quadro congelado por 9 quadros (é o que o script de origem desenha); a
seguinte é um fundo preto liso e um quadro da referência, os dois com **as mesmas** propriedades
`reveal_*` (referência: aresta inclinada `atan(118/540)` = 12,33°, de x = −300 a x = 2229 px, 9
quadros a 30 fps, `smoothStep`), depois a sequência normal; o feixe é o grupo `beam` acima. Provas:

- **Posição da aresta contra a referência** (`tools/compare-transition-23s.py`, linha y = 1000 do
  quadro 1920×1080, coluna da linha branca): quadros 693 a 697 → 259/259, 658/658, 1077/1078,
  1476/1477 e 1813/1813 px (diferença de 0 a 1 px); a faixa suave 40 px à esquerda da linha tem
  RGB a 3–11 níveis do original (o quadro de referência é RGBA, com alfa de 240 a 249 nessa
  faixa; o nativo vem composto sobre o preto).
- **Antes e depois**: os quadros 689 e 699 a 701 do render nativo batem com os da referência
  (PSNR do RGB acima de 60 dB).
- **MP4** H.264 1920×1080, 60 quadros, 2 s (sem áudio: o corte de som não faz parte da prova da
  passagem); contato dos quadros 16 a 24 em `out/transicao-23s-nativa-contato.png`.

Limites da demonstração: a "tomada seguinte" é um quadro congelado (o quadro 699) durante os 9
quadros da varredura — o script de origem desenha a tomada seguinte 0,12 s adiante, em movimento —,
porque as entradas animadas dessa tomada ainda são PNGs pré-renderizados (a etapa 6 as troca por
grupos); o RGBA dos quadros de origem é composto sobre um fundo preto, não sobre os mundos SHARP;
legenda, logo e trilho de progresso vêm embutidos nesses quadros e são mascarados com eles (no
script de origem ficam por cima, sem máscara).

### Compatibilidade

Sem `reveal_mode` ou `stops` o documento é byte a byte o de antes (as chaves só são gravadas
quando diferem do padrão); o plano só ganha `reveal` quando a revelação está ativa; as formas sem
`stops` desenham como antes; `smoothStep` é um valor novo de `easing` (editor anterior recusa o
documento que o usa, como nas outras etapas). Testado: o clipe sem revelação planeja e desenha
exatamente o que desenhava.

### Limitações conhecidas

- A aresta só viaja na horizontal e se inclina; não há revelação vertical, circular, por forma
  livre, inversa nem com borda suave de largura configurável (a suavização é de meio pixel).
- Uma revelação por clipe. Revelar uma tomada de vários clipes exige as mesmas propriedades em
  cada um (ou um grupo, que dá a mesma aresta a todas as camadas).
- A revelação é em espaço de tela: ela não gira nem escala com o clipe.
- Dividir um clipe no meio da varredura: a metade direita continua pela soma em `reveal_offset`;
  o piso de −240 quadros só corta uma varredura que já terminou.
- O feixe não se liga sozinho à revelação (duas fontes dos mesmos números); mudar `reveal_*` não
  move as chaves do grupo.
- Arredondamento do controle: `angle` 2 casas, `from`/`to` 3 casas (≈ 0,02 px a 1920).
- Faixas: `reveal_offset` de −240 a 240, `reveal_duration` de 1 a 240 quadros.
- Sem edição da aresta no monitor nem marcadores na timeline. Sem verificação com ponteiro físico
  nem captura da janela aberta: a janela foi provada pelo teste GPUI sobre o mesmo `Workstation`.
  A janela e o socket do usuário não foram tocados.

### Dependências ainda necessárias para a etapa 6

1. Um **preset ou receita** que crie revelação + feixe juntos, com as chaves de `x` derivadas de
   `reveal_*` (hoje são dois conjuntos de números).
2. **Transição entre tomadas na timeline**: hoje a tomada anterior precisa continuar por baixo
   durante a varredura (alça estendida ou clipe congelado, como na demonstração) e a seguinte vai
   numa trilha acima; falta uma operação que prepare isso nos cortes marcados.
3. **Durações por corte** (a referência tem varreduras diferentes por tomada) e a lista dos 29
   cortes gráficos aplicada em lote.
4. **Tomadas de verdade sem PNGs**: os cartões, o player e os textos de cada tomada como grupos
   nativos (etapas 2 a 4 já cobrem as primitivas; falta compor cada tomada), para que a tomada
   seguinte entre em movimento sob a máscara em vez de congelada.
5. **Camada de cromo** (logo, legenda, trilho de progresso) acima das tomadas e fora da máscara.
6. Fundos SHARP por baixo da passagem (hoje só o preto liso).
