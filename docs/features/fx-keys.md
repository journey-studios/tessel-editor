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
- Orçamento de dados (recusa `FX_LIMIT`, não truncamento): 256 chaves por curva, 1024 por clipe, 16 384 por camada
  e **1024 ops por lote** (`MAX_FX_BATCH_OPS`, também por `update` de gesto). Os limites de op/clipe/camada são
  conferidos **antes** de crescer ou copiar a curva (8000 chaves falham cedo, no `FX_LIMIT` de `ops`/`keys.<param>`,
  sem materializar nada) e o lote continua atômico (recusa = documento, histórico e revisão intactos; no-op válido
  continua sem entrada de undo). É independente do aviso de 8 passes (`FX_PASS_BUDGET`), que continua só avisando.
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
| `scale_keys {clip,param?,anchor,num,den}` | Distância de cada chave à âncora vezes `num/den` (racional positivo, meio para cima, exato (BL082.31): quociente/resto euclidianos, o numerador nunca é dobrado, então `i64::MIN` de âncora e `num = i64::MAX` ficam em `i128` e viram recusa tipada `KEY_OUT_OF_CLIP`, nunca pânico nem volta). Colisão ou fora do clipe é recusa, nunca deduplicação |
| `clear_keys {clip,param?}` | Remove a animação do parâmetro (sem `param`, de todo o clipe); explícito |
| `set_seed {clip,seed}` | `uint32` explícita, ou `null` para voltar à derivação pelo ID; mesma seed é no-op |
| `set_param` (existente) | Edita só a **base**. Sob chaves a resposta traz o aviso `FX_STATIC_UNDER_KEYS` e a curva é mantida |

Custo (BL082.32): as ops de chave única localizam o clipe e copiam no máximo uma curva; `shift_keys`,
`scale_keys` e `clear_keys` sem `param` podem copiar todas as curvas do clipe (até 1024 itens).
Os contadores determinísticos `FxWork` — `ops_run`, `document_scans`, `curve_items_copied` — são conferidos em
`crates/core/tests/scene_fx_key_bounds.rs`; o custo dominante de um lote é a revalidação do importador, **por lote**
e proporcional ao documento, não por op. Medição pareada (teste ignorado por padrão, perfil release, macOS, uma
amostra, só informativa): camada vazia 1 op 43 µs / 64 ops 164 µs; camada cheia (16 384 chaves) 1 op 13,4 ms /
64 ops 13,2 ms. Por isso o orçamento é por lote.

Regras: o reset do visual **não** apaga animação nem seed; um lote que se anula não cria undo; dry-run/`inspect`/`frame`
não gravam; encolher a duração ou o clipe com chave fora do novo intervalo é recusado atomicamente (divergência
declarada abaixo). Cada op trabalha numa cópia do JSON e o **importador** julga o conjunto (única regra de validade).

### Gesto ordenado (`scene.fx.gesture.*`)

`begin` captura a base; cada `update` traz a lista completa de ops e deriva dela (não do update anterior); uma proposta
recusada mantém o **último candidato legal**; `commit` é **uma** entrada de undo (zero se voltou ao início); `cancel`
e a invalidação (cena, projeto, undo externo, revisão) não confirmam nada. Mover, deslocar e escalar chaves usam o
mesmo ciclo que o valor dos sliders.

**Intenção do gesto (BL082.33, `scene_edit::fx_intent`).** O primeiro `update` aceito liga o gesto a uma intenção
(`FxIntent`: família do verbo + parâmetro + a chave única, nomeada pelo frame **da base em que o gesto começou**; um
`move_key` nomeia o `from`). Todo `update` seguinte com outro verbo (`gesture_intent_mismatch`) ou o mesmo verbo em
outro parâmetro/chave (`gesture_target_mismatch`) é recusado sem tocar o candidato, o último estado legal nem a
vinculação de revisão, e fica como a recusa mais recente. Misturar verbos num mesmo lote de gesto é recusado do mesmo
modo. Escritas estáticas (`set_param`, `set_field`, `set_hidden`) são **uma** intenção, `value`, como antes (um slider
pode trocar de campo entre updates). `scene.fx.edit` direto **não** é gesto e continua aceitando um lote deliberado
que mistura chaves e base.

### Inspector (`scene_edit::fx_animation`, F057)

`scene.fx.inspect {scene, frame?|time?}` devolve, por clipe, `animation`: para cada parâmetro `base`, `sampled`
(no frame pedido), `animated`, `keys` (frame/valor/easing) e `operations` com `enabled`, `reason` tipado e o `template`
da op que seria enviada; para a seed, `explicit`, `base` e `at_frame`. O `value` estático de P2 mantém seu significado.
Nada chaveia por si: a captura é sempre uma op enviada pela pessoa ou por um script. Um parâmetro animado traz
`animated: true` e as operações `go_previous`/`go_next`/`go_first`/`go_last` (motivo `no_frame` quando o clipe não
tem quadro de edição no monitor); o inspector devolve também a chave anterior/seguinte ao quadro pedido.

### Navegação (`scene.fx.seek`, BL082.30)

`scene.fx.seek {scene, clip, param, to}` com `to` ∈ `previous | next | first | last | <quadro do clipe>` move o
**monitor** (playhead) para uma chave. É regra do core (`EditorState::fx_seek`): a chave (local) → quadro do bundle
(`start_frame + local`) → tempo exato pelo relógio do próprio bundle → quadro da sequência
`ceil(tempo × taxa)`, conferido de ida e volta. Não escreve documento, histórico nem revisão. Recusas tipadas
(nada se move): `unknown_param`, `no_keys`, `no_previous_key`/`no_next_key`/`unknown_keyframe`,
`outside_sequence` (alvo fora da sequência, nunca limitado) e `frame_not_on_sequence` (bundle e sequência com taxas
diferentes, p.ex. 24 e 30000/1001: o quadro do bundle não cai na grade da sequência; nunca um vizinho). Início de
clipe/FX diferente de 0 é coberto (a chave local 0 de um clipe que começa no quadro 12 leva ao quadro 12).
`speed`/offset: **N/A** neste caminho (não existe clipe de cena com velocidade ou deslocamento de fonte).

### Concorrência e entrada tipada (BL082.30)

`scene.fx.edit` aceita `base_revision` (a `project.revision` em que o rascunho foi lido); revisão diferente é
`stale_revision` (`expected`/`revision` nos detalhes) e nada é escrito. A janela liga cada editor a
projeto/cena/clipe/parâmetro/quadro **mais** a revisão; o rascunho sujo não segue o modelo, então uma edição
obsoleta é recusada, nunca aplicada a outra chave. Texto inválido (`abc`, vazio, `NaN`, `inf`, `1e999`, `0,5`,
quadro fracionário) é `invalid_value` e **não escreve 0**; Enter e Blur confirmam **uma** vez (o primeiro marca o
rascunho como assentado). Editar um canal de cor/ponto grava o valor inteiro preservando os demais canais da
**mesma** chave.

### Base sob chaves (BL082.30)

A base de um parâmetro animado é desabilitada na UI e rotulada "(base)"; o valor amostrado no quadro do monitor
aparece **separado** (`sampled`). Pelo nó, escrever a base é `base_under_keys` (`edit_with: "keys"`); pela API direta,
`set_param` continua gravando a base com o aviso `FX_STATIC_UNDER_KEYS` e **sem autokey**. Redefinir a base
(reset) mantém as chaves. Coberto para numérico, booleano, cor e ponto.

## CLI headless

`./scripts/headless` (JSONL): as ops acima em `scene.fx.edit`, o ciclo `scene.fx.gesture.*`, `scene.fx.inspect` com
`frame`, `scene.plan`. Prova com processo real (`tessel-headless`, sem janela, sem GPU):
`a_keyed_grain_session_through_the_real_process_answers_like_the_library` em `crates/core/tests/scene_fx_keys.rs`.

## UI e CLI da janela

Seção "Efeitos" (`workstation/scene_fx_keys.rs` e `scene_fx_editors.rs`, Kit + tokens). Por parâmetro animado:
navegação **anterior / próxima / ir para a chave** (`go:`; ícones `SkipBack`/`SkipForward`/`Crosshair`, que chamam o
handler do core `scene.fx.seek`), botão de capturar chave no quadro do monitor, remover, e por chave um `Input` de
**quadro** (mover para qualquer quadro, `key_frame:`), um editor de **valor** tipado (número; canais de cor e de
ponto; `Switch` para booleano), o easing por chave (botão que percorre `linear → easeInOutCubic → easeOutCubic`; não há
`Select` do Kit com ícone-só aqui, e o ciclo é o mesmo comando `set_key_easing`) e, por curva, **deslocar** (`Input`
com delta) e **escalar** (âncora + `num/den`). A seed tem `Input` tipado (uint32 arbitrário), passo e "automática".
A base de um parâmetro animado fica desabilitada ("(base)", motivo `base_under_keys`) e o valor amostrado aparece
separado. Todos os `Input` confirmam por Enter **ou** Blur, uma única vez; erro tipado aparece no painel, nunca grava 0.
Nó `scene.fx.clip:<id>`, todos pelos handlers compartilhados (`SceneCommand::FxEdit`/`FxSeek`):

- campos: `seed` (uint32; só efeitos com ruído, `not_applicable` em outros), `key:<controle>:<frame>` (o valor dessa
  chave; ID do controle, como `color_r` para `fade.color` e `center_x` para `vignette.center`), `key_frame:<param>:<frame>` (move a chave para o quadro dado),
  `shift_keys:<param>` (delta inteiro), campos booleanos `"type":"boolean"` (um número é `invalid_value`); o campo da
  **base** de um parâmetro animado vem `enabled:false`, `animated`, `base`, `edit_with:"keys"`, `sampled`;
- ações: `key:<param>` (captura no frame do monitor; `no_frame` sem quadro da cena, `frame_outside_clip` fora do clipe), `key:<param>:<frame>`,
  `remove_key:<param>:<frame>`, `easing:<param>:<frame>:<nome>`, `move_key:<param>:<de>:<para>`,
  `shift_keys:<param>:<delta>`, `scale_keys:<param>:<âncora>:<num>:<den>`,
  `go:<param>:<previous|next|first|last|quadro>`, `remove_animation:<param>`, `seed:auto`. A lista anuncia captura no monitor, remoção, easing, movimentos ±1, navegação anterior/próxima/primeira/última e seed automática com `enabled` e `disabled_reason`; as formas parametrizadas de captura explícita, deslocamento e escala são aceitas por `invoke`, sem metadados de ação para cada argumento possível; o `set` de qualquer campo e o JSONL aceitam `base_revision`;
- o estado do nó inclui `animation` (o JSON do core). Os sliders editam a **base**; nunca criam chave. JSONL:
  `scene.fx.seek`, `scene.fx.edit` (`base_revision`) e o resto, as mesmas respostas.

Equivalentes: `./scripts/ui invoke scene.fx.clip:fx-1 key:amount`, `ui set scene.fx.clip:fx-1 seed 77`,
`ui set scene.fx.clip:fx-1 key_frame:amount:10 14`, `ui set scene.fx.clip:fx-1 key:color_r:10 0.5` (preserva os outros canais da chave), `ui invoke scene.fx.clip:fx-1 go:amount:next`,
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
| Janela (GPUI) | `scene_fx_keys_tests.rs` e `scene_fx_editors_tests.rs`: editores tipados (quadro, valor, cor/ponto preservando canais irmãos, seed arbitrária, deslocar/escalar), Enter+Blur = um comando, rascunho obsoleto recusado, erro sem gravar 0, base desabilitada, navegação, cliques reais (`simulate_click`) |
| Janela real (semântica) | `scripts/scene-fx-grain-ui-proof.py` (chaves por ações, easing, mover, remover animação, `base_under_keys`, `key_frame`/`key` inválidos, `go:*` no monitor, gesto, salvar/reabrir, export); `scripts/fx-history-smoke-proof.py` |
| Core (BL082.31–.34) | `scene_fx_key_bounds.rs` (aritmética `i64::MIN`, meio para cima, limites antes de copiar, contadores de trabalho, medição pareada), `scene_fx_seek.rs` (navegação, início ≠ 0, NTSC 30000/1001, recusas), unitários de `fx_keys` e `fx_intent`, gesto com intenção, `key_param` no-op tipado |
| Persistência/undo/export | `crates/render/tests/scene_fx_export.rs` (hashes do export iguais ao `scene.render`, re-export determinístico, seek, salvar/reabrir, um undo) |

Reservado ao coordenador (nits, fora desta rodada): F4 constantes `SHAPES` não usadas, F7 o motivo `no_frame`
(nome/uso), F9 contagens/UTF-8/notas de documentação.

Lacunas: sem timeline de chaves (P6); sem interpolação entre cenas; ponteiro físico não provado (BL043).

### Integração e nits da revisão P3 (BL082.34)

A captura sem quadro da cena recusa com `no_frame`; fora do clipe, com `frame_outside_clip`.
A regressão GPUI distingue os dois casos e verifica histórico intacto. A descrição das formas de op
e dos campos/ações foi alinhada ao parser. Manifesto e spec do corpus usam UTF-8 explícito;
as specs passaram 24/24 tanto normal quanto `-O`, e `verify` offline passou.
O merge de main preserva as alterações de gizmo e de backlog; os gates de integração e a
rechecagem independente ainda são necessários. As provas nativas do implementador registram
HEAD `93d1044` e 29 arquivos alterados antes dos commits; não comprovam por si o head limpo integrado.

Integração P3 pelo coordenador: fmt e diff-check limpos; GPUI filtro `capture_` 8/8;
check workspace locked e clippy workspace/all-targets locked com -D warnings passaram.
Não foi repetida aqui a suíte workspace, o build desktop nem a prova nativa no head integrado;
esses resultados permanecem pendentes da rechecagem independente.

Rechecagem independente P3 no head limpo `74dc1b9`: workspace com runtime obrigatório
2208/0/13 (113 resultados de suíte), fmt/check/clippy/build passaram, core sem gpui/wry;
janela FX153/153, grão104/104 e histórico42/42, JSONs dirty_files=0. Probes independentes
debug/release4/4, incluindo 21060 combinações de escala. Logs: /tmp/p3rev2/. A diferença
de contagem para o relato2144/0/13 do implementador não foi explicada pelo revisor.
A revisão aprova core/UI com nits baixos, corrigidos posteriormente em docs e no ramo morto
de diagnóstico. Os gates completos continuam ancorados em74dc1b9; não foram repetidos
após esses nits. Sem GPU real/Windows/ponteiro físico/motor original executado.
