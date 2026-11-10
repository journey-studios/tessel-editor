# F058 — Chaves, easing e seed dos efeitos (BL082.2, P3)

## Objetivo e limites

Os parâmetros de um clipe de efeito (F016/F057) podem ser **animados por chaves** (`keys`) e os efeitos que
desenham ruído têm uma **seed**. Tudo é regra do core: o importador valida, o resolvedor puro amostra, o plano
carrega o valor já resolvido, o runtime só desenha. A biblioteca, o JSONL sem janela e a seção "Efeitos" da
janela editam o mesmo documento pelos mesmos comandos. Não é a timeline dirigida (P6): não há faixa de
chaves, nem arrasto de diamantes; o movimento de chaves é por controle semântico e por gesto ordenado.

Fora desta entrega (continuam recusados com código tipado): escopo `shot`/`node` (`FX_SCOPE_NOT_YET`) e
`transitions` (`FX_TRANSITION_NOT_YET`). O código `FX_KEYS_NOT_YET` deixou de existir: `keys` agora é válido.

Fonte (somente leitura, snapshot `d40ac3f7c0`): `fx/fx-keys.ts`, `keyframes/{keyframe-rules,resolve-keyframes}.ts`,
`camera/easing.ts`, `fx/fx-clip-sample.ts`, `fx/fx-hash.ts`; blobs pinados em
`docs/migration/fixtures/pilot-fx/grain-manifest.json`.

## Contrato do core / biblioteca

### Formato

```json
{ "id": "g1", "effect": "filmGrain", "startFrame": 0, "durationFrames": 48,
  "params": { "amount": 0.5 },
  "keys": { "amount": [ { "frame": 0, "value": 0.0 }, { "frame": 20, "value": 1.0, "easing": "easeOutCubic" } ] },
  "seed": 2024 }
```

- `params` é a **base** estática; `keys` associa o ID de um parâmetro a uma curva `{frame, value, easing?}`.
- `frame` é inteiro **local ao clipe**, em `[0, durationFrames)`. Frames estritamente crescentes, sem duplicatas.
  O importador **não** ordena nem deduplica: a recusa é `KEYFRAME_ORDER`; fora do clipe, `KEY_OUT_OF_CLIP`
  (um frame negativo dá só `KEY_OUT_OF_CLIP`; um frame fracionário, `INVALID_VALUE`); easing desconhecido,
  `UNKNOWN_KEYFRAME_EASING`; parâmetro inexistente, `UNKNOWN_PARAM`; tipo errado, `PARAM_TYPE`/`PARAM_OUT_OF_RANGE`.
- Valor da chave tem o tipo do parâmetro (número, cor `{r,g,b}`, ponto `{x,y}` ou **booleano**); a cor e o ponto
  guardam o valor inteiro, nunca um canal solto.
- `seed` é `uint32` (0 a 4294967295); ausente significa "derivada do ID do clipe" e **nunca** é materializada
  no documento ao consultar ou renderizar. Seed negativa, fracionária ou acima de `u32::MAX` é recusada com `INVALID_VALUE`.
- Orçamento de dados (recusa `FX_LIMIT`, não truncamento): 256 chaves por curva, 1024 por clipe, 16 384 por camada.
  É independente do aviso de 8 passes (`FX_PASS_BUDGET`), que continua só avisando.
- Campos desconhecidos do documento são preservados (contrato lossless); um `keys` sem curvas sai do documento.
  Bundles sem `keys` e sem `seed` mantêm bytes, planos e pixels de antes (golden dos planos de vinheta e fade).

### Amostragem (`directed_scene::fx::keys`, função pura)

1. Sem chaves: `params`, senão o padrão do catálogo.
2. Com chaves: antes da primeira **segura o valor da primeira**; depois da última, o da última. A base não é usada
   antes da primeira chave.
3. No frame exato de uma chave devolve o **valor armazenado** (sem interpolar em t = 1).
4. Entre chaves o easing é o da **chave de chegada** (`linear`, `easeInOutCubic`, `easeOutCubic`; sem hold/step).
5. Números interpolam linearmente após o easing; **cores** interpolam por canal em luz linear; pontos por componente.
6. **Booleanos** mantêm o valor anterior até o frame da próxima chave e então assumem exatamente o novo; sem frações.
7. A intensidade animada é resolvida antes de `amount = clamp(intensity × weight)`; envelope e intensidade entram uma só vez.
8. Nada depende da ordem dos pedidos, de frames anteriores ou de relógio; mesma entrada, mesma saída.

O mesmo resolvedor serve ao plano, à captura de uma chave e ao inspector; não há segunda implementação em JS.

### Seed

`seed_at(frame_in_clip)` = base (explícita, ou FNV-1a dos **bytes UTF-8** do ID) combinada com o frame por
`lowbias32(base ^ frame)` quando o efeito é `per-frame` (o `filmGrain`). Detalhes e vetores em [F059](film-grain.md).
Só os efeitos que desenham ruído carregam `seed` no passe do plano (os bytes dos planos de vinheta/fade não mudam).

### Operações (`scene.fx.edit`, lote atômico, uma entrada de undo)

| Op | Contrato |
|---|---|
| `key_param {clip,param,frame,value?,easing?}` | Insere em ordem ou **substitui** nesse frame. Chave nova sem easing fica sem easing escrito (vale `linear`); chave existente **preserva** o easing quando omitido. Sem `value` captura o valor avaliado pelo core no frame (a base só enquanto nada está animado) |
| `set_key_easing {clip,param,frame,easing}` | Altera a chave exata; nomear um easing não escrito grava-o uma vez |
| `remove_key {clip,param,frame}` | Remove só essa chave; inexistente é `UNKNOWN_KEYFRAME`. Remover a última remove a curva e **revela a base**; `params` nunca é tocado |
| `move_key {clip,param,from,to}` | Move sem cruzar vizinhos nem colidir (recusa atômica); preserva valor e easing |
| `shift_keys {clip,param?,delta}` | Desloca a curva (sem `param`, todas as curvas do clipe) com aritmética checada; recusa se alguma chave sair do clipe |
| `scale_keys {clip,param?,anchor,num,den}` | Distância de cada chave à âncora vezes `num/den` (racional positivo, meio para cima, aritmética exata em i128); colisão ou fora do clipe é recusa, nunca deduplicação |
| `clear_keys {clip,param?}` | Remove a animação do parâmetro (sem `param`, de todo o clipe); explícito |
| `set_seed {clip,seed}` | `uint32` explícita, ou `null` para voltar à derivação pelo ID; mesma seed é no-op |
| `set_param` (existente) | Edita só a **base**. Sob chaves a resposta traz o aviso `FX_STATIC_UNDER_KEYS` e a curva é mantida |

Regras: o reset do visual **não** apaga animação nem seed; um lote que se anula não cria undo; dry-run/`inspect`/`frame`
não gravam; encolher a duração ou o clipe com chave fora do novo intervalo é recusado atomicamente (divergência
declarada abaixo). Cada op trabalha numa cópia do JSON e o **importador** julga o conjunto (única regra de validade).

### Gesto ordenado (`scene.fx.gesture.*`)

`begin` captura a base; cada `update` traz a lista completa de ops e deriva dela (não do update anterior); uma proposta
recusada mantém o **último candidato legal**; `commit` é **uma** entrada de undo (zero se voltou ao início); `cancel`
e a invalidação (cena, projeto, undo externo, revisão) não confirmam nada. Mover, deslocar e escalar chaves usam o
mesmo ciclo que o valor dos sliders.

### Inspector (`scene_edit::fx_animation`, F057)

`scene.fx.inspect {scene, frame?|time?}` devolve, por clipe, `animation`: para cada parâmetro `base`, `sampled`
(no frame pedido), `animated`, `keys` (frame/valor/easing) e `operations` com `enabled`, `reason` tipado e o `template`
da op que seria enviada; para a seed, `explicit`, `base` e `at_frame`. O `value` estático de P2 mantém seu significado.
Nada chaveia por si: a captura é sempre uma op enviada pela pessoa ou por um script.

## CLI headless

`./scripts/headless` (JSONL): as ops acima em `scene.fx.edit`, o ciclo `scene.fx.gesture.*`, `scene.fx.inspect` com
`frame`, `scene.plan`. Prova com processo real (`tessel-headless`, sem janela, sem GPU):
`a_keyed_grain_session_through_the_real_process_answers_like_the_library` em `crates/core/tests/scene_fx_keys.rs`.

## UI e CLI da janela

Seção "Efeitos" (`workstation/scene_fx_keys.rs`, Kit + tokens): por parâmetro, botão de capturar chave no frame do
monitor, remover e retemporizar; ícone de easing por chave; passo/automática da seed; `Switch` para booleano.
Nó `scene.fx.clip:<id>`, todos pelos handlers compartilhados (`SceneCommand::FxEdit`):

- campos: `seed` (uint32; só efeitos com ruído, `not_applicable` em outros), `key:<param>:<frame>` (coloca o valor
  nessa chave), campos booleanos `"type":"boolean"` (um número é `invalid_value`);
- ações: `key:<param>` (captura no frame do monitor; `frame_outside_clip` fora do clipe), `key:<param>:<frame>`,
  `remove_key:<param>:<frame>`, `easing:<param>:<frame>:<nome>`, `move_key:<param>:<de>:<para>`,
  `shift_keys:<param>:<delta>`, `remove_animation:<param>`, `seed:auto`, cada uma com `enabled` e `disabled_reason`;
- o estado do nó inclui `animation` (o JSON do core). Os sliders editam a **base**; nunca criam chave.

Equivalentes: `./scripts/ui invoke scene.fx.clip:fx-1 key:amount`, `ui set scene.fx.clip:fx-1 seed 77`,
`ui invoke scene.fx.clip:fx-1 easing:amount:10:easeOutCubic`. Apresentação (foco, painel) é N/A fora do adapter.

## Divergências declaradas em relação ao piloto

- Reduzir duração/redimensionar com chave fora do novo intervalo é **recusa atômica** (o piloto edita o clipe).
- Um easing que ninguém escreveu continua não escrito (o piloto o materializa como `linear`).
- Frame de chave negativo dá só `KEY_OUT_OF_CLIP`; fracionário, `INVALID_VALUE`.
- Capturar (`key_param` sem valor) sobre chave existente não redefine o easing.

## Evidências e status

Estado: **validado neste branch** (macOS/SwiftShader), sem GPU real, sem Windows, sem ponteiro físico (BL043), sem
execução do motor original (`ORIGIN_NOT_EXECUTED`). Números e hashes no relatório de entrega e em
[F059](film-grain.md#evidências-e-status).

| Superfície | Prova |
|---|---|
| Lib (core) | `crates/core/tests/scene_fx_keys.rs` (19): importador, amostragem exata, ops atômicas, gesto, persistência, plano; unitários de `keys`, `hash`, `fx_animation` |
| Corpus | `crates/core/tests/pilot_grain_corpus.rs` contra `grain-cases.json` (interpolação, easing, seed) gerado por `scripts/pilot-grain-corpus.py` independente; mutações discriminantes |
| JSONL (processo real) | teste de processo acima, plano igual ao da lib |
| Janela | `scripts/scene-fx-grain-ui-proof.py` (chaves por ações, easing, mover, remover animação, gesto, salvar/reabrir, export); GPUI `scene_fx_keys_tests.rs` (6) |
| Persistência/undo/export | `crates/render/tests/scene_fx_export.rs` (hashes do export iguais ao `scene.render`, re-export determinístico, seek, salvar/reabrir, um undo) |

Lacunas: sem timeline de chaves (P6); sem interpolação entre cenas; ponteiro físico não provado (BL043).
