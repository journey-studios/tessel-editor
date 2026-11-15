# P5 — duração e cortes dos planos da cena dirigida (BL082.5, BL082.44)

Estado: **plano**, versão 2 (2026-11-15), depois da revisão independente R1 (§12); Q1, Q3 e Q4 decididas pelo dono no
mesmo dia (§10). Feature reservada: **F065** ("Duração e
cortes dos planos"). Base lida: `main` `9073e77` (P4b S0, S1a, S1b e S3a integradas; transições com a admissão pública
fechada). O dono escolheu começar a P5 em 2026-11-15, com a S2 da P4b em andamento no coordenador; a ordem entre as duas
está na §8.

## 0. Como ler

Superfícies, como na P4b: **L** biblioteca (`tessel-editor-core`), **H** processo headless real (`tessel-headless`, JSONL),
**G** GPUI sintético (testes do desktop), **W** janela real por requisições semânticas (`scripts/ui` ou socket próprio),
**P** pixels do monitor, **E** export, **X** referência independente (Python, fora do editor). "Origem" é o contrato do
piloto trazido literalmente em [scene-shot-trim-plan.md](origin-specs/scene-shot-trim-plan.md) (plano) e
[scene-shot-trim-decisions.md](origin-specs/scene-shot-trim-decisions.md) (decisões `#1`–`#34`). Este plano decide como
cada regra da origem vira regra do core; onde diverge, a divergência é declarada (Dnn).

## 1. Objetivo e escopo

A pessoa (ou um agente) muda a duração de um plano, move o corte entre dois planos e muda a duração da edição inteira,
sem regerar o bundle, pela lib, pela CLI sem janela e pela UI, com o mesmo resultado nas três. Quatro operações:

| Origem | Op no core (JSONL) | Efeito |
|---|---|---|
| `trimShot {shotId, edge, frame, mode}` | `trim_shot {shot, edge: "start"\|"end", frame, mode?}` | move uma borda do plano para `frame` (quadro da edição; `end` é exclusivo) |
| `moveCut {fromShotId, toShotId, frame}` | `move_cut {from, to, frame}` | **roll**: o corte entre dois planos vizinhos vai para `frame`; a edição não muda de tamanho |
| `setShotDuration {shotId, durationFrames, mode}` | `set_shot_duration {shot, frames, mode?}` | atalho de `trim_shot edge:"end"` |
| `setEditDuration {frames}` | `set_edit_duration {frames}` | alonga ou encurta **só o último** plano (#21) |

Opções por op (#19): `mode` (`ripple`, padrão, ou `roll`), `content` (`anchor` \| `follow` \| `auto`, padrão `auto`,
#28), `keep_absolute` (bool), `shrink_transitions` (bool), `with_narration` (bool; recusado nesta fase, D13).

**Fora do escopo da P5** (como na origem, §8 do plano): dividir, apagar, reordenar ou inserir planos; `slip`; câmera
contínua entre clipes diferentes; estender ou gerar áudio. **Fora por fase:** a trilha "Planos" com arrasto na timeline
dirigida (P6; o desktop não tem timeline dirigida, §6.3), mover narração (M5), escopo `node` (BL082.40).

## 2. Fontes primárias

- Contrato da origem: os dois documentos acima, cópias literais do piloto (BL162), com a numeração das decisões que o
  código do piloto cita. **O código do piloto (`packages/scene-engine/src/shots/`) não está neste Windows**: o commit
  `d40ac3f7c0` não existe no clone da origem em `E:\www\tesselapp` (pin `d6adac15`, que não tem `shots/`). O padrão da P5,
  como na P4b, é **`ORIGIN_NOT_EXECUTED`**: o oráculo é uma referência Python transcrita das decisões. Executar o piloto no
  Mac é exceção, só se o coordenador liberar o Mac como host nesta rodada (a P4b §14 diz que não é).
- Schema e validação da origem no pin `d6adac15` (lido localmente): para a auditoria de relógio e de forma de cada campo
  adiado (S0).
- Código-alvo: mapa da §4, lido na `main` `9073e77`.

## 3. Regras da origem → decisões

| # | Regra da origem | Decisão no core | Por quê |
|---|---|---|---|
| D01 | Quatro comandos puros, `code/message/hint` (§4) | Ops `snake_case` como as do `fx` (§1), num módulo novo `scene_edit/shots/` que monta o **JSON candidato**; o candidato passa pelo portão de §5.4 | O documento é JSON; o `Bundle` tipado é só uma visão refeita a cada import (`directed_scene/model.rs:1-10`) |
| D02 | Um comando por entrada; entradas seguidas rodam como lote, um `finalizeBundle` no fim (#17) | `scene.shots.edit {scene, ops[], dry_run?, base_revision?}`: as ops rodam em ordem sobre um candidato, um portão no fim, **uma** entrada de desfazer ("Editar planos"); um erro leva o `index` da op | Igual ao `scene.fx.edit` (`scene_edit/fx.rs:271-346`) |
| D03 | `ripple`: a borda anda e tudo depois anda junto; `roll`: o vizinho absorve; borda inicial em ripple não desliza o plano (#11); `set_edit_duration` só no último (#21) | Igual | Contrato da origem |
| D04 | `content` `anchor`/`follow`/`auto` para o lado de entrada que vai para antes (#28–#32), `auto` ancora `min(d, instanceStart, cameraIn)` menos a reserva da transição (#29) | Igual. A reserva só existe com `fx.transitions`, que a admissão pública recusa hoje (`FX_TRANSITION_NOT_YET`): em produção é 0 até a S2A da P4b, e a matriz tem linhas que a S2A dispara (T14b) | Contrato da origem; a P4b decide quando transições entram |
| D05 | Junção `continuous`: ripple do fim puxa a continuação; roll vale; cortar a cabeça de quem continua é `CONTINUOUS_JOIN`; `follow` para dentro de uma junção contínua é `CONTINUOUS_JOIN`, e `auto` nunca cai para `follow` ali (#8, #30) | Igual, e o candidato passa também pelo portão **temporal** das junções (`joins.rs:142-240`, `CONTINUOUS_*_JUMP`): mudar onde a junção cai pode reprovar a pose mesmo com as regras inteiras certas; a recusa vem tipada do portão, sem reparo | O editor já roda esse portão em toda edição; a origem roda `validateEditJoins` no `finalizeBundle` |
| D06 | `editLocked`: planos da mesma instância que estavam "em passo" continuam em passo, senão `LOCKED`; sob `follow`, `auto` fica em `anchor` quando outro plano da instância estava em passo (#10, #30) | Igual, **só** nas ops de plano | Hoje o core lê `edit_locked` e não o aplica (`scene_interaction/tree.rs:195`) |
| D07 | A instância cresce para caber, nunca encolhe (#6) | Cresce, nunca encolhe, com nota. **Avaliação dos quadros novos:** no editor são avaliados como reais (canais `cycle` continuam o ciclo, `event` seguram o valor final; `animation/channels.rs:113`, `:148`). A S0 confere no texto da origem se ali também são reais (o efeito colateral que a #6 aceita, handles maiores para os outros planos da instância, indica que sim); só se a origem segurar de fato vira divergência declarada | Sem crescer, o avaliador estrito recusa (`temporal/frame.rs:292-305`) |
| D08 | A câmera segura o último pose sem editar o clipe (#7) | Já é o comportamento (`temporal/camera.rs:741-764`) | — |
| D09 | Clipes `fx` por escopo (#12, #31) | **`shot`:** andam com a imagem do plano por `contentShift` = Δ(`startFrame − instanceStartFrame`) do plano (#12/#31: no ripple da cabeça, #11, o plano fica e os clipes andam −d; em `follow`, −f) e são cortados nas bordas novas; o que fica sem quadro é removido com nota. **`output`:** pela cadeia de emendas: depois do ponto andam inteiros; os que cruzam esticam ou encolhem; o que fica **inteiro dentro** do trecho removido é removido com nota (duração 0 seria `FX_CLIP_EMPTY`, `fx/read.rs:629`); com `keep_absolute` nada anda, e uma edição que encolhe pode deixar um clipe fora (`FX_CLIP_OUT_OF_RANGE`, `fx/read.rs:640-651`), recusa legítima do `fx`, como a `FX_CLIP_OVERLAP` da origem. **Keys** (locais ao clipe, `fx/keys.rs:7`): clipe que **anda inteiro** não mexe nas keys (elas andam com ele); clipe com a **cabeça** cortada ou esticada (o início anda, o fim fica) desloca as keys pelo oposto da mudança da cabeça e poda as que ficam abaixo de 0; clipe com a **cauda** cortada poda as keys no novo fim ou depois; podas com nota. **Fades** maiores que o clipe novo são reduzidos com nota | Sem isso o importador do `fx` recusa (`KEY_OUT_OF_CLIP`, `FX_SCOPE_MISMATCH`); a P5 monta um candidato que o importador aceita, e ele continua o juiz |
| D10 | Transições presas por id ao corte; `TRANSITION_TOO_LONG` diz qual encurtar, `shrink_transitions` reduz ao máximo (#13) | Igual (`fx/window.rs:103` `max_transition_frames`). Cumpre o contrato que a P4b deixou para a P5 (`p4b-cross-dissolve-plan.md` §9.5 b). Antes da S2A, provado em L com `TransitionAdmission::Accept`, como a S3a; a S2A dispara as linhas H/G (T14b) e o `TRANSITION_NO_HANDLES` da cauda (#29). O dissolve antigo `transitionInto: dissolve` é recusado pelo importador hoje (`DISSOLVE_UNSUPPORTED`, `parse.rs:672-692`); quando a S6 da P4b o aceitar, a P5 o reavalia como as transições (dependência declarada) | Contrato P4b §9.5 |
| D11 | Keys de luz nos quadros da edição andam depois do ponto; as de dentro do trecho removido são descartadas com nota; lista vazia vira ausente (#15) | Igual para `sceneDefinitions[].lighting.lights[].keys.{intensity,color,position}` (quadros da edição, `lighting.rs:38-42`); trilha vazia é **removida** (o leitor recusa lista vazia, `lighting.rs:275-286`). **Portão:** `Lighting::read` só roda na montagem do plano (`scene_plan/build.rs:322`), não no import; então o portão das ops de plano roda `Lighting::read` em cada definição tocada, antes e depois, e `Ok` → `Err` é recusa tipada (sem isso a luz sumiria do monitor em silêncio). Seção de luz que o leitor **já** recusa antes da op (sombras, `ambient ≠ 1`, mais de 4 luzes; `lighting.rs:11-17`): as keys andam como dado simples pela mesma regra da D12 | A luz é modelada (F060) mas o import não a valida |
| D12 | (a origem move tudo que está em quadros da edição, #15) | **Decidido pelo dono (Q4, 2026-11-15):** campos adiados cuja auditoria da S0 mostrar **quadros da edição** e forma `[{frame: inteiro, …}]` (ex.: `weatherKeys`) são remapeados como **dado simples** pela cadeia de emendas, com a regra da #15 (descartados no trecho removido, lista vazia removida), sem ler o resto do campo; campos com tempo na edição em **outra forma** ou de relógio incerto são recusados com `SHOTS_LAYER_NOT_YET` (caminho do campo, dica `keep_absolute`). `lighting` sai da auditoria (está na lista de adiados, `mod.rs:89`, mas é a D11) | Recusar tudo obrigaria `keep_absolute`, que é tudo ou nada e congelaria também clipes e luzes; o remapeamento é aritmética de quadros, sem fingir suporte ao clima |
| D13 | Narração não anda, salvo `withNarration` (#16) | Não anda. `with_narration: true` é recusado com `SHOTS_OPTION_NOT_YET` (narração é M5, campo adiado) | — |
| D14 | Áudio nunca anda; edição mais longa ou mais curta que o áudio é válida, o mix completa com silêncio (#1, #3) | O áudio nunca anda. **Decidido pelo dono (Q1, 2026-11-15): o export completa com silêncio**, como a origem. Hoje ele recusa `audio_window_short` quando a faixa passa da janela do áudio (`scene_export/audio.rs:291-308`); a S5 troca a recusa por silêncio depois do fim do áudio e prova (T24) | A decisão #1 da origem é do dono; a recusa atual vem de F042A/BL016.11 |
| D15 | Nenhuma propriedade de plano no catálogo (#14) | Igual: nenhum caminho `shot/…` nem `edit/…` em `scene_edit/catalog.rs`; um teste falha se alguém acrescentar | Uma propriedade escreveria o intervalo sem o ripple |
| D16 | `explainShots` com lugar, `intent`, `handles` e avisos (#20) | `scene.shots.explain {scene}` → `{edit, shots[{id, start, end, frames, instance, instance_start, camera_clip, camera_in, transition_into, intent, handles{scene_before, scene_after, camera_before, camera_after}, locked}], audio, warnings[]}`. Avisos: `EDIT_LONGER_THAN_AUDIO`, `EDIT_SHORTER_THAN_AUDIO` (folga de um quadro). `NARRATION_PAST_EDIT` só se a S0 mostrar que a narração é legível sem modelar a M5; senão fica declarado ausente | Consulta para a UI e agentes, sem efeito |
| D17 | `limitFrame` nas recusas que têm quadro-limite (#33) | `details.limit_frame` em `SHOT_TOO_SHORT`, `NO_HEAD_FRAMES`, `EDIT_TOO_LONG`, `TRANSITION_TOO_LONG`, sempre um **quadro da edição** no `frame` da própria op; em `set_shot_duration`/`set_edit_duration` é o **fim** do plano, não uma duração (#33), e o campo "Duração" da UI converte. Prova: a op no limite não recusa pelo **mesmo** código | A UI e a P6 travam no quadro legal sem adivinhar regra |
| D18 | `MAX_EDIT_FRAMES = 216 000` (#18) | Igual, `EDIT_TOO_LONG` | Uma hora a 60 fps |
| D19 | Códigos estáveis (§4, #8, #9, #10) | `SHOT_TOO_SHORT`, `SHOT_NOT_FOUND`, `SHOTS_NOT_ADJACENT`, `EDIT_TOO_LONG`, `LOCKED`, `CONTINUOUS_JOIN`, `NO_HEAD_FRAMES`, `SHOTS_LAYER_NOT_YET`, `SHOTS_OPTION_NOT_YET`; `TRANSITION_TOO_LONG` **reaproveita** a projeção e o texto que já existem (`fx/transition_problem.rs`). Texto pt-br/en/es no core, sem braço padrão; um teste enumera todos | Uma tabela de textos, não duas |
| D20 | Studio: trilha "Planos" com arrasto, Alt, fantasma que trava no limite (#24–#27, #34) | **Decidido pelo dono (Q3, 2026-11-15): a P5 não desenha trilha.** A UI da P5 é uma seção "Planos" no painel da cena com campos e seleções (§6.3); arrasto, Alt e fantasma ficam para a P6 sobre os mesmos comandos e o `limit_frame` | Não há timeline dirigida no desktop |
| D21 | Desfazer exato por fatia `ShotsSlice` (#24) | Desnecessário: o histórico guarda a cena inteira (`state/history.rs:125`), `replace_scene` troca o documento e desfazer devolve o anterior | Arquitetura do editor |
| D22 | O transporte reposiciona com `seekToFrame(min(atual, última))` quando a edição encurta (#25) | O playhead da cena dirigida no monitor é limitado ao último quadro novo depois de uma op que encurta a edição | Senão o monitor pediria um quadro que não existe |
| D23 | (não previsto na origem) | Um gesto aberto (`fx` ou gizmo) fica inválido pela troca de revisão, como em qualquer edição (`scene_interaction/gesture.rs:281`); a op de plano não espera nem confirma o gesto | Regra existente, só declarada |

## 4. Estado do código-alvo (`main` `9073e77`)

- **Documento**: `DirectedScene { document: Value, bundle, notices, admission }` (`directed_scene/mod.rs:146-154`). `Shot`
  (`model.rs:320-343`), `SceneInstance` com `edit_locked` (`model.rs:150-157`), `edit_duration_frames` (`model.rs:366`). A
  cena fica em `.tessel.json` `scenes.items[].bundle` (`scene_project/store.rs`).
- **Portão do candidato**: `DirectedScene::reimport` (`mod.rs:222-224`) chama `import_value_admitting` (`mod.rs:229-299`:
  parse, `integrity::check`, grafo, `fx::transition_issues`); depois `edit_join_issues` (`mod.rs:351`) e `temporal_issues`
  (`mod.rs:367`). Regras de planos em `integrity.rs:533-618` (`EDIT_RANGE_START`, `SHOT_EXCEEDS_INSTANCE_DURATION`,
  `EDIT_GAP_OR_OVERLAP`, `EDIT_RANGE_INVALID`, `EDIT_DURATION_MISMATCH`); junções em `joins.rs:33-240`. O par
  junção+temporal aparece em cinco lugares: as duas edições (`scene_edit/batch.rs:442`, `scene_edit/fx.rs:342`), que fazem
  reimport + par, e três checagens de cena já importada (`scene_project/run.rs:38`, `scene_export/snapshot.rs:123`,
  `render/src/scene.rs:66`), que só fazem o par. **A S1 extrai `candidate_gate` (reimport + par) para as duas edições e a
  P5**; as três checagens de cena importada ficam como estão (não reimportam, outro contrato).
- **Efeitos**: `FxClip` (`fx/mod.rs:247-266`), keys locais (`fx/keys.rs:7`), containment de `shot` (`fx/scope.rs:221`),
  `FX_CLIP_EMPTY` e `FX_CLIP_OUT_OF_RANGE` (`fx/read.rs:629-651`), janela e máximo de transição (`fx/window.rs:80-130`).
  Ajudantes `pub(super)` de `scene_edit/fx.rs` usáveis por um módulo irmão: `tracks_mut` (`:137`), `clip_ref` (`:230`),
  `clip_object` (`:521`); `set_field` (`:755`), `run_op` (`:395`) e o `resize` de `fx_transition.rs:717` são privados, e
  `apply_fx` (`:240`) reimporta a cada chamada (num documento intermediário, planos mudados e clipes não, ele recusa).
- **Luz**: `Lighting::read` (`lighting.rs:375`) só é chamado na montagem do plano (`scene_plan/build.rs:322`).
- **Caminho de edição a imitar**: `ops.rs:647` (`scene_command`), `SceneCommand` (`scene_project/command.rs:102`),
  `apply_scene` (`scene_project/run.rs:166`), `edit_fx` (`scene_project/fx.rs:176-237`: revisão base, dry run, no-op,
  `replace_scene`), `record_edit` (`state/history.rs:226`), erros (`scene_edit/error.rs:72-183`), textos
  (`fx/transition_problem.rs:107`, `project_problem` em `:739`).
- **Nada edita planos hoje**; `state/timeline.rs` é a sequência do projeto, não planos. Nenhuma timeline dirigida no
  desktop; o painel da cena é `workstation/scene_panel.rs` (922 linhas) e a seção Efeitos `workstation/scene_fx.rs`; a
  seção é registrada em `scene_ui.rs` (982 linhas).

## 5. Contrato do core

### 5.1 Planejamento (puro)

Um planejador por op (`scene_edit/shots/plan.rs`) lê os planos **ordenados por início** (o documento pode tê-los fora de
ordem; o integrity ordena) e devolve um `ShotPlan`: as faixas novas de cada plano tocado, `instanceStartFrame` e
`cameraInFrame` novos, instâncias que crescem, `editDurationFrames` novo, as **bordas** que a op moveu (para o
`limit_frame` de `TRANSITION_TOO_LONG`, #33), o `contentShift` de cada plano (D09) e a **cadeia de emendas** (`splices`,
#31) que as camadas em quadros da edição seguem. Sem I/O, sem JSON: entram os `Shot`/`SceneInstance` tipados e a op, sai o
plano ou a recusa, com `limit_frame` quando a regra tem um.

### 5.2 Aplicação ao JSON

`scene_edit/shots/apply.rs` escreve o plano no `document` clonado: `shots[i].editRange`, `instanceStartFrame`,
`cameraInFrame`, `sceneInstances[j].durationFrames` (só cresce), `editDurationFrames`. Cada camada (§5.3) é ajustada no
**mesmo** JSON pela cadeia de emendas e pelo `contentShift`, com os ajudantes `pub(super)` do `fx` (§4); a lógica de keys,
fades e remoção é da P5 (não passa por `apply_fx`, que reimportaria um documento intermediário). Se uma fatia precisar
tornar público algo privado de `scene_edit/fx.rs` ou `fx_transition.rs`, isso é um commit mecânico separado, combinado com
o coordenador (a P4b mexe nesses arquivos). Cada op de um lote parte do candidato que a anterior deixou (D02).

### 5.3 Camadas

| Camada | Regra | Fatia |
|---|---|---|
| planos, edição, instâncias | §5.1/5.2 | S1 |
| câmera, `performanceTracks`, rig, caminhos, reveal | não andam (#5) | S1 (prova de que não mudam) |
| clipes `fx` `shot` | `contentShift`, cortados nas bordas, removidos se vazios (D09) | S2 |
| clipes `fx` `output` | cadeia de emendas; removidos se inteiros no trecho removido; `keep_absolute` (D09) | S2 |
| keys e fades de clipes | por caso: anda inteiro, cabeça, cauda (D09) | S2 |
| `fx.transitions` | presas por id; `TRANSITION_TOO_LONG` ou `shrink_transitions` (D10) | S2 (L com `Accept`) |
| keys de luz pontual | cadeia de emendas; portão `Lighting::read` antes/depois (D11) | S2 |
| campos adiados com tempo na edição | remapeados como dado simples ou `SHOTS_LAYER_NOT_YET`, pela lista da S0 (D12) | S1 (recusa) e S2 (remapeamento) |
| narração | nunca anda; `with_narration` recusado (D13) | S1 |
| áudio | nunca anda (D14) | — |

### 5.4 Portão e efeitos

O candidato passa por `candidate_gate` (§4) e, para as ops de plano, pelo portão de luz da D11: qualquer achado vira
`CANDIDATE_REJECTED` com os achados ligados à op pelo ponteiro JSON, como no `fx`. Revisão base (`base_revision`),
`dry_run` (só tamanho, `would_change`) e no-op (`changed: false`, sem entrada de desfazer) seguem `edit_fx`. Resultado:
`{changed, notes[], revision}`. Notas (#6, #12, #15, #28): instância cresceu, clipe removido, keys podadas, fade reduzido,
`auto` caiu para `follow`. Depois de uma op que encurta a edição, o playhead da cena é limitado (D22).

### 5.5 Consulta

`scene.shots.explain` (D16). O nó `scene.shots` da janela devolve o mesmo JSON.

### 5.6 Erros e textos

Códigos da D19, mensagem em inglês na resposta e `details.errors[]` localizados pela mesma via da S3a (`project_problem`
com `Locale`), numa tabela única pt-br/en/es no core. `details` leva `shot`, `edge`, `asked_frame`, `limit_frame` quando
couber.

## 6. Superfícies

### 6.1 Lib (L)

`EditorState::execute(Command::Scene(SceneCommand::ShotsEdit{…}))` e `SceneCommand::ShotsExplain`; o planejador e o
aplicador são públicos para testes de propriedade.

### 6.2 CLI headless (H)

JSONL: `{"op":"scene.shots.edit","scene":"…","ops":[{"trim_shot":{…}}…],"dry_run":false}` e
`{"op":"scene.shots.explain","scene":"…"}`. Os nomes de op aceitos ficam no parser (`ops.rs`), com a lista das formas
aceitas no `INVALID_OP`. `scene.capabilities` e `scope.rs` passam a listar `shots.edit` em `validated` só quando a S3
fechar.

### 6.3 UI (G, W)

Seção **"Planos"** no painel da cena, num arquivo novo (`workstation/scene_shots.rs`), com componentes do Kit. Registrar a
seção em `scene_ui.rs` (982 linhas) deve somar o mínimo; se passar de 1000, a S4 extrai antes (padrão BL082.55).

| Controle | Comando | Nó |
|---|---|---|
| lista dos planos (início, fim, quadros, `intent`, travado) | `scene.shots.explain` | `scene.shots` (`get`) |
| campo numérico "Duração" por plano | `set_shot_duration` | `scene.shot:<id>` (`set duration`) |
| campo "Corte" entre dois planos | `move_cut` | `scene.shots.cut:<from>:<to>` (`set frame`) |
| campo "Duração da edição" | `set_edit_duration` | `scene.shots` (`set edit_duration`) |
| seleção "Modo" (ripple/roll) e "Conteúdo" (auto/anchor/follow) | opções das ops | `scene.shots` (`set mode`, `set content`) |

Os nós seguem o padrão do `fx` (nós de estado em `state/tree.rs`, parser de `set`/`invoke` em `core::commands`, como a S3b
da P4b faz para transições; AGENTS.md exige `Command` + nó em `tree.rs` + parser em `core::commands`). Recusa: o campo volta
ao valor do documento e a linha de status mostra o texto do core no idioma da UI (nada montado no desktop). Sem cena
dirigida, os controles ficam desabilitados com `disabled_reason`. Sem arrasto (D20), sem diálogo novo. Undo/Redo pelos
botões existentes.

### 6.4 Pixels e export (P, E)

Depois de uma op, o plano da cena é refeito pelo mesmo `scene_plan` (o monitor mostra o novo tempo). Provas na S5: plano
alongado além do clipe de câmera desenha a câmera segurada; plano alongado além da instância desenha os quadros avaliados
(D07); o export de uma edição alterada tem `editDurationFrames` quadros e os mesmos pixels do monitor. O `flicker` da luz
usa os segundos da edição (`lighting.rs:42`): depois do ponto editado a imagem muda de propósito, e as provas comparam
monitor com export no mesmo documento, nunca antes com depois.

## 7. Matriz de aceite

| T | Caso | L | H | G | W | P/E | X |
|---|---|---|---|---|---|---|---|
| T01 | `set_shot_duration` ripple alonga: os seguintes andam, a edição cresce | S1 | S3 | S4 | S4 | S5 | S0 |
| T02 | `move_cut` roll: edição igual, os dois planos ajustam instância e câmera | S1 | S3 | S4 | S4 | — | S0 |
| T03 | `trim_shot start` ripple (#11) e para antes com `anchor`/`follow`/`auto` (#28–#32) | S1 | S3 | — | — | — | S0 |
| T04 | `set_edit_duration` só no último; abaixo do mínimo `SHOT_TOO_SHORT` com `limit_frame` | S1 | S3 | S4 | — | — | S0 |
| T05 | Junção contínua: ripple do fim puxa a cadeia; roll válido; `CONTINUOUS_JOIN`; `auto` nunca cai para `follow` numa junção contínua de entrada (#8, #30) | S1 | S3 | — | — | — | S0 |
| T06 | Portão temporal da junção reprova um corte que quebra a pose (D05) | S1 | — | — | — | — | — |
| T07 | `LOCKED`; `auto` fica em `anchor` quando outro plano da instância está em passo (#10, #30) | S1 | S3 | — | — | — | S0 |
| T08 | Instância cresce e nunca encolhe, com nota (D07) | S1 | S3 | — | — | S5 | S0 |
| T09 | `NO_HEAD_FRAMES` só com `anchor`, com `limit_frame` (#9) | S1 | S3 | — | — | — | S0 |
| T10 | `EDIT_TOO_LONG`, `SHOT_NOT_FOUND`, `SHOTS_NOT_ADJACENT` | S1 | S3 | S4 | — | — | S0 |
| T11 | Clipe `shot` anda por `contentShift`, é cortado e removido com nota (D09, BL082.44) | S2 | S3 | — | S4 | — | S0 |
| T12 | Clipe `output` pela emenda; removido se inteiro no trecho removido; `keep_absolute` e `FX_CLIP_OUT_OF_RANGE` (D09) | S2 | S3 | — | — | — | S0 |
| T13 | Keys e fades: anda inteiro (intactas), cabeça (deslocadas e podadas), cauda (podadas), fade reduzido; nenhum `KEY_OUT_OF_CLIP` (D09) | S2 | — | — | — | — | S0 |
| T14 | Transição presa ao corte, `TRANSITION_TOO_LONG` com `limit_frame`, `shrink_transitions` (D10, `Accept`) | S2 | — | — | — | — | S0 |
| T14b | O mesmo, com a admissão aberta (disparado pela S2A da P4b), e `TRANSITION_NO_HANDLES` da cauda (#29) | S2A+ | S2A+ | S2A+ | — | — | S0 |
| T15 | Keys de luz andam, descartadas no trecho removido, trilha vazia removida; `Ok`→`Err` do leitor recusado (D11) | S2 | S3 | — | — | S5 | S0 |
| T16 | Campo adiado: remapeado como dado simples ou `SHOTS_LAYER_NOT_YET`, conforme a lista da S0 (D12) | S1/S2 | S3 | — | — | — | S0 |
| T17 | `with_narration` recusado; narração e áudio intactos byte a byte (D13, D14) | S1 | S3 | — | — | — | — |
| T18 | Lote atômico: uma op recusada não muda nada; uma entrada de desfazer; desfazer devolve o documento exato; `dry_run`; no-op; `base_revision` velha; gesto aberto invalidado (D23) | S3 | S3 | S4 | S4 | — | — |
| T19 | `explain` com handles, travado e avisos de áudio | S3 | S3 | S4 | S4 | — | S0 |
| T20 | Textos pt-br/en/es de todos os códigos, sem braço padrão | S3 | — | S4 | — | — | — |
| T21 | Propriedade: sequências aleatórias de ops válidas sempre passam o portão; recusadas não mudam nada; no `limit_frame` a op não recusa pelo mesmo código | S1/S2 | — | — | — | — | S0 |
| T22 | Nenhum caminho `shot/…` ou `edit/…` no catálogo (D15) | S1 | — | — | — | — | — |
| T23 | Monitor e export de um plano alongado (câmera segura, instância avaliada) iguais entre si; playhead limitado depois de encurtar (D22) | — | — | S4 | — | S5 | — |
| T24 | Export de edição mais longa que o áudio, conforme Q1 | — | — | — | — | S5 | — |

## 8. Fatias

| Fatia | Entrega | Depende de | Arquivos (ownership) |
|---|---|---|---|
| **S0** contrato e oráculo | `docs/features/shot-duration.md` (F065) e o patch da linha **pendente** da matriz (cada fatia atualiza a linha depois); auditoria de relógio e de forma de cada campo de `DEFERRED_FIELDS` exceto `lighting`, no schema da origem `d6adac15` (fixa a lista da D12); conferência no texto da origem da avaliação dos quadros da instância que cresce (D07/Q2); **referência independente** `scripts/shot-trim-reference.py` transcrita das decisões (sem ler o código Rust), corpus `docs/migration/fixtures/shot-trim/{cases,manifest}.json` com `--check` e spec, `ORIGIN_NOT_EXECUTED` | este plano revisado | só docs, `scripts/shot-trim-reference*.py`, fixtures |
| **S1** planejador e planos | `candidate_gate` extraído (sem mudança de comportamento) para `batch`/`fx`/P5; `scene_edit/shots/{mod,plan,apply,error}.rs`: as quatro ops em planos, edição e instâncias, `content`, junções contínuas, `LOCKED`, limites, recusas D12/D13; teste do corpus da S0 em `crates/core/tests/shot_trim_corpus.rs`; propriedade (T21) | S0 | `crates/core/src/scene_edit/shots/**`; a extração em `scene_edit/{batch,fx}.rs` e `directed_scene/mod.rs` |
| **S2** camadas | clipes `fx` (`shot`, `output`, keys, fades), transições (L com `Accept`), keys de luz e o portão de luz, remapeamento dos campos adiados da lista; notas | S1 | `scene_edit/shots/layers_{fx,lights,deferred}.rs`; só leitura de `fx.rs`/`fx_transition.rs` (visibilidade nova só por commit mecânico combinado, §5.2) |
| **S3** comando e CLI | `SceneCommand::{ShotsEdit,ShotsExplain}`, parser JSONL, rótulo de desfazer, `dry_run`/`base_revision`/no-op, `explain`, textos pt-br/en/es, `capabilities`/`scope.rs`, playhead (D22), testes de processo | S2 | `scene_project/{command,run}.rs` (+ `shots.rs` novo), `ops.rs`, `scene_project/scope.rs`, `directed_scene/shots_problem.rs` (textos) |
| **S4** UI | seção "Planos", nós em `state/tree.rs` e parser em `core::commands` (§6.3), testes GPUI, prova na janela `scripts/shot-duration-proof.py` (+ spec) | S3; S3b da P4b integrada (dona de `commands.rs`/`tree.rs`) | `crates/desktop/src/workstation/scene_shots.rs` (+ testes), registro em `scene_ui.rs`, entradas novas em `state/tree.rs` e `commands.rs` |
| **S5** pixels, export e fechamento | provas P/E (T23, T24), a decisão Q1 aplicada, mutantes, relatório, linha F065 da matriz | S4; Q1 | `crates/render/tests/shot_duration_frames.rs`; `scene_export/audio.rs` só se Q1 = completar com silêncio |

**Ordem e conflitos com a P4b.** As fatias da P5 são sequenciais. Nenhuma fatia edita `docs/backlog.md`,
`docs/features/README.md` ou a matriz de migração: manda o patch das suas linhas ao coordenador.

- **S2 da P4b** (runtime e renderer): sem arquivo comum até a S5 da P5.
- **S2A da P4b** é dona da admissão em `directed_scene`, de `scene_project/fx.rs`, de `scene_export/mod.rs` e da lista
  `refused` do escopo (`p4b-cross-dissolve-plan.md:852`). A extração de `candidate_gate` (S1 da P5) toca
  `directed_scene/mod.rs` e `scene_edit/fx.rs`: entra **antes** da S2A ou é refeita sobre ela, combinada com o
  coordenador. A S3 da P5 acrescenta `shots.edit` em `scope.rs` depois da S2A.
- **S3b da P4b** é dona de `commands.rs` e `state/tree.rs` (`:853`): a S4 da P5 só acrescenta entradas depois que a S3b
  entrar.
- `scene_project/run.rs` e `ops.rs` são comuns: a S3 da P5 acrescenta braços, sem mover os existentes.

## 9. Gates

Por fatia, no Windows e no Mac: `fmt`, `clippy --workspace --all-targets -D warnings`, `cargo test --workspace` com
`TESSEL_REQUIRE_RUNTIME=1`, `cargo tree -p tessel-editor-core` sem gpui/wry. A partir da S0, a spec da referência normal e
`-O`; a partir da S3, o processo headless; na S4, a prova na janela; na S5, os pixels e o export. Revisão independente por
fatia (achados P0–P2 corrigidos antes do merge); CodeRabbit executado conta só como revisão adicional.

## 10. Decisões do dono (2026-11-15)

- **Q1 (D14), decidida:** a edição mais longa que o áudio exporta com **silêncio no fim** (origem #3); a S5 troca a recusa
  `audio_window_short` por silêncio e prova (T24).
- **Q2 (D07), condicional:** só se a S0 mostrar que a origem segura o último quadro da instância que cresce, o dono decide se
  a avaliação real dos quadros novos (ciclos continuam) fica como divergência declarada.
- **Q3 (D20), decidida:** a UI da P5 é a seção "Planos" com campos e seleções; o arrasto na trilha fica para a P6.
- **Q4 (D12), decidida:** campos adiados em quadros da edição com forma `[{frame, …}]` são remapeados como dado simples pela
  regra da origem (#15); os de outra forma ou relógio incerto recusam com `SHOTS_LAYER_NOT_YET`.

## 11. Linhas propostas para o backlog

| ID | Pendência | Estado proposto |
|---|---|---|
| BL082.5 | Ripple/roll de shots (P5) | Em andamento — plano v2 revisado (R1); F065 reservada |
| BL082.5.1 | S0: contrato F065, auditoria dos campos adiados, referência e corpus | Pendente |
| BL082.5.2 | S1: `candidate_gate`, planejador, planos/edição/instâncias, recusas | Pendente |
| BL082.5.3 | S2: camadas `fx`, transições, luz e campos adiados | Pendente |
| BL082.5.4 | S3: comando, CLI, `explain`, textos, playhead | Pendente |
| BL082.5.5 | S4: seção "Planos", nós e prova na janela | Pendente (depende da S3b da P4b) |
| BL082.5.6 | S5: pixels, export e fechamento | Pendente |
| BL082.5.7 | Q1: export de edição mais longa que o áudio | Decidido: silêncio no fim (a S5 aplica) |
| BL082.5.8 | T14b: transições com a admissão aberta | Pendente (dispara com a S2A da P4b) |
| BL082.44 | Edições de shots mantêm os clipes `shot` | passa a depender de BL082.5.3 |

## 12. Revisão

**R1** (2026-11-15, revisão independente do plano v1 contra o código da `main` `9073e77` e os documentos da origem;
somente leitura): "aprovar com correções". Achados e o que a v2 fez:

| Achado | Severidade | Correção na v2 |
|---|---|---|
| Regra de keys deslocava as de clipes que andam inteiros (os de `output` depois do ponto e os de `shot` com a imagem) | P0 | D09 por caso: anda inteiro, cabeça, cauda; T13 |
| O portão do candidato não valida luz (`Lighting::read` só no plano) | P1 | D11: portão de luz antes/depois; seção já recusada segue a D12; `lighting` fora da auditoria |
| Conflitos de arquivo com a S2A e a S3b da P4b; `scope.rs`, `commands.rs` e `tree.rs` fora do ownership | P1 | §8: regras de ordem e ownership; §6.3: padrão dos nós |
| Reuso do `fx` como descrito não funciona (`set_field` privado, `apply_fx` reimporta) | P1 | §4, §5.2: JSON direto com os ajudantes `pub(super)`; visibilidade nova só por commit mecânico combinado |
| Clipe `output` inteiro no trecho removido; `keep_absolute` encolhendo | P2 | D09, T12 |
| `contentShift` sem regra nomeada | P2 | D09, §5.1 |
| D07/Q2 pode ser divergência falsa | P2 | S0 confere antes; Q2 condicional |
| Transições só em L; S2A e dissolve antigo | P2 | D10, T14b, BL082.5.8 |
| Linha da matriz só na S5 | P2 | S0 manda a linha pendente |
| P5 em paralelo com a P4b | P2 | Registrado no cabeçalho: escolha do dono em 2026-11-15; §8 ordena |
| Q4 mal enquadrada | P2 | D12 e Q4 reescritas (remapear como dado simples) |
| Linhas de `reimport`; o par de checagens em cinco lugares | P3 | §4 corrigida, com o motivo de extrair só dois |
| Semântica de `limit_frame` em durações | P3 | D17 |
| Texto de `TRANSITION_TOO_LONG` duplicado | P3 | D19 reaproveita |
| `flicker`, playhead, gesto aberto | P3 | §6.4, D22, D23 |
| Tamanho de `scene_ui.rs` | P3 | §6.3 |
| Mac como host do oráculo | P3 | §2: `ORIGIN_NOT_EXECUTED` padrão |
| Linhas de matriz para `auto`/`LOCKED` e junção contínua | P3 | T05, T07 |
