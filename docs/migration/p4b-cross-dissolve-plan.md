# P4b — Transições entre planos: `crossDissolve` (BL082.4)

2026-11-14. **Plano de implementação. Nenhuma linha de produção foi alterada por este documento**: plano pronto
não é feature implementada. Base lida: `origin/main` `2b54ac1` (PR #57 integrada). Frente W3 da rodada Windows de
14/11/2026; a avaliação do plano e o início da implementação são do coordenador.

P4b é a segunda metade da P4 de [pilot-parity-execution-plan.md](pilot-parity-execution-plan.md): `transitions[]`
no `fx`, **dois decks** no plano, a primeira transição (`crossDissolve`) e a capacidade nova no runtime. A P4a
([p4a-shot-scope-fx-plan.md](p4a-shot-scope-fx-plan.md), [F062](../features/fx-shot-scope.md), PR #48) deixou a
costura: o conceito de **deck** (a imagem de um plano antes da saída), as duas etapas da composição, a capacidade
`fxShotScope` e a regra única `scope_fault`. O backlog continua sendo a fila única: aqui só se **propõem** IDs
(seção 15); o coordenador os cria.

## 0. Como ler este plano

| Seção | Conteúdo |
|---|---|
| 1 | Escopo, fora de escopo e por que esta fatia vem agora |
| 2 | Leituras primárias, o que faltou e o que reconfirmar antes de codar |
| 3 | Regras da origem e a decisão tomada para cada uma |
| 4 | Estado atual do destino: caminhos e símbolos que a P4b toca |
| 5 | Contrato do documento: `fx.transitions[]`, IDs, importação, validação, ida e volta |
| 6 | Tempo: janela, adjacência, duração, FPS racional, bordas, sobreposição, ausência, alvo apagado |
| 7 | Avaliação por quadro e plano de dois decks (pesos, extremos, ordem, grão, cor, alpha) |
| 8 | Capacidade, versão, runtime antigo, orçamento, alvos retidos, resize, perda de contexto, erro, dispose, seek, export |
| 9 | Edição: operações atômicas, dry-run, no-op, undo, revisão, gesto, invalidação, plano apagado/encurtado |
| 10 | Persistência, pacote e intercâmbio |
| 11 | UI real (Kit), automação e contrato `Command`/nó/parser |
| 12 | Aceite por superfície, referência independente e controles negativos |
| 13 | Fases (tracer bullets), dependências exatas, ownership e arquivos |
| 14 | Gates separados, ambiente Windows/Mac, mbx e disco, riscos |
| 15 | IDs propostos, perguntas reais ao usuário e entrega ao coordenador |

Convenções: **L** lib Rust sem janela · **H** processo real `tessel-headless` (JSONL) · **G** eventos GPUI sintéticos
(`gpui_kit::test`) · **W** janela real por `scripts/ui` sem coordenadas · **P** pixels do monitor (WebView) ·
**R** renderer headless (`crates/render`) · **E** export com FFmpeg · **X** referência independente em Python.
Sintético não é ponteiro físico, GPU real nem áudio ouvido. Nomes de símbolos novos são **propostas**;
os existentes foram lidos em 14/11/2026 na `main` `2b54ac1`.

## 1. Escopo

**Entra (BL082.4, versão 1):**

1. Camada `fx.transitions[]` do documento (formato do piloto), com importação, validação tipada, ida e volta e
   `catalogVersion` mantido em 1.
2. Um efeito de transição: `crossDissolve` (parâmetros `curve` e `intensity`), em uma família nova `transition` do
   catálogo (`family: "transition"`), com um novo tipo de parâmetro de escolha (`ParamKind::Choice`).
3. Janela por corte (`alignment`: `center|start|end`), duração em quadros, `handles` (`use|hold|refuse`) e `easing`.
4. Avaliação pura por quadro no core, plano com **dois decks** e capacidade `fxTransition` (14ª).
5. Runtime Three.js que desenha os dois decks em alvos separados e mistura em luz linear, antes dos passes de saída.
6. Edição atômica no core (`scene.fx.edit`, `scene.fx.gesture.*`, `scene.fx.inspect`), persistência, pacote,
   intercâmbio e export.
7. UI no Kit (seção de transições no painel de efeitos), nós semânticos, parser e testes nas três superfícies.

**Fica fora (cada item é BL próprio, nada habilitado por associação):**

- As outras 27 transições do piloto e os demais efeitos: o catálogo passa de 3 para 4 dos 59 efeitos do piloto
  (`vignette`, `fade`, `filmGrain` + `crossDissolve`) e os 55 restantes ficam fora. Nenhum controle de efeito ausente
  é habilitado.
- **`keys` em transições** (chaves de parâmetro com origem no início da janela): recusadas com código tipado
  (decisão D10). O motor de chaves da F058 é de clipe (`FxClip`) e o inspector/gesto de chaves está acoplado a clipes.
- Importação do **dissolve legado** (`shots[].transitionInto.type = "dissolve"`) como `crossDissolve` sintético:
  fatia própria no fim da P4b (S6, decisão D04) porque muda o veredito de um caso do corpus de equivalência.
- Escopo `node` (BL082.40), ripple/roll e edição estrutural de `shots[]` (BL082.5/P5), trilhas de timeline de
  transições (P6), aviso de transição sobre fala (não há narração no editor: BL017/BL026), `UNSUPPORTED_ALPHA`
  (divergência anterior que vale também para os looks: pergunta Q3 da seção 15).
- Motor antigo inteiro, nuvem, serviços e execução da origem. **A origem JS não foi executada, instalada nem alterada**;
  o golden `crossDissolve.png` da origem existe mas **não foi lido como pixels nem comparado**
  (`ORIGIN_GOLDEN_NOT_COMPARABLE` continua valendo; ver seção 2).

**Por que agora e por que não esperar P5/P6/nodeScope.** A transição precisa só de: documento de shots estável,
avaliação por quadro e plano. Todos existem. O escopo `node` (BL082.40) é ortogonal (outro alvo de passe); P5
(BL082.5) edita `shots[]`, e a P4b não depende dele porque **recusa** o candidato que invalidaria uma transição,
exatamente como a P4a faz com clipes `shot` (BL082.44); P6 (trilhas na timeline) só consome o que a P4b entrega.
Nenhum deles bloqueia a P4b por associação, e a P4b não os habilita.

## 2. Leituras primárias, lacunas e o que reconfirmar

| Fonte | Estado em 14/11/2026 |
|---|---|
| `AGENTS.md`, skill `tessel-feature-contract`, `docs/backlog.md`, `docs/features/README.md`, `docs/core-architecture.md`, `docs/README.md` | Lidos na `main` `2b54ac1` |
| `docs/features/effects.md` (F016), `fx-shot-scope.md` (F062), `p4a-shot-scope-fx-plan.md`, `pilot-fx-shots-studio-features.md`, `pilot-parity-execution-plan.md`, `origin-specs/scene-fx-decisions.md` | Lidos |
| Código do destino (seção 4) | Lido nos caminhos citados; **símbolos novos aqui são propostas** |
| Origem JS, só leitura por `git show` em `refs/remotes/origin/t3/scene-engine-pilot` (**ponta `365579076782c1fc19c6464f3bc0199b30c0ab8f`, 2026-11-03**) | Lidos: `fx/fx-windows.ts`, `fx-explain-transitions.ts`, `fx-normalize-transitions.ts`, `fx-frame-decks.ts`, `fx-frame-derived.ts`, `fx-explain-budget.ts`, `fx-command-transitions.ts`, `resolve-fx-frame.ts` (+ spec de transições), `fx-schema.ts`, `catalog/crossDissolve.ts`, `catalog/fx-effect-meta.ts`, `edit/resolve-shot-at-offset.ts`, `camera/easing.ts`, `runtime/fx/fx-compositor.ts`, `runtime/fx/shaders/crossDissolve.ts`, `fx-explain-handles.spec.ts` |
| **Pin do snapshot do piloto `d40ac3f7c0` (citado em `source-feature-inventory.md`)** | **Ausente no clone Windows da origem** (`git cat-file -t d40ac3f7c0` falha). As regras abaixo vêm da **ponta** `3655790`; a relação entre a ponta e o pin **não foi verificada** |
| Golden da origem (`catalog/__golden__/crossDissolve.png`) | Existe na ponta; **não aberto, não comparado**, origem não executada |
| Corpus de bundles (`docs/migration/fixtures/bundles/`) | Lido: `valid-minimal-continuous`, `valid-cut-instance-offsets`, `invalid-dissolve-transition` |

**Leitura primária faltante antes do implementador (passo 0 da S0):** (a) obter o pin `d40ac3f7c0` (ou declarar a
ponta como nova referência) em um clone de leitura e registrar `git rev-parse <ref>:<caminho>` (blob) dos arquivos
acima no manifesto do corpus; (b) reconferir, na referência escolhida, os pontos de maior risco: a fórmula do shader,
`transitionWindow`, `transitionIssues` (códigos e ordem), `shotClipFrame`, `resolveShotAtOffset` (clamp independente
de instância e câmera) e a regra de duplicata com o dissolve legado; (c) o que mudar entre pin e ponta vira nota no
manifesto, **nunca** ajuste silencioso de expectativa. Não se declara paridade com a origem: a prova é "fórmulas
transcritas por Python independente", como em `pilot-fx` (vinheta, `fade`, `filmGrain`).

## 3. Regras da origem e decisão para cada uma

Decisões rotineiras estão justificadas aqui e **não** viram pergunta ao usuário. As poucas escolhas de produto
estão em Q1–Q4 (seção 15).

| ID | Regra na origem (ponta `3655790`) | Decisão no Tessel | Justificativa |
|---|---|---|---|
| D01 | Camada `fx.transitions[]` com `id`, `effect`, `fromShotId`, `toShotId`, `durationFrames`, `alignment`, `handles`, `easing`, opcionais `params`, `keys`, `seed`, `hidden` | **Mesmos nomes de campo.** O rascunho de `effects.md` (~l.450: `type`, `fromShot`, `toShot`) fica **superado**: atualizar F016 na S1a | `fx/mod.rs` já declara o formato como "o do piloto"; documentos do piloto precisam importar sem conversão |
| D02 | Uma transição por corte (`fromShotId\|toShotId`); `setTransition` substitui a do mesmo corte | Idem. `TRANSITION_DUPLICATE` na importação; `set_transition` é *upsert* por corte, preserva o ID | Evita duas janelas no mesmo corte; edição previsível |
| D03 | Janelas por `alignment`: `center` começa em `corte − floor(D/2)`, `start` em `corte`, `end` em `corte − D`; `quadrosAntes = max(0, min(fim,corte) − início)` | Idem, em inteiros, sem segundos | A janela é aritmética de quadros; nada nela depende do FPS (seção 6.1) |
| D04 | `transitionInto: dissolve` legado é lido como `crossDissolve` centrada, `hold`, `easing linear`, `curve linear`, ID `dissolve-<shot>`; duplicata com `fx` é erro | **Fora de S0–S5.** Entra como S6 (BL082.4.7) | Muda o veredito de `invalid-dissolve-transition` no corpus BB10 (o oráculo vem de um pin mais antigo que o piloto); trocar veredito de oráculo na mesma PR que cria o encanamento mistura duas revisões. BL082.4 só fecha com S6 |
| D05 | `handles`: `use` e `refuse` exigem quadros **reais** (instância **e** câmera, contados à parte); só `hold` aceita congelados; `refuse ≡ use` no cálculo (decisões #6) | Idem, os 3 valores aceitos e preservados | Documentos do piloto importam; nenhum dado é migrado depois se `refuse` ganhar sentido |
| D06 | Padrões de transição nova: `alignment center`, `handles hold`, `easing easeInOutCubic`; duração omitida = típica do efeito | Idem; a duração típica (24) é **cortada** ao que os dois planos suportam, **dada** explícita demais é recusada (`TRANSITION_TOO_LONG`) | Mesma política de F062 (`fx::default_duration`: omitida corta, dada recusa) |
| D07 | Fórmula: `p = ease((f − início + 0,5)/D)`; `shaped = mix(p, smoothstep(p), smooth)`; `saída = mix(mix(a,b,p), mix(a,b,shaped), amount)`, `alpha = 1` | Idem. `smooth = (curve == "smooth")`, `amount = clamp01(intensity)`. **`intensity = 0` não é identidade**: mistura os decks usando `progress` já eased, sem a segunda modelagem smoothstep. Com `curve=linear`, intensity não altera a saída | Fidelidade; o validador do plano **não** pode aplicar a regra `amount = intensity × peso` dos looks |
| D08 | Mistura em luz linear, decks com 8 bits codificados em sRGB (a origem decodifica para half-float linear) | Decodificar IEC 61966-2-1 → misturar → codificar, nos alvos de 8 bits do runtime (como `vignette`/`fade`/`filmGrain` em `output-fx.js`) | Mesma convenção do runtime atual; a diferença de arredondamento é declarada e medida (D13) |
| D09 | Por quadro: ordem **passes do plano A/B → mistura → passes de saída** | Idem | Ordem de F062 + um estágio |
| D10 | `keys` em transições: recortadas para o intervalo da janela | **Recusadas**: `FX_TRANSITION_KEYS_NOT_YET` (código tipado, sem ignorar) | Chaves de F058 são de clipe; reaproveitar tocaria inspector e gesto de chaves. BL082.4.9 |
| D11 | Transição `hidden` não desenha | Corte seco naquele corte; sem `transition` no plano | Idem origem |
| D12 | Aviso `TRANSITION_OVER_SPEECH`, `TRANSITION_ON_CONTINUOUS`, `TRANSITION_SPLAT_COST`, `FX_PASS_BUDGET` | Portar `ON_CONTINUOUS`, `SPLAT_COST` e o orçamento como **avisos** (nunca recusa). `OVER_SPEECH` é N/A (sem narração) e fica registrado | Aviso não bloqueia; o editor não tem narração ainda |
| D13 | Teste de pixel da origem usa o golden dela | Referência **independente em Python** + tolerância declarada **antes** de rodar; `ORIGIN_GOLDEN_NOT_COMPARABLE` mantido | Regra da rodada: não adulterar tolerância/oráculo |
| D14 | `hold` prende o quadro de cada lado **à instância e à câmera, separados** (`heldInstance`, `heldCamera`) | Idem: o deck vizinho usa `instância = clamp(inicioInst + deslocamento, 0, durInst − 1)` e `câmera = clamp(camIn + deslocamento, 0, quadrosCam − 1)`, mais as duas bandeiras | Um plano pode ter cena e não ter câmera (caso real do corpus, seção 12) |
| D15 | Clipe `shot:X` que encosta na borda do plano segura o quadro mais próximo nos quadros de handle; clipe no meio não entra | Idem (`shotClipFrame`) | Efeito não some no corte |
| D16 | (sem equivalente: a origem desenha overlays sobre o deck do topo) | Durante a janela, gizmo de nó e alças de efeito são **recusados com motivo tipado** `transition_window`; edição por propriedades segue permitida | O plano do topo é um de dois decks: uma alça sobre ele não corresponde ao que se vê. Reversível (Q1) |

## 4. Estado atual do destino (`main` `2b54ac1`) e o que a P4b toca

| Caminho | Símbolo (lido) | Estado hoje | P4b |
|---|---|---|---|
| `crates/core/src/directed_scene/fx/mod.rs` | `Fx { tracks }`, `FxClip`, `FxPass`, `Fx::resolve`, `pass_budget`, `FX_CATALOG_VERSION = 1`, `FX_PASS_BUDGET = 8`, `FX_TRANSITION_NOT_YET` | Só `tracks`; `transitions` não vira dado | `transitions: Vec<FxTransition>`; remover `FX_TRANSITION_NOT_YET` quando nada mais o produzir |
| `.../fx/read.rs` | `read_fx` (~l.672–718) | Recusa `transitions` não vazio (l.700–712) | `read_transitions`, códigos `TRANSITION_*`, `FX_TRANSITION_KEYS_NOT_YET` |
| `.../fx/catalog.rs` | `EffectSpec`, `ParamKind` (sem escolha), `CATALOG: [&EffectSpec; 3]`, `family: "look"` | 3 looks | `crossDissolve` (família `transition`), `ParamKind::Choice`, `ParamValue::Choice` |
| `.../fx/scope.rs`, `handles.rs`, `control.rs` | `FxScope`, `scope_fault`, handles da vinheta | P4a | Sem mudança; transições não têm alça de monitor |
| `crates/core/src/directed_scene/parse.rs` | `transition()` (~l.669: `DISSOLVE_UNSUPPORTED`), `joins.rs` (`MISSING_TRANSITION`, `CUT_WITHOUT_REASON`) | Dissolve legado inválido | Só S6; o corte continua exigindo `transitionInto: cut` com motivo |
| `crates/core/src/temporal/frame.rs` | `resolve_frame_mapping`, `mapping_inside_instance` (estrito), `evaluate_frame_with` | Só quadro dentro do plano | Avaliador novo `evaluate_shot_at_offset` (clamp de D14) em módulo próprio; o estrito não muda |
| `crates/core/src/scene_plan/{model,build,timeline,validate,json,mod}.rs` | `ScenePlan` (um deck), `Capability::ALL` (13), `PlanTimeline::at_frame`, `check_fx` (l.459), `derive_requires` | Um deck; passe `shot` só do plano do quadro | `transition` aditivo, `Capability::FxTransition`, `validate_deck`, união de recursos |
| `crates/core/src/scene_edit/{fx,fx_inspector,fx_choice,fx_intent,fx_animation}.rs` | `parse_fx_batch`, ops de clipe, inspector, escolha nomeada, `fx.rs:593` cria `fx` com `transitions: []`, `fx.rs:753` poda `fx` vazio | Sem ops de transição | Ops `set_transition`…; a poda do `fx` vazio já testa `transitions == []` |
| `crates/core/src/scene_project/{fx,fx_gesture,query,command,fx_seek,run}.rs` | `scene.fx.*`; `fx.rs:58` anuncia `refused: {transitions}` | Leituras sem transições | Leituras e gesto com `transition`; tirar `transitions` de `refused` |
| `crates/core/src/ops.rs` | parser `scene.fx.edit` (l.666, `dry_run`, `base_revision`), `scene.fx.gesture.begin` (`clip` obrigatório), `scene.fx.inspect` | | `gesture.begin {transition}` (exclusivo com `clip`), `inspect {transition\|cut}` |
| `crates/core/src/state/tree.rs` | nós `scene.fx` | | Nós de transição e de corte (seção 11) |
| `crates/core/src/scene_export/snapshot.rs` | snapshot congelado do export | | Inclui `fx.transitions`; teste de que mudança durante o export não vaza |
| `crates/render/src/{scene,scene_renderer,scene_host}.rs`, `crates/render/tests/scene_fx*.rs` | renderer headless, 54 sondas por quadro contra a referência do core | | `scene_fx_transition.rs` |
| `runtime/src/directed-scene.js`, `compositor.js`, `output-fx.js`, `assets.js` | `SCENE_CAPABILITIES`, `parseDirectedLayer`, `prepareDirectedScene` (~l.877–964), `sceneTarget`, `fxPipeline`, `dropFxPipeline`, `splatLease`, `SRGB_CURVES` | Um deck por camada | `parseTransition`, shader, alvos `primary`/`other`, ciclo de vida |
| `runtime/test/fixtures/fx-catalog.json`, `output-fx.test.mjs`, `directed-scene-fx-browser.test.mjs` | catálogo espelhado e testes Chrome | | Entradas do catálogo; testes novos |
| `crates/desktop/src/workstation/scene_fx*.rs`, `automation/commands.rs` | seção Efeitos, nós `scene.fx.clip:<id>`, `scene.fx.budget`, `fx_run_guarded` (`stale_revision`) | | Seção Transições, `scene.fx.transition:<id>`, `scene.fx.cut:<de>\|<para>` |
| `scripts/pilot-fx-corpus.py`, `pilot-fade-corpus.py`, `pilot-grain-corpus.py`, `scene-fx-shot-scope-proof.py`, `scene-fx-ui-proof.py`, `docs/migration/fixtures/pilot-fx/`, `docs/migration/p4a-shot-scope/mutate.py` | corpora independentes, provas da janela, corredor de mutantes | | `pilot-dissolve-corpus.py`, `scene-fx-transition-proof.py`, `p4b-cross-dissolve/mutate.py` |

Lacunas confirmadas no código atual, que a P4b fecha: o importador recusa `transitions`; não há tipo de parâmetro
de escolha; não há avaliador de plano fora do seu intervalo (o mapeamento é estrito); o `ScenePlan` carrega **um**
deck; o runtime desenha **uma** cena por camada dirigida; `check_fx` amarra os passes de plano ao `frame.shot_id`.

## 5. Contrato do documento

### 5.1 Formato

```json
{ "fx": {
    "catalogVersion": 1,
    "tracks": [],
    "transitions": [{
      "id": "tr-1",
      "effect": "crossDissolve",
      "fromShotId": "shot-a",
      "toShotId": "shot-b",
      "durationFrames": 12,
      "alignment": "center",
      "handles": "hold",
      "easing": "easeInOutCubic",
      "params": { "curve": "smooth", "intensity": 1 },
      "seed": 0,
      "hidden": false
    }]
} }
```

- `id`: mesma regra de ID do restante do `fx` (regex e reservados, case-insensitive: BL082.9), único **entre
  trilhas, clipes e transições**.
- `effect`: precisa ser do catálogo **e** de família `transition`; um look em `transitions[]` (ou o inverso) é
  `UNKNOWN_EFFECT` com dica da família.
- `durationFrames` inteiro ≥ 1; `alignment` ∈ {`center`,`start`,`end`}; `handles` ∈ {`use`,`hold`,`refuse`};
  `easing` ∈ nomes de easing já aceitos para a câmera (a mesma tabela do core; o código do erro segue o da origem,
  `UNKNOWN_KEYFRAME_EASING`, **a confirmar na S1a** contra o nome que o core já usa);
  `params` estático (sem chaves), validado pelo catálogo: `curve` ∈ {`smooth`,`linear`} (padrão `smooth`),
  `intensity` ∈ [0,1] (padrão 1); parâmetro desconhecido ou fora da faixa é recusado com o caminho do JSON.
- `seed` (uint32) é aceito e **preservado** (o `crossDissolve` não sorteia ruído, então não entra no plano:
  `Reseed::Never`); `hidden` booleano; `keys` presente é recusado (D10).
- Chaves desconhecidas dentro de uma transição: **preservadas** na ida e volta apenas onde a política de chaves
  desconhecidas do documento já as preserva (F031); nunca interpretadas.

### 5.2 Importação e validação tipada

Todo o contrato vive no core (`directed_scene::fx`), sem GPUI. Ordem das conferências por transição (para o primeiro
erro ser previsível): efeito/parâmetros → easing/enum → existência dos shots (`UNKNOWN_SHOT`, um por shot ausente) →
adjacência (`TRANSITION_NOT_AT_CUT`) → duplicata por corte (`TRANSITION_DUPLICATE`) → duração (`TRANSITION_OUT_OF_RANGE`)
→ janela (`TRANSITION_OUT_OF_RANGE`: início < 0 ou fim > duração da edição) → comprimento
(`TRANSITION_TOO_LONG`: `|from| < quadrosAntes` ou `|to| < quadrosDepois`) → handles (`TRANSITION_NO_HANDLES`, que diz
**qual** falta: instância, câmera ou as duas, quantos quadros e o remédio) → sobreposição entre janelas
(`TRANSITION_OVERLAP`, ordenadas por início, `depois.início < antes.fim`). Códigos e mensagens seguem a origem (leitura
`fx-explain-transitions.ts`) com texto próprio em pt-br/en/es no inspector; **nenhum** código é inventado depois de ver
um resultado.

Regras adicionais do Tessel: (1) `FX_LIMIT` para o número de transições (no máximo `shots − 1`, verificado antes do
custo); (2) o importador é **atômico**: qualquer erro recusa o documento inteiro, e o candidato de uma edição é
validado pelo mesmo importador (`scene_edit/fx.rs` já refaz a leitura do candidato); (3) `catalogVersion` **continua 1**:
adicionar um efeito e uma família não muda o significado de nenhum documento existente; um binário antigo recusa o
documento porque `transitions` não vazio é hoje `FX_TRANSITION_NOT_YET` (comportamento do código de `read.rs:700–712`;
a S1a confere que há teste e o mantém como controle negativo do binário anterior) e, no plano, pela capacidade (seção 8).

Avisos (nunca recusa, aparecem em `scene.fx.list`, `scene.fx.frame` e no painel): `TRANSITION_ON_CONTINUOUS`
(a junta é `continuous`), `TRANSITION_SPLAT_COST` (os dois planos têm splat; a origem mediu ≈2× o custo de um deck
em um spike próprio, **número da origem, não do Tessel**) e `FX_PASS_BUDGET` (seção 8.2).

### 5.3 Ida e volta

`fx` é o JSON do documento; `scene_edit/fx.rs` edita o `serde_json::Value` e o importador relê. A ordem de `transitions[]`
é a do documento; `set_transition` anexa no fim; remover preserva a ordem das demais. Teste obrigatório: documento
válido → importar → serializar → bytes iguais ao canônico, e idempotência de importar a saída; cópia-edição-desfazer
devolve **bytes** iguais (o undo guarda o documento, não um diff).

## 6. Tempo

### 6.1 Janela

Seja `C = to.editRange.startFrame` (o quadro do corte), `D = durationFrames`:

| `alignment` | início | fim (exclusivo) | quadros antes do corte | quadros depois |
|---|---|---|---|---|
| `center` | `C − ⌊D/2⌋` | início + D | `max(0, min(fim,C) − início)` | `D − antes` |
| `start` | `C` | `C + D` | 0 | `D` |
| `end` | `C − D` | `C` | `D` | 0 |

Exemplos fixos (C = 48): `center`, D=12 → [42,54) (6 + 6); `center`, D=5 → [46,51) (2 + 3); `center`, D=1 → [48,49)
(0 + 1); `start`, D=12 → [48,60); `end`, D=12 → [36,48).

- **Quem desenha o quê.** Nos quadros `[início, C)` o plano do topo é o `from` (o plano sob a agulha) e o deck vizinho é o
  `to`, em **deslocamento negativo** (cabeça); em `[C, fim)` o do topo é o `to` e o vizinho é o `from`, **depois do seu
  fim** (cauda). Daí `quadrosAntes` precisa caber como cabeça do `to` e `quadrosDepois` como cauda do `from`
  (validação `NO_HANDLES` para `use|refuse`) e, nos dois, **dentro do comprimento do plano que os contém**
  (`TOO_LONG`).
- **Adjacência.** `to` é o plano que **segue** `from` na ordem da edição. A junta continua exigindo `transitionInto`
  `cut` com motivo (regra existente de `joins.rs`); uma transição **veste** um corte, não o substitui. Em junta
  `continuous` o aviso `ON_CONTINUOUS` aparece.
- **Sobreposição.** Janelas de cortes vizinhos não compartilham quadro. A janela de um corte nunca cruza **outro** corte
  porque as partes dentro de cada plano cabem no plano (`TOO_LONG`).
- **Bordas.** Janela que começa no quadro 0 ou termina em `editDuration` é válida; início negativo ou fim maior que a
  edição é `OUT_OF_RANGE`. D=1 é válida (um quadro de mistura, `p = ease(0,5)`).
- **Ausência.** Sem transição (ou `hidden`), o quadro é o corte seco de hoje; o plano **não ganha** a chave
  `transition` e seus bytes são os de hoje.
- **Alvo apagado.** Documento com `fromShotId`/`toShotId` inexistente: `UNKNOWN_SHOT`, documento recusado (nada de
  transição órfã silenciosa). Sobre **editar** shots, ver 9.5.
- **FPS racional.** A janela é em quadros, sem segundos; 24/1, 25/1, 30000/1001 e 60/1 dão a mesma janela e o mesmo
  progresso para os mesmos números de quadro. O tempo de instância do deck vizinho (luz com cintilação, relógio da
  cena) usa o FPS racional existente a partir do quadro de instância já preso: o teste NTSC fixa isso (seção 12).

### 6.2 Progresso (vetores a mão)

`p = ease(easing, (f − início + 0,5)/D)`: o progresso é amostrado no **meio do quadro** (`+ 0,5`). Vetores para
D = 12 (a janela de qualquer corte; só `f − início` importa), calculados à mão com as fórmulas acima; a referência
Python da S0 os regenera e o manifesto os fixa:

| `f − início` | `t` sem easing | `progress` após `easeInOutCubic` | `weight` após smoothstep (`intensity=1`) |
|---|---|---|---|
| 0 | 0,041667 | 0,000289 | 0,000000 |
| 1 | 0,125000 | 0,007813 | 0,000182 |
| 3 | 0,291667 | 0,099248 | 0,027595 |
| 5 | 0,458333 | 0,385127 | 0,330723 |
| 6 | 0,541667 | 0,614873 | 0,669277 |
| 11 | 0,958333 | 0,999711 | 1,000000 |

Simetria `p(k) + p(11−k) = 1` vale nas três colunas. Vetor de cor: canal 200 (A) e 40 (B), `f − início = 5`, padrão:
decodificados 0,577580 e 0,021219, peso 0,330723, mistura linear 0,393579, codificado **168,39 → 168**. Consequência
documentada: com os padrões do piloto (`easeInOutCubic` + `smooth` somados) os três primeiros e os três últimos
quadros de uma janela de 12 são quase um deck puro; é fiel ao piloto e **não** é bug.


Para esse mesmo quadro (`curve=smooth`, easing cúbico), `intensity=0` dá canal162,41→162 e
`intensity=0,5` dá165,43→165; `intensity=1` dá168,39→168. São vetores documentais da fórmula,
não saída do motor executado. `curve=linear` torna os pesos iguais em qualquer intensity e
não discrimina o mutante que ignora amount; T07/T29 devem usar também smooth.

## 7. Avaliação por quadro e plano

### 7.1 Core: `Fx::resolve` com transições

Proposta de API pura (sem I/O, sem relógio), sucessora de `Fx::resolve(frame)`:

```rust
// Esboço: nomes e lifetimes definitivos na S1b.
pub struct FxFrame {
    pub primary_passes: Vec<FxPass>,   // clipes `shot` do plano do topo, depois nada de saída
    pub transition: Option<FxTransitionFrame>,
    pub output_passes: Vec<FxPass>,    // clipes `output`, depois da mistura
}
pub struct FxTransitionFrame {
    pub transition: &FxTransition, pub primary: DeckSide /* From | To */,
    pub start_frame: i64, pub frame_in_window: u32, pub progress: f64, pub amount: f64,
    pub other_shot: ShotId, pub other_passes: Vec<FxPass>, pub held_instance: bool, pub held_camera: bool,
}
```

`Fx::resolve` atual **continua** (chamado por quem ainda não conhece transições) e é definido como
`resolve_frame(..).primary_passes ++ output_passes` quando não há transição. Regras:

1. **Pesos.** Passe de clipe segue o peso de F058/F059 inalterado. Passe do plano vizinho usa o **quadro de borda**
   (D15): `deckEditFrame = clamp(f, plano.início, plano.fim − 1)`, e o clipe só entra se `shotClipFrame` o aceita.
   O peso é o do quadro segurado, não zero nem interpolado.
2. **Extremos.** `frame_in_window = 0` e `D − 1` são quadros normais (p ≈ pequeno, ≈ 1), nunca "puro A/B por
   exceção".
3. **Ordem.** Passes do plano do topo → passes do plano vizinho (em seu alvo) → mistura → passes de saída. Dentro de
   cada lista, ordem das trilhas (F062).
4. **Grão (`filmGrain`) e `seed`.** O ruído é um passe de **saída**: age sobre a imagem já misturada, com a semente que
   o core resolve para o **quadro de edição** (`Reseed`), idêntica à do mesmo quadro sem transição. Um `filmGrain` com
   `scope shot` age só no seu deck (antes da mistura), com a semente do quadro real. Teste: grão de saída sobre a janela
   = mesma semente por quadro com e sem transição.
5. **Pureza.** Dado o documento e o número do quadro, o resultado é o mesmo em qualquer ordem de chamada (seek
   aleatório, sub-intervalo de export, jogo repetido). Nenhum estado acumulado entre quadros.

### 7.2 Avaliador de plano fora do intervalo

Novo `evaluate_shot_at_offset(shot, edit_frame)` em módulo próprio do core (`crates/core/src/temporal/` ou
`scene_plan/`; ver seção 13), **sem** alterar `mapping_inside_instance` (estrito). Devolve o mesmo
`FrameEvaluation` do avaliador atual, com `instance_frame` e `camera_clip_frame` presos como em D14 e as bandeiras
`held_instance`/`held_camera` (verdadeiras só quando o prendimento ocorreu). O `edit_frame` do vizinho é o **quadro
real** da edição (relógio de luz/clima/cintilação); só instância e câmera são presos. Teste obrigatório com o caso do
corpus `valid-cut-instance-offsets` (instância `inst-a` 60 quadros, plano `shot-a` em [0,40) com `instanceStart 10`:
cauda de instância = 10; câmera `cam-a` com 40 quadros e `cameraIn 5`: cauda de câmera = 0; o corte está em C = 40).
`center` D=12 com `handles: use` exige 6 quadros depois do corte em `shot-a`: do lado de `shot-a` a **câmera** falta (6) e a
instância não (10 ≥ 6), e `NO_HANDLES` diz "camera clip" e o remédio para esse lado; `shot-b` começa no quadro 0 da
instância e da câmera, então também falta cabeça nos dois (6), e o erro lista os dois lados. Com `hold`, o vizinho usa os
quadros reais que existem e prende só o que falta, instância e câmera **separadas**. Um caso de "só a câmera falta"
exige uma variante do bundle (instância mais longa), registrada na S0.

### 7.3 Plano de dois decks

Princípio: o **topo** do `ScenePlan` continua sendo o deck do plano sob a agulha (consumidores atuais, gizmos, lista de
recursos e `scene.fx.frame` não mudam); o vizinho é aditivo. Chave aditiva `transition`, **ausente** quando não há
transição (digests de planos sem transição ficam idênticos; teste de ouro sobre os planos existentes do corpus):

```json
"transition": {
  "id": "tr-1", "effect": "crossDissolve",
  "fromShotId": "shot-a", "toShotId": "shot-b", "primary": "from",
  "startFrame": 42, "durationFrames": 12, "frameInWindow": 5,
  "easing": "easeInOutCubic", "progress": 0.385127..., "amount": 1.0,
  "params": [["curve","smooth"],["intensity",1.0]],
  "heldInstance": false, "heldCamera": true,
  "other": { /* ScenePlan do deck vizinho */ }
}
```

- `other` é um `ScenePlan` (mesmo parser e serializador; **sem** segundo modelo de plano), validado por
  `validate_deck`: `version`, `output` e `fps` iguais aos do topo; `other.frame.shot_id` = o vizinho;
  `other.frame.edit_frame` = o do topo (quadro real); `other.transition` ausente (sem aninhamento); `other.fx` só com
  passes `shot` do vizinho; `other.resources` vazio (recursos referenciados resolvem na tabela do topo).
- `requires` do topo = união ordenada de `requires` do topo, de `other.requires` e de `fxTransition`
  (`derive_requires` estendido; um runtime que recusa capacidade desconhecida vê tudo que precisa).
- **Recursos** do topo = união determinística (primeiro os do topo, depois os novos de `other`, na ordem do plano),
  sem duplicata; o `ResourceId` do `other` resolve na tabela do topo (validador confere).
- `progress` é o valor **já com easing**; o runtime não calcula easing nem decide regra. O validador recomputa
  `ease(easing, (frameInWindow+0,5)/durationFrames)` e confere com tolerância 1e-9 (`/transition/progress`), confere
  `frameInWindow = frame.edit_frame − startFrame ∈ [0, durationFrames)`, `amount = clamp01(intensity)` (**não** a regra
  de `expected_amount` dos looks), `params` na ordem e nos tipos do catálogo (`parse_param` com `Choice`) e
  `primary` coerente com o lado do corte (`from` em `< C`, `to` em `≥ C`).
- **Estágios** (`check_fx` estendido): lista do topo = passes `shot` do topo, depois `output`; passes `shot` do
  topo nomeiam o `frame.shot_id`; passes de `other.fx` nomeiam `other.frame.shot_id`. Um clipe aparece **uma vez por
  quadro** entre os dois decks.
- **Alpha.** A saída da mistura é **opaca** (`alpha = 1`, como o shader da origem e o resto do renderer). Não se
  introduz `UNSUPPORTED_ALPHA` retroativo (Q3).
- **Cor.** Alvos de 8 bits codificados em sRGB; decodificar → misturar → codificar (D08). Perfis `rec709-video` já são
  recusados pelo documento.
- **Referência independente (X).** `scripts/pilot-dissolve-corpus.py` (stdlib) transcreve da **fonte TS** (não do Rust):
  janela, progresso, escolha dos quadros dos decks (D14/D15) e a mistura por pixel; gera `dissolve-cases.json` e
  `dissolve-manifest.json` com os hashes de blob da origem lida (seção 2) e a proveniência de cada expectativa.
  O teste Rust `crates/core/tests/pilot_dissolve_corpus.rs` **só lê o JSON** (roda no Windows sem Python), como
  `pilot_fx_corpus.rs`.

## 8. Capacidade, versão, runtime, ciclo de vida e export

### 8.1 Capacidade e versão

- `Capability::FxTransition` (`"fxTransition"`, 14ª em `Capability::ALL`), exigida **só** por plano com `transition`.
  Nome escolhido por coerência com `fxShotScope`/`fxNoise`; o `transitionBlend` do rascunho de F016 fica superado.
- `PLAN_VERSION` **continua 1** (chave aditiva, ausente quando não usada), `FX_CATALOG_VERSION` **continua 1**.
- `SCENE_CAPABILITIES` em `directed-scene.js` ganha `fxTransition`; o teste Rust que confere o espelho com
  `Capability::parse` já existe e passa a cobri-la.
- **Runtime antigo falha fechado:** plano com `fxTransition` em runtime sem ela retorna `unsupported` com
  `missing: ["fxTransition"]` e **não desenha**. Teste obrigatório (runtime com a capacidade removida da lista).
  Plano sem transição segue idêntico em runtime novo e antigo.

### 8.2 Orçamento

Mesma política de F016 (aviso, nunca recusa). Um quadro de transição custa: passes de saída + passes de **cada deck**
(os dois) + a **mistura** (1 passe, `internalPasses` do efeito). O varredor de `pass_budget` hoje trata clipes por
intervalo; passa a considerar, por quadro de janela, (a) o `+1` da mistura em `[início,fim)`, (b) os clipes `shot` do
vizinho que seguram a borda (D15) e (c) o vizinho inteiro durante a janela. Também: o limite de recursos por quadro
(splats, texturas) **soma** os dois decks e recusa com o erro tipado já existente ao exceder. Custo/FPS **não são
medidos** aqui (BL082.4.8, sob lock da GPU, rodada própria).

### 8.3 Runtime (`runtime/src/*`)

- `parseDirectedLayer` lê `plan.transition` por `parseTransition`: valida chaves/tipos, `effect ∈ TRANSITION_EFFECTS`
  (lista espelhada pelo fixture do catálogo), `primary`, `progress`/`amount` finitos em [0,1], `other` pelo mesmo
  parser de deck e recusa o resto com `invalid` e o caminho (como já faz com passe de plano).
- Desenho: deck do topo → alvo `primary`; deck vizinho → alvo `other` (ambos via `sceneTarget`, chaves
  `ds:<ordem>:primary|other`); mistura com `SRGB_CURVES` → alvo da cadeia de passes (ou direto no alvo da camada
  quando não há passe de saída) → passes de saída existentes. A ordem em `prepareDirectedScene` (~l.877–964) deve ser
  reconciliada na S2; o desenho não depende de nenhum estado do quadro anterior.
- **Alvos retidos (`keep`).** O alvo do vizinho conta no orçamento de retenção configurável (F-viewport) como os demais;
  a chave de cache é **por deck** (digest do plano do próprio deck + tamanho + amostras): trocar uma propriedade do
  plano **vizinho** com a agulha na janela **redesenha** (teste de invalidação do cache).
- **Resize.** Alvos recriados no novo tamanho; o quadro após o resize é o de um render novo naquele tamanho.
- **Perda de contexto.** Sem histórico temporal, o próximo quadro redesenha os dois decks do zero; o quadro após a
  recuperação é igual ao de antes da perda (hash).
- **Erro de um deck** (recurso ausente, `splatLease` não pronto, textura inválida): erro tipado da camada com
  `deck: "primary"|"other"`, **nunca meia mistura**; o caminho de erro existente do host mostra a causa.
- **Dispose/descarregar.** Sair da camada, trocar o plano ou `dropFxPipeline` libera alvos dos dois decks e as locações
  de recurso do vizinho; teste de contagem de alvos/locações antes e depois.
- **Seek e export.** Plano puro por quadro: seek aleatório e sub-intervalo do export reproduzem o quadro da sequência
  completa. Export de um sub-intervalo que **começa no meio** da janela (ex.: [44,52) com janela [42,54)) deve dar
  quadros idênticos aos mesmos números do export completo (comparação em quadros brutos antes do codec; o H.264 só é
  comparado contra a referência com tolerância declarada).

## 9. Edição

### 9.1 Operações (core, `scene.fx.edit`)

Lote atômico, uma entrada de undo, `dry_run`, `base_revision`: **as mesmas regras de F016/F062**. Novas operações no
`parse_fx_batch` (forma no `ops.rs`, regra no core):

| Op | Campos | Efeito |
|---|---|---|
| `set_transition` | `fromShotId`, `toShotId`, `effect`, `durationFrames?`, `alignment?`, `handles?`, `easing?`, `params?`, `id?`, `hidden?` | *Upsert* por corte; nova usa D06; existente preserva `id` e o que a op omite |
| `resize_transition` | `id`, `durationFrames`, `alignment?` | Muda janela; recusa pelas regras de 5.2 |
| `set_transition_param` | `id`, `param`, `value` | `curve` (escolha) ou `intensity` (número, passo e faixa do catálogo) |
| `set_transition_hidden` | `id`, `hidden` | |
| `remove_transition` | `id` | `UNKNOWN_TRANSITION` se não existir |

- **Dry-run:** devolve o relatório (`modified`, avisos, `refused[]` tipado) **sem** tocar o documento, a revisão nem o
  histórico. **No-op** (valores iguais, ou `set_transition` repetido idêntico): `modified = false`, **sem** entrada de
  undo, sem incremento de revisão (a mesma regra líquida de `EditSummary::modified`).
- **Atomicidade:** uma operação inválida do lote recusa **o lote inteiro**; o documento fica byte a byte igual.
- **Padrões** (D06): `alignment center`, `handles hold`, `easing easeInOutCubic`, `curve smooth`, `intensity 1`, duração
  = a típica (24) cortada a `maxTransitionFrames(alignment, |from|, |to|)`; se esse máximo é 0, `TRANSITION_TOO_LONG`.
- **Revisão:** `base_revision` obsoleta → `stale_revision` (o guarda que `fx_run_guarded` já usa na janela).
- **Troca de efeito** (quando houver mais de um efeito de transição): o que deixa de valer é descartado **com nota**
  (nunca em silêncio), como na origem. Com um só efeito (`crossDissolve`) a regra não é testável: **N/A justificado**,
  registrado para o segundo efeito.

### 9.2 Gesto e candidato

`scene.fx.gesture.begin {transition}` (exclusivo com `clip`), `update {ops}` só com ops da própria transição,
`commit` = **uma** entrada de undo equivalente (bit a bit) ao mesmo `scene.fx.edit`, `cancel` restaura. O candidato do
gesto é o documento com as ops aplicadas e **validado pelo importador**; `scene.fx.list|frame|inspect {gesture}` leem o
candidato (o monitor mostra a prévia, como nos clipes). **Invalidação do gesto** quando: a revisão do documento muda
por fora; a transição some ou é substituída; um dos dois shots deixa de existir ou de ser vizinho. Cancelar um gesto
já invalidado é válido e devolve estado limpo; `commit` de gesto invalidado é recusado com o código existente de
gesto obsoleto.

### 9.3 Inspector

`scene_edit::fx_inspector` ganha o modelo de campos de transição (`TransitionInspector`): `origem` e `destino`
(somente leitura, derivados do corte), `durationFrames` (passo 1, mín 1, máx `maxTransitionFrames`), `alignment`,
`handles`, `easing`, `curve` (escolha nomeada de `fx_choice.rs`), `intensity`, `hidden`, `at_default` e **reset por
campo**; rótulos pt-br/en/es no core, com teste sobre os IDs oferecidos (regra de F057). `scene.fx.inspect
{transition|cut, probe}` valida um valor candidato sem editar (é o "dry-run por campo" que a UI usa).

### 9.4 Lista de problemas (UI e CLI)

`scene.fx.inspect {cut, probe}` e `scene.fx.edit {dry_run}` devolvem `errors[]` tipados `{code, message, hint, transitionId?,
shotId?, details}` (a **mesma** estrutura do importador), em uma só função do core; a UI apenas mostra. Não há texto de erro
montado no desktop.

### 9.5 Shot apagado ou encurtado

Não existe hoje comando de editar `shots[]` (P5/BL082.5). A P4b entrega o **contrato**: (a) uma mudança de documento
que apague um shot citado ou torne uma janela inválida (shot encurtado, não vizinho, handles insuficientes) é **recusada
pelo importador do candidato** com o código da regra violada, **sem cortar a transição**, como BL082.44 faz com clipes `shot`;
(b) quando a P5 existir, ela reavalia as transições (e os clipes `shot`) do lote **antes** de aplicar e propaga a recusa;
(c) teste da P4b: substituir o documento da cena por outro sem o shot (`scene.import`/anexo) é recusado e nada muda.
Isto fica como dependência de P5, não como trabalho aqui.

## 10. Persistência, pacote e intercâmbio

- **Salvar/reabrir:** `fx.transitions` está no JSON da cena do projeto, junto do `fx`; salvar → fechar → reabrir →
  `scene.fx.list` idêntico; undo/redo depois de reabrir funciona sobre o documento reaberto.
- **Pacote** (Directory/Archive) e **intercâmbio** (`project.import_scenes`): o documento da cena viaja inteiro; teste:
  pacote → abrir sem a origem → mesmos bytes de `fx` e mesmo quadro de janela (pixels no renderer).
- **Compatibilidade:** documento sem `transitions` (ou `[]`) não muda; a poda de `fx` vazio de `scene_edit/fx.rs` continua.
  Binário antigo + documento com transição: `FX_TRANSITION_NOT_YET` (falha fechada).
- **Export:** o `snapshot.rs` congela o documento no pedido; uma edição durante o export **não** vaza; o export desenha
  as janelas pelo mesmo plano (seção 8.3).
- **Relink/recursos:** nenhum recurso novo; a tabela de recursos do plano é a união de 7.3. O gerenciamento de
  dependências (BB06) já cobre os recursos dos dois planos porque ambos já são planos da cena.

## 11. UI real e automação

### 11.1 Seção "Transições" (Kit, no painel de efeitos)

Componentes do Kit (`Select`, `Slider`, `Switch`, `Button`, `Accordion`, ícones `IconName`); tokens de
`workstation/tokens.rs`; nada de hex solto. Layout: uma linha por **corte** da edição (entre shots vizinhos):

| Controle | Kit | Comando |
|---|---|---|
| Origem e destino (rótulos, só leitura) | texto + ícone | derivados do corte |
| Efeito (`Nenhuma` / `Mistura (crossDissolve)`) | `Select` | `set_transition` / `remove_transition` |
| Duração (quadros) | `Slider` + campo | `resize_transition` (gesto: `begin`/`update`/`commit` ao arrastar) |
| Alinhamento, Handles, Easing, Curva | `Select` | `resize_transition`/`set_transition`/`set_transition_param` |
| Intensidade | `Slider` | `set_transition_param` (gesto) |
| Oculta | `Switch` | `set_transition_hidden` |
| Remover | `Button` | `remove_transition` |
| Lista de erros | texto por item (`code` + mensagem + dica) | só leitura de `errors[]` |

- Prévia: arrastar abre o gesto e o monitor mostra o **candidato** (como a P2/P3); soltar confirma, Escape cancela.
- Controles sem sentido ficam **desabilitados com `disabled_reason`**: "só há uma transição por corte", "o corte é
  `continuous`" (aviso, não bloqueio), "sem handles: troque para `hold`" etc.; nunca com callback vazio.
- Janela de diálogo: **nenhum**; a seção é inline, então `Workflow::has_dialog` **não se aplica**. Se a S4 introduzir
  qualquer diálogo (por exemplo confirmar remoção), ele marca `has_dialog` para esconder a WebView.
- **Overlays durante a janela (decisão D16):** o plano do topo é só um dos dois decks; as alças do gizmo e as alças da
  vinheta apontariam para um deck que não é "o que se vê". Durante `[início, fim)` o gizmo e as alças são **recusados
  com motivo tipado** (`transition_window`) e o monitor mostra o aviso; editar por propriedades continua permitido
  (Q1: reversível se o usuário preferir alças sobre o deck do topo).

### 11.2 Nós, parser e handlers compartilhados

Cada controle é `Command` → `Workstation::execute*` (regra do `AGENTS.md`): `state/tree.rs` (nós), `automation/commands.rs`
(parser) e **o mesmo handler** que lib e JSONL usam (`fx_run_guarded`, `fx_gesture`). Nós propostos:
`scene.fx.transitions` (lista de cortes), `scene.fx.cut:<fromShotId>|<toShotId>` (corte, com `set effect`) e
`scene.fx.transition:<id>` (campos `duration`, `alignment`, `handles`, `easing`, `curve`, `intensity`, `hidden`, ações
`remove`, `reset:<campo>`), mais `scene.fx.budget` (já existe, passa a refletir os dois decks). Não existe "ação só da
CLI": todo caminho de `scripts/ui` chama o mesmo handler da UI, e um teste GPUI e um teste de JSONL provam o **mesmo**
`EditSummary`/entrada de histórico.

## 12. Aceite por superfície, referência independente e controles negativos

Cada linha é um caso com ID estável (`T..`) no teste **ou** no corpus, e a coluna diz em que superfícies **deve existir
evidência separada** (lib e processo real são provas distintas; janela sintética ≠ ponteiro físico).

| ID | Caso | L | H | G | W | P | R | E | X |
|---|---|---|---|---|---|---|---|---|---|
| T01 | Documento mínimo válido, ida e volta em bytes, idempotente | ● | ● | | | | | | |
| T02 | Cada recusa: efeito/família, easing, parâmetro, shot ausente, não vizinho, duplicata, duração 0, janela fora, longa, sem handles (só instância / só câmera / ambas), sobreposição, `keys`, ID repetido/reservado, `catalogVersion` novo, limite | ● | ● | ● | ● | | | | |
| T03 | Avisos: `ON_CONTINUOUS`, `SPLAT_COST`, `FX_PASS_BUDGET` com dois decks | ● | ● | ● | ● | | | | |
| T04 | `hidden` e ausência = corte seco com os bytes do plano de hoje | ● | ● | | | ● | ● | | ● |
| T05 | Janela: tabela de alinhamento/duração (par, ímpar, D=1, `start`, `end`), bordas 0 e fim da edição, −1 e +1 inválidas | ● | ● | | | | | | ● |
| T06 | FPS racional 24, 25, 60, 30000/1001: mesma janela/progresso; tempo de instância do vizinho racional | ● | ● | | | | ● | | ● |
| T07 | Progresso: tabela da seção 6.2; `linear/linear`; `curve=smooth` com `intensity` 0, 0,5, 1 (mistura pelo progresso já eased versus curva; controles discriminantes) | ● | ● | | | | | | ● |
| T08 | Planos **sem** transição idênticos em bytes e digest ao `main` (corpus existente) | ● | ● | | | | ● | | |
| T09 | Plano na janela: topo/vizinho/`primary`, recursos em união, `requires`, capacidade, JSON ida e volta; cada adulteração recusada (progresso, vizinho, passe de saída no `other`, ordem, recurso não resolvido) | ● | ● | | | | | | |
| T10 | `hold`: instância e câmera independentes; `use` com quadros reais; caso do corpus `valid-cut-instance-offsets` | ● | ● | | | | ● | | ● |
| T11 | Clipe `shot` que encosta na borda segura o quadro; clipe no meio não entra | ● | ● | | | | ● | | ● |
| T12 | Passe de saída cobrindo a janela age **depois** da mistura; ordem observável em pixel | ● | | | | ● | ● | | ● |
| T13 | `filmGrain` de saída: mesma semente por quadro com e sem transição | ● | ● | | | | ● | | ● |
| T14 | Orçamento conta os dois decks e a mistura | ● | ● | ● | | | | | |
| T15 | **Runtime sem `fxTransition`** recusa `unsupported`+`missing`, não desenha | | | | | ● | ● | | |
| T16 | Pixel: decks de cor constante (200 → 40 canal, `f−início = 5` → 168 ± tolerância) e texturizados, por quadro da janela | | | | | ● | ● | | ● |
| T17 | Resize e perda de contexto no meio da janela; quadro depois = quadro de render novo | | | | | ● | ● | | |
| T18 | Erro de um deck (recurso ausente): erro tipado `deck`, sem meia mistura; **dispose** libera alvos e locações | | | | | ● | ● | | |
| T19 | Seek aleatório = sequencial (hash); invalidação do cache ao mudar o **vizinho**; alvos retidos dentro do orçamento | | | | | ● | ● | | |
| T20 | Edição: add (padrões), substituir no mesmo corte (ID preservado), resize, parâmetro, oculta, remover; **uma** entrada de undo cada; `dry_run` não muda nada; no-op sem entrada; lote com erro inalterado; `stale_revision` | ● | ● | ● | ● | | | | |
| T21 | Gesto: begin/update/commit ≡ `scene.fx.edit`; cancel; invalidação (revisão, transição trocada/removida, shot sumiu) | ● | ● | ● | ● | | | | |
| T22 | Substituir o documento sem o shot / encurtar o shot é **recusado** e nada muda | ● | ● | | ● | | | | |
| T23 | Salvar → reabrir → undo/redo; pacote e intercâmbio preservam `fx.transitions` e o quadro | ● | ● | | ● | | ● | | |
| T24 | UI sintética (GPUI): seção, seletores, sliders, switch, remover, lista de erros pt-br/en/es, prévia, `disabled_reason` | | | ● | | | | | |
| T25 | Janela real por `scripts/ui` (sem coordenadas): `tree/get/set/invoke` dos nós de 11.2, sondas de pixel nos quadros da janela contra a referência, recusa tipada | | | | ● | ● | | | ● |
| T26 | Alças e gizmo recusados na janela (`transition_window`) e voltando fora dela | ● | | ● | ● | | | | |
| T27 | Export FFmpeg completo e sub-intervalo que começa no meio da janela; quadros brutos idênticos entre os dois; vídeo comparado à referência com a tolerância do codec **declarada antes** | | | | | | ● | ● | ● |
| T28 | Corrupto/limites: JSON truncado, tipos errados, `NaN`, nomes unicode, contagens enormes, `other` corrompido no JSON do plano, janela do tamanho máximo, transição entre dois splats | ● | ● | | | | | | |
| T29 | **Mutantes** (corredor `mutate.py`): `+0,5` do progresso, easing ignorado, decks trocados, prendimento junto de instância e câmera, saída antes da mistura, `⌊D/2⌋`→`⌈D/2⌉`, `start`/`end` trocados, capacidade omitida, `amount` ignorado, vizinho sem invalidar cache; **todos mortos** | ● | | | | ● | ● | | |

**Tolerâncias (propostas aqui, **registradas no manifesto da S0 antes de qualquer execução**; não se mexe depois):** para
decks de 8 bits o esperado é a conta exata arredondada, então a proposta é diferença máxima **1/255** por canal no monitor e
no renderer; no export H.264 vale a tolerância de codec das provas de F016/F062 (referência: pior 7/255 observado em
`scene_fx_export`; o limite vigente é reconferido no manifesto antes de usar). Se a diferença exceder, o resultado é
**FAIL**, não ajuste.

**O que cada conjunto não prova:** o golden da origem não foi comparado (paridade é "por fórmula"); nada aqui mede
desempenho, GPU real, ponteiro físico, WebView nativa do Windows ou áudio ouvido; humano não aceitou a aparência (BL058).

## 13. Fases (tracer bullets), dependências e ownership

Cada fase é uma PR revisável. "Gate" é o que a PR precisa comprovar **para si**; nenhuma fase altera `docs/backlog.md`,
a matriz ou o índice por conta própria (o coordenador integra as linhas propostas).

| Fase | Entrega e aceite | Depende de | Arquivos (propostos) | Dono proposto |
|---|---|---|---|---|
| **S0 — Contrato e oráculo** (docs + Python, sem produção) | `docs/features/fx-transitions.md` (F064 proposta), patch da linha da matriz ao coordenador, reconferência no pin/ponta (seção 2), `scripts/pilot-dissolve-corpus.py` + spec, `docs/migration/fixtures/pilot-fx/dissolve-{cases,manifest}.json` com hashes de blob; vetores das seções 6.1/6.2 regenerados. **Aceite:** `--check` reproduz os JSON; spec Python verde com controles negativos; nenhum Rust alterado | este plano | `docs/features/fx-transitions.md`, `docs/features/effects.md` (nota de que o rascunho de extensão está superado), `scripts/pilot-dissolve-corpus.py(.spec.py)`, `docs/migration/fixtures/pilot-fx/dissolve-*.json` | Doc/oráculo |
| **S1a — Documento, janela, validação** | `Fx.transitions`, `FxTransition`, catálogo + `ParamKind::Choice`, `read_transitions`, janela/handles/`TRANSITION_*`, avisos, leituras `scene.fx.list/catalog` com transições, `FX_LIMIT`. `ScenePlan::build` em quadro de janela devolve `PlanError::Unsupported { domain: "transitions" }` **tipado** (nunca ignora). **Aceite:** T01–T03, T05, T28 (L e H) | S0 | `crates/core/src/directed_scene/fx/{mod,read,catalog,transition,window}.rs`, `scene_project/fx.rs`, testes `crates/core/tests/scene_fx_transition/` | Core |
| **S1b — Avaliação e plano** | `evaluate_shot_at_offset`, `Fx::resolve_frame`, `ScenePlan.transition`, `Capability::FxTransition`, `validate_deck`, JSON, `PlanTimeline` do vizinho, orçamento de dois decks, `scene.fx.frame` com `transition`. **Aceite:** T04, T06–T11, T13, T14, T28 (L e H); `pilot_dissolve_corpus.rs` verde; T08 (planos sem transição inalterados) | S1a | `crates/core/src/temporal/shot_offset.rs`, `scene_plan/{model,build,timeline,validate,json}.rs`, `directed_scene/fx/mod.rs`, `crates/core/tests/pilot_dissolve_corpus.rs`, `.../scene_fx_transition/plan.rs` | Core |
| **S2 — Runtime e render** | `parseTransition`, shader, alvos, ciclo de vida, `SCENE_CAPABILITIES`; `crates/render/tests/scene_fx_transition.rs`; testes `node --test` do runtime; teste de runtime antigo. **Aceite:** T12, T15–T19 (R, P) e T16 contra a referência X; export de quadros brutos | S1b | `runtime/src/{directed-scene,compositor,output-fx}.js`, `runtime/test/*`, `runtime/test/fixtures/fx-catalog.json`, `crates/render/tests/scene_fx_transition.rs` | Runtime/render |
| **S3 — Edição, gesto e persistência (core)** | Ops, dry-run/no-op/undo/revisão, gesto, `TransitionInspector`, `errors[]`, contrato de shot apagado, persistência/pacote/intercâmbio. **Aceite:** T20–T23 (L, H) | **S1b** (não S2) | `scene_edit/{fx,fx_inspector,fx_choice}.rs`, `scene_project/{fx,fx_gesture,query,command}.rs`, `ops.rs`, `state/tree.rs`, `crates/core/tests/scene_fx_transition/edit.rs` | Core (outro agente que a S2 pode rodar **em paralelo**) |
| **S4 — UI Kit e automação** | Seção Transições, nós, parser, handlers, overlays recusados na janela, prova da janela. **Aceite:** T24–T26 (G, W, P) | S2 **e** S3 | `crates/desktop/src/workstation/scene_fx_transition*.rs`, `automation/commands.rs`, `scripts/scene-fx-transition-proof.py(.spec.py)` | UI |
| **S5 — Fechamento** | Export de ponta a ponta (T27), mutantes (T29), NTSC e limites já verdes, docs F064/F016/F062 e matriz, relatório com evidências por superfície. **Aceite:** tabela da seção 12 com cada célula marcada *executado* ou *A validar* | S2–S4 | `docs/migration/p4b-cross-dissolve/{mutate.py,test_mutate.py}`, `crates/render/tests/scene_fx_transition_export.rs`, docs | Coordenador de integração |
| **S6 — Dissolve legado** (BL082.4.7) | Importar `transitionInto: dissolve` como `crossDissolve` sintético (D04), com o corpus e o oráculo emendados **com registro**; `TRANSITION_DUPLICATE` com `fx`. **Aceite:** o veredito de `invalid-dissolve-transition` muda por decisão registrada, e a S10 reexecuta | S5 e decisão Q2 | `directed_scene/{parse,joins}.rs`, corpus de bundles, F044 | Core |

**Dependências exatas.** S0 → S1a → S1b → {S2 ∥ S3} → S4 → S5 → S6. S3 **não** espera S2 (só precisa do plano e do
documento). S4 precisa dos dois porque exibe pixel e edita. Nenhuma fase depende de `node` scope (BL082.40), P5 (BL082.5) ou P6.
Conflito de arquivos entre S2 e S3: nenhum (S2 só toca `runtime/`+`crates/render`; S3 só `crates/core`). S1a e S1b
**dividem** `directed_scene/fx/mod.rs`: sequenciais, nunca paralelas.

**Tracer bullet mínimo (primeiro resultado verificável):** um bundle do corpus (`valid-cut-instance-offsets`) com uma
transição `crossDissolve` `center` D=12 `hold` (corte em C = 40) → `tessel-headless` real responde `scene.fx.frame` no
quadro 37 com o plano de dois decks (janela [34,46), `frameInWindow` 3, `primary: "from"`, vizinho `shot-b` com
`heldInstance` e `heldCamera` verdadeiros, progress (após easing) 0,099248; peso efetivo (após smoothstep, intensity=1) 0,027595), e o teste de corpus confere janela,
progresso e a mistura de pixel de duas cores constantes. É o fim da S1b, antes de qualquer runtime.

## 14. Gates, ambiente, mbx e disco

Gates **separados** e nenhum implica o outro:

| Gate | O que prova | Não prova |
|---|---|---|
| **Windows** | `fmt`, `check`, `clippy --all-targets -D warnings`, testes **focados** de cada fase, `build -p tessel-editor`, runtime após `./scripts/runtime setup`, provas de janela pela `scripts/ui` (D3D11) | Mac, WKWebView, ponteiro físico, desempenho |
| **Mac** | O mesmo e a janela real (WKWebView/Metal), provas Python | Windows |
| **Review** | Revisão independente do head (coordenador) | Execução |
| **CI** | Checks do GitHub no head | Janela real |
| **Merge** | Decisão do coordenador, sem *force* | |

- **Windows hoje não é verde** no workspace inteiro (BL167: gizmo BL167.3, ledger BL167.6, helper BL167.7). A PR de cada
  fase registra a **lista vermelha de base** da `main` (nomes de teste) antes da primeira mudança e afirma "sem falha nova",
  **nunca** "workspace verde". Python no Windows é BL167.5: provas `*.py` e o `--check` do corpus rodam no Mac ou depois
  de BL167.5; os testes Rust do corpus **só leem JSON** e rodam em qualquer lugar.
- **Cargo só por `./scripts/cargo`**, alvo padrão; **nunca** `CARGO_TARGET_DIR`, `--target-dir`, `build.target-dir` ou
  `TESSEL_CARGO_LANE`; sem `clean`/`gc`/limpeza de cache; artefatos/cache em `tmp` > 500 MB proibidos; reserva de
  disco ≥ 16 GiB para trabalho pesado (piso 12); evitar `--workspace`/release concorrentes (há outra frente Windows).
- **Lock da GPU só para desempenho.** BL082.4.8 (custo do quadro de dois decks e do par de splats) fica separado; sem o
  lock, desempenho **não** é declarado aceito. Provas funcionais não o tomam.
- **Origem não executada.** `ORIGIN_NOT_EXECUTED` e `ORIGIN_GOLDEN_NOT_COMPARABLE` continuam no manifesto.

| Risco | Mitigação |
|---|---|
| Pin da origem ausente | Passo 0 da S0; nota no manifesto; sem alegar paridade |
| Dupla modelagem do plano (`other`) | `other` é `ScenePlan` validado por `validate_deck`; sem segundo tipo |
| Digest dos planos existentes mudar | Chave aditiva ausente quando não usada; T08 fixa os bytes do corpus |
| Cache servir pixel velho do vizinho | Chave por deck; T19 e mutante |
| Meia mistura em erro | Erro da camada, nunca desenho parcial; T18 |
| `intensity = 0` tratado como identidade ou progresso sem easing | D07 e T07 explícitos; usar curve=smooth e progress já eased |
| Prova só da lib vendida como UI | Cada superfície tem a sua coluna; sintético ≠ físico |

## 15. IDs propostos, perguntas reais e entrega

**IDs propostos** (o coordenador cria; `BL082.4` é o agregador existente, Pendente):

| ID | Pendência | Depende de |
|---|---|---|
| BL082.4.1 | S0 — contrato F064, reconferência do pin e corpus independente | BL082.4 |
| BL082.4.2 | S1a — documento, janela e validação tipada | .1 |
| BL082.4.3 | S1b — avaliação por quadro e plano de dois decks | .2 |
| BL082.4.4 | S2 — runtime e renderer | .3 |
| BL082.4.5 | S3 — edição, gesto, persistência (core) | .3 |
| BL082.4.6 | S4 — UI Kit e automação | .4, .5 |
| BL082.4.7 | S6 — dissolve legado | .6 e Q2 |
| BL082.4.8 | Desempenho do quadro de dois decks (lock da GPU) | .4 |
| BL082.4.9 | `keys` em transições (D10) | .5 |
| BL082.4.10 | S5 — export, mutantes e fechamento | .4–.6 |

Matriz: **F064** proposta, `docs/features/fx-transitions.md` (o coordenador confirma o número; hoje a última é F063).

**Perguntas reais ao usuário** (todo o resto foi decidido e justificado acima):

- **Q1.** Durante a janela, recusar gizmo e alças (D16) ou aceitá-las sobre o deck do topo? Recomendação: recusar.
- **Q2.** Autorizar a emenda do veredito de `invalid-dissolve-transition` (S6)? Altera o oráculo BB10; recomendação:
  só depois de reconfirmar o pin da origem.
- **Q3.** `UNSUPPORTED_ALPHA` da origem para saídas com efeito: portar (para todos os efeitos) ou manter opaco? Fora da P4b.
- **Q4.** Aviso `TRANSITION_OVER_SPEECH`: adiar até a narração existir (BL017/BL026)? Recomendação: adiar.

**Entrega a quem implementa:** a primeira ação de S0 é a reconferência do pin; a última de cada fase é o relatório
com a tabela da seção 12 preenchida por célula (*executado*, *A validar* ou *N/A* com motivo). **Plano pronto não é
feature implementada, nada daqui foi executado, e nenhum controle novo existe antes da S4.**
