# Duração dos planos: achados, decisões e desvios

Este documento guarda o que o [plano de duração dos planos](scene-shot-trim-plan.md) deixou para decidir lendo o código (a política de ripple da §5 do plano) e o que a implementação da etapa 1 (engine, CLI, skills) mudou em relação ao plano. A numeração (`#N`) é a que o código cita (`decisions #1`, `#5`, `#14`). Mesmo molde de [scene-fx-decisions.md](scene-fx-decisions.md).

Código: `packages/scene-engine/src/shots/` (comandos puros), `src/edit/edit-bundle.ts` (porta `{ shots }`), `bin/scene-shots.mjs` (CLI).

## Auditoria: em que relógio cada camada conta o tempo

Antes de decidir quem anda no ripple, cada camada foi lida para saber se conta **quadros da edição** (anda com o ponto editado) ou **quadros da cena** (o relógio da instância, que não muda quando um plano cresce).

| Camada | Relógio | Anda no ripple? |
| --- | --- | --- |
| `shot.editRange`, `editDurationFrames` | edição | é o que o comando muda |
| `shot.instanceStartFrame`, `cameraInFrame` | cena / clipe de câmera | só quando o início do plano muda (cabeça cortada ou acrescentada) |
| `sceneInstance.durationFrames` | cena | cresce quando o plano passa do fim do conteúdo; nunca encolhe |
| rig, caminhos, reveal, `performanceTracks` | cena (`instanceFrame`) | **não** (#5) |
| `cameraClips` (segmentos, poses) | clipe de câmera | **não** (#5); o fim do clipe segura o último pose |
| clipes de `fx` com `scope: shot` | edição, presos ao plano | andam com o plano (#12) |
| clipes de `fx` com `scope: output` / `node` | edição absoluta | andam depois do ponto editado, esticam/encolhem se cruzam (#12) |
| `fx.transitions` e `shot.transitionInto` | edição, ligadas por `fromShotId/toShotId` | acompanham o corte (#13) |
| keys de luz (intensidade, cor, posição) e `weatherKeys` | edição | andam depois do ponto editado (#15) |
| narração (`bundle.narration`) | segundos da edição | **não**, a menos que `withNarration` (#16) |
| áudio (`bundle.audio`) | segundos da edição / do mix | **nunca** (#1) |
| `outline` (grupos do Studio) | não tem tempo | não há o que mover (#23) |

## Decisões

### 1. Imagem e áudio são independentes (decisão do usuário)

Mudar a duração de um plano não move o áudio nem a narração. O vídeo pode terminar antes ou depois do áudio: isso é permitido, nunca é erro e nunca deve ser "consertado". O que existe é **aviso** (`explainShots` e `scene validate`): `EDIT_LONGER_THAN_AUDIO`, `EDIT_SHORTER_THAN_AUDIO` (com um quadro de folga) e `NARRATION_PAST_EDIT`. Cada aviso diz na `hint` que a diferença é permitida.

### 2. A narração além da edição deixa de ser erro

`explainNarration(narration)` não recebe mais a duração da edição e o erro `CLIP_PAST_END` foi removido (a fala que passa do fim cai no silêncio do fim, e a que começa depois do fim nem toca). O que sobra é `narrationPastEdit(narration, segundos)`, um aviso. A validação do bundle (`render-bundle.schema.ts`) segue a mesma regra. Os specs de `narration.spec.ts` foram atualizados (o caso `CLIP_PAST_END` passou a afirmar aceitação).

### 3. A janela do áudio pode ser maior ou menor que a edição

`audioMixPlan` ganhou `mix.sourceSeconds` (o tamanho da janela do mix) e `audio-graph.ts` corta (`atrim`) em `min(duração da edição, sourceSeconds)` e completa com silêncio (`apad`) até a duração da edição. Assim uma edição mais longa tem silêncio no fim e uma mais curta corta o áudio onde a imagem acaba. Specs novos em `audio-graph.spec.ts` (edição maior e menor) e `mix-plan.spec.ts`.

### 4. Recusas que contradiziam a decisão 1 foram removidas

- `render.ts`: a checagem `AUDIO_VIDEO_DURATION` (áudio e vídeo com durações diferentes) saiu. `AUDIO_SHORT` (mix menor que a janela declarada, um bundle quebrado) ficou.
- `apps/scene-studio/src/engine/createStudioSession.ts`: a recusa "Audio edit window shorter than edit timeline duration" saiu, com comentário do porquê. Tirar a recusa não bastava (#22): o transporte e a forma de onda também assumiam áudio e edição do mesmo tamanho.

### 5. Quem conta quadros da cena não anda

Rig, caminhos, reveal, `performanceTracks` e os clipes de câmera usam o quadro da instância (`instanceFrame`) ou do clipe. Quando um plano cresce ou encolhe, o relógio da cena é o mesmo; mexer neles faria a cena "voltar". O ripple deles é **zero**, por construção. Só `instanceStartFrame`/`cameraInFrame` mudam, e só quando o início do plano muda (#11).

### 6. A instância cresce para caber o plano

`SHOT_EXCEEDS_INSTANCE_DURATION` obriga `instanceStartFrame + duração do plano <= sceneInstance.durationFrames`, e `evaluateShotFrame` lança além da duração. Então, quando um plano passa do conteúdo, `durationFrames` da instância cresce até caber (nota no resultado: "Scene instance X now lasts N frames (was M): past its content it holds its last frame"). O runtime segura o último quadro da cena (rig, reveal e overrides seguram a última pose: `heldInstance`). Nunca encolhe: outros planos podem usar a instância. Efeito colateral aceito: os `handles` da instância (`explainFx`) aumentam também para os outros planos que a compartilham.

### 7. A câmera segura o último pose, sem mudar o clipe

Alongar o plano além do clipe de câmera não edita `cameraClips`. `resolveCameraAtClipFrame` já segura o último pose (`heldCamera`), e o spec `shot-runtime.spec.ts` prova que preview/render avaliam esses quadros sem lançar, e que a câmera e a cena seguram em momentos diferentes.

### 8. Junção `continuous`: a continuação anda junto

Uma junção `continuous` exige continuidade exata de instância e câmera (`validateEditJoins`). Ripple do **fim** de um plano que continua no seguinte puxa o seguinte junto: ele passa a começar no quadro da cena em que o primeiro agora termina (e o mesmo para uma cadeia de continuações). Um `roll` em cima de uma junção contínua é válido: os dois lados do corte avançam juntos (`instanceStartFrame` e `cameraInFrame` do segundo seguem o corte), então a continuidade se mantém. O **splice** (o ponto de tempo da edição que as camadas em quadros da edição seguem: clipes `output`, keys de luz e clima, narração) de um ripple de fim fica no **fim da cadeia** (o fim antigo do último plano que continua), não no fim do plano editado: como a continuação desliza junto com os quadros da cena, cada quadro da edição que continua existindo mostra a mesma imagem de antes até o fim da cadeia, e o que a edição realmente perdeu (ou ganhou) é a **cauda** da tomada. Sem continuação, o fim da cadeia é o próprio fim do plano. Já cortar ou acrescentar a **cabeça** de um plano que continua o anterior (`trimShot edge: "start"` em ripple) é recusado com `CONTINUOUS_JOIN` (código extra, não estava no plano), porque a imagem pularia no corte.

### 9. `NO_HEAD_FRAMES` (código extra; só com `content: "anchor"` desde a #28)

Manter a imagem **nos mesmos quadros da edição** quando o início de um plano vai para antes (ou o corte vai para o lado que dá quadros ao plano seguinte) precisa de quadros da cena e do clipe antes do que o plano mostra. Se `instanceStartFrame` ou `cameraInFrame` já são 0 (ou mais cedo que o corte pedido), o comando com `content: "anchor"` recusa com `NO_HEAD_FRAMES` e a `hint` diz até onde o início pode ir ("Its start can go no earlier than edit frame N"), que também vai em `limitFrame` (#33). Não segura o primeiro quadro para trás: seria inventar imagem antes do começo da cena.

**Revisão (uso real do Studio):** esta era a política única e quebrava a edição mais comum. Todos os planos de `full-v2` (e de todo bundle de `build-full-bundle.mjs`) começam no quadro 0 da cena e do clipe, então **qualquer** corte movido para a esquerda era recusado. O padrão passou a ser `content: "auto"` (#28): nunca recusa por falta de quadros de cabeça; `anchor` é a exigência de exatidão para scripts.

### 10. `LOCKED`: reinterpretação

O plano diz `LOCKED (sceneInstance.editLocked)`. Mas o gerador marca **todas** as instâncias com `editLocked` e nenhum código da engine lê o campo; recusar tudo que toca uma instância travada inutilizaria os comandos. A leitura adotada segue o que a spec diz que o campo significa ("a cena avança com o tempo da edição", cutaway): os planos de uma instância `editLocked` que estavam **em passo** (mesmo `instanceStartFrame - startFrame`) precisam continuar em passo, senão a cena pularia na segunda volta. O comando que desfaria o passo é recusado com `LOCKED`. Instâncias de um plano só não têm nada para manter. Dúvida registrada: se o produto quiser o bloqueio literal, é uma linha em `shot-plan.ts` (`lockedProblem`). Sob `content: follow` (#28) a leitura vale igual: o plano que começa antes e a continuação que anda junto (#30) ficam em passo entre si; um plano da mesma instância **fora** dessa tomada deixaria de estar em passo, e então `follow` é `LOCKED` e `auto` fica em `anchor`.

### 11. O que "ripple" faz na borda inicial

`trimShot edge: "start"` em ripple: o plano **fica onde está** na edição; os quadros antes de `frame` saem da cabeça dele (`instanceStartFrame`/`cameraInFrame` avançam), ou entram quadros de cabeça se `frame` é anterior, e os planos seguintes andam pelo que isso custou. O plano não desliza contra o anterior porque o anterior não mudou. Em `roll`, é o `moveCut` com o vizinho.

### 12. Clipes de efeito: por escopo

Implementado com os comandos do próprio `fx` (`moveClip`, `resizeClip`, `removeClip`), para as regras dele (keys fora do clipe podadas, fades ajustados, notas) ficarem num lugar só.

- `scope: shot`: o clipe acompanha o plano (`contentShift`) e é cortado nas novas bordas, porque `FX_SCOPE_MISMATCH` recusa clipe que sai do plano. Clipe que fica sem nenhum quadro é removido, com nota.
- `scope: output` e `node`: pela emenda: depois do ponto editado andam pelo delta; os que cruzam esticam ou encolhem. `keepAbsolute: true` deixa todos onde estão (pode produzir `FX_CLIP_OVERLAP`, que é recusa legítima do fx).
- Limitação: as keys de parâmetro de um clipe que cruza o ponto não são re-temporizadas (ficam nos quadros absolutos em que estavam, as que caem fora são podadas).

### 13. Transições

Acompanham o corte por id. Quando um plano encolhe e a transição não cabe (`maxTransitionFrames`), `TRANSITION_TOO_LONG` diz qual encurtar; `shrinkTransitions: true` reduz ao máximo que os dois planos aguentam (`resizeTransition`, ou o dissolve antigo `transitionInto`, que `normalizeTransitions` trata como `dissolve-<toShotId>`). Só se julgam as transições que a mudança tocou.

### 14. Propriedades: nenhuma entrada

`properties/` não ganhou caminhos de plano nem de duração da edição. Uma propriedade escreveria um `editRange` sem o ripple, que é a razão de existir dos comandos, e o registro não tem como chamá-los. O que existe de timing (`fx/<id>/start`, `/duration`, `transition/<id>/duration`) já é do efeito e não mexe nos planos. `shot-properties.spec.ts` falha no dia em que alguém adicionar um caminho `shot/…` ou `edit/…` e manda ler esta decisão.

### 15. Keys de luz e clima

`lighting.lights[].keys` (intensidade, cor, posição) e `weatherKeys` estão nos quadros da edição (schema: "on the edit's own frames"). Andam como qualquer ponto de tempo da edição: depois do ponto, pelo delta; as que caem dentro do trecho removido são **descartadas** (com nota), e quando não sobra nenhuma a lista fica `undefined`. `keepAbsolute` as deixa onde estão. Sem keys, a mesma luz/clima de antes.

### 16. Narração: só com `withNarration`

Sem a opção, nenhuma fala anda. Com ela, as falas que começam depois do ponto editado andam pelo delta, em segundos (via `moveClip` do próprio `narration`, quadros arredondados em 1e-6). Falas sem take próprio (a voz está no mix, `fromMix`) se afastam da voz que legendam: o comando avisa em nota. O mix nunca anda.

Uma fala que **começa dentro do trecho removido** por um ripple que tira quadros não tem quadro onde começar (todas cairiam no ponto da emenda e o `narration` recusaria o lote por `CLIP_OVERLAP`). Elas são empilhadas a partir do ponto da emenda (ou do fim da fala anterior, se for depois), na ordem e com a duração que têm, no espaço antes da próxima fala que mantém seu lugar. A que não cabe é **removida**, com uma nota por fala (preferimos isso a encurtar: a duração de uma fala com take é a do take). Nunca recusa a edição inteira por isso; uma fala que já ocupava o ponto da emenda desde antes de começar o trecho (um overlap que não nasceu aqui) segue sendo problema do `narration`.

### 17. `{ shots: <comando> }`, um por entrada

O plano diz `{ shots: [...] }` (lista). Seguimos a porta `{ fx }`: uma entrada é um comando, e entradas seguidas da mesma camada são agrupadas (`BATCHED`) e rodam como um lote, com um `finalizeBundle` no fim da lista inteira. Um erro guarda o `index` da entrada e `layer: "shots"`. A CLI monta `[{ shots: comando }]`; `scene edit '[{"shots":{...}}]'` aceita a lista.

### 18. `MAX_EDIT_FRAMES = 216_000`

Uma hora a 60 fps. Sem um teto, um dígito a mais viraria uma edição que derruba o preview. `EDIT_TOO_LONG` diz o teto.

### 19. Opções por comando

`withNarration`, `keepAbsolute`, `shrinkTransitions` e `content` (`anchor` | `follow` | `auto`, padrão `auto`, #28) são campos opcionais de cada comando (e flags `--with-narration`, `--keep-absolute`, `--shrink-transitions`, `--content`). O comando é então o pedido inteiro: o Studio o manda como está, a CLI o monta das flags, e um lote pode pedir uma coisa diferente em cada entrada.

### 20. `explainShots` e `scene show --layer shots`

Devolve `{ edit, shots[], audio, narrationEndSeconds, warnings }`; cada plano com seu lugar, `intent`, `transitionInto` e `handles` (quadros reais de cena e de câmera antes/depois do que o plano mostra). Além dos `handles` a cena e a câmera seguram o último quadro (decisão 7), então um plano pode ser maior mesmo sem handle. `scene validate` anexa os avisos de `explainShots` a `warnings` (nunca mudam o exit code).

### 21. `setEditDuration` só mexe no último plano

Como diz o plano: alonga ou encurta o último plano, e só ele. O menor valor é "início do último plano + 1". Abaixo disso o erro é `SHOT_TOO_SHORT` com a `hint` do limite.

### 22. Estúdio: o transporte e a forma de onda com áudio e edição de tamanhos diferentes

Só remover a recusa deixava dois defeitos, ambos corrigidos na engine (com specs):

- `createWebAudioTransport.playFromEditSeconds` saía antes de armar o relógio quando não havia áudio a tocar (começar depois do fim do áudio): `playing` ficava falso e o playhead congelava no ponto de partida. O relógio é armado antes de decidir se há som; o áudio acaba e o resto da edição é silêncio, e uma edição mais curta que a janela corta o áudio onde a imagem acaba (`remainingEditSeconds`).
- `peaksForEditWindow` espalhava a janela do áudio por toda a largura da forma de onda. Agora recebe a duração da edição: os baldes cobrem a **linha do tempo da edição**, com baldes silenciosos depois do fim do áudio (edição mais longa) ou o áudio cortado onde a edição acaba (mais curta).

### 23. `outline` não tem tempo

`bundle.outline` agrupa camadas, luzes e trilhas e diz o que cada grupo oculta ou muda; não guarda quadros nem segundos. O plano listava "grupos do outline" entre as camadas com tempo: pela leitura, não há nada a deslocar.

### 24. Estúdio: a fatia `shots` e a rota `PUT /bundle-shots`

Um arrasto muda cinco coisas de uma vez: os planos, as instâncias de cena (crescem para caber um plano maior), a duração da edição, a camada `fx` (clipes e transições seguem os planos) e as keys de luz e clima. Salvar e desfazer isso como UM passo pede um valor só. Escolhemos o caminho menos invasivo que mantém o `finalizeBundle` como porta do arquivo:

- `ShotsSlice` (`packages/scene-engine/src/shots/shot-slice.ts`): `{ editDurationFrames, shots, sceneInstances, fx | null, scenes[] }`. `scenes` traz **todas** as cenas, com ou sem keys: um comando pode tirar a última key de uma cena, e uma fatia que a omitisse não a devolveria no desfazer.
- `PropertyChange` ganha o domínio `{ domain: "shots", slice }` (`applyPropertyChanges` roda o `finalizeBundle` no fim, como nos outros). Isto **não** contradiz a #14 ("propriedades: nenhuma entrada"): nenhuma *propriedade* escreve a fatia; o domínio existe só para o Estúdio e o `/bundle-shots` do harness.
- O Estúdio **não** manda o comando: manda a fatia depois do comando. A engine não tem o inverso de um ripple; já a fatia de antes do arrasto, escrita de volta, devolve exatamente o bundle que existia. Desfazer e refazer são o mesmo PUT com a fatia de antes e de depois.
- A narração fica **fora** da fatia: os comandos só a movem com `withNarration`, que o Estúdio nunca pede (#16). Por isso a engine do arrasto usa os padrões de `withNarration`, `keepAbsolute` e `shrinkTransitions` (#19).
- `commitShots` roda o `finalizeBundle` no bundle com a fatia **antes** de gravar o passo no histórico: uma fatia que o servidor recusaria nunca vira passo de desfazer.
- A fatia se sobrepõe aos domínios `fx`, `lighting` e `weatherKeys` do store. Ao confirmar, o store os "adota" (`adopt`): uma edição posterior deles parte do que a fatia escreveu, e o desfazer de cada um não volta a um valor velho.

### 25. Estúdio: o que a runtime relê quando a duração muda

`createStudioSession.setShots(slice)` troca no bundle (no lugar, a runtime e o avaliador leem o mesmo objeto) `shots`, `sceneInstances`, `editDurationFrames` e `fx`, sempre por **arrays novos**: o cache derivado do fx é chaveado pela identidade de `bundle.shots`/`bundle.fx`/`outline`, então trocar a referência o invalida sem código extra. Depois:

- `TransportController.setEditDuration(frames)` atualiza `config` e `editTimelineEndSeconds` e avisa a porta de áudio (`setEditTimelineDuration?`, opcional; a do Web Audio guarda a duração para cortar o áudio onde a imagem acaba). Não move o playhead; o Estúdio reposiciona com `seekToFrame(min(atual, última))`.
- `controller.bindShots(slice.shots)` refaz a lista que o transporte usa para achar o plano de um quadro.
- A forma de onda deixou de ser um valor fixo: `readiness.peaksFor(frames)` a calcula pela duração da edição (com o último resultado em cache).
- Memos que dependiam de `session.bundle` (que não muda de referência) passam a usar `useEditedBundle(session)`, que devolve uma referência nova a cada mudança da fatia.

### 26. Estúdio: a semântica do arrasto

- A borda direita de uma barra é `trimShot edge:"end"` em `ripple`; o corte entre dois planos é `moveCut` (roll). Alt troca um pelo outro **onde os dois fazem sentido**: a borda do último plano não tem plano depois para rolar, então Alt não muda nada ali (o comando recusaria com `SHOTS_NOT_ADJACENT`).
- O mínimo é 1 quadro, garantido pelo planejador (`planShotDrag`), para o preview não piscar em recusas óbvias; o resto quem decide é a engine. Num ripple, quanto a edição pode crescer é `EDIT_TOO_LONG`.
- Recusa da engine que **tem um quadro-limite** (`SHOT_TOO_SHORT`, `TRANSITION_TOO_LONG`, `NO_HEAD_FRAMES`, `EDIT_TOO_LONG`: `limitFrame`, #33): o fantasma **trava no quadro legal mais próximo** em vez de voltar atrás, o crachá diz o limite e soltar ali confirma esse quadro (#34). A engine só recusa o corte arrastado para antes por `NO_HEAD_FRAMES` com `anchor`, que o Studio nunca pede: com o padrão `auto` o arrasto passa.
- Recusa **sem** quadro-limite (`LOCKED`, `CONTINUOUS_JOIN`, `SHOTS_NOT_ADJACENT`, e as das outras camadas, como `TRANSITION_NO_HANDLES`): o fantasma fica no tom de erro, o último estado legal continua na tela, soltar reverte e o motivo vai para a linha de status (mesmo padrão do `FxEditStatus`). Esc cancela e reverte. O bundle nunca fica inválido.
- A escala da linha do tempo é **congelada** durante o arrasto (`timelineHold`): um ripple muda a duração da edição, e sem isso as linhas se reescalariam sob o ponteiro. O congelamento vale também para a largura da forma de onda e é solto ao soltar ou cancelar.
- O preview de cada movimento não grava passo nem salva; soltar grava UM passo de desfazer e faz um PUT.

### 27. Estúdio: o comprimento da edição e os avisos

O cabeçalho do transporte mostra o comprimento da edição (quadros, timecode, segundos) e, quando as camadas não combinam, um aviso em linha (`role="status"`, nunca bloqueia nada): a imagem passa do áudio, o áudio passa da imagem, uma fala passa do fim da imagem. São os avisos de `explainShots`, que seguem a #1 (camadas independentes; "não corrija"). **Limite conhecido:** a narração vive no estado do `useNarration`, fora do store, então o aviso de narração lê a narração como foi carregada e não vê falas editadas na mesma sessão até recarregar.


### 28. Política de conteúdo do plano que começa antes: `content`

Quando o **início** de um plano vai para antes (`moveCut` para um quadro anterior, `trimShot edge: "start"` ou `edge: "end"` em `roll` para antes do corte, `setShotDuration` em `roll`, e o ripple da cabeça com `frame` anterior), o plano de entrada ganha quadros e alguém tem que dizer **quais**. O campo `content` (`anchor` | `follow` | `auto`, padrão `auto`; CLI `--content`) escolhe:

- `anchor`: a imagem fica **nos mesmos quadros da edição** (um roll à la NLE). O plano ganha quadros de cabeça: `instanceStartFrame` e `cameraInFrame` recuam. Sem quadros suficientes, `NO_HEAD_FRAMES` (#9).
- `follow`: a cena (e o clipe de câmera) **começam no novo início**. `instanceStartFrame`/`cameraInFrame` não recuam (nunca vão a negativo): o plano só começa antes, fica maior e mostra mais da **cauda** da cena (a regra de segurar o último quadro, #6 e #7, cobre o que passa do fim). A imagem do plano inteiro desliza para antes na edição.
- `auto`: `anchor` pelos quadros de cabeça que existem, `follow` pelo resto. Nunca recusa por falta de cabeça. Quando cai para `follow`, o resultado leva uma nota ("Scene frame 0 reached: the scene now starts at the cut, N frames earlier"; no ripple da cabeça: "... the N frames its head could not hold are added at the end of its take").

Só vale para o lado de **entrada** que vai para antes. Corte para depois, fim do plano, duração da edição e cortar a cabeça (para depois) funcionam como antes, qualquer que seja o `content`. O que desliza em `follow` é uma só função linear (o plano mostra um quadro por quadro), então `auto` ancora `a` quadros e segue `d - a`; não existe "ancorar os primeiros quadros e seguir os últimos".

### 29. `auto`: o que ele ancora

`a = min(d, instanceStartFrame, cameraInFrame)` menos a **reserva de transição**: a cena e o clipe recuam **juntos** (ficam em passo entre si, como sempre) e o menor dos dois limita. A reserva são os quadros de cabeça que a transição **para dentro** do plano desenha antes do corte (`transitionWindow(...).framesBeforeCut`, só com `handles` diferente de `hold`): se `auto` ancorasse tudo, a transição perderia a cabeça de que precisa (`TRANSITION_NO_HANDLES`) e o padrão voltaria a recusar por falta de cabeça. Com `anchor` não há reserva: a recusa é da camada de efeitos, com as palavras dela. A **cauda** do plano que fica mais comprido (que `follow` gasta) não é reservada: uma transição que sai dele e precisa de quadros reais de cauda recusa com `TRANSITION_NO_HANDLES` e a `hint` dela (`handles: "hold"`); é a única recusa de cena que o padrão ainda pode dar neste caminho.

### 30. `follow` e as junções: `CONTINUOUS_JOIN`, a continuação e `editLocked`

- **Junção contínua para dentro do plano** (`from` e `to` são uma tomada): `follow` seria um salto na imagem, então é `CONTINUOUS_JOIN` (a `hint` manda usar `anchor` ou `auto`). `auto` nunca cai para `follow` aqui: ancora, e ancorar sempre é possível (os quadros antes do plano são o fim da tomada do anterior). Cortar a cabeça (ripple) de um plano que continua o anterior segue recusado como na #8.
- **Continuação para fora do plano** (o plano que começa antes é continuado pelo seguinte, e este pelo outro…): o fim do plano na cena avança `N` quadros, então a cadeia inteira **desliza `N` quadros na cena** (`instanceStartFrame` e `cameraInFrame` de cada um, sem mudar as faixas de edição), igual ao ripple do fim (#8), para a continuidade exata da junção continuar valendo (e a instância cresce para caber, #6).
- **`editLocked`**: ver #10. `follow` mexe no passo (`instanceStartFrame - startFrame`) do plano; a continuação que anda junto fica em passo com ele. Se **outro** plano da instância, fora da tomada, estava em passo, `follow` é `LOCKED` e `auto` fica em `anchor`.

### 31. As camadas em quadros da edição seguem a imagem: o splice virou uma cadeia

Em `follow` a imagem de um trecho da edição **desliza para antes**; as camadas que estão em quadros da edição e pertencem à imagem têm que ir junto, e fora do trecho ficar onde estão. Um `Splice` não descreve isso (ele desloca tudo o que vem depois), então `ShotPlan.splice` virou `splices`: uma **cadeia**, cada splice nas coordenadas que o anterior deixou (`mapStart`, `mapEnd`, `isRemoved`, `remapKeys` aceitam a cadeia). O que cada comando monta quando `followed > 0` (`a` quadros ancorados, `f` seguidos, `d = a + f`):

| Comando | Cadeia |
| --- | --- |
| `moveCut` para antes (roll; a edição não muda de tamanho) | tira `d` quadros do fim do plano anterior (em `frame`), põe os `a` quadros de cabeça ancorados no mesmo ponto, e põe `f` quadros no **fim da tomada** (`fim - f`, no fim antigo da última continuação do plano) |
| ripple da cabeça para antes (a edição cresce `d`) | põe `a` quadros no início do plano; põe `f` quadros no fim da tomada (já depois dos `a`) |

Sem `follow` (`f = 0`) a cadeia é a de antes (vazia no roll; um splice de `d` quadros no ripple). Por camada, em `follow`:

- **Clipes de `fx` com `scope: shot`**: andam com a imagem do plano (`contentShift`, que já era `início da edição - início na cena`: `-f` para o plano e para a continuação que deslizou) e são cortados nas bordas novas do plano (como antes).
- **Clipes `output`/`node`, keys de luz e clima** (`shot-layers-fx.ts`, `shot-layers-keys.ts`): passam pela cadeia. O que está dentro da tomada que deslizou anda `-f`; o que fica no fim do plano anterior que foi cedido ao corte é cortado (clipe) ou descartado com nota (key); o que vem depois do fim da tomada fica onde estava. `keepAbsolute` os deixa onde estão, como sempre.
- **Narração** (`withNarration`, #16): a mesma cadeia. Sem `withNarration`, nunca anda; o áudio nunca anda (#1).
- **Transições**: continuam presas aos planos por id; o tamanho dos dois planos é checado como sempre (`TRANSITION_TOO_LONG` ou `shrinkTransitions`). O plano que ganha quadros nunca perde a janela; o que cede quadros ao corte pode.
- **Câmera, rig, caminhos, reveal, `performanceTracks`** (#5): não andam. **`outline`** não tem tempo (#23).

A cadeia fica no fim da tomada, não no fim do plano, pelo mesmo motivo da #8.

### 32. O ripple da cabeça para antes sem cabeça

`trimShot edge: "start"` ripple com `frame` anterior ao início: o plano **fica onde está**, ganha `d` quadros e os seguintes andam `d` (#11). Com `follow`, os `f` quadros que a cabeça não tem **acabam no fim da tomada** (o plano mostra mais da cauda) em vez de inventar imagem antes do começo da cena: para o primeiro plano de `full-v2`, `--frame=-N` ganha `N` quadros no fim do plano. É o mesmo efeito de alongar o fim, e a nota diz isso.

### 33. `limitFrame`: o quadro que a recusa já sabe

`ShotCommandError` (e a recusa do planejador) ganhou `limitFrame?`: o quadro mais próximo, no `frame` do próprio comando (o fim do plano em `setShotDuration`/`setEditDuration`), em que **esta** regra não recusa. Existe para `SHOT_TOO_SHORT` (os dois lados, ripple e roll), `NO_HEAD_FRAMES` (`anchor`), `EDIT_TOO_LONG` e `TRANSITION_TOO_LONG` (o plano curto demais para a janela, medido a partir da borda que o comando não deixa onde estava: `ShotPlan.edges` diz qual é). Os outros códigos não têm. É um limite desta regra: outra pode recusar nesse quadro, e o propriedade-teste só afirma que o comando no limite não recusa **pelo mesmo código**. A CLI o devolve no JSON do erro.

### 34. Estúdio: o fantasma trava no quadro legal

`judgeClampedShotPlan` (`shotsEdit.ts`): recusa com `limitFrame` diferente do quadro proposto vira o mesmo plano em `limitFrame` (`clampShotDragPlan`, puro, em `shotsDrag.ts`); se ele for legal, o veredito é `legal` com `clamp: { code, askedFrame, limitFrame }`, o preview mostra esse quadro e o crachá acrescenta o limite (`studio.shots.drag.limit`). Soltar confirma o quadro travado, um passo de desfazer. Se mesmo no limite a engine recusa (outra regra), vale a recusa normal (#26). O Studio pede o padrão `auto`, então arrastar o corte para antes passa sem recusa.

## Verificação feita

- Propriedade (`shot-property.spec.ts`): 120 sequências de 6 comandos aleatórios (semente fixa, `trimShot`/`moveCut`/`setShotDuration`/`setEditDuration`, modos e opções sorteados) em dois bundles (o de teste puro e um com efeitos, transição `hold`, keys de luz/clima e narração). Cada comando aceito produz um bundle que passa `finalizeBundle`; os recusados deixam o bundle como estava. Recusas vistas: `SHOT_TOO_SHORT`, `SHOTS_NOT_ADJACENT`, `NO_HEAD_FRAMES`, `CONTINUOUS_JOIN`, `TRANSITION_TOO_LONG` e as do fx (`FX_CLIP_OVERLAP`, `CLIP_OVERLAP`).
- Runtime: `shot-runtime.spec.ts` (câmera e cena seguram separadas; instância cresce; transição desenha o lado seguro).
- Política de conteúdo (`shot-content.spec.ts`, `shot-splice.spec.ts`, `shot-property.spec.ts` com `content` sorteado e a regra "o padrão nunca recusa `NO_HEAD_FRAMES`" e "o comando no `limitFrame` não recusa pelo mesmo código"; CLI em `scene-shots-cli.spec.ts`). Exemplo na CLI, numa cópia de `full-v2` (`cp -Rc`): `scene shots <bundle> move-cut shot_scribe shot_hands --frame 136`: com `--content anchor`, `NO_HEAD_FRAMES` (limite 161); sem a flag, o corte vai para 136, `shot_hands` fica com `instanceStartFrame` 0 e 120 quadros, a instância `inst_hands` cresce de 95 para 120 quadros e `scene validate` passa antes e depois.
- Render: `scripts/fx-determinism.mjs` rodado numa cópia de `full-v2` editada pela CLI (plano 2 alongado, corte movido, edição de 600 quadros): todos os hashes iguais, incluindo os quadros acrescentados. `fx-pixel-baseline verify`, `fx-golden` e `fx-lab-check` verdes no bundle sem edição.
- Estúdio (`apps/scene-studio`): specs puras do arrasto (`shotsDrag.spec.ts`: mapeamento ponteiro→comando, Alt, limites), da geometria (`shotsTimeline.spec.ts`), do veredicto da engine e das recusas (`shotsEdit.spec.ts`), dos textos e da paridade de chaves pt-br/en/es (`shotsText.spec.ts`) e do store (`editorStoreShots.spec.ts`: preview sem gravar, um passo de desfazer por arrasto, PUT em `/bundle-shots`, desfazer exato, recusa reverte). Na engine, `shot-slice.spec.ts` e os specs do `TransportController.setEditDuration`. Não foi feita verificação ponta a ponta no navegador.
