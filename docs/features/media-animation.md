# F073 — Imagem, GIF e vídeo animados na composição (BL169.6, etapa 6)

## Objetivo e limites

Importar vídeo, GIF animado e imagem e animá-los na composição pela timeline, pelo inspetor e pela
CLI comuns, **sem editor dedicado**. A auditoria da capacidade real mostrou o que já funcionava e o
que faltava. Esta entrega fecha as lacunas sem criar um tipo de clipe novo: a mídia segue sendo um
`ClipKind::Video` (imagem, GIF e vídeo são o mesmo clipe visual).

Auditoria (antes desta entrega):

| Tipo | Antes | Lacuna |
|---|---|---|
| Vídeo (mp4, mov, webm, …) | Importado, decodificado em quadros RGBA PNG, playback e seek exatos | Nenhuma de playback; sem animação por chaves |
| PNG, JPEG, WebP, BMP | Desenhado direto | Nenhuma |
| TIFF, JPEG 2000, JPEG-LS | Declarado como suportado em [F019](media-io.md) | O navegador do runtime não os lê: tela em branco. Passam a ser decodificados para um PNG derivado |
| GIF | Recusado no probe (`unsupported_format`) | Sem GIF. APNG e WebP animado importavam como a **primeira imagem** parada |
| Animar posição, zoom, rotação e opacidade de um clipe de mídia | Só inspetor estático, fades e Ken Burns | Sem quadros-chave de clipe; sem repetição (`loop`) de uma animação curta |
| Transparência no preview | O proxy JPEG padrão perdia o alfa | Proxy ignorado quando o asset tem alfa |

## Formatos e limites reais

A coluna "Provado" refere-se a `crates/render/tests/media_formats.rs` (pixels por coordenada no
`render.frame`, em ordem embaralhada de seek, com a camada de baixo à mostra onde há alfa).

| Formato | Quadros | Transparência | Como é desenhado | Provado |
|---|---|---|---|---|
| PNG | 1 | Sim | direto | pixels, alfa sobre a camada abaixo |
| WebP (imagem) | 1 | Sim | direto | pixels, alfa |
| BMP | 1 | Não | direto | pixels |
| JPEG | 1 | Não | direto | pixels |
| TIFF (RGBA) | 1 | Sim | PNG derivado no import (`Cache::decode_still`, `format=rgba`) | pixels, alfa, `decoded` só nele |
| GIF animado | todos | Sim (um bit por pixel) | quadros PNG derivados, na grade de tempo do GIF | todos os quadros, seek em qualquer ordem, alfa |
| APNG animado | todos | Sim | idem | todos os quadros, alfa |
| WebP animado | todos | Sim | idem | todos os quadros, alfa |
| Vídeo QuickTime Animation (`qtrle`, argb) | todos | Sim | quadros RGBA decodificados | alfa por coordenada |
| Vídeo ProRes 4444 (`yuva444p10le`) | todos | Sim | idem | alfa por coordenada |
| Vídeo H.264 (mp4/mov, yuv420p) | todos | Não (a metade "transparente" vira preto) | idem | cores por quadro |
| Vídeo VP9 (webm) | todos | **Perdida**: o FFmpeg decodifica opaco | idem | cores por quadro |

Limites declarados:

- **Tempo do GIF.** O import lê o instante de exibição de cada quadro dos pacotes (ffprobe). Se os
  atrasos têm um passo comum (máximo divisor comum), a grade é exata e cada quadro aparece pelo seu
  tempo real. Se não têm (atrasos irregulares sem divisor útil), a animação é conformada a uma grade
  de FPS médio e o import avisa `animation_timing_conformed` (o `export.preflight` repete o aviso).
- **Uma só imagem** num contêiner animado (GIF de um quadro) é tratada como imagem (`still`).
- **BMP e JPEG não têm alfa.** O VP9 em webm perde o alfa porque o decoder do FFmpeg não o expõe.
- **Preview.** O proxy JPEG padrão é ignorado para assets com alfa (`Reason::HasAlpha`), para que a
  transparência apareça igual no monitor e no export.
- **`loop`.** Uma animação curta repete para preencher o clipe. Só a imagem repete: o som do vídeo
  não repete (aviso `loop_audio_not_repeated`). Sem `loop`, um clipe maior que a fonte continua
  recusado (`source_out_of_range`), como antes. O trecho que repete vai de `source_start` ao fim da
  fonte (`duração − source_start`).
- **Chaves em tempo de fonte** (ver abaixo): num clipe com `loop`, as chaves repetem junto com a mídia.
- Uma faixa de zoom **substitui** o Ken Burns do clipe (a deriva é um par de chaves de zoom).

## Contrato do core / biblioteca

- `crates/media/src/animated.rs`: `is_animated_format`, `grid_of` (grade exata ou fallback por FPS
  médio), `display_times` (tempos dos pacotes), `probe_animated`. Devolve `None` para uma imagem só.
- `crates/media/src/probe.rs`: GIF deixou de ser recusado. Animado vai para `probe_animated`; senão é
  `Probe` de `MediaKind::Still`.
- `crates/media/src/derive.rs`: `Cache::decode_still` e o braço `MediaKind::Still` em `prepare` para
  formatos que o navegador não lê (`Meta::still_needs_png`).
- `crates/core/src/assets.rs`: `NATIVE_STILL_FORMATS`, `Meta::still_needs_png`, `Asset::has_alpha`.
- `crates/core/src/backend.rs`: um still com quadros derivados vira `{"type":"file","path":…}` no plano.
- `crates/core/src/clip_keys.rs`: modelo de chaves de clipe. Reaproveita `group::KeyTrack/Key/Prop` e
  `easing::KeyEasing` (`linear`, `easeInOutCubic`, `easeOutCubic`, `smoothStep`). Propriedades:
  `position_x`, `position_y`, `zoom`, `rotation`, `opacity`, nos mesmos valores do inspetor
  (porcentagem, graus). O tempo da chave é **inteiro em ms do tempo de fonte** do clipe
  (`source_start` + decorrido na velocidade do clipe): mover, aparar a cabeça ou dividir o clipe não
  reescreve chaves. Opacidade com faixa continua multiplicada pelos fades. Cada propriedade aceita
  até `group::MAX_KEYS` (1024) chaves, o mesmo limite das trilhas de grupo ([BL169.6.9](../backlog.md));
  uma chave a mais é recusada (`invalid_value`) sem alterar o clipe.
- `crates/core/src/plan.rs`: `loop_span`, `wrap`, valor chaveado de transformação e opacidade
  (`clip_keyed_value`). Monitor, `render.frame` e export usam o mesmo plano.
- Persistência: esquema 2 só quando há chaves ou a propriedade `loop`; o editor do esquema 1 recusa.

## CLI e operações

| Superfície | Operação |
|---|---|
| Importar | `media.import` (GIF, APNG, WebP, TIFF, vídeo, imagem) |
| Colocar | `clip.add` (aceita `properties`, p. ex. `{"loop": true}`, aplicadas atomicamente) |
| Repetir | `clip.property clip=<id> property=loop value=true` (seção do inspetor) |
| Chave nova ou alterada | `clip.key clip=<id> prop=<prop> time_ms=<ms> value=<n> [easing=<e>]` |
| Chave no ponto de leitura | `clip.key.capture` |
| Editar uma chave | `clip.key.set clip=<id> prop=<prop> index=<n> field=<time_ms\|value\|easing> value=<…>` |
| Remover | `clip.key.remove`, `clip.keys.clear` |

Cada operação tem undo/redo byte a byte. Uma edição inválida (tempo duplicado, valor fora do limite,
propriedade desconhecida) é recusada sem alterar o documento.

## Janela

- Seção **Quadros-chave** do inspetor do clipe: uma linha por chave (tempo e valor) e botões para
  adicionar uma chave no ponto de leitura, remover e limpar. A árvore semântica expõe
  `clip_keys.capture:<prop>`, `clip_keys.clear:<prop>`, `clip_keys.clear` e `clip_key:<clip>:<prop>:<n>`
  para `ui op set|invoke|tree`.
- Interruptor `loop` entre as propriedades comuns do clipe.
- Limite: o valor de uma chave existente se edita por `clip.key.set` ou pelo `ui op set` do nó;
  o botão de reset da seção fica desabilitado na janela (o `modified` do nó da árvore já conta as chaves).

## Evidências

- Mídia (`crates/media`): 5 testes de unidade em `animated.rs` (grade exata, fracionária, atrasos
  mistos, formatos reconhecidos, uma imagem não é animação).
- Chaves do core (`crates/core/tests/clip_keys.rs`, 6): interpolação e retenção, easing e fades,
  edições recusadas, undo/redo byte a byte, salvar/reabrir e recusa do esquema 1, aparar/mover/dividir,
  fluxo dos nós.
- GPUI (`crates/desktop/src/workstation/clip_keys_tests.rs`, 2).
- Quadro e export reais (`crates/render/tests/media_formats.rs`, 6, com `TESSEL_REQUIRE_RUNTIME=1`):
  1. todos os formatos de imagem e a transparência sobre a camada abaixo;
  2. GIF, APNG e WebP animados, todos os quadros, seek em qualquer ordem;
  3. vídeos com e sem alfa;
  4. `loop` (a imagem n no quadro n módulo 3; sem `loop` o clipe é recusado; desligar mantém o último quadro);
  5. chaves de opacidade em PNG, GIF e MP4 e de posição, com seek para trás exato;
  6. GIF em `loop` e com chaves: monitor == export == reabertura.

Não houve ponteiro físico nem captura da janela real. Alvos vermelhos fora desta entrega:
`scene_gizmo_rotation`, `scene_gizmo_scale`, `directed_resources_transport`, `media/scene_export`,
`scene_resource_integration`, `scene_resource_publication` e `render/scene_export` (`shasum` ausente
no Windows), iguais antes das mudanças.
