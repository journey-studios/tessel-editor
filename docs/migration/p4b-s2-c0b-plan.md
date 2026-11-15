# P4b S2-C0b — contrato temporal, teto válido e consumidor X

Data do plano: 2026-11-15. Base: main c9c5fa081a4b09193885b0c4afa91a1fdcc8ecb0,
squash da PR69/C0a. Backlog: BL082.4.4, .20, .21, .23 e parte de estado da .17.
Este documento fixa o contrato antes do código; o backlog concentra status e atribuição.

## Objetivo e limites

Preparar um ScenePlan com transição para o futuro parser externo, verificando seus relógios
por dados explícitos e consumindo a referência X representável no core real.
Esta PR é só C0b. Não implementar H, R ou S2A; não tocar runtime, host loaders, renderer,
export, UI ou admissão pública. Refuse continua em toda entrada pública e F064 produto P.
Rust mantém 14 capabilities; JS mantém 13 sem fxTransition. Nenhum Domain novo.

A C0a foi aceita no código 16fa526/head9f7cc83: Windows R1 192/10/5/1 em debug,
preserve e release, lib615, desktopFX80; Mac core1699/0/7 e estáticos/no-run/build.
Quatro qualificações defensivas de .19 ficaram sem alcance comprovado. Isso não é prova nova
da C0b. Windows ainda tem dois gizmos vermelhos da base; não declarar workspace global verde.

## Decisões de wire (.20/.23)

1. Manter PLAN_VERSION=1, PlanFrame e todos os bytes de planos SEM transição.
   O wire interno COM transição ganha alignment e um bloco timing obrigatório. Antigo wire
   com transição sem esses campos é recusado, sem defaults nem fallback. Era interno atrás
   de Refuse/capability ainda ausente no JS; documentar a quebra desse formato experimental.
2. timing possui editDurationFrames e os lados from/to. Cada lado declara:
   shotStartFrame, shotEndFrameExclusive, instanceStartFrame, instanceDurationFrames,
   cameraInFrame, cameraClipDurationFrames, sampledInstanceFrame, sampledCameraClipFrame.
   Campos de quadro são inteiros estritos não negativos; conversões/aritmética i64 verificadas.
   Os IDs do shot estão em fromShotId/toShotId; instância em source.sceneInstanceId e câmera
   em frame.cameraClipId do respectivo deck. Não duplicar IDs nem tempos autoritativos.
3. C é timing.to.shotStartFrame. Exigir timing.from.shotEndFrameExclusive == C.
   Intervalos não vazios, dentro da edição; f pertence ao intervalo do primary.
   primary=from para f<C e to desde f=C. alignment é o enum do core; reutilizar
   transition_window(C,duration,alignment,editDuration) para conferir a janela inteira,
   startFrame, duration e frameInWindow. Overflow é recusa, nunca saturação nem None==None.
4. Extrair uma única regra pura de clocks no core, usada pelo avaliador de offset, pelo
   montador e pelo validador; não copiar fórmulas em parser/validator/build/timeline.
   Para cada lado: delta=f-shotStart; rawInstance=instanceStart+delta;
   rawCamera=cameraIn+delta. Cada clock é preso independentemente a [0,L-1].
   Preservar L=0 => amostra0 e held=true; último quadro real => held=false.
   Resultado nomeado distingue raw, sampled e held por clock e é acessível pela lib.
5. Legado primário: frame.instanceFrame/cameraClipFrame e seus tempos continuam BRUTOS.
   A instância do primário é real e a integridade do shot cabe na instância.
   Câmera primária pode estar além de seu comprimento; não trocar o avaliador estrito por
   offset nem prometer que metadado sozinho muda pose/pixels. Os sampled* de timing dizem
   o resultado da regra temporal de transição. Exemplo legado .23: raw23/sample11.
   Legado other: frame guarda amostras; heldInstance/heldCamera existentes são exatamente
   os flags derivados, inclusive false. Substituir a regra antiga "cabeça presa implica0":
   com cameraIn além do fim uma câmera pode prender na cauda mesmo antes do corte.
6. Tempos de frame são Ratio exato pelo FPS: editTime real nos dois decks; tempos legados
   de instância/câmera correspondem aos respectivos contadores legados. FPS inválido,
   tempo incompatível ou overflow são recusados. Compartilhar cálculo verificado do core,
   evitando Rate::frame_to_time se sua falha puder virar zero.
7. Mesma instância/câmera identificada nos dois decks não declara comprimentos diferentes.
   Leitura e validação tipada exigem as mesmas relações, com ponteiro preciso sob
   /transition/timing ou campo legado afetado. Campo ausente, desconhecido ou tipo inválido
   recusa. Outros invariantes de versão/recursos/params/progress/depth continuam.
8. Isto valida COERÊNCIA interna, não autenticidade de origem. Um plano inteiramente
   adulterado e coerente pode declarar outra cena. Não alegar reconstrução ou autenticação
   de bundle, nem reenviar regras de admissão do documento aos adapters.
9. timing não aumenta a contagem de itens; os seis arrays continuam a unidade contratada.
   Seus bytes contam normalmente nos 4MiB. Nenhum teto, tolerância ou baseline é relaxado.

Antes de implementar, salvar este plano e a descrição exata dos nomes/tipos/API/pointers em
commit próprio de docs. Ajustes de nomes só se melhorarem clareza e forem documentados antes
do código; preservar os fatos acima, sem aguardar confirmação de rotina.

## Marco A — modelo, montagem e validação

- Tipos pequenos próprios para timing e clocks; separar JSON e validação em módulos focados.
- ScenePlan::build e PlanTimeline derivam os mesmos dados dos shots/instâncias reais e do
  comprimento de CameraTimeline (máximo da trilha principal e layers), usando o cache da timeline.
- Preservar o cold/timeline, ordem aleatória/inversa e repetição sem estado residual.
- Explicar em PlanFrame o legado bruto do primário e a amostra de other; accessor não inventa
  uma terceira interpretação. Não adicionar metadata de transição a planos sem transição.
- Testar typed validate(), from_json(Value) e from_json_str(text) no positivo e nos negativos.
  Incluir missing timing/alignment, campos extra/tipo float/bool/string/null, extremos,
  cut errado, primary trocando cedo/tarde, janela incompatível, raw/sample/held inconsistentes,
  held=false adulterado, FPS/Ratio incorretos, IDs compartilhados/comprimentos divergentes.
- Bordas: C-1/C, D1, duração ímpar center, start/end, zero length no helper, último quadro
  sem hold, primeiro preso, cameraIn além do fim antes de C; main/layers com comprimentos
  distintos e câmera primária curta. .23 ganha teste explícito raw23/sample11.
- Baseline M0 sem transição deve continuar EXATO em debug/release e preserve_order.
  Não regenerar capturas, nem incluir novos goldens derivados do candidato.

## Marco B — plano externo válido com 20.000 itens (.21)

Unidade existente: nodes + layers + splats + fx + resources + lights, dos DOIS decks.
Plano aceito precisa ser completo e estruturalmente válido, não um array de nulls que
passou só no pré-check. Os bytes são teto só da porta textual; Value/typed não têm texto.

Construção viável apontada por leitura (não executada; conferir no código/teste):
partir de plan_at(spec_scene([tr({})]),44), cada deck com 3nodes+2layers+2resources.
Acrescentar 9.993 passes fade válidos em cada deck (19.986 no total), IDs track/clip globais
curtos e únicos, sem seed, params na ordem do catálogo, amount=weight=intensity=1.
Primário pode ter scope Output; vizinho usa scope Shot do próprio shot (sem output no other).
Acrescentar requires corretos por contrato. O orçamento é warning, não corta passes.
Cada pass fica aproximadamente196/209 bytes; com vírgulas a adição é4.067.151 bytes,
deixando ~127KiB para base e timing sob4.194.304. Medir bytes reais no teste.

- Contagem independente por comprimentos literais dos seis arrays de cada deck:
  [3,2,0,9993,2,0] em cada um; 2*(3+2+9993+2)=20000.
  Não usar own_items/transition_items para certificar o próprio contador.
- validate(), from_json e from_json_str aceitam, is_complete=true e roundtrip igual.
  Um pass válido extra =>20001 e recusa /transition nas três portas, texto ainda<4MiB.
- No próprio plano20000, whitespace até exatamente4.194.304 bytes aceita; +1 recusa.
- Provar lights no contador TIPADO: preparar19.999 itens não luminosos; uma luz válida
  leva a20000 e aceita; a segunda leva a20001 e recusa, sem violar limite4/deck.
- Mutantes do teto (> para >=), exclusão do vizinho e exclusão de lights devem morrer.
- Se outra construção mais simples couber, preferi-la com a mesma prova independente.
  Registrar que validade de plano externo não significa que um bundle com MAX_FX_TRACKS64
  consegue gerar tantos passes. Não alterar quotas do documento para fabricar esse alcance.
- Gerar em memória no teste; não commitar JSON gigante nem artefatos >500MB em TEMP.

## Marco C — consumidor de ESTADOS do X representável (.17)

Ler fixtures commitados de PR65, sem executar Python, regenerar dados ou adaptar os bundles.
7cenários,98quadros,390sondas de cor e12PNG permanecem byte-intactos.
O teste novo consome ScenePlan/PlanTimeline reais sob Accept de teste, sem novo Accept em src.

- Comparar papéis de frames[].primary/neighbour em C-1/C, não resumo do cenário removido.
- Na janela, conferir OS DOIS decks e seu timing: raw contra instance_wanted/camera_wanted,
  amostras contra instance_frame/camera_clip_frame, held independente, offsets/IDs/limites,
  Ratio da edição real e dos clocks, incluindo30000/1001 e start/offset.
- Comparar estados existentes de câmera (posição/target/FOV) e marcador/nós/matrizes e
  lâmpadas avaliadas no relógio real com os números analíticos. Se algum campo de referência
  for diagnóstico sem correspondente no plano, explicitar o mapeamento e a limitação;
  não reduzi-lo a comparar o gerador consigo mesmo.
- Cenários representáveis cobrem seu shot. Não alongar instância, pular deck ou reproduzir
  o skip dos lit-held antigos. Esses legados preservam suas adaptações e a divergência .23.
- Nos quadros fora da janela (start-1/end), conferir plano simples/ausência de transition e
  o primário real. A referência traz dois lados, mas não afirmar outro deck serializado fora
  da janela. O avaliador de offset pode prover comparação separada claramente identificada.
- Conferir cold/timeline em ordem crescente, inversa e repetida. Casos para isolamento:
  luz do vizinho no frame errado, instância/câmera sem clamp, held errado, fps nominal30,
  primary errado no corte, comprimentos de câmera medidos só na trilha principal.
- Hold de câmera precisa de estado; pose saturada não discrimina pixel. UNH estático também
  exige estado. Provar esses negativos nos metadados/estados, não colorir uma alegação pixel.
- Nenhum pixel é desenhado/comparado nessa PR; 390sondas, textura, quantização e viés de cor
  continuam para S2-R. Granularidade do viés por quadro/cenário precisa de decisão prévia
  antes do engine, sem calibrar tolerância pelo resultado; não decidir por medição C0b.

## Ownership e tamanho

Permitidos: crates/core/src/scene_plan/{transition,transition_json,transition_validate,build,
timeline,model,mod}.rs; novos módulos pequenos de timing/clocks sob scene_plan/temporal;
crates/core/src/directed_scene/temporal/{shot_offset,frame,mod}.rs só para regra compartilhada;
testes core de transição, baseline e novo consumidor representável de estado; docs próprios.
Extensão rotineira de reexports/tipos necessária: registrar arquivo e motivo antes de alterar.
Sem tocar produção desktop/media/render/runtime, Cargo.lock/deps, Python ou fixtures.

Docs: este plano, p4b-cross-dissolve-plan.md nas cláusulas afetadas, fx-transitions.md,
scene-plan.md e relatório novo p4b-s2-c0b-report.md. Backlog/matriz/índice ficam com coordenador:
entregar PATCH LITERAL das linhas existentes .4/.17/.20/.21/.23 e F064/índice no relatório.
Não criar fila concorrente nem IDs locais; propostas novas vão ao coordenador.

Arquivo acima1000linhas não cresce: extrair módulo com comportamento preservado primeiro,
delta líquido<=0. Módulos novos focados e preferencialmente<800linhas. Não engrossar
transition_validate.rs com parser/fixture, nem juntar tudo num helper genérico de opções.

## Gates e entrega

- Antes/depois: cwd, origin, branch/base/head, status, disco e processos próprios.
  Base c9c5fa0, branch própria feat/p4b-s2-c0b-plan-clocks; preservar trabalho alheio.
- ./scripts/cargo pelo Git Bash REAL (C:\Program Files\Git\bin\bash.exe).
  Sem target-dir/lane/CARGO_TARGET_DIR, clean/gc manual, override ou TEMP>500MB.
- fmt; check/clippy workspace --all-targets --locked -Dwarnings; workspace --no-run;
  build -p tessel-editor; core --no-fail-fast; transições+corpus+X+baseline isolado e
  preserve_order com TESSEL_EXPECT_PRESERVE_ORDER correto; baseline release; lib/desktopFX
  proporcionais. Registrar dois gizmos Windows da BASE como falha, não pular nem relaxar.
- Não repetir workspace global travado/não verde como gate silenciosamente omitido.
  Declarar o que foi compilado vs executado, ignores/skips e fontes por head.
- Mutantes discriminantes de A/B/C, um por vez, restaurar por bytes/hash e atualizar mtime
  antes do controle limpo para não reutilizar binário mutado. Relatar sobreviventes, equivalentes
  e não concluídos. Fazer só testes que comprovem estes invariantes, sem duplicatas triviais.
- Source repo E:\www\tesselapp permanece intocado; origem não executada; nenhum GPU lock.
- Usar skill commit se disponível e convenções; nenhum trailer/referência a assistentes.
  Se harness inserir trailer, sanear só commits locais ainda não publicados via plumbing,
  árvore idêntica, nunca force. Commitar só ownership; não staging amplo do trabalho alheio.
- Push normal, PR contra main. link_pull_request com URL completo como próxima toolcall.
  Filho não faz watch/poll/merge. Relatório: contrato final, SHA dos dados antes/depois,
  gates por head, contagens, mutantes, limites e patches literais dos arquivos comuns.
- Aceite após revisão independente Windows e gates Mac por SHA. C0b interna aceita não
  libera H/R/S2A automaticamente e nunca promove F064 a produto pronto.

## Contrato final de nomes, tipos, API e ponteiros (fixado ANTES do código)

Esta seção só detalha os fatos acima; não os altera. Foi escrita depois de ler `build.rs`, `timeline.rs`,
`transition*.rs`, `shot_offset.rs`, `frame.rs`, os testes de transição e o fixture X (PR65). Os nomes abaixo são os
que o código usa; uma mudança posterior é registrada no relatório com o motivo.

### 1. Regra única de clocks (`directed_scene/temporal/shot_offset.rs`)

`hold_to` deixa de ser privado e vira a regra pública; o avaliador de offset a usa no lugar da cópia atual.

```rust
pub struct ClockSource { pub start_frame: u64, pub length: u64 }   // onde o shot entra no clipe e o clipe
pub struct ClockSample { pub raw: i64, pub sampled: u64, pub held: bool }
pub struct DeckClocks  { pub instance: ClockSample, pub camera: ClockSample }
impl ClockSample { pub fn of(raw: i64, length: u64) -> Result<Self, TemporalError> }
impl DeckClocks  { pub fn at(edit_frame: i64, shot_start_frame: u64,
                             instance: ClockSource, camera: ClockSource) -> Result<Self, TemporalError> }
```

* `delta = edit_frame − shot_start_frame`; `raw = start_frame + delta`, checados; `sampled = clamp(raw, 0, length − 1)`;
  `held = (length == 0) || sampled != raw`; `length == 0` amostra 0. Cada relógio é preso à própria duração.
* `evaluate_shot_at_offset_with` chama `DeckClocks::at`; `held_instance`/`held_camera` de `OffsetEvaluation` são
  `clocks.instance.held`/`clocks.camera.held`. Montador e validador do plano chamam a mesma função; nenhuma fórmula é copiada.
* Tempo exato: `frame_start_time(rate: Rate, frame: u64) -> Result<Ratio, TemporalError>` em `frame.rs`
  (`Bundle::frame_time` passa a chamá-la). Overflow é erro, nunca zero (não usa `Rate::frame_to_time`).
* Reexports: `directed_scene/temporal/mod.rs` e `directed_scene/mod.rs` (extensão rotineira; motivo: expor estes tipos à lib).

### 2. Tipos do plano (`scene_plan/temporal/`, módulos pequenos)

```rust
pub struct DeckTiming {                       // todos u64 (inteiros estritos não negativos no wire)
    pub shot_start_frame: u64,      pub shot_end_frame_exclusive: u64,
    pub instance_start_frame: u64,  pub instance_duration_frames: u64,
    pub camera_in_frame: u64,       pub camera_clip_duration_frames: u64,
    pub sampled_instance_frame: u64, pub sampled_camera_clip_frame: u64,
}
pub struct TransitionTiming { pub edit_duration_frames: u64, pub from: DeckTiming, pub to: DeckTiming }
```

`PlanTransition` ganha `alignment: Alignment` (enum do core, `directed_scene::fx::Alignment`) e
`timing: TransitionTiming`. API de leitura da lib (sem cópia de fórmula):

* `TransitionTiming::deck(side: DeckSide) -> &DeckTiming` e `TransitionTiming::cut() -> u64` (= `to.shot_start_frame`).
* `DeckTiming::clocks_at(edit_frame: i64) -> Result<DeckClocks, TemporalError>`: a regra do item 1 sobre os dados do lado.
* `PlanTransition::clocks(side: DeckSide) -> Result<DeckClocks, PlanError>`: o quadro real é
  `start_frame + frame_in_window`; raw, amostrado e `held` por relógio, inclusive do primário (que não é serializado).
* A montagem: `TransitionTiming::derive(...)` em `scene_plan/temporal/assemble.rs` a partir do `Bundle`, dos dois `Shot` e do
  comprimento de `CameraTimeline::frames()` (máximo da trilha principal e dos layers) de cada clipe. `build.rs` e `timeline.rs`
  só fornecem esse comprimento (a timeline, do cache); não há fórmula nova neles.

### 3. Wire (somente planos COM transição; `PLAN_VERSION` e planos sem transição não mudam)

`transition` ganha `alignment` (`center`/`start`/`end`) e `timing` obrigatórios; os 15 campos antigos ficam, os campos novos
elevam a chave para 17. Chaves do timing: `editDurationFrames`, `from`, `to`; de cada lado: `shotStartFrame`,
`shotEndFrameExclusive`, `instanceStartFrame`, `instanceDurationFrames`, `cameraInFrame`, `cameraClipDurationFrames`,
`sampledInstanceFrame`, `sampledCameraClipFrame`. Campo ausente, desconhecido, float, bool, string, null ou negativo recusa
no próprio ponteiro (`/transition/alignment`, `/transition/timing`, `/transition/timing/{from|to}/<campo>`). O wire antigo
com transição (sem os dois campos) é recusado em `/transition/alignment`, sem default: era formato interno atrás de Refuse
e da capability ausente no JS, e é uma quebra declarada de um formato experimental.

Módulos: `temporal/json.rs` (escrita e leitura do timing e do alignment), `temporal/validate.rs` (a regra abaixo),
`temporal/assemble.rs` (montagem), `temporal/mod.rs` (tipos e acessores). `transition_json.rs` apenas delega;
`transition_validate.rs` apenas chama `check_transition_timing` no lugar do bloco "cabeça presa implica 0".

### 4. Validação (`ScenePlan::check_transition_timing`, depois das relações já existentes entre decks)

Cada recusa é `PlanError::Invalid` com ponteiro; as três portas (`validate`, `from_json`, `from_json_str`) são a mesma validação.

1. Conversões u64→i64 e somas checadas; falha recusa no campo (`/transition/timing/<lado>/<campo>`).
2. Cada lado: `shotStartFrame < shotEndFrameExclusive <= editDurationFrames` (ponteiro `.../shotEndFrameExclusive`).
3. Corte `C = to.shotStartFrame`: `from.shotEndFrameExclusive == C` (ponteiro `/transition/timing/from/shotEndFrameExclusive`).
4. `primary` esperado: `from` se `f < C`, `to` desde `f = C` (`/transition/primary`); `f` dentro do intervalo do primário.
5. Janela: `transition_window(C, durationFrames, alignment, editDurationFrames)`; duração recusada em `/transition/durationFrames`,
   janela fora da edição ou `startFrame` diferente em `/transition/startFrame`, overflow em `/transition/timing/to/shotStartFrame`.
   `frameInWindow = f − startFrame` já é regra de campos (a janela fixa `startFrame`, logo a relação é transitiva).
6. Mesma instância (`source.sceneInstanceId`) ou mesmo clipe de câmera (`frame.cameraClipId`) nos dois decks não declara
   comprimentos diferentes: `/transition/timing/to/instanceDurationFrames` e `/transition/timing/to/cameraClipDurationFrames`.
7. Integridade do primário: `instanceStartFrame + (end − start) <= instanceDurationFrames`
   (`/transition/timing/<primário>/instanceDurationFrames`). Câmera primária pode passar do comprimento (decisão 5).
8. Relógios: para cada lado `DeckClocks::at`; `sampledInstanceFrame`/`sampledCameraClipFrame` iguais ao amostrado
   (`.../sampledInstanceFrame`, `.../sampledCameraClipFrame`).
9. Legado do primário: `frame.instanceFrame`/`cameraClipFrame` são o `raw` (`/frame/instanceFrame`, `/frame/cameraClipFrame`).
   Legado do vizinho: são o `sampled` (`/transition/other/frame/instanceFrame`, `.../cameraClipFrame`), e
   `heldInstance`/`heldCamera` são exatamente `held` de cada relógio, inclusive `false` (`/transition/heldInstance`,
   `/transition/heldCamera`). A regra antiga "cabeça presa implica 0" é removida.
10. Tempos exatos `frame_start_time(fps, n)` (só em planos com transição, para não recusar planos simples antigos): `editTime`
    nos dois decks e `instanceTime`/`cameraClipTime` de cada deck contra o próprio contador legado
    (`/frame/editTime`, `/frame/instanceTime`, `/frame/cameraClipTime` e os de `/transition/other/frame/...`).

### 5. Teto de itens (Marco B) e o consumidor X (Marco C)

Nenhum código de produção muda para o teto: a prova é um teste que gera em memória o plano válido de 20.000 itens e as
variações de +1 (passe, luz) e de bytes. O contador independente usa os comprimentos literais dos seis arrays, sem
`own_items`/`transition_items`. O consumidor X é um teste novo (`pilot_dissolve_representable_state.rs`), sob o `Accept` de
teste, sem tolerância nova e sem pixel; o mapeamento dos campos de referência e as limitações vão para o relatório.

### 6. Arquivos tocados (registro prévio)

Produção: `directed_scene/temporal/{shot_offset,frame,mod}.rs`, `directed_scene/mod.rs` (reexports),
`scene_plan/{mod,transition,transition_json,transition_validate,build,timeline}.rs`, `scene_plan/temporal/*` (novos).
Testes: `tests/scene_fx_transition/{plan,limits,timing,ceiling}.rs`, `tests/scene_fx_transition.rs` (registro de módulos),
`tests/pilot_dissolve_representable_state.rs` (novo). Docs: este plano, `p4b-cross-dissolve-plan.md`, `fx-transitions.md`,
`scene-plan.md` e `p4b-s2-c0b-report.md`. `docs/backlog.md`, `docs/features/README.md` e `docs/README.md` são do coordenador.
Sem `Cargo.lock`, dependências, Python, fixtures, desktop, media, runtime ou renderer.