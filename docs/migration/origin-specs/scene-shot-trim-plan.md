# Plano — encurtar, alongar e mover o corte das cenas (planos)

Estado: plano para implementação. Dono do pedido: o usuário. Refs JOU-1175 (VGP).

## 1. Problema

Hoje a duração de cada plano (`shot.editRange`) só nasce no `build-full-bundle.mjs`, a partir da narração. Nem o
Studio nem a CLI mudam o intervalo de um plano depois: quem quer uma cena maior ou menor regera o bundle. O usuário
quer, na timeline, **alongar ou encurtar um plano, mover o corte entre dois planos e alongar ou encurtar a edição
inteira (ripple)**, sem regerar nada.

## 2. Decisões já tomadas (do usuário)

1. Mover o corte E alongar/encurtar a edição inteira, com ripple.
2. **A imagem pode ficar desalinhada do áudio.** Isso não é um erro nem um aviso a bloquear: é permitido. O áudio e a
   narração são uma camada independente; mudar o tamanho dos planos não os move nem os corta por padrão. Todas as
   skills que ensinam a editar o vídeo (`scene-cli`, `scene-element-recipes`, `scene-fx`, quando existir, e o
   `scene-engine-index.md`) têm que dizer isso com todas as letras, para um agente não "corrigir" a falta de sincronia.

## 3. O que um plano é (para quem implementa)

`Shot` (`schema/render-bundle.schema.ts`): `id`, `sceneInstanceId`, `editRange {startFrame, endFrameExclusive}`,
`instanceStartFrame` (primeiro quadro da cena que ele mostra), `cameraClipId`, `cameraInFrame` (primeiro quadro do
clipe de câmera), `transitionInto` (dissolve antigo), `intent`. O bundle tem `editDurationFrames`. Os planos cobrem a
edição sem buracos nem sobreposição (`validate/` confere). Quadros do plano = quadros da edição; quadros da cena =
`instanceStartFrame + (editFrame - editRange.startFrame)` (ver `edit/resolve-shot-at-offset.ts`,
`resolve-shot-at-edit-frame.ts`).

Alongar o fim de um plano mostra quadros da cena depois dos que mostrava. Se a cena ou o clipe de câmera acabam antes,
**segura o último quadro** (a câmera já segura o último pose; a cena também, ver §5). Encurtar o início de um plano
(mover o corte para a direita no plano seguinte) avança `instanceStartFrame` e `cameraInFrame` no mesmo tanto, para a
cena não "voltar".

## 4. Operações (camada de comandos pura, na engine)

Novo grupo de comandos de plano, no mesmo molde de `fx-commands` e `camera-commands` (puros, sem I/O, lista de comandos
→ bundle novo ou erros com `code/message/hint`), exposto por `editBundle` como `{ shots: [...] }` e pela CLI:

| Comando | O que faz |
|---|---|
| `trimShot {shotId, edge: "start"\|"end", frame, mode}` | Muda a borda do plano para `frame` (quadro da edição; para `end`, o fim exclusivo). |
| `moveCut {fromShotId, toShotId, frame}` | **Roll**: muda o corte entre dois planos vizinhos para `frame`. A duração total não muda. Os dois planos ajustam `instanceStartFrame`/`cameraInFrame` do lado que perde ou ganha quadros. |
| `setShotDuration {shotId, durationFrames, mode}` | Atalho de `trimShot edge:"end"`. |
| `setEditDuration {frames}` | Muda o fim da edição: alonga ou encurta o ÚLTIMO plano (e só ele). |

`mode`:
- `ripple` (padrão): a borda se move e **tudo que vem depois na linha do tempo anda junto** (os planos seguintes
  mudam de `editRange` pelo mesmo delta; `editDurationFrames` muda pelo delta).
- `roll`: só vale em `moveCut`/`trimShot` entre vizinhos; o vizinho absorve a diferença, `editDurationFrames` fica.

Regras de erro (códigos estáveis): `SHOT_TOO_SHORT` (mínimo 1 quadro), `SHOT_NOT_FOUND`, `SHOTS_NOT_ADJACENT`,
`TRANSITION_TOO_LONG` (uma transição ligada ao plano não cabe mais: usar `maxTransitionFrames`; o comando recusa e diz
qual transição encurtar, ou aceita `shrinkTransitions: true` que a reduz), `EDIT_TOO_LONG` (limite razoável, p. ex.
`MAX_EDIT_FRAMES`), `LOCKED` (`sceneInstance.editLocked`).

## 5. O que acompanha o ripple (política, decidir e documentar em `docs/video-production/scene-shot-trim-decisions.md`)

Princípio: **o que está preso a um plano anda com o plano; o que está no tempo absoluto da edição não anda, a menos
que o usuário peça.** Para cada camada, a regra proposta e o ponto a confirmar lendo o código:

| Camada | Regra |
|---|---|
| Instância de cena (`sceneInstance.durationFrames`) | Se o plano alonga além do que a cena dura, a instância cresce até caber (a cena segura o último quadro) — confirmar em `validate/bundle-integrity.ts` que a instância pode ser maior que o conteúdo; senão, `instanceStartFrame` + duração nunca passa da instância e o excedente é "segurar o último quadro" no runtime (`resolve-shot-at-offset`). |
| Câmera (`cameraClips`, segmentos) | Não muda. O fim do clipe é `hold`; alongar o plano só aumenta o `hold` desenhado. O resolvedor já segura o último pose. |
| Transições (`fx.transitions`, `shot.transitionInto`) | Ligadas por `fromShotId/toShotId`: acompanham o corte. A janela é recalculada; erro `TRANSITION_TOO_LONG` quando não cabe. |
| Clipes de efeito com `scope: shot` | Andam com o plano: `startFrame` desloca pelo delta do início do plano; se o clipe ultrapassa o novo fim do plano, é cortado no fim (ou fica como está, se o escopo permite — decidir e documentar). |
| Clipes de efeito com `scope: output` ou `node` | Tempo absoluto da edição: **com ripple, os que começam depois do ponto editado andam pelo delta**; os que cruzam o ponto são esticados/encurtados. Opção `keepAbsolute: true` deixa todos onde estão. |
| Keys de luz, clima, pista de rig, grupos do outline (frames da edição) | Mesma regra dos clipes `output`: depois do ponto editado, deslocam pelo delta (ripple). Verificar em cada módulo se o tempo está em quadros da edição (desloca) ou da cena (não desloca). |
| Narração (`bundle.narration`, tempo em segundos da edição) e áudio (`bundle.audio`) | **Não andam por padrão** (decisão do usuário: a imagem pode desalinhar do áudio). Opções explícitas: `withNarration: true` desloca as falas depois do ponto pelo delta em segundos. O áudio nunca é cortado nem esticado. |
| Janela do áudio (`audio.editStartSeconds/editEndSecondsExclusive`) | Hoje a Studio recusa abrir se a janela de áudio for menor que a edição ("Audio edit window shorter than edit timeline", `createStudioSession.ts`) e a validação da narração usa a duração da edição. Isso tem que mudar: **edição mais longa que o áudio é válida** (o que passa do fim do áudio é silêncio) e narração além da edição vira aviso, não erro. Atualizar `explainNarration`, o integrity check e o Studio. |

## 6. Camadas do trabalho (na ordem)

1. **Engine, comandos puros** (`packages/scene-engine/src/shots/`: `shot-commands.ts`, `shot-schema`, specs): os quatro
   comandos, o ripple de cada camada da §5, erros com `code/message/hint`, `explainShots`. Cobertura: propriedade
   "depois de qualquer sequência de comandos válidos o bundle passa `validateBundle`", casos limite (plano de 1
   quadro, último plano, transição que não cabe, `editLocked`, alongar além da cena).
2. **`editBundle`**: variante `{ shots: ShotCommand[] }` (como `{ fx }`), uma validação final por `finalizeBundle`.
3. **CLI** (`bin/scene.mjs`, módulo `bin/scene-shots.mjs`, JSON no stdout, exit 0/1/2, nada gravado sem `--write`):
   `scene shots <bundle> list | trim <shotId> --edge start|end --frame N [--mode ripple|roll] | move-cut <from> <to>
   --frame N | duration <shotId> --frames N | edit-duration --frames N`, com `--with-narration`, `--keep-absolute`,
   `--shrink-transitions`. `scene show --layer shots`.
4. **Propriedades** (`properties/`): `scene/shot/<id>/editRange` etc., só leitura ou via comando (o registro de
   propriedades não pode driblar o ripple).
5. **Runtime**: confirmar que alongar além da cena/clipe segura o último quadro no preview e na renderização
   (`resolveShotAtOffset`, `evaluateShotFrame`, compositor de efeitos: transições seguram quadros via `FxSide`).
6. **Studio** (`apps/scene-studio`): uma trilha **"Planos"** na timeline (barras de cada plano, rótulo = `intent`,
   cores alternadas por cena), com:
   - alça direita de cada plano: arrasta = `trimShot end` em ripple (os seguintes andam, a edição muda), com o selo
     flutuante com a duração e o delta;
   - alça esquerda de um plano / a divisa entre dois planos: arrasta = `moveCut` (roll);
   - tecla modificadora (Alt) alterna o modo (ripple ↔ roll); campo numérico opcional;
   - Delete NÃO apaga plano (fora de escopo);
   - mesma estrutura de arraste, desfazer em UM passo e `PUT /bundle-…` que os efeitos já usam (`useTimelineDrag`,
     `useFramePointer`, `clickGuard`, `useFxEdit` como modelo; `editorStore` ganha um domínio `shots`);
   - aviso visível quando a edição passa do áudio ("a imagem passa do áudio em N s") ou a narração passa da edição,
     sem bloquear;
   - o preview recarrega as janelas de transição e das trilhas (as outras trilhas já desenham todos os planos).
7. **Skills e docs** (parte obrigatória do pedido): em `.claude/skills/scene-cli/SKILL.md` e
   `.claude/skills/scene-element-recipes/SKILL.md` (e `scene-fx` quando for escrita): a seção "Duração dos planos",
   com os comandos acima e a regra **"imagem e áudio são independentes: mudar a duração de um plano não move o áudio
   nem a narração, e o vídeo pode terminar antes ou depois do áudio; isso é permitido, não conserte"**, o que anda
   com o ripple (§5), e um exemplo executado (encurtar o plano 2 e conferir `scene validate`). Atualizar
   `scene-engine-index.md` e os `build-full-bundle.mjs` doc comments que dizem que a duração vem da narração.

## 7. Critérios de pronto

- Comandos: specs de propriedade e de casos limite; todo resultado passa `validateBundle`/`finalizeBundle`.
- CLI: cada subcomando executado numa cópia de `full-v2` (`~/www/.data/scene-engine/gen-lab/full-v2/bundle`) e a
  saída conferida; `scene validate` verde antes e depois; nada gravado sem `--write`.
- Estúdio: arrastar a alça direita de um plano muda a duração, move os seguintes, um passo de desfazer; mover o corte
  não muda a duração total; a edição mais longa que o áudio abre sem erro; testes `*.spec.ts` dos helpers puros.
- Determinismo: `scripts/fx-pixel-baseline.mjs verify` e `fx-determinism.mjs` continuam idênticos num bundle que não
  foi editado.
- Revisão com OpenQodex (`thermo-qodex`) e `code-review` antes de qualquer push.

## 8. Fora de escopo (agora)

Dividir um plano em dois (`splitShot`), apagar/reordenar planos, estender o áudio ou gerar novo áudio, câmera
contínua atravessando o corte, "slip" (mudar `instanceStartFrame` sem mexer na duração).
