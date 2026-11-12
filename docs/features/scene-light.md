# F060 — Luz pontual com posição e alcance editáveis (G2.1, BL023/BL002/BL003/BL065)

## Objetivo e limites

A primeira fatia de iluminação de cenas dirigidas: **somente a luz pontual**, de ponta a ponta. O subconjunto de
`sceneDefinitions[].lighting.lights[]` é modelado no core, avaliado por quadro, entra no plano (`Capability::PointLights`),
é desenhado pelo runtime sobre planos de imagem, editado por comandos numéricos e por um gizmo (marcador, setas e um
puxador de alcance) que **reusa o gizmo de F055** e chama as mesmas funções puras do core que os comandos numéricos.

**Fora desta entrega (recusado com motivo tipado, nunca descartado):** ambiente diferente de 1, sombras, mais de
4 luzes, luz malformada; mapas de normais/relevo, clima/partículas, **iluminação de splats** (não são iluminados: ver
[Limite dos splats](#limite-dos-splats)), snapping, multisseleção, câmera espacial, efeitos de saída. O esquema da origem
não tem luz spot, direcional nem ambiente-como-luz: toda entrada de `lights[]` é pontual.

**ID.** F060 é o primeiro livre na `main` desta entrega (README até F059). Quem integrar por último confere a numeração.

Fonte (somente leitura, não executada): `lighting` do bundle da origem (`resolveLightFrame`, `MAX_SCENE_LIGHTS`, esquema
`lighting`). Nada da origem foi alterado ou copiado.

## Contrato do core / biblioteca

Tudo em `tessel-editor-core` (`directed_scene::lighting`, `scene_plan`, `scene_edit`, `scene_interaction::gizmo::light`,
`scene_project::gizmo::light`). Sem GPUI, janela, socket ou Chrome; `cargo tree -p tessel-editor-core` não lista gpui/wry.

### Unidades, espaços e convenções (decididas antes do código, fixadas por testes)

| Campo | Contrato |
|---|---|
| `position` | Unidades de cena. Sem `parent`: posição **no mundo**. Com `parent` (um nó da cena): **deslocamento somado à translação mundial** do pai no quadro (animação e sockets incluídos). Rotação e escala do pai **não** entram: a luz segue só a translação, como `resolveLightFrame` da origem. Logo, um deslocamento `Δ` da luz no mundo é exatamente `Δ` no `position` guardado. |
| `color` | RGB **linear**, cada canal em `[0, 1]`. |
| `intensity` | `[0, 20]`. |
| `radius` ("alcance") | Distância em unidades de cena em que a luz caiu a **um quarto**: `falloff(d) = (1 / (1 + (d/radius)²))²`, então `falloff(radius) = 1/4`. Não é corte duro. O editor desenha uma esfera desse raio. |
| `flicker` | `[0, 1]`; tira até 45% da intensidade com o ruído determinístico da origem (id da luz, tempo de edição em segundos); em 0 vale exatamente 1. |
| `keys.intensity\|color\|position` | Nos **quadros de edição**, com easing como `resolveKeyframes` da origem: antes da primeira chave vale a primeira, depois da última a última, na chave exatamente o valor. Um canal com chaves ignora o campo estático. |
| `hidden` | A luz fica no documento e no plano (marcada), **não é desenhada** e não ocupa slot; não exige a capacidade. |

Efeito sobre o pixel (runtime): no espaço **linear**, `cor_final = codifica(decodifica(cor) · (1 + Σ cor_luz · intensidade ·
max(0, n·l) · falloff²))`. `n` é a normal do plano voltada para a câmera (plano de dupla face: iluminado no lado que se vê),
`l` aponta do ponto para a luz. Sem luzes visíveis o ramo é pulado: a imagem é **idêntica** à de antes.

### Recusas (BL065)

`lighting` não é descartado por inteiro: o plano continua `unsupported` para o domínio, com `reason` tipado
(campo aditivo, omitido sem motivo): `ambient_not_neutral`, `shadows`, `too_many_lights`, `invalid_lighting`. O que
a fatia suporta (ambiente 1 e até 4 luzes válidas) é avaliado e desenhado. O aviso de escopo `deferred_in_use`
`sceneDefinitions[].lighting` do `scene.attach` **continua** (BL001: validações adiadas de outros campos da seção).

### Plano e capacidade

`PLAN_VERSION` segue 1; o campo `lights` é aditivo (ausente sem luzes: os bytes dos planos anteriores não mudam).
`Capability::PointLights` (`"pointLights"`) é exigida **somente** com ao menos uma luz visível, negociada como
`outputFx`/`fxNoise`: um runtime sem ela é recusado antes de enviar qualquer ativo ou quadro (`unsupported` /
`scene_plan_capabilities`, `missing` com `pointLights`), e o runtime recusa `invalid_plan` (`/requires`) um plano com luz
visível sem a capacidade. Nenhuma reconstrução do plano por quadro além da que já existe.

### Edição

Uma luz é `EntityKind::Light`, endereço `scene/<definição>/light/<id>`; propriedades: `position`, `position/x|y|z`,
`intensity`, `radius`, `color/r|g|b` (cor não tem propriedade vetorial: canais são números). O catálogo passa a 30
propriedades. Lote atômico, **um undo por gesto**. Valores fora da faixa → `invalid_value`. Canal com chaves → recusa
`animated_property` (uma edição nunca escreve chave e nunca desloca em silêncio).

### Gizmo (F055, sem matemática duplicada)

- Marcador selecionável (apertar o marcador seleciona a luz), setas `x/y/z` e centro `screen` da translação, mais o
  puxador `range` à direita da luz **como a câmera a vê**, e a esfera (3 grandes círculos). Distância arrastada = distância
  assinada do ponteiro ao longo desse eixo desde o clique: `radius' = radius + Δ`.
- `light_position` e `light_range` (puras, em `scene_interaction::gizmo::light`) são chamadas **pelo arrasto e pelos
  comandos numéricos** (`scene.translate`, novo `scene.range`): o documento resultante é igual bit a bit.
- Mesmo ciclo begin/update/commit/cancel, uma entrada de undo por gesto, sem histórico quando nada muda, candidato
  inválido preserva o último legal, e as guardas existentes de revisão/quadro/seleção/época. Nenhuma chave é escrita.
- Recusas (`disabled_handles` / erros tipados): `animated_position` (como `animated_scale`), `light_has_no_rotation`,
  `light_has_no_scale`, `light_not_evaluated`, `radius_not_positive`, `radius_too_large`, `position_not_finite`.
  `scene.range` recusa `not_rangeable`; `scene.translate` em algo intraduzível, `not_translatable`.
- JSON do overlay v1 (aditivo): `sphere`, `light_markers`, `metrics.marker_radius`, handle `kind: "range"`.

## Runtime (`runtime/`)

Luzes são **uniformes** dos materiais de plano existentes (`makeLightAware`, `customProgramCacheKey`
`tessel-point-lights-v1`): editar uma luz nunca recompila (`material_created` fica em 0 entre quadros). Até 4 luzes visíveis.
O marcador, a esfera e o puxador são desenhados no **canvas 2D do overlay**, separado do canvas WebGL: **não entram no quadro
exportado nem no documento** (provado por hash igual).

### Limite dos splats

**Splats não são iluminados.** O Spark os desenha com a própria cor; esta fatia não infere iluminação de splats (BL025 /
MIG058 seguem pendentes). Um plano com luzes e splats ilumina só os planos de imagem; o limite é declarado, não escondido.

## Superfícies

| Superfície | Como |
|---|---|
| Biblioteca | `scene.edit` (propriedades), `scene.range`, `scene.translate`, `scene.gizmo.state`, `scene.pointer.*`, `scene.plan` |
| CLI headless | as mesmas operações em `./scripts/headless` (JSONL); prova em `scripts/point-light-headless-proof.py` |
| UI (janela) | inspector da cena: `scene.property:<caminho>` de cor, intensidade, alcance e posição (campos numéricos e slider de alcance, mesma função do gizmo), marcador/esfera/puxador no monitor, `scene.entity:<caminho>` na árvore; recusas com `disabled_reason`; prova semântica em `scripts/point-light-ui-proof.py` |

Controles ausentes (rotação, escala, posição com chaves) ficam desabilitados com `disabled_reason`, nunca com callback vazio.
Nenhum diálogo novo (nenhum `Workflow::has_dialog` a marcar).

## Evidências e status

Status: **a validar** (código e provas locais executados; sem aceite humano, WKWebView física, ponteiro físico, desempenho
nem Windows). Executado no macOS com Chrome/SwiftShader:

- Core (`crates/core/tests/scene_light.rs`, 23): plano e capacidade, composição com pai transformado (só translação),
  chaves e easing, luz oculta (não exige a capacidade), recusas por motivo, edição numérica (um undo por edição), valores
  inválidos, posição animada recusada, marcador/esfera/puxador, arrasto de alcance == `scene.range` bit a bit, arrasto das
  setas == `scene.translate`, zero escreve nada, cancelar/sem mover não deixa histórico, candidato inválido mantém o último
  legal, gesto obsoleto invalidado.
- GPUI (`workstation/scene_light_tests.rs`, 6): nós do inspector, equivalência inspector × `scene.range`/`scene.translate`,
  recusa tipada, posição animada desabilitada com motivo, arrasto da página (uma entrada de undo, Esc sem rastro), slider.
- Runtime JS (`runtime/test/directed-scene-light.test.mjs`, 12): referência independente escrita no teste (luz de frente,
  raio, atrás/intensidade 0, plano inclinado ±40°, tinta, duas luzes, oculta == sem luz por hash, material retido, overlay
  fora do quadro).
- Render (`crates/render/tests/scene_light_frames.rs`, 3, runtime real): referência **escrita no teste** a partir dos números
  do plano; 147 sondas do céu comparadas, pior diferença por canal 0,68 (uma luz e duas); a comparação exige mudança visível
  ≥10; luz com pai segue o nó entre quadros; luz oculta == hash do baseline. `scene_light_export.rs` (1, FFmpeg real): hash
  de `scene.export` == `scene.render` por quadro, o export iluminado difere do apagado, `scene.edit` de raio 6,5 e
  `scene.range` +1,5 desenham a mesma imagem, undo restaura. `scene_seam.rs`: runtime sem `pointLights` recusado antes de
  qualquer quadro só com luz visível.
- `scripts/point-light-headless-proof.py` (JSONL sem desktop) e `scripts/point-light-ui-proof.py` (janela real, só pedidos
  semânticos).
- Mutações discriminantes (`scripts/point-light-mutation-proof.py`), uma por vez, arquivo restaurado por sha256: expoente
  da atenuação, iluminar por trás, ordem de cor, espaço sRGB, luz oculta ocupando slot, sinal do alcance, guarda de canal
  com chaves, luz oculta exigindo a capacidade — **8/8 mortas** (a última sobreviveu aos testes do core e ganhou uma asserção).

### Gates no código `945b9f8` (macOS, SwiftShader, árvore limpa)

`cargo fmt --all -- --check`, `./scripts/cargo check --workspace --locked`, `./scripts/cargo clippy --workspace --all-targets --locked -- -D warnings`
e `./scripts/cargo build -p tessel-editor --locked` passaram; `TESSEL_REQUIRE_RUNTIME=1 ./scripts/cargo test --workspace --locked --no-fail-fast`:
118 suítes, **2327 passaram, 0 falharam, 13 ignorados**, nenhum `SKIPPED`; runtime JS (`node --test`, runtime obrigatório) 255/255;
`cargo tree -p tessel-editor-core` sem gpui/wry. As provas `scripts/point-light-*-proof.py` passaram. Medição de desempenho **não** feita
(BL023.6, pendente).

### Não provado

Ponteiro físico, WKWebView nativa pintando o overlay, popover/foco de teclado, **desempenho** (medição separada sob o lock
da GPU; pendente), iluminação de splats, Windows, aceite humano. Um `ui op`/socket da janela não é fluxo visual completo.
