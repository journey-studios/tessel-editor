# P4b — Transições entre planos: `crossDissolve` (BL082.4)

2026-11-14, **versão 2** (depois da revisão independente do head `5e64dd9` e das correções de evidência do `ab4cc69`).
**Plano de implementação. Nenhuma linha de produção foi alterada por este documento**: plano pronto não é feature
implementada. Base lida: `origin/main` `2b54ac1` (PR #57 integrada). Frente W3 da rodada Windows de 14/11/2026; a
avaliação do plano e o início da implementação são do coordenador. Os achados da revisão e onde cada um foi resolvido
estão na [seção 16](#16-rastreabilidade-da-revisão-de-14112026).

P4b é a segunda metade da P4 de [pilot-parity-execution-plan.md](pilot-parity-execution-plan.md): `transitions[]`
no `fx`, **dois decks** no plano, a primeira transição (`crossDissolve`) e a capacidade nova no runtime. A P4a
([p4a-shot-scope-fx-plan.md](p4a-shot-scope-fx-plan.md), [F062](../features/fx-shot-scope.md), PR #48) deixou a
costura: o conceito de **deck** (a imagem de um plano antes da saída), as duas etapas da composição, a capacidade
`fxShotScope` e a regra única `scope_fault`. O backlog continua sendo a fila única: aqui só se **propõem** linhas
(seção 15); o coordenador as integra.

## 0. Como ler este plano

| Seção | Conteúdo |
|---|---|
| 1 | Escopo, fora de escopo e por que esta fatia vem agora |
| 2 | Leituras primárias, o que faltou e o que reconfirmar antes de codar |
| 3 | Regras da origem e a decisão tomada para cada uma (D01–D21) |
| 4 | Estado atual do destino: caminhos, símbolos e tamanhos que a P4b toca |
| 5 | Contrato do documento: `fx.transitions[]`, IDs, importação, validação, limites, ida e volta |
| 6 | Tempo: janela, aritmética verificada, disponibilidade de quadros, relógio, FPS racional, bordas |
| 7 | Avaliação por quadro e plano de dois decks (pesos, ordem, recursos, limites, cor, alpha) |
| 8 | Capacidade, **portão público (8.4)**, runtime, orçamento, ciclo de vida e export |
| 9 | Edição: operações atômicas, gesto, inspector, erros, shot apagado, **edição durante a janela (9.6)** |
| 10 | Persistência, pacote e intercâmbio |
| 11 | UI real (Kit), automação e contrato `Command`/nó/parser |
| 12 | Aceite por superfície **e por fase**, referência independente, tolerâncias e controles negativos |
| 13 | Fases, dependências exatas, ownership e arquivos |
| 14 | Gates, ambiente Windows/Mac, mbx, Python e disco, riscos |
| 15 | Linhas propostas ao backlog, decisões registradas e entrega |
| 16 | Rastreabilidade da revisão independente |

Convenções: **L** lib Rust sem janela · **H** processo real `tessel-headless` (JSONL) · **G** eventos GPUI sintéticos
(`gpui_kit::test`) · **W** janela real por `scripts/ui` sem coordenadas · **P** pixels do monitor (WebView) ·
**R** renderer headless (`crates/render`) · **E** export com FFmpeg · **X** referência independente em Python.
Sintético não é ponteiro físico, GPU real nem áudio ouvido. Nomes de símbolos novos são **propostas**;
os existentes foram lidos em 14/11/2026 na `main` `2b54ac1`.

## 1. Escopo

**Entra (BL082.4, versão 1):**

1. Camada `fx.transitions[]` do documento (formato do piloto), com importação, validação tipada, limites, ida e volta e
   `catalogVersion` mantido em 1.
2. Um efeito de transição: `crossDissolve` (parâmetros `curve` e `intensity`), em uma família nova `transition` do
   catálogo, com um novo tipo de parâmetro de escolha (`ParamKind::Choice`).
3. Janela por corte (`alignment`: `center|start|end`), duração em quadros, `handles` (`use|hold|refuse`) e `easing`.
4. Avaliação pura por quadro no core, plano com **dois decks** e capacidade `fxTransition` (14ª).
5. Runtime Three.js que desenha os dois decks em alvos separados e mistura em luz linear, antes dos passes de saída.
6. **Portão público** (seção 8.4): a entrada pública do documento recusa `transitions` até a ativação, que só acontece
   quando monitor, export e preflight já desenham a janela (fase S2A). Nenhum consumidor vê uma transição antes disso.
7. Edição atômica no core (`scene.fx.edit`, `scene.fx.gesture.*`, `scene.fx.inspect`), persistência, pacote,
   intercâmbio e export.
8. UI no Kit (seção de transições no painel de efeitos), nós semânticos, parser e testes nas três superfícies.
9. Recusa de gizmo/alças durante a janela por **um** predicado do core (D16, seção 9.6).

**Fica fora (cada item é BL próprio, nada habilitado por associação):**

- As outras 27 transições do piloto e os demais efeitos: o catálogo passa de 3 para 4 dos 59 efeitos do piloto
  (`vignette`, `fade`, `filmGrain` + `crossDissolve`) e os 55 restantes ficam fora. Nenhum controle de efeito ausente
  é habilitado.
- **`keys` em transições** (chaves de parâmetro com origem no início da janela): recusadas com código tipado
  (decisão D10). O motor de chaves da F058 é de clipe (`FxClip`) e o inspector/gesto de chaves está acoplado a clipes.
- Importação do **dissolve legado** (`shots[].transitionInto.type = "dissolve"`): fatia própria no fim da P4b (S6,
  BL082.4.7), **expansão de contrato** com faixa de oráculo nova (decisão D04), não correção do corpus antigo.
- Escopo `node` (BL082.40), ripple/roll e edição estrutural de `shots[]` (BL082.5/P5), trilhas de timeline de
  transições (P6), aviso de transição sobre fala (não há narração no editor: BL017/BL026; decisão Q4).
- **`UNSUPPORTED_ALPHA`** da origem: **divergência de veredito declarada**, não paridade (D20, decisão Q3). O piloto
  recusa `alphaMode` diferente de `opaque` (com `fx` ou com `dissolve`); o Tessel aceita e desenha opaco, hoje já para os
  três looks.
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
| `docs/features/effects.md` (F016), `fx-shot-scope.md` (F062), `p4a-shot-scope-fx-plan.md`, `pilot-fx-shots-studio-features.md`, `pilot-parity-execution-plan.md`, `origin-specs/scene-fx-plan.md` e `scene-fx-decisions.md` | Lidos |
| Código do destino (seção 4) | Lido nos caminhos citados; **símbolos novos aqui são propostas** |
| Origem JS, só leitura por `git show` em `refs/remotes/origin/t3/scene-engine-pilot` (**ponta `365579076782c1fc19c6464f3bc0199b30c0ab8f`, 2026-11-03**) | Lidos pela autora (Windows): os 16 arquivos de regra citados logo abaixo da tabela 2.1, na ponta |
| **Pin do snapshot do piloto `d40ac3f7c0`** (citado em `source-feature-inventory.md`) | A autora (Windows) **não leu o pin**: `git cat-file -t d40ac3f7c0` falha no clone Windows. O **coordenador (Mac)** comparou depois, **por objetos Git, somente leitura**, 21 blobs do pin com a ponta: **19 idênticos e 2 diferentes** (2.1). Relato do coordenador, **não reconferido pela autora**; **a origem não foi executada** |
| Golden da origem (`catalog/__golden__/crossDissolve.png`) | Existe na ponta; **não aberto, não comparado**, origem não executada |
| Corpus de bundles (`docs/migration/fixtures/bundles/`) | Lido: `valid-minimal-continuous`, `valid-cut-instance-offsets`, `invalid-dissolve-transition` (`renderProfile.alphaMode: "straight"` nos três; `valid-cut-instance-offsets`: 2 shots, edição de 100 quadros) |

### 2.1 Relação pin × ponta (relato do coordenador, a reproduzir no manifesto da S0)

Todos os caminhos sob `packages/scene-engine/src/`. "Idêntico" = o blob do pin é igual ao da ponta.

| Arquivo | Pin × ponta |
|---|---|
| `fx/fx-windows.ts`, `fx/fx-explain-transitions.ts`, `fx/fx-normalize-transitions.ts`, `fx/fx-frame-decks.ts`, `fx/fx-frame-derived.ts`, `fx/fx-explain-budget.ts`, `fx/fx-command-transitions.ts`, `fx/resolve-fx-frame.ts` | idênticos |
| `fx/catalog/crossDissolve.ts`, `fx/catalog/fx-effect-meta.ts`, `edit/resolve-shot-at-offset.ts`, `camera/easing.ts` | idênticos |
| `runtime/fx/fx-compositor.ts`, `runtime/fx/shaders/crossDissolve.ts` | idênticos |
| `fx/fx-explain-handles.spec.ts`, `fx/resolve-fx-frame-transitions.spec.ts` (o spec **de transições**), `fx/fx-explain-legacy-dissolve.spec.ts` | idênticos |
| `validate/edit-joins.ts`, `fx/fx-explain.ts` | idênticos |
| `fx/fx-schema.ts` | **difere** (`5ce47498…` no pin, `5c85ce56…` na ponta): só um comentário (`CLIP_OVERLAP` → `CLIP_PAST_END`) |
| `schema/render-bundle.schema.ts` | **difere** (`5bdbb497…` no pin, `11c3a49d…` na ponta). **Não é comentário**: na ponta o arquivo importa `editFrameToSeconds` e chama `explainNarration` com a duração, isto é, muda o comportamento da **narração** |

Consequências: (a) as regras de transição (janela, easing, shader, handles, adjacência, avisos) são as mesmas no pin e
na ponta; (b) para `render-bundle.schema.ts` a S0 **fixa o blob do pin** e mantém o plano válido **sem narração**: o
editor não tem narração (BL017/BL026) e a P4b não amplia o escopo para isso; (c) o manifesto da S0 grava os 21 blobs
completos (pin e ponta), e a diferença de `fx-schema.ts` como nota de comentário, a de `render-bundle.schema.ts` como
nota de comportamento de narração, **nunca** como ajuste silencioso de expectativa.

**Manifesto da S0, além dos 16 arquivos de regra** (`fx-windows`, `fx-explain-transitions`, `fx-normalize-transitions`,
`fx-frame-decks`, `fx-frame-derived`, `fx-explain-budget`, `fx-command-transitions`, `resolve-fx-frame`, `fx-schema`,
`catalog/crossDissolve`, `catalog/fx-effect-meta`, `resolve-shot-at-offset`, `camera/easing`, `runtime/fx/fx-compositor`,
`runtime/fx/shaders/crossDissolve`, `fx-explain-handles.spec`): os specs com **nome exato**
(`fx/resolve-fx-frame-transitions.spec.ts`, `fx/fx-explain-legacy-dissolve.spec.ts`, `fx/fx-explain-handles.spec.ts`);
`validate/edit-joins.ts`, `schema/render-bundle.schema.ts` e `fx/fx-explain.ts` (origem de `UNSUPPORTED_ALPHA`), todos
necessários para a S6.

**Leitura primária que a S0 ainda faz** (a reconferência do pin deixou de ser pré-requisito de leitura, mas o manifesto
precisa **reproduzi-la**): (a) em um clone de leitura que tenha o pin **e** a ponta, `git rev-parse <ref>:<caminho>` dos
21 arquivos; (b) reconferir, na referência escolhida, os pontos de maior risco: a fórmula do shader, `transitionWindow`,
`transitionIssues` (códigos e **ordem**), `shotClipFrame`, `resolveShotAtOffset` (clamp independente de instância e
câmera) e **de onde vem o relógio de cena do deck vizinho** (D17); (c) o que mudar entre pin e ponta vira nota no
manifesto. Não se declara paridade com a origem: a prova é "fórmulas transcritas por Python independente", como em
`pilot-fx` (vinheta, `fade`, `filmGrain`).

## 3. Regras da origem e decisão para cada uma

Decisões rotineiras estão justificadas aqui e **não** viram pergunta ao usuário. As escolhas de produto estão
registradas como decisões do coordenador (Q1–Q4, seção 15).

| ID | Regra na origem (ponta `3655790`) | Decisão no Tessel | Justificativa |
|---|---|---|---|
| D01 | Camada `fx.transitions[]` com `id`, `effect`, `fromShotId`, `toShotId`, `durationFrames`, `alignment`, `handles`, `easing`, opcionais `params`, `keys`, `seed`, `hidden` | **Mesmos nomes de campo.** O rascunho de `effects.md` (~l.450: `type`, `fromShot`, `toShot`) fica **superado**: atualizar F016 na S0/S1a | `fx/mod.rs` já declara o formato como "o do piloto"; documentos do piloto precisam importar sem conversão |
| D02 | Uma transição por corte (`fromShotId\|toShotId`); `setTransition` substitui a do mesmo corte | Idem. `TRANSITION_DUPLICATE` na importação; `set_transition` é *upsert* por corte, preserva o ID. "Já existe uma transição neste corte" **não** é motivo de `disabled_reason` (o upsert executa a operação real) | Evita duas janelas no mesmo corte; edição previsível |
| D03 | Janelas por `alignment`: `center` começa em `corte − floor(D/2)`, `start` em `corte`, `end` em `corte − D`; `quadrosAntes = max(0, min(fim,corte) − início)` | Idem, em **inteiros de 64 bits com aritmética verificada** (6.1); `D ≤ duração da edição` antes de qualquer conta; sem segundos | A janela é aritmética de quadros; nada nela depende do FPS (6.1); overflow é recusa tipada, **nunca** saturação que mude o significado |
| D04 | `transitionInto: dissolve` legado é lido como `crossDissolve` centrada, `hold`, `easing linear`, `curve linear`, ID `dissolve-<shot>`; duplicata com `fx` é erro | **Fora de S0–S5.** S6 (BL082.4.7) é **expansão de contrato** com **faixa de oráculo nova**, não correção do corpus antigo; o acervo antigo e suas asserções ficam como histórico (seção 13, S6) | O piloto aceita `dissolve` desde a decisão #35 e o valida desde a #49 (`origin-specs/scene-fx-plan.md` l.147–175; `scene-fx-decisions.md` #35 e #49); o corpus de equivalência BB10 vem de um pin **anterior** a isso. Trocar o veredito dele "por decisão" mistura duas revisões e não é paridade com o piloto |
| D05 | `handles`: `use` e `refuse` exigem quadros **reais** (instância **e** câmera, contados à parte); só `hold` aceita congelados; `refuse ≡ use` no cálculo (decisões #6) | Idem, os 3 valores aceitos e preservados | Documentos do piloto importam; nenhum dado é migrado depois se `refuse` ganhar sentido |
| D06 | Padrões de transição nova: `alignment center`, `handles hold`, `easing easeInOutCubic`; duração omitida = típica do efeito | Idem; a duração típica (24) é **cortada** ao que os dois planos suportam, **dada** explícita demais é recusada (`TRANSITION_TOO_LONG`) | Mesma política de F062 (`fx::default_duration`: omitida corta, dada recusa) |
| D07 | Fórmula: `p = ease((f − início + 0,5)/D)`; `shaped = mix(p, smoothstep(p), smooth)`; `saída = mix(mix(a,b,p), mix(a,b,shaped), amount)`, `alpha = 1` | Idem. `smooth = (curve == "smooth")`, `amount = clamp01(intensity)`. **`intensity = 0` não é identidade**: mistura os decks usando `progress` já eased, sem a segunda modelagem smoothstep. Com `curve=linear`, `intensity` não altera a saída (6.2) | Fidelidade; o validador do plano **não** pode aplicar a regra `amount = intensity × peso` dos looks |
| D08 | Mistura em luz linear; a origem decodifica os decks para half-float linear | Decodificar IEC 61966-2-1 → misturar → codificar, nos alvos de **8 bits** do runtime (como `vignette`/`fade`/`filmGrain` em `output-fx.js`). A referência Python **modela a quantização de 8 bits dos decks** (D13); a diferença para o half-float da origem é **declarada**, não escondida | Mesma convenção do runtime atual; paridade "por fórmula" só vale se a referência ver os mesmos 8 bits que o runtime |
| D09 | Por quadro: ordem **passes do plano A/B → mistura → passes de saída** | Idem | Ordem de F062 + um estágio |
| D10 | `keys` em transições: recortadas para o intervalo da janela | **Recusadas**: `FX_TRANSITION_KEYS_NOT_YET` (código tipado, sem ignorar) | Chaves de F058 são de clipe; reaproveitar tocaria inspector e gesto de chaves. BL082.4.9 |
| D11 | Transição `hidden` não desenha | Corte seco naquele corte; sem `transition` no plano | Idem origem |
| D12 | Avisos `TRANSITION_OVER_SPEECH`, `TRANSITION_ON_CONTINUOUS`, `TRANSITION_SPLAT_COST`, `FX_PASS_BUDGET` | Portar `ON_CONTINUOUS`, `SPLAT_COST` e o orçamento como **avisos** (nunca recusa, **nunca** motivo de desabilitar controle). `OVER_SPEECH` é **N/A registrado** (sem narração, sem controle); quando houver narração o dissolve legado também conta (origem) | Aviso não bloqueia; o editor não tem narração ainda |
| D13 | Teste de pixel da origem usa o golden dela | Referência **independente em Python** + tolerância declarada **antes** de rodar, no contrato (seção 12); `ORIGIN_GOLDEN_NOT_COMPARABLE` mantido | Regra da rodada: não adulterar tolerância/oráculo |
| D14 | `hold` prende o quadro de cada lado **à instância e à câmera, separados** (`heldInstance`, `heldCamera`) | Idem: o deck vizinho usa `instância = clamp(inicioInst + deslocamento, 0, durInst − 1)` e `câmera = clamp(camIn + deslocamento, 0, quadrosCam − 1)`, mais as duas bandeiras | Um plano pode ter cena e não ter câmera (caso real do corpus, seção 7.2) |
| D15 | Clipe `shot:X` que encosta na borda do plano segura o quadro mais próximo nos quadros de handle; clipe no meio não entra | Idem (`shotClipFrame`) | Efeito não some no corte |
| D16 | (sem equivalente: a origem desenha overlays sobre o deck do topo) | Durante a janela, gizmo de nó e alças de efeito são **recusados com motivo tipado** `transition_window` por **um predicado do core** (9.6); edição por propriedades segue permitida com alvo explícito | O plano do topo é um de dois decks: uma alça sobre ele não corresponde ao que se vê. Decisão Q1; reversível porque o predicado é único |
| D17 | `resolveShotAtOffset` e `fx-frame-decks.ts` (a S0 confirma **de onde** vem o relógio do vizinho) | O deck vizinho tem **três relógios**: instância e câmera **presas** (D14) e o **relógio de cena** (luz com cintilação, clima futuro) no **quadro de edição real** `f`. Um deck em `hold` ainda cintila. Se a leitura da S0 mostrar outra regra na origem, ela é registrada como divergência **antes** da S1b | `lighting.rs:447–452` calcula a cintilação em `edit_frame/fps`; o avaliador já recebe o quadro de edição |
| D18 | Família errada: `WRONG_FAMILY` (`pilot-fx-shots-studio-features.md` l.53–54; `scene-fx-plan.md` l.131) | **Adotar `WRONG_FAMILY`**: look em `transitions[]` ou efeito de transição em clipe de trilha. `UNKNOWN_EFFECT` só para nome fora dos dois catálogos. **Com o portão fechado (D19) os dois lados mantêm o comportamento de hoje** | "Nenhum código é inventado": é o código da origem |
| D19 | (sem equivalente) | **Portão de admissão** (8.4): a entrada pública do documento recusa `transitions` não vazio com `FX_TRANSITION_NOT_YET` (comportamento de hoje) **até a S2A**. S1a/S1b/S2/S3a entregam código inalcançável pelos adaptadores públicos; cada PR mantém os documentos, planos e catálogo públicos byte a byte; o hello do runtime ganha a capacidade aditiva na S2 | Evita export que falha no quadro 34 de 100 e corte seco silencioso: ninguém consome o que ainda não é desenhado |
| D20 | `UNSUPPORTED_ALPHA`: `alphaMode ≠ opaque` com `fx` não vazio **ou com dissolve** é erro (`scene-fx-decisions.md` #19, #46, #49) | **Divergência de veredito declarada** (Q3): o Tessel aceita e a transição desenha **opaco**, para os três looks e para a transição. O `renderProfile` do bundle do tracer é `straight`: os casos dele são "contrato Tessel", **não** "aceito pela origem". Um caso com deck de pixels parcialmente transparentes fixa o comportamento. Registrada em F016, F064 e na S6 | A regra de alpha global é outra fatia (fora da P4b); esconder a divergência seria declarar paridade falsa |
| D21 | Limites e entrada hostil do plano (a origem valida no schema) | Limites explícitos no Rust **e** no JS (5.3): profundidade de `transition.other` 1 com recusa **sem recursar**, teto de bytes e de itens somando os dois decks, inteiros estritos | Entrada hostil não pode esgotar pilha nem memória antes da recusa tipada |

## 4. Estado atual do destino (`main` `2b54ac1`) e o que a P4b toca

Contagens de linha lidas em 14/11/2026. **Arquivos que já passam de 1000 linhas** (BL082.55/BL082.62 proíbem crescê-los):
`scene_edit/fx.rs` 1235, `ops.rs` 1217, `state/tree.rs` 1643, `scene_project/gizmo.rs` 1404,
`desktop/workstation/scene_fx.rs` 1469, `commands.rs` 1399, `scene_edit/fx_inspector.rs` 1045,
`state/mod.rs` 1533 e `runtime/src/compositor.js` 1166. `overlay.js` (945) está a ~55 linhas do limite.
**Regra desta rodada (H10):** o código novo vai para **módulos novos**; o gancho necessário num arquivo acima de 1000 linhas
tem delta líquido ≤ 0, compensado por extração equivalente de código existente na mesma PR; `ops.rs` e `tree.rs` só ganham
**uma entrada por op/nó novo** (a exceção que o BL082.62 já prevê), com o corpo em módulo novo.

| Caminho | Símbolo (lido) | Estado hoje | P4b |
|---|---|---|---|
| `crates/core/src/directed_scene/fx/mod.rs` (554) | `Fx { tracks }`, `FxClip`, `FxPass`, `Fx::resolve`, `pass_budget`, `FX_CATALOG_VERSION = 1`, `FX_PASS_BUDGET = 8`, `FX_TRANSITION_NOT_YET` | Só `tracks`; `transitions` não vira dado | `transitions: Vec<FxTransition>`; `Fx::resolve_frame` em módulo novo; remover `FX_TRANSITION_NOT_YET` na S2A, quando nada mais o produzir |
| `.../fx/read.rs` (700) | `read_fx` (~l.672–718) | Recusa `transitions` não vazio (l.700–712) | `read_transitions` em módulo novo (`fx/transition_read.rs`), códigos `TRANSITION_*`, `FX_TRANSITION_KEYS_NOT_YET`, `WRONG_FAMILY`; `read.rs` só ganha o desvio condicionado ao portão |
| `.../fx/catalog.rs` (414) | `EffectSpec`, `ParamKind` (sem escolha), `CATALOG: [&EffectSpec; 3]`, `family: "look"` | 3 looks | Tabela de transições **separada** (`fx/transition_catalog.rs`) até a S2A: o seletor de efeito de clipe **não** passa a oferecer `crossDissolve`; `ParamKind::Choice`, `ParamValue::Choice` |
| `.../fx/scope.rs`, `handles.rs`, `control.rs` | `FxScope`, `scope_fault`, `FxClip::handles_refusal` (F061) | P4a | Sem mudança; o predicado de janela (9.6) é irmão de `handles_refusal`, módulo novo `fx/window_guard.rs` |
| `crates/core/src/directed_scene/parse.rs` (785), `joins.rs` | `transition()` (~l.669: `DISSOLVE_UNSUPPORTED`), `MISSING_TRANSITION`, `CUT_WITHOUT_REASON` | Dissolve legado inválido | Só S6; o corte continua exigindo `transitionInto: cut` com motivo |
| `crates/core/src/directed_scene/temporal/frame.rs` (354) | `resolve_frame_mapping`, `mapping_inside_instance` (estrito), `evaluate_frame_with` | Só quadro dentro do plano | Avaliador novo `evaluate_shot_at_offset` em `temporal/shot_offset.rs`; o estrito não muda |
| `crates/core/src/scene_plan/{model,build,timeline,validate,json,error}.rs` (444/686/277/768/817) | `ScenePlan` (um deck), `Capability::ALL` (13), `PlanTimeline::at_frame`, `check_fx` (l.459), `derive_requires`, `plan_resources` (`BTreeMap` por `assetId`, `used_by` = ids de **nó**), `PlanError::Unsupported { domains }` com `Domain::ALL` (13) | Um deck | Campo aditivo `transition`, `Capability::FxTransition`, código novo em `scene_plan/transition{,_json,_validate}.rs`, `resource_union` derivada. **Nenhum `Domain` novo** (7.3, 8.4) |
| `crates/core/src/scene_edit/{fx,fx_inspector,fx_choice,fx_intent,fx_animation}.rs` | `parse_fx_batch`, ops de clipe, inspector, escolha nomeada; `fx.rs:593` cria `fx` com `transitions: []`, `fx.rs:753` poda `fx` vazio; `fx_inspector.rs:727` `REASON_FAMILY_NOT_YET = "FX_TRANSITION_NOT_YET"` | Sem ops de transição | Ops em `scene_edit/fx_transition.rs` (novo), inspector em `fx_transition_inspector.rs` (novo); ganchos com delta ≤ 0 |
| `crates/core/src/scene_project/{fx,fx_gesture,query,command,fx_seek,run}.rs` | `scene.fx.*`; `fx.rs:58–60` anuncia `refused: {transitions}` | Leituras sem transições | Leituras e gesto com `transition`; tirar `transitions` de `refused` na S2A |
| `crates/core/src/scene_project/gizmo.rs` (1404), `fx_handles.rs`, `crates/core/src/scene_interaction/fx_handles.rs`, `runtime/src/overlay.js` (945) | gizmo de nó, alças da vinheta, `scene.gizmo.hit` ≡ press | Sem noção de janela | **Só fiação** do predicado (S4a), depois que a base W1 estiver integrada (13, H13) |
| `crates/core/src/ops.rs` (1217) | parser `scene.fx.edit` (l.666, `dry_run`, `base_revision`), `scene.fx.gesture.begin` (`clip` obrigatório), `scene.fx.inspect` | | **Uma** entrada por op; `gesture.begin {transition}` exclusivo com `clip`; `inspect {transition\|cut}` |
| `crates/core/src/commands.rs` (parser semântico compartilhado), `crates/core/src/state/tree.rs` (1643) | `desktop/src/automation/commands.rs` tem 2 linhas e só reexporta `tessel_editor_core::commands` | | Parser e nós de estado no **core** (S3b); nós de painel no desktop (S4b) |
| `crates/core/src/scene_export/snapshot.rs` (291) | snapshot congelado do export | | Inclui `fx.transitions`; teste de que mudança durante o export não vaza |
| `crates/render/src/{scene,scene_renderer,scene_host}.rs`, `crates/render/tests/scene_fx*.rs` | renderer headless, 54 sondas por quadro contra a referência do core | | `scene_fx_transition.rs` |
| `runtime/src/directed-scene.js` (672), `compositor.js` (1166), `output-fx.js` (464), `assets.js` (844) | `SCENE_CAPABILITIES`, `parseDirectedLayer`, `prepareDirectedScene` (~l.877–964), `sceneTarget`, `fxPipeline`, `dropFxPipeline`, `splatLease`, `SRGB_CURVES` | Um deck por camada | Módulos novos `directed-transition.js` (parse e limites) e `transition-blend.js` (alvos, shader, ciclo de vida); `compositor.js` só ganha o gancho, com extração equivalente |
| `runtime/test/fixtures/fx-catalog.json`, `output-fx.test.mjs`, `directed-scene-fx-browser.test.mjs` | catálogo espelhado e testes Chrome | | Entradas do catálogo; testes novos |
| `crates/desktop/src/workstation/scene_fx*.rs` (`scene_fx.rs` 1469) | seção Efeitos, nós `scene.fx.clip:<id>`, `scene.fx.budget`, `fx_run_guarded` (`stale_revision`) | | Seção Transições e nós `scene.fx.transition:<id>`, `scene.fx.cut:<de>\|<para>` em arquivos novos (`scene_fx_transition*.rs`) |
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
      "seed": 0
    }]
} }
```

- `id`: mesma regra de ID do restante do `fx` (regex e reservados, case-insensitive: BL082.9), único **entre
  trilhas, clipes e transições**.
- `effect`: precisa ser do catálogo **e** de família `transition`. Look em `transitions[]` e efeito de transição em clipe
  são `WRONG_FAMILY` (D18) com dica da família; nome fora dos dois catálogos é `UNKNOWN_EFFECT`.
- `durationFrames` e `seed`: números JSON **inteiros estritos** (`12.5` e `12.0` recusados no Rust; `NaN` e `Infinity` não
  existem em JSON e o parser os recusa). `durationFrames ≥ 1` e `≤ editDurationFrames`; `seed` em `[0, 4294967295]`
  (uint32, o mesmo `MAX_FX_SEED` do runtime). Nenhum *cast* de ponto flutuante coage um valor inválido.
- `alignment` ∈ {`center`,`start`,`end`}; `handles` ∈ {`use`,`hold`,`refuse`}; `easing` ∈ nomes de easing já aceitos para a
  câmera (a mesma tabela do core); o código do erro é **`UNKNOWN_KEYFRAME_EASING`**, o nome que o core já usa
  (`fx/mod.rs:112`, `read.rs`, `scene_fx_keys.rs`).
- `params` estático (sem chaves), validado pelo catálogo: `curve` ∈ {`smooth`,`linear`} (padrão `smooth`),
  `intensity` ∈ [0,1] (padrão 1); parâmetro desconhecido ou fora da faixa é recusado com o caminho do JSON.
- `seed` é aceito e **preservado** (o `crossDissolve` não sorteia ruído, então não entra no plano: `Reseed::Never`);
  `hidden` booleano; `keys` presente é recusado (D10).
- Chaves desconhecidas dentro de uma transição: **preservadas** na ida e volta apenas onde a política de chaves
  desconhecidas do documento já as preserva (F031); nunca interpretadas.

### 5.2 Importação e validação tipada

Todo o contrato vive no core (`directed_scene::fx`), sem GPUI. **Ordem das conferências por transição** (para o primeiro
erro ser previsível; a S0 confere a ordem contra `fx-explain-transitions.ts` e registra qualquer diferença):

1. estrutura e tipos (objeto, inteiros estritos, `seed` uint32) → recusa com o caminho JSON, no mesmo formato dos demais
   campos de `fx` (a S1a copia o mecanismo de `read.rs`);
2. efeito e família (`UNKNOWN_EFFECT`, `WRONG_FAMILY`), depois parâmetros;
3. easing e enums;
4. existência dos shots (`UNKNOWN_SHOT`, um por shot ausente);
5. adjacência (`TRANSITION_NOT_AT_CUT`);
6. duplicata por corte (`TRANSITION_DUPLICATE`);
7. **duração: `D ≥ 1` e `D ≤ editDurationFrames`** (`TRANSITION_OUT_OF_RANGE`), **antes de qualquer aritmética de janela**;
8. janela, em aritmética verificada (6.1): início < 0, fim > duração da edição **ou overflow** →
   `TRANSITION_OUT_OF_RANGE`;
9. comprimento (`TRANSITION_TOO_LONG`: `|from| < quadrosAntes` ou `|to| < quadrosDepois`);
10. handles (`TRANSITION_NO_HANDLES`, que diz **qual** falta: instância, câmera ou as duas, quantos quadros e o remédio);
11. sobreposição entre janelas (`TRANSITION_OVERLAP`, ordenadas por início, `depois.início < antes.fim`).

Códigos e mensagens seguem a origem (leitura `fx-explain-transitions.ts`) com texto próprio em pt-br/en/es; **nenhum**
código é inventado depois de ver um resultado. O único código novo do Tessel é `FX_TRANSITION_KEYS_NOT_YET` (D10);
`UNKNOWN_TRANSITION` (remoção de ID inexistente) vem de `fx-command-transitions.ts`.

Regras adicionais do Tessel: (1) `FX_LIMIT` para o número de transições (no máximo `shots − 1`, verificado antes do
custo); (2) o importador é **atômico**: qualquer erro recusa o documento inteiro, e o candidato de uma edição é
validado pelo mesmo importador (`scene_edit/fx.rs` já refaz a leitura do candidato); (3) `catalogVersion` **continua 1**:
adicionar um efeito e uma família não muda o significado de nenhum documento existente; um binário anterior recusa o
documento porque `transitions` não vazio é hoje `FX_TRANSITION_NOT_YET` (`read.rs:700–712`; a S1a confere que há teste e o
mantém como controle negativo) e, no plano, pela capacidade (seção 8).

Avisos (nunca recusa, aparecem em `scene.fx.list`, `scene.fx.frame` e no painel): `TRANSITION_ON_CONTINUOUS`
(a junta é `continuous`), `TRANSITION_SPLAT_COST` (os dois planos têm splat; a origem mediu ≈2× o custo de um deck
em um spike próprio, **número da origem, não do Tessel**) e `FX_PASS_BUDGET` (seção 8.2).

### 5.3 Limites e entrada hostil

O documento só carrega `transitions[]` (≤ `shots − 1` itens, cada um com `params` validado pelo catálogo). O risco
recursivo está no **plano**, onde `transition.other` é um `ScenePlan`:

- **Profundidade 1.** O parser do plano (Rust `ScenePlan::from_json_str` e `parseTransition` em JS) leva um indicador de
  profundidade e **recusa `other.transition` sem recursar** (`/transition/other/transition`, "sem aninhamento"). Não
  existe leitura de profundidade 2.
- **Tetos sobre os dois decks somados:** `MAX_TRANSITION_PLAN_BYTES` (tamanho do JSON do plano com `transition`) e
  `MAX_TRANSITION_PLAN_ITEMS` (nós + camadas + splats + passes + recursos + luzes dos dois decks). Os **números** são
  fixados no contrato da S0 (antes do código), são os mesmos no Rust e no JS, e a recusa é `invalid` com o caminho. Plano
  **sem** `transition` não tem teto novo (seus bytes não mudam).
- Testes obrigatórios: JSON aninhado hostil de profundidade 2 e uma "bomba" (`other` repetido), recusados em tempo e
  memória limitados, sem estouro de pilha; `other` corrompido (T28); teto no limite exato e um item acima.

### 5.4 Ida e volta

`fx` é o JSON do documento; `scene_edit/fx.rs` edita o `serde_json::Value` e o importador relê. A ordem de `transitions[]`
é a do documento; `set_transition` anexa no fim; remover preserva a ordem das demais. Teste obrigatório: documento
válido → importar → serializar → bytes iguais ao canônico, e idempotência de importar a saída; cópia-edição-desfazer
devolve **bytes** iguais (o undo guarda o documento, não um diff).

## 6. Tempo

### 6.1 Janela

Seja `C = to.editRange.startFrame` (o quadro do corte), `D = durationFrames`. **Toda a aritmética é em `i64` com
operações verificadas** (`checked_add`/`checked_sub`); overflow é `TRANSITION_OUT_OF_RANGE`, nunca saturação. A ordem é a
de 5.2: `D` inteiro, `1 ≤ D ≤ editDurationFrames`, só então as contas.

| `alignment` | início | fim (exclusivo) | quadros antes do corte | quadros depois |
|---|---|---|---|---|
| `center` | `C − ⌊D/2⌋` | início + D | `max(0, min(fim,C) − início)` | `D − antes` |
| `start` | `C` | `C + D` | 0 | `D` |
| `end` | `C − D` | `C` | `D` | 0 |

Exemplos fixos (C = 48): `center`, D=12 → [42,54) (6 + 6); `center`, D=5 → [46,51) (2 + 3); `center`, D=1 → [48,49)
(0 + 1); `start`, D=12 → [48,60); `end`, D=12 → [36,48).

**`max(0, …)` é semântico.** Nas três alinhamentos o `min(fim,C) − início` nunca é negativo com `D ≥ 1`; o `max(0, …)` é
mantido por fidelidade à origem. Já as **disponibilidades** de quadros reais de um plano (7.2) são `max(0, …)` **por
significado**: um plano pode não ter nenhum quadro real de cauda, e o valor 0 é legítimo, não é underflow nem erro. Elas são
calculadas em `i64`, nunca em tipo sem sinal.

- **Quem desenha o quê.** Nos quadros `[início, C)` o plano do topo é o `from` (o plano sob a agulha) e o deck vizinho é o
  `to`, em **deslocamento negativo** (cabeça); em `[C, fim)` o do topo é o `to` e o vizinho é o `from`, **depois do seu
  fim** (cauda). Daí `quadrosAntes` precisa caber como cabeça do `to` e `quadrosDepois` como cauda do `from`
  (validação `NO_HANDLES` para `use|refuse`) e, nos dois, **dentro do comprimento do plano que os contém**
  (`TOO_LONG`).
- **Adjacência.** `to` é o plano que **segue** `from` na ordem da edição. A junta continua exigindo `transitionInto`
  `cut` com motivo (regra existente de `joins.rs`); uma transição **veste** um corte, não o substitui. Em junta
  `continuous` o aviso `ON_CONTINUOUS` aparece e **nada é bloqueado**.
- **Sobreposição.** Janelas de cortes vizinhos não compartilham quadro. A janela de um corte nunca cruza **outro** corte
  porque as partes dentro de cada plano cabem no plano (`TOO_LONG`).
- **Bordas.** Janela que começa no quadro 0 ou termina em `editDuration` é válida; início negativo ou fim maior que a
  edição é `OUT_OF_RANGE`. D=1 é válida (um quadro de mistura, `p = ease(0,5)`).
- **Ausência.** Sem transição (ou `hidden`), o quadro é o corte seco de hoje; o plano **não ganha** a chave
  `transition` e seus bytes são os de hoje.
- **Alvo apagado.** Documento com `fromShotId`/`toShotId` inexistente: `UNKNOWN_SHOT`, documento recusado (nada de
  transição órfã silenciosa). Sobre **editar** shots, ver 9.5.
- **Relógios do deck vizinho (D17).** Instância e câmera são presas por D14 (`heldInstance`, `heldCamera`); o relógio de
  cena (luz com cintilação) usa o **quadro de edição real** `f`, com o FPS racional existente. Não existe "tempo de
  instância preso" para a luz: a regra vale também em `hold`. T06 cobre as duas coisas; o texto de 7.2 repete a mesma regra.
- **FPS racional.** A janela é em quadros, sem segundos; 24/1, 25/1, 30000/1001 e 60/1 dão a mesma janela e o mesmo
  progresso para os mesmos números de quadro. O teste NTSC fixa isso e fixa também a cintilação do vizinho em `f/fps`.

### 6.2 Progresso (vetores a mão)

`p = ease(easing, (f − início + 0,5)/D)`: o progresso é amostrado no **meio do quadro** (`+ 0,5`). Vetores para
D = 12 (a janela de qualquer corte; só `f − início` importa), calculados à mão com as fórmulas acima; a referência
Python da S0 os regenera e o manifesto os fixa:

| `f − início` | `t` sem easing | `progress` após `easeInOutCubic` | `weight` após smoothstep (`curve=smooth`, `intensity=1`) |
|---|---|---|---|
| 0 | 0,041667 | 0,000289 | 0,000000 |
| 1 | 0,125000 | 0,007813 | 0,000182 |
| 3 | 0,291667 | 0,099248 | 0,027595 |
| 5 | 0,458333 | 0,385127 | 0,330723 |
| 6 | 0,541667 | 0,614873 | 0,669277 |
| 11 | 0,958333 | 0,999711 | 1,000000 |

Simetria `p(k) + p(11−k) = 1` vale nas três colunas. `progress` é o que o **plano guarda**; `weight` é interno ao shader
(`mix(p, smoothstep(p), smooth)` com `amount`), e o plano nunca o guarda. Vetor de cor: canal 200 (A) e 40 (B),
`f − início = 5`, padrão: decodificados 0,577580 e 0,021219, peso 0,330723, mistura linear 0,393579, codificado
**168,39 → 168**. Consequência documentada: com os padrões do piloto (`easeInOutCubic` + `smooth` somados) os três
primeiros e os três últimos quadros de uma janela de 12 são quase um deck puro; é fiel ao piloto e **não** é bug.

**Vetores de `intensity`** (mesmo quadro, canal 200 → 40, `easeInOutCubic`, decodificar → misturar → codificar):

| `curve` | `intensity` 0 | `intensity` 0,5 | `intensity` 1 |
|---|---|---|---|
| `smooth` | 162,4061 → **162** | 165,4320 → **165** | 168,3881 → **168** |
| `linear` | 162,4061 → **162** | 162,4061 → **162** | 162,4061 → **162** |

São vetores **documentais** da fórmula, recalculados em `node` na revisão de 14/11/2026 (checagem de documento, **não**
saída do motor). Dois fatos decorrem deles: `intensity = 0` com `smooth` **não** é a mistura "linear simples" de um
progresso cru, é a mistura pelo `progress` já eased (162, e não 168); e com `curve=linear` os dois `mix` coincidem,
então `intensity` **não faz nada**. Logo o teste `linear/linear` **não** mata o mutante "`amount` ignorado": T07 e T29
exigem os casos `smooth` com `intensity` 0 e 0,5 e **fixam estes vetores antes de qualquer execução**.

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

Novo `evaluate_shot_at_offset(shot, edit_frame)` em `crates/core/src/directed_scene/temporal/shot_offset.rs`, **sem**
alterar `mapping_inside_instance` (estrito). Devolve o mesmo `FrameEvaluation` do avaliador atual, com `instance_frame` e
`camera_clip_frame` presos como em D14 e as bandeiras `held_instance`/`held_camera` (verdadeiras só quando o prendimento
ocorreu). **Relógios (D17):** o `edit_frame` entregue ao relógio de cena (luz, cintilação, clima futuro) é o **quadro real
da edição**; só instância e câmera são presas. Com `hold`, a luz do vizinho continua cintilando em `f/fps`.

**Disponibilidade de quadros reais** (em `i64`, `max(0, …)` semântico, 6.1), por plano:

- cabeça de instância = `instanceStart`; cabeça de câmera = `cameraIn`;
- cauda de instância = `max(0, instDur − (instanceStart + len))`;
- cauda de câmera = `max(0, camFrames − (cameraIn + len))`.

Teste obrigatório com o caso do corpus `valid-cut-instance-offsets` (edição de 100 quadros): `shot-a` em [0,40) com
`instanceStart 10` sobre `inst-a` de 60 quadros dá **cauda de instância = 60 − (10 + 40) = 10**; `cam-a` tem 40 quadros
com `cameraIn 5` e o plano tem 40, então a câmera **já passa do fim do clipe dentro do plano** e a **cauda de câmera é
`max(0, 40 − 45) = 0`**. O corte está em C = 40. `center` D=12 com `handles: use` exige 6 quadros depois do corte em
`shot-a`: do lado de `shot-a` falta a **câmera** (6) e a instância não (10 ≥ 6), e `NO_HANDLES` diz "camera clip" e o
remédio para esse lado; `shot-b` começa no quadro 0 da instância e da câmera, então também falta cabeça nos dois (6), e o
erro lista os dois lados. Com `hold`, o vizinho usa os quadros reais que existem e prende só o que falta, instância e
câmera **separadas**. Um caso de "só a câmera falta" exige uma variante do bundle (instância mais longa), registrada na S0.

### 7.3 Plano de dois decks

Princípio: o **topo** do `ScenePlan` continua sendo o deck do plano sob a agulha (consumidores atuais, gizmos, lista de
recursos e `scene.fx.frame` não mudam); o vizinho é aditivo. Chave aditiva `transition`, **ausente** quando não há
transição (digests de planos sem transição ficam idênticos; teste de ouro sobre os planos existentes do corpus):

```json
"transition": {
  "id": "tr-1", "effect": "crossDissolve",
  "fromShotId": "shot-a", "toShotId": "shot-b", "primary": "from",
  "startFrame": 42, "durationFrames": 12, "frameInWindow": 5,
  "easing": "easeInOutCubic", "progress": 0.385127, "amount": 1.0,
  "params": [["curve","smooth"],["intensity",1.0]],
  "heldInstance": false, "heldCamera": true,
  "other": { /* ScenePlan do deck vizinho */ }
}
```

- `other` é um `ScenePlan` (mesmo parser e serializador; **sem** segundo modelo de plano), validado por `validate_deck`:
  `version`, `output` e `fps` iguais aos do topo; `other.frame.shot_id` = o vizinho; `other.frame.edit_frame` = o do topo
  (quadro real); `other.transition` ausente (profundidade 1, 5.3); `other.fx` só com passes `shot` do vizinho.
- **Recursos: cada deck mantém a sua própria tabela.** `other.resources` **não** é vazio: é a tabela do vizinho,
  validada pelo **mesmo** validador atual (ordenada por `assetId`, `used_by` = ids de nó **do próprio deck**). Os IDs de nó
  **colidem entre decks**: no corpus `valid-cut-instance-offsets`, `scene-a` e `scene-b` têm os mesmos
  `root/backdrop/figure` e os mesmos assets, e `plan_resources` ordena por `assetId` num `BTreeMap`. Por isso **não existe,
  nem se inventa, um identificador de recurso que atravesse decks**: a referência é sempre o par (deck, id de nó) e o
  recurso é identificado por `assetId`.
- **União derivada, não serializada.** Para `requires`, preflight **e todos os carregadores do host**, `ScenePlan::resource_union()` (função pura,
  fora do JSON) devolve `BTreeMap<assetId, UnionResource>` na mesma ordem estável por `assetId`, com
  `used_by: Vec<{ deck: "primary" | "other", node_id }>` **qualificado por deck** (ou sem cruzamento: nunca ids de nó crus
  misturados). Se o mesmo `assetId` aparece nos dois decks com `revisionId`, `contentHash` ou caminho diferentes, o plano é
  inválido (`/transition/other/resources`, "mesmo `assetId`, conteúdo diferente"); com a mesma cena isso é impossível por
  construção e o validador se defende mesmo assim. O runtime lê cada deck na **própria** tabela, pelo `assetId`.
- `requires` do topo = união ordenada de `requires` do topo, de `other.requires` e de `fxTransition`
  (`derive_requires` estendido; um runtime que recusa capacidade desconhecida vê tudo que precisa). **Teste obrigatório
  com nós e assets colidentes** (o corpus acima): `resource_union` ordenada, `used_by` qualificado, `requires` completo.
- **Carregadores do host (R2/R1).** `SceneResourceMap::from_root` e o `resolve` do renderer
  (`crates/render/src/scene.rs`), `approved_map` (`crates/render/src/scene_operation.rs`) e
  `resource_map` do export (`crates/render/src/scene_export/mod.rs`) iteram **resource_union()**,
  em vez de apenas `plan.resources`. Os quatro caminhos pertencem a S2 e são condição de S2A.
  T10 e T18 incluem um asset usado **apenas pelo other**, além de ids de nós colidentes: quando presente,
  monitor/render/export resolvem e desenham os dois decks; quando ausente, recusam antes de desenhar/exportar,
  com identificação do deck. Uma fixture em que ambos usam os mesmos assets não prova esse caminho.
- `progress` é o valor **já com easing**; o runtime não calcula easing nem decide regra. O validador recomputa
  `ease(easing, (frameInWindow+0,5)/durationFrames)` e confere com tolerância 1e-9 (`/transition/progress`), confere
  `frameInWindow = frame.edit_frame − startFrame ∈ [0, durationFrames)`, `amount = clamp01(intensity)` (**não** a regra
  de `expected_amount` dos looks), `params` na ordem e nos tipos do catálogo (`parse_param` com `Choice`) e
  `primary` coerente com o lado do corte (`from` em `< C`, `to` em `≥ C`).
- **Estágios** (`check_fx` estendido): lista do topo = passes `shot` do topo, depois `output`; passes `shot` do
  topo nomeiam o `frame.shot_id`; passes de `other.fx` nomeiam `other.frame.shot_id`. Um clipe aparece **uma vez por
  quadro** entre os dois decks.
- **Alpha (D20).** A saída da mistura é **opaca** (`alpha = 1`, como o shader da origem e o resto do renderer). Não se
  introduz `UNSUPPORTED_ALPHA`: é divergência de veredito declarada (Q3). O caso de pixels parcialmente transparentes
  (T33) fixa o comportamento: cada deck é composto como o plano puro faria (opaco) e **depois** misturado.
- **Cor.** Alvos de 8 bits codificados em sRGB; decodificar → misturar → codificar (D08). Perfis `rec709-video` já são
  recusados pelo documento.
- **Referência independente (X).** `scripts/pilot-dissolve-corpus.py` (stdlib) transcreve da **fonte TS** (não do Rust):
  janela, progresso, escolha dos quadros dos decks (D14/D15) e a mistura por pixel; gera `dissolve-cases.json` e
  `dissolve-manifest.json` com os hashes de blob da origem lida (seção 2) e a proveniência de cada expectativa. Os
  decks texturizados da referência são os **8 bits do runtime** em quadros puros **realmente equivalentes**
  (T08 prova que esse plano é o de hoje), e a mistura é calculada em Python sobre esses bytes (D08, seção 12).
  Isso **não** serve para hold misto (instância presa, câmera correndo) nem para o relógio da luz em f: podem
  não corresponder a nenhum quadro puro. Para esses casos, S0 fixa estados temporais esperados e sondas
  analíticas independentes de cenas simples (marcadores/cores/luz conhecidos), quantizados a 8 bits; S2 compara
  a projeção/cor dessas sondas e o plano a X. Sem esse oráculo, o caso continua A validar e bloqueia seu aceite;
  não fabricar um quadro puro equivalente nem obter ambos os lados pelo mesmo avaliador de transição. O teste Rust
  `crates/core/tests/pilot_dissolve_corpus.rs` **só lê o JSON** (roda no Windows sem Python), como `pilot_fx_corpus.rs`.
- **Nenhum `Domain` novo.** `PlanError::Unsupported` só aceita `Domain::ALL` (13 valores: `weather` … `relief`) e a
  transição **não é** um domínio não avaliado: com o portão (8.4) o plano nunca vê uma transição antes de poder
  desenhá-la, então não há erro intermediário a tipar.

## 8. Capacidade, versão, runtime, portão, ciclo de vida e export

### 8.1 Capacidade e versão

- `Capability::FxTransition` (`"fxTransition"`, 14ª em `Capability::ALL`, hoje 13), exigida **só** por plano com
  `transition`. Nome escolhido por coerência com `fxShotScope`/`fxNoise`; o `transitionBlend` do rascunho de F016 fica
  superado. Entra no Rust na S1b; **o espelho `SCENE_CAPABILITIES` em `directed-scene.js` só ganha `fxTransition` na S2**,
  junto do desenho (o teste de espelho confere que cada capacidade do JS é reconhecida pelo Rust, então a S1b não o quebra,
  e o runtime nunca anuncia o que não desenha).
- `PLAN_VERSION` **continua 1** (chave aditiva, ausente quando não usada), `FX_CATALOG_VERSION` **continua 1**.
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

- `parseDirectedLayer` lê `plan.transition` por `parseTransition` (módulo novo `directed-transition.js`): valida
  chaves/tipos, **profundidade 1 e tetos de bytes/itens (5.3)**, `effect ∈ TRANSITION_EFFECTS` (lista espelhada pelo fixture
  do catálogo), `primary`, `progress`/`amount` finitos em [0,1], inteiros estritos, `other` pelo mesmo parser de deck e
  recusa o resto com `invalid` e o caminho (como já faz com passe de plano).
- Desenho: deck do topo → alvo `primary`; deck vizinho → alvo `other` (ambos via `sceneTarget`, chaves
  `ds:<ordem>:primary|other`); mistura com `SRGB_CURVES` → alvo da cadeia de passes (ou direto no alvo da camada
  quando não há passe de saída) → passes de saída existentes. A ordem em `prepareDirectedScene` (~l.877–964) deve ser
  reconciliada na S2; o desenho não depende de nenhum estado do quadro anterior.
- **Alvos retidos (`keep`) e cache.** O alvo do vizinho conta no orçamento de retenção configurável (F-viewport) como os
  demais. **Cada deck** tem chave de cache **própria** (digest do plano do próprio deck + tamanho + amostras): trocar uma
  propriedade do plano **vizinho** com a agulha na janela **redesenha** (T19). **O resultado da mistura nunca é cacheado
  só pelo digest dos decks:** a chave da mistura inclui os dois digests **e** `progress`, `amount`, `curve` e `easing`.
  Mudar só `intensity` com decks idênticos **remistura sem redesenhar os decks** (contadores de redesenho dos decks
  inalterados, contador de mistura +1, pixel muda); isso é T31 e mutante de T29.
- **Resize.** Alvos recriados no novo tamanho; o quadro após o resize é o de um render novo naquele tamanho.
- **Perda de contexto.** Sem histórico temporal, o próximo quadro redesenha os dois decks do zero; o quadro após a
  recuperação é igual ao de antes da perda (hash).
- **Splats e leases (T30).** Transição splat→splat usa os dois leases. Quando a junta é `continuous` (os dois decks usam a
  **mesma** instância e o **mesmo** splat) a contagem de leases e de ordenação não duplica trabalho nem libera cedo: teste
  com contagem antes, durante e depois da janela.
- **Erro de um deck** (recurso ausente, `splatLease` não pronto, textura inválida): erro tipado da camada com
  `deck: "primary"|"other"`, **nunca meia mistura**; o caminho de erro existente do host mostra a causa.
- **Dispose/descarregar.** Sair da camada, trocar o plano, perda do contexto ou `dropFxPipeline` libera, **dos dois decks**,
  alvos, **texturas, materiais e geometria**, e as locações de recurso do vizinho; teste de contagem de cada categoria antes
  e depois, e nos casos de contexto perdido e de erro no meio da janela.
- **Seek e export.** Plano puro por quadro: seek aleatório e sub-intervalo do export reproduzem o quadro da sequência
  completa. Export de um sub-intervalo que **começa no meio** da janela (ex.: [44,52) com janela [42,54)) deve dar
  quadros idênticos aos mesmos números do export completo (comparação em quadros brutos antes do codec; o H.264 só é
  comparado contra a referência com tolerância declarada, seção 12).

Os quatro carregadores citados em 7.3 são parte deste estágio: monitor (`approved_map`), renderer
(`from_root` e `resolve`) e export (`resource_map`) resolvem a união derivada por assetId. A ativação
S2A exige T10/T18 com asset exclusivo do vizinho presente e ausente, em P/R/E.

### 8.4 Portão público: nada consome uma transição antes de poder desenhá-la

**Estratégia escolhida (D19): recusar publicamente até a ativação (S2A), com controles negativos por consumidor.** A
alternativa, preflight de intervalo inteiro mais recusas tipadas em todo consumidor, foi descartada: espalha a regra por
dez superfícies, e qualquer uma esquecida seria um corte seco silencioso ou um export que falha no quadro 34.

- **Um só ponto de estrangulamento: a admissão do documento.** A cena nasce por um único caminho de leitura. A S1a
  acrescenta à leitura uma **política de admissão** (`transitions: Refuse | Accept`, nome definitivo na S1a) que é
  **gravada na cena** e herdada pelo candidato de toda edição. A entrada **pública** (`scene.import`, anexar, abrir
  projeto, pacote, intercâmbio, o candidato relido de `scene.edit`/`scene.fx.edit`) usa `Refuse` e devolve
  `FX_TRANSITION_NOT_YET`, **como hoje** (`read.rs:700–712`). Uma cena admitida sob `Refuse` **nunca contém** uma
  transição; portanto plano, monitor, export, preflight, `scene.fx.*` e o inspector não precisam saber que elas existem.
  **A S1a enumera por leitura todo caminho que constrói uma cena** (importar, anexar, abrir, pacote, intercâmbio, candidato
  de edição), lista-os no relatório e prova, caminho por caminho, que passam pela mesma leitura; um caminho que contorne a
  admissão é achado **bloqueante** da S1a, porque a garantia do portão depende de haver um só.
- **A entrada interna** (`Accept`) é só para testes e corpus das fases S1a–S3a: `#[doc(hidden)] pub`, porque os testes de
  integração de `crates/core`, `crates/render` e o corpus precisam chamá-la. Nenhum adaptador (desktop, headless, `ops`,
  media, render de produção) a chama: um **teste de fronteira** que varre as fontes falha se um deles referencia o símbolo.
  Esta política é um **gate de compatibilidade e ativação dos adaptadores suportados**, não uma barreira
  de segurança: doc(hidden) pub continua chamável por outro crate. O contrato de recusa vale para as
  entradas normais enumeradas acima; a API de ensaio deve declarar que não é uma entrada suportada de produção.
- **Catálogo e seletor não mudam antes da S2A.** A tabela de transições é separada do `CATALOG` dos looks (seção 4), o
  seletor de efeito de clipe continua oferecendo só os 3 looks e `scene.fx.catalog` não lista `crossDissolve`;
  `REASON_FAMILY_NOT_YET` (`fx_inspector.rs:727`) e `refused: {transitions}` (`scene_project/fx.rs:58–60`) ficam como estão.

| Consumidor | Até a ativação (S1a–S3a) | Controle negativo (T32) | Depois da S2A |
|---|---|---|---|
| Importador (`scene.import`, anexar, abrir, pacote, intercâmbio) | recusa com `FX_TRANSITION_NOT_YET` | L, H e G | aceita; round-trip por bytes |
| `scene.edit`, `scene.fx.edit` (candidato relido) | candidato com transição recusado pelo mesmo código; ops de transição recusadas pela mesma via | L e H | as ops da S3a editam |
| `scene.fx.list\|frame\|inspect\|catalog` | `refused: {transitions}`, catálogo sem `crossDissolve` | L e H | listam transições; `refused` perde `transitions` |
| `scene.plan`, `scene.render`, monitor | nunca recebem transição; planos idênticos (T08) | L e R | plano de dois decks |
| `scene.export` e preflight | idem | L e R | preflight por intervalo (abaixo) |
| Inspector e seletor de efeito de clipe | só os 3 looks | G | `WRONG_FAMILY` (D18) |

**Condições para a S2A** (depende de S2 **e S3a**; ela só abre o portão se todas valerem; senão as células ficam "A validar" e o portão fecha):

1. o runtime e o renderer desenham a janela (T12, T15–T19, T30, T31, T33 em P e R) e o export de quadros brutos é igual
   entre o export completo e o sub-intervalo (T27, E);
2. o **preflight do export cobre o intervalo inteiro**: `requires` de todos os planos do intervalo, incluindo o `other` de
   cada quadro de janela. Um runtime sem `fxTransition` falha **no preflight, antes do primeiro quadro**, e um recurso
   que só o vizinho usa e está ausente também (o preflight de recursos é por `assetId` e do bundle inteiro, F038; a S2A
   prova com teste que o do vizinho está coberto e reconfere como o preflight do export lê `requires` hoje);
3. a leitura pública (`H`) reproduz as células L das fases S1a/S1b (T01–T03, T05–T11, T13, T14, T28) por
   `tessel-headless` real;
4. a prova negativa se inverte: cada linha acima passa de "recusa" para "aceita" no mesmo commit que remove a política e
   `FX_TRANSITION_NOT_YET` (que passa a não ter produtor; o controle do **binário anterior** continua em T15 e em T01).

**Estado intermediário honesto.** Depois da S2A e antes da S4b o documento com transição é admitido, desenhado e exportado,
e editável por JSONL (S3a), mas o painel de efeitos **não** tem controle de transição (lacuna registrada, sem controle
desabilitado de enfeite) e um documento com transição **não** quebra o painel dos clipes (teste G da S2A).

## 9. Edição

### 9.1 Operações (core, `scene.fx.edit`)

Lote atômico, uma entrada de undo, `dry_run`, `base_revision`: **as mesmas regras de F016/F062**. Novas operações no
`parse_fx_batch` (forma no `ops.rs`, regra no core):

| Op | Campos | Efeito |
|---|---|---|
| `set_transition` | `fromShotId`, `toShotId`, `effect`, `durationFrames?`, `alignment?`, `handles?`, `easing?`, `params?`, `id?`, `hidden?` | *Upsert* por corte; nova usa D06; existente preserva `id` e o que a op omite |
| `resize_transition` | `id`, `durationFrames`, `alignment?` | Muda janela; recusa pelas regras de 5.2 |
| `set_transition_param` | `id`, `param`, `value` | `curve` (escolha) ou `intensity` (número, passo e faixa do catálogo). Com `curve: linear` a op é aceita e o valor é gravado (sem efeito visual; o inspector marca o campo como inerte, 9.3) |
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
gesto obsoleto. (O gesto de **overlay** que entra na janela é o caso de 9.6, outra fase.)

### 9.3 Inspector

`scene_edit::fx_transition_inspector` (módulo novo) entrega o modelo de campos (`TransitionInspector`): `origem` e
`destino` (somente leitura, derivados do corte), `durationFrames` (passo 1, mín 1, máx `maxTransitionFrames`), `alignment`,
`handles`, `easing`, `curve` (escolha nomeada de `fx_choice.rs`), `intensity`, `hidden`, `at_default` e **reset por
campo**; rótulos pt-br/en/es no core, com teste sobre os IDs oferecidos (regra de F057). **Inércia vem do core:** com
`curve = linear` o campo `intensity` traz `inert: curve_linear` (o desktop só mostra a nota, não calcula a regra). O
rótulo da UI é **"Suavização"** (o campo do documento continua `intensity`): "Intensidade" sugeriria força de mistura,
e ela só mistura a forma suave com a linear. `scene.fx.inspect {transition|cut, probe}` valida um valor candidato sem
editar (é o "dry-run por campo" que a UI usa).

### 9.4 Lista de problemas (UI e CLI)

`scene.fx.inspect {cut, probe}` e `scene.fx.edit {dry_run}` devolvem `errors[]` tipados `{code, message, hint, transitionId?,
shotId?, details}` (a **mesma** estrutura do importador), em uma só função do core; a UI apenas mostra. Não há texto de erro
montado no desktop. **O texto das três línguas mora no core** (a pendência BL082.56 é o braço padrão silencioso de
`reason_text`): um teste enumera **todos** os códigos novos (`TRANSITION_*`, `WRONG_FAMILY`, `FX_TRANSITION_KEYS_NOT_YET`,
`UNKNOWN_TRANSITION`, `transition_window`) e exige texto em pt-br, en e es, sem braço padrão.

### 9.5 Shot apagado ou encurtado

Não existe hoje comando de editar `shots[]` (P5/BL082.5). A P4b entrega o **contrato**: (a) uma mudança de documento
que apague um shot citado ou torne uma janela inválida (shot encurtado, não vizinho, handles insuficientes) é **recusada
pelo importador do candidato** com o código da regra violada, **sem cortar a transição**, como BL082.44 faz com clipes `shot`;
(b) quando a P5 existir, ela reavalia as transições (e os clipes `shot`) do lote **antes** de aplicar e propaga a recusa;
(c) teste da P4b: substituir o documento da cena por outro sem o shot (`scene.import`/anexo) é recusado e nada muda.
Isto fica como dependência de P5, não como trabalho aqui.

### 9.6 Edição e overlays durante a janela (D16, Q1)

**Um predicado do core decide tudo.** Módulo novo `directed_scene/fx/window_guard.rs`, irmão de `FxClip::handles_refusal`
(F061, a "única decisão" de ter alças):

```rust
// Esboço: nomes definitivos na S3a. Pura: depende só do documento e do quadro.
pub enum WindowRefusal { TransitionWindow { transition_id: String, start_frame: i64, end_frame: i64 } }
impl Fx { pub fn overlay_refusal(&self, edit_frame: i64, target: OverlayTarget) -> Option<WindowRefusal>; }
```

- **Consumidores, todos chamam o mesmo predicado** (nenhum recalcula a janela): o *hit-test* do gizmo e da alça
  (`scene.gizmo.hit`), o *press* que abre o gesto, o desenho das alças e o guarda de gesto aberto (abaixo). **Equivalência:**
  `scene.gizmo.hit` responde recusa **se e somente se** o press recusa, em todo quadro de `início − 1` a `fim + 1` (teste
  por quadro, como F061 já exige de `fx_handle`).
- **Gesto aberto fora da janela que entra nela** (seek, reprodução, avanço de quadro): o guarda reavalia o predicado a cada
  mudança de `edit_frame`; se agora há recusa, o gesto é **cancelado com o estado restaurado** e relatado com o código
  tipado `transition_window`; um `commit` posterior é recusado como gesto obsoleto (9.2). Nunca fica um gesto aplicado a um
  deck que não é o que se vê.
- **Alvo de edição durante a janela.** Uma propriedade (inspector, `scene.edit`) vale quando o alvo é **explícito**: a
  entidade pertence ao plano do topo, ou o chamador informa o quadro e ele está dentro do `editRange` do plano **dono** da
  entidade. Uma entidade do deck vizinho **sem** quadro explícito válido é recusada com um código **tipado** (proposto
  `neighbor_deck_frame_out_of_shot`), **nunca** uma chave gravada fora do `editRange` do plano dela. A S3a confere como o
  dono de uma entidade é resolvido hoje e registra a regra; edição por propriedades sem chave não depende do quadro.
- **Reversibilidade (Q1).** Aceitar alças sobre o deck do topo é trocar o corpo de **um** predicado e inverter os testes
  de T26; nada espalhado.

## 10. Persistência, pacote e intercâmbio

Tudo desta seção depende do portão aberto (S2A); antes dele, a admissão recusa (8.4).

- **Salvar/reabrir:** `fx.transitions` está no JSON da cena do projeto, junto do `fx`; salvar → fechar → reabrir →
  `scene.fx.list` idêntico; undo/redo depois de reabrir funciona sobre o documento reaberto.
- **Pacote** (Directory/Archive) e **intercâmbio** (`project.import_scenes`): o documento da cena viaja inteiro; teste:
  pacote → abrir sem a origem → mesmos bytes de `fx` e mesmo quadro de janela (pixels no renderer).
- **Compatibilidade:** documento sem `transitions` (ou `[]`) não muda; a poda de `fx` vazio de `scene_edit/fx.rs` continua.
  Binário anterior + documento com transição: `FX_TRANSITION_NOT_YET` (falha fechada).
- **Export:** o `snapshot.rs` congela o documento no pedido; uma edição durante o export **não** vaza; o export desenha
  as janelas pelo mesmo plano (seção 8.3).
- **Relink/recursos:** nenhum recurso novo; o manifesto é por `assetId` e do bundle inteiro, então já cobre os recursos dos
  dois planos (7.3). O gerenciamento de dependências (BB06) não muda.

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
| Suavização (`intensity`) | `Slider` | `set_transition_param` (gesto) |
| Oculta | `Switch` | `set_transition_hidden` |
| Remover | `Button` | `remove_transition` |
| Lista de erros e avisos | texto por item (`code` + mensagem + dica) | só leitura de `errors[]` e avisos |

- Prévia: arrastar abre o gesto e o monitor mostra o **candidato** (como a P2/P3); soltar confirma, Escape cancela.
- **Quando desabilitar (H2).** Um controle só fica desabilitado quando a operação real **não pode** executar, e o motivo
  (`disabled_reason`) **vem do core**, nunca do desktop. Motivos legítimos: o corte não existe (a edição tem um só plano),
  a cena não está admitida ou a revisão ficou obsoleta (os motivos que o painel de efeitos já usa), e nenhum efeito de
  transição disponível no catálogo (não ocorre com `crossDissolve`). **Não são motivo de desabilitar:** junta `continuous`
  (D12: aviso `TRANSITION_ON_CONTINUOUS` e o controle continua habilitado); falta de handles (é **recusa** da operação,
  vira `errors[]` com o remédio "troque para `hold`", e o `Select` de handles continua habilitado); "só há uma transição
  por corte" (o `set_transition` faz *upsert*, D02); `intensity` com `curve: linear` (campo marcado como inerte, 9.3).
  Nunca callback vazio.
- Janela de diálogo: **nenhum**; a seção é inline, então `Workflow::has_dialog` **não se aplica**. Se a S4b introduzir
  qualquer diálogo (por exemplo confirmar remoção), ele marca `has_dialog` para esconder a WebView.
- **Overlays durante a janela (D16, 9.6):** o gizmo e as alças são recusados pelo predicado do core e o monitor mostra o
  aviso `transition_window`; editar por propriedades continua permitido com alvo explícito.

### 11.2 Nós, parser e handlers compartilhados

Cada controle é `Command` → `Workstation::execute*` (regra do `AGENTS.md`): o parser semântico compartilhado vive em
`crates/core/src/commands.rs` (o `desktop/automation/commands.rs` só o reexporta), os nós de estado em `state/tree.rs`
(uma entrada por nó, corpo em módulo novo) e os nós de painel no desktop (`workstation/scene_fx_transition*.rs`), todos
com **o mesmo handler** que lib e JSONL usam (`fx_run_guarded`, `fx_gesture`). Nós propostos:
`scene.fx.transitions` (lista de cortes), `scene.fx.cut:<fromShotId>|<toShotId>` (corte, com `set effect`) e
`scene.fx.transition:<id>` (campos `duration`, `alignment`, `handles`, `easing`, `curve`, `intensity`, `hidden`, ações
`remove`, `reset:<campo>`), mais `scene.fx.budget` (já existe, passa a refletir os dois decks). Não existe "ação só da
CLI": todo caminho de `scripts/ui` chama o mesmo handler da UI, e um teste GPUI e um teste de JSONL provam o **mesmo**
`EditSummary`/entrada de histórico. **Divisão de ownership:** o que é do core (ops, parser, nós de estado) é da S3b; a
S4b só acrescenta os nós de painel e a apresentação.

## 12. Aceite por superfície, referência independente e controles negativos

Cada linha é um caso com ID estável (`T..`) no teste **ou** no corpus. **Cada célula tem um dono de fase** (códigos:
`0` S0 · `1a` S1a · `1b` S1b · `2` S2 · `2A` S2A (ativação) · `3a` S3a · `3b` S3b · `4a` S4a · `4b` S4b · `5` S5).
Célula vazia = a superfície não se aplica ao caso. Uma célula que **não** for executada no fim da sua fase fica **A validar**
na PR dela, nunca verde por associação. `○` marca **controle negativo** (recusa pública do portão, 8.4) e `→2A` que a
mesma célula se inverte na ativação: **antes** da S2A, um H positivo seria falso, porque a admissão recusa.

| ID | Caso | L | H | G | W | P | R | E | X |
|---|---|---|---|---|---|---|---|---|---|
| T01 | Documento mínimo válido, ida e volta em bytes, idempotente | 1a | 2A | | | | | | |
| T02 | Cada recusa: efeito/família (`WRONG_FAMILY`), easing, parâmetro, shot ausente, não vizinho, duplicata, duração 0 e não inteira, janela fora, longa, sem handles (só instância / só câmera / ambas), sobreposição, `keys`, ID repetido/reservado, `catalogVersion` novo, limite | 1a | 2A | 4b | 4b | | | | |
| T03 | Avisos: `ON_CONTINUOUS` e `SPLAT_COST` (1a), `FX_PASS_BUDGET` com dois decks (1b) | 1a, 1b | 2A | 4b | 4b | | | | |
| T04 | `hidden` e ausência = corte seco com os bytes do plano de hoje | 1b | 2A | | | 2 | 2 | | 0 |
| T05 | Janela: tabela de alinhamento/duração (par, ímpar, D=1, `start`, `end`), bordas 0 e fim da edição, −1 e +1 inválidas | 1a | 2A | | | | | | 0 |
| T06 | FPS racional 24, 25, 60, 30000/1001: mesma janela/progresso; **relógio de cena do vizinho em `f/fps`** (luz com cintilação continua cintilando em `hold`) | 1b | 2A | | | | 2 | | 0 |
| T07 | Progresso: tabela de 6.2; `linear/linear`; **`curve=smooth` com `intensity` 0, 0,5 e 1 (162, 165, 168 no vetor de 6.2)**; `linear` com os três valores iguais (controle que **não** discrimina `amount`) | 1b | 2A | | | | | | 0 |
| T08 | Planos **sem** transição idênticos em bytes e digest ao `main` (corpus existente) | 1b | 2A | | | | 2 | | |
| T09 | Plano na janela: topo/vizinho/`primary`, `requires`, capacidade, JSON ida e volta; cada adulteração recusada (progresso, vizinho, passe de saída no `other`, ordem, recurso do `other` adulterado, `assetId` repetido com conteúdo diferente) | 1b | 2A | | | | | | |
| T10 | `hold`: instância e câmera independentes; `use` com quadros reais; caso do corpus `valid-cut-instance-offsets`; **ids de nó e assets colidentes entre decks** (`resource_union`, `used_by` por deck); asset exclusivo do other presente e ausente em todos os quatro carregadores | 1b | 2A | | | 2 | 2 | 2 | 0 |
| T11 | Clipe `shot` que encosta na borda segura o quadro; clipe no meio não entra | 1b | 2A | | | | 2 | | 0 |
| T12 | Passe de saída cobrindo a janela age **depois** da mistura; ordem observável em pixel | 1b | | | | 2 | 2 | | 0 |
| T13 | `filmGrain` de saída: mesma semente por quadro com e sem transição | 1b | 2A | | | | 2 | | 0 |
| T14 | Orçamento conta os dois decks e a mistura | 1b | 2A | 4b | | | | | |
| T15 | **Runtime sem `fxTransition`** recusa `unsupported`+`missing`, não desenha | | | | | 2 | 2 | | |
| T16 | Pixel: decks de cor constante (200 → 40 canal, `f−início = 5` → 168 dentro da tolerância da seção 12) e texturizados, por quadro da janela | | | | | 2 | 2 | | 0 |
| T17 | Resize e perda de contexto no meio da janela; quadro depois = quadro de render novo | | | | | 2 | 2 | | |
| T18 | Erro de um deck (recurso exclusivo do other ausente nos quatro carregadores): erro tipado `deck`, sem meia mistura; **dispose** libera alvos, texturas, materiais, geometria e locações dos dois decks | | | | | 2 | 2 | 2 | |
| T19 | Seek aleatório = sequencial (hash); invalidação do cache ao mudar o **vizinho**; alvos retidos dentro do orçamento | | | | | 2 | 2 | | |
| T20 | Edição: add (padrões), substituir no mesmo corte (ID preservado), resize, parâmetro (inclusive `intensity` com `linear`), oculta, remover; **uma** entrada de undo cada; `dry_run` não muda nada; no-op sem entrada; lote com erro inalterado; `stale_revision`; **texto pt-br/en/es de todo código novo** | 3a | 3b | 4b | 4b | | | | |
| T21 | Gesto: begin/update/commit ≡ `scene.fx.edit`; cancel; invalidação (revisão, transição trocada/removida, shot sumiu) | 3b | 3b | 4b | 4b | | | | |
| T22 | Substituir o documento sem o shot / encurtar o shot é **recusado** e nada muda | 3a | 3b | | 4b | | | | |
| T23 | Salvar → reabrir → undo/redo; pacote e intercâmbio preservam `fx.transitions` e o quadro | 3b | 3b | | 4b | | 3b | | |
| T24 | UI sintética (GPUI): seção, seletores, sliders, switch, remover, lista de erros pt-br/en/es, prévia, `disabled_reason` (só os motivos legítimos de 11.1; `continuous` **habilitado** com aviso) | | | 4b | | | | | |
| T25 | Janela real por `scripts/ui` (sem coordenadas): `tree/get/set/invoke` dos nós de 11.2, sondas de pixel nos quadros da janela contra a referência, recusa tipada | | | | 4b | 4b | | | 0 |
| T26 | **Predicado de janela:** puro (3a); fiação em gizmo e alças; `scene.gizmo.hit` ≡ press por quadro; gesto aberto que entra na janela por seek/play é cancelado; alvo do deck vizinho com quadro explícito válido/inválido; alças e gizmo voltam fora da janela | 3a, 4a | 4a | 4a | 4a | | | | |
| T27 | Export FFmpeg: sub-intervalo que começa no meio da janela = quadros brutos do export completo (2); vídeo comparado à referência com a tolerância do codec **declarada** (5) | | | | | | 2 | 2, 5 | 0 |
| T28 | Corrupto/limites: JSON truncado, tipos errados, `NaN`, nomes unicode, contagens enormes, janela do tamanho máximo, **`other.transition` de profundidade 2 e "bomba"**, `other` corrompido, teto de bytes/itens no limite e acima | 1a, 1b | 2A | | | | | | |
| T29 | **Mutantes** (corredor `mutate.py`): `+0,5` do progresso, easing ignorado, decks trocados, prendimento junto de instância e câmera, saída antes da mistura, `⌊D/2⌋`→`⌈D/2⌉`, `start`/`end` trocados, capacidade omitida, `amount` ignorado, **vizinho sem invalidar cache**, **mistura cacheada só pelo digest dos decks**, **lease do splat contado errado**; **todos mortos** | 5 | | | | 5 | 5 | | |
| T30 | **Splat → splat** e junta `continuous` (mesma instância e mesmo splat): leases, ordenação, custo declarado | 1b | | | | 2 | 2 | | |
| T31 | **Cache da mistura:** mudar só `intensity` (ou `curve`) com decks idênticos remistura **sem redesenhar** os decks | | | | | 2 | 2 | | |
| T32 | **Portão público (8.4):** cada consumidor recusa com `FX_TRANSITION_NOT_YET` antes da ativação e aceita depois; fronteira (nenhum adaptador chama a entrada interna); seletor de clipe só com os 3 looks; planos e catálogo sem mudança | 1a○,3a○→2A | 1a○,3a○→2A | 1a○→2A | | | 2○→2A | | |
| T33 | **Alpha (D20):** bundle com `renderProfile.alphaMode: "straight"` aceito e desenhado **opaco**; deck de pixels parcialmente transparentes; mesma política dos 3 looks, registrada como divergência | 1b | | | | 2 | 2 | | 0 |

**Tolerâncias** (**contrato fixado no manifesto da S0 antes de qualquer execução do motor; não se mexe depois**).
A referência `X` toma como decks os bytes de 8 bits do **próprio runtime** para cada plano puro e calcula a mistura em
float de 64 bits; o resultado esperado é a conta **arredondada** (a única diferença legítima é o arredondamento da GPU):

- **Monitor e renderer:** diferença **máxima de 1 código (1/255)** por canal **e** viés médio com sinal
  `|média(Δ)| ≤ 0,25` código sobre todas as sondas do quadro, para um erro sistemático não ser engolido pelo 1/255. Há
  precedente em BL018 ("pixels dentro de 1/255"). O limite não vem de pior caso observado desta feature.
- **Export H.264:** **não** se calcula tolerância a partir do pior valor observado. Importa-se **só** o limite já declarado
  em F016 (`effects.md`: tolerância 16 sobre o baseline decodificado, com o requisito de que a maior mudança do mutante
  a ultrapasse, "maior mudança 69 — discrimina"), com o racional (quantização do codec e subamostragem de croma). A S0
  reconfere o texto vigente em F016/F062 e o copia no manifesto com a data; sem esse limite declarado, o resultado é
  **A validar**, nunca um número escolhido depois.
- Se a diferença exceder, o resultado é **FAIL**, não ajuste.

**O que cada conjunto não prova:** o golden da origem não foi comparado (paridade é "por fórmula", sobre 8 bits, e a
diferença de half-float da origem é declarada); nada aqui mede desempenho, GPU real, ponteiro físico, WebView nativa do
Windows ou áudio ouvido; humano não aceitou a aparência (BL058).

## 13. Fases, dependências e ownership

Cada fase é uma PR revisável. "Gate" é o que a PR precisa comprovar **para si**; nenhuma fase altera `docs/backlog.md`,
`docs/features/README.md` ou `docs/README.md` por conta própria: **o implementador entrega ao coordenador o patch das
linhas** (inclusive a linha da matriz) e o coordenador as integra. Com o portão (8.4), **S1a, S1b, S2 e S3a são integráveis
isoladamente**: documentos, planos e catálogo públicos ficam byte a byte como hoje; o hello do runtime ganha fxTransition na S2 (T08, T32 e as suítes existentes), e o aceite de
cada uma inclui o controle negativo público. Não há "pilha rascunho": o gate do head conjunto é o da S2A.

| Fase (BL) | Entrega e aceite | Depende de | Arquivos (propostos; código novo em módulos novos) | Dono proposto |
|---|---|---|---|---|
| **S0 — Contrato e oráculo** (BL082.4.1; docs + Python, sem produção) | `docs/features/fx-transitions.md` (F064), **patch** da linha da matriz e de `effects.md` ao coordenador, manifesto com os **21 blobs** (2.1), tolerâncias e tetos (5.3, 12) **como contrato**, `scripts/pilot-dissolve-corpus.py` + spec, `docs/migration/fixtures/pilot-fx/dissolve-{cases,manifest}.json`; vetores de 6.1/6.2 regenerados (inclusive `intensity`); **divergências declaradas** (D17, D20). **Aceite (X):** `--check` reproduz os JSON, a spec Python passa com controles negativos e **recusa o stub do Windows Store como interpretador**; nenhum Rust alterado. Executa em Windows com Python **real** (14) | este plano | `docs/features/fx-transitions.md`, `docs/features/effects.md` (nota de que o rascunho de extensão está superado), `scripts/pilot-dissolve-corpus.py(.spec.py)`, `docs/migration/fixtures/pilot-fx/dissolve-*.json` | Doc/oráculo |
| **S1a — Documento, janela, validação (portão fechado)** (BL082.4.2) | `Fx.transitions`, `FxTransition`, tabela de transições + `ParamKind::Choice`, `read_transitions`, janela e aritmética verificada, `TRANSITION_*`, `WRONG_FAMILY`, avisos de documento, `FX_LIMIT`, **política de admissão** e entrada interna; leituras com transição só pela entrada interna. **Aceite:** T01, T02, T03 (documento), T05, T28 (documento) em L; **T32 negativo em L, H e G** | S0 | `crates/core/src/directed_scene/fx/{mod,read,transition,transition_read,transition_catalog,window}.rs`, `scene_project/fx.rs`, testes `crates/core/tests/scene_fx_transition/` | Core |
| **S1b — Avaliação e plano (portão fechado)** (BL082.4.3) | `evaluate_shot_at_offset`, `Fx::resolve_frame`, `ScenePlan.transition`, `Capability::FxTransition` (Rust), `validate_deck`, `resource_union`, JSON, `PlanTimeline` do vizinho, orçamento de dois decks, `scene.fx.frame` com `transition` (pela entrada interna). **Aceite (L):** T03 (orçamento), T04, T06–T14, T28 (plano), T30, T33; `pilot_dissolve_corpus.rs` verde; T08 e T32 (planos e catálogo públicos inalterados) | S1a | `crates/core/src/directed_scene/temporal/shot_offset.rs`, `directed_scene/fx/frame.rs`, `scene_plan/{model,json}.rs` (campo e capacidade) e `scene_plan/transition{,_json,_validate}.rs`, `crates/core/tests/pilot_dissolve_corpus.rs`, `.../scene_fx_transition/plan.rs` | Core |
| **S2 — Runtime, render e export de quadros brutos** (BL082.4.4) | `parseTransition` e limites, shader, alvos, ciclo de vida, `SCENE_CAPABILITIES`; `crates/render/tests/scene_fx_transition.rs`; testes `node --test`; teste de runtime antigo. **Aceite (P, R, E):** T04 em P/R; T06, T08, T10–T13 em R; T10/T18 com asset exclusivo do other em P/R/E; T32 negativo R; T12, T15–T19, T30, T31, T33 em P e R; T16 contra a referência X; **T27 (quadros brutos iguais entre export completo e sub-intervalo)** | S1b | `runtime/src/{directed-transition,transition-blend}.js` (novos), ganchos em `directed-scene.js`/`compositor.js` (delta ≤ 0), `output-fx.js`, `runtime/test/*`, `runtime/test/fixtures/fx-catalog.json`, `crates/render/tests/scene_fx_transition.rs`, `crates/render/src/scene.rs` (from_root/resolve), `scene_operation.rs` (approved_map), `scene_export/mod.rs` (resource_map) | Runtime/render |
| **S3a — Edição no core (portão fechado)** (BL082.4.5) | Ops, dry-run, no-op, undo, revisão, `errors[]` com texto nas 3 línguas, contrato de shot apagado, **predicado de janela** (puro). **Aceite (L):** T20, T22, T26 (predicado); T32 negativo L/H das novas ops pela entrada normal | S1b (**não** S2) | `scene_edit/fx_transition.rs` (novo), `scene_project/fx.rs` (já existente), `ops.rs` (uma entrada por op), `directed_scene/fx/window_guard.rs`, `crates/core/tests/scene_fx_transition/edit.rs` | Core (outro agente; **paralela** à S2) |
| **S2A — Ativação** (BL082.4.4, última PR) | Abre o portão (8.4): remove a política e `FX_TRANSITION_NOT_YET`, tabela de transições no catálogo público, `refused` sem `transitions`, **preflight do intervalo inteiro**, `scene.fx.budget` com os dois decks; inverte T32. **Aceite (H, G):** T01–T03, T05–T11, T13, T14, T28 pela entrada pública por `tessel-headless` real; T32 invertido L/H/G/R; painel de clipes não quebra com transição no documento (G); tracer bullet público | S2 e S3a | `directed_scene` (admissão), `scene_project/fx.rs`, `scene_edit/fx_inspector.rs` (família), `crates/render/src/scene.rs`, `scene_operation.rs`, `scene_export/mod.rs` (checagem dos quatro carregadores), `scene_export` (preflight), testes | Core + coordenação |
| **S3b — Gesto, inspector, persistência (core)** (BL082.4.5) | Gesto de transição, `TransitionInspector`, persistência/pacote/intercâmbio, parser (`commands.rs`) e nós de estado (`tree.rs`, uma entrada por nó). **Aceite:** T20 (H), T21, T22 (H), T23 (L, H, R) | **S3a e S2A** | `scene_edit/fx_transition_inspector.rs`, `scene_project/fx_gesture.rs`, `commands.rs`, `state/tree.rs` (entradas), `crates/core/tests/scene_fx_transition/{gesture,persist}.rs` | Core |
| **S4a — Guarda de overlay (core + aviso no monitor)** (BL082.4.6) | Fiação do predicado em gizmo, alças, gesto aberto e alvo do vizinho; aviso `transition_window` no monitor. **Aceite:** T26 (L, H, G, W). **Só começa com a base W1 (PR #59, BL167.3/.7) integrada e validada** (seção 14) | S2A, S3a, **W1** | `scene_project/gizmo.rs` (delta ≤ 0), `scene_project/fx_handles.rs`, `scene_interaction/fx_handles.rs`, `scene_project/fx_seek.rs`; `runtime/src/overlay.js` só se o aviso exigir (BL082.62: extrair antes) | Core |
| **S4b — UI Kit e automação** (BL082.4.6) | Seção Transições, nós de painel, handlers compartilhados, prova da janela. **Aceite:** T02/T03/T20/T21 (G, W), T14 (G), T22 (W), T24, T25 (W, P) | S3b e S4a | `crates/desktop/src/workstation/scene_fx_transition*.rs`, `scripts/scene-fx-transition-proof.py(.spec.py)` | UI |
| **S5 — Fechamento** (BL082.4.10) | Mutantes (T29), tolerância do codec (T27 E, 5), NTSC e limites já verdes, docs F064/F016/F062 e matriz (patch), relatório com evidências por superfície. **Aceite:** tabela da seção 12 com cada célula marcada *executado*, *A validar* ou *N/A* | S2A, S3b, S4a, S4b | `docs/migration/p4b-cross-dissolve/{mutate.py,test_mutate.py}`, `crates/render/tests/scene_fx_transition_export.rs`, docs | Coordenador de integração |
| **S6 — Dissolve legado, expansão de contrato** (BL082.4.7) | Faixa de oráculo **nova** do piloto, importar `transitionInto: dissolve` como `crossDissolve` sintético (D04) **sem editar o acervo antigo**. **Aceite:** abaixo | **S5 (BL082.4.10)** e decisão Q2 | `directed_scene/{parse,joins}.rs`, `docs/migration/fixtures/pilot-dissolve/`, registro de divergência, F044 | Core |

**S6 em detalhe (B4).** O piloto aceita `dissolve` e o valida (decisões #35 e #49; `origin-specs/scene-fx-plan.md` l.147–175);
o acervo BB10 foi gerado de um pin **anterior** a isso e o Tessel recusa o caso com `DISSOLVE_UNSUPPORTED`
(`parse.rs:681`, `directed_scene.rs:838`). Por isso:

- **Não se edita** `invalid-dissolve-transition.expected.json` nem seu `bundleSha256`, nem o bundle: o acervo antigo e as
  asserções de origem ficam como **histórico do pin antigo**. O consumidor Rust do acervo ganha um **registro de
  divergência de veredito** (no estilo do `amendments.json` da S10, `scripts/directed-scene-acceptance/`; a S6 confere o
  schema e o caminho): "o oráculo do pin antigo recusa; o Tessel passa a aceitar, por decisão, em tal fase". Todo caso fora
  do registro continua tendo de bater com o oráculo.
- **Faixa nova** (`docs/migration/fixtures/pilot-dissolve/`, manifesto com pin `d40ac3f7c0` e os blobs de 2.1, **origem não
  executada**, expectativa por transcrição Python): fixtures com `alphaMode: "opaque"` para os casos que o piloto **aceita**
  e fixture `straight`, cujo veredito **diverge**: o piloto recusa `UNSUPPORTED_ALPHA` mesmo sem `fx` e o Tessel aceita e
  desenha opaco (D20). O bundle `invalid-dissolve-transition` (`straight`) **não** vira paridade com o piloto por ser
  reclassificado como válido.
- **O que o piloto fazia e a faixa afirma:** `dissolve` entre instâncias diferentes passa **sem checagem de pose** (como o
  `cut`); nenhum `fx` é exigido; `MISSING_TRANSITION` continua; avisos de transição valem **só para as declaradas**;
  `TRANSITION_DUPLICATE` só contra uma transição **declarada** no mesmo corte; o legado conta para janela,
  `TRANSITION_TOO_LONG`, `OUT_OF_RANGE` e `OVERLAP`; e, quando houver narração, para `TRANSITION_OVER_SPEECH`.
- **Aceite:** faixa nova verde; acervo antigo **inalterado em bytes** (hash); o registro de divergência cobre exatamente os
  casos mudados; o harness S10 reexecuta onde as pré-condições valham.

**Dependências exatas.** S0 → S1a → S1b → {S2 ∥ S3a} → S2A → S3b → S4a → S4b → S5 → S6, com S3b ∥ S4a permitido quando a
base W1 já estiver integrada. S3a **não** espera S2 (só precisa do plano e do documento); S2A **espera S3a**: ambas tocam scene_project/fx.rs e executam sequencialmente; após a ativação as ops JSONL já existem. Nenhuma fase
depende de `node` scope (BL082.40), P5 (BL082.5) ou P6. Conflito de arquivos entre S2 e S3a: nenhum (S2 só toca
`runtime/`+`crates/render`; S3a só `crates/core`). S1a e S1b **dividem** `directed_scene/fx/mod.rs`: sequenciais, nunca
paralelas. **S4a toca `gizmo.rs`, `fx_handles.rs` e (se preciso) `overlay.js`, que a frente W1 também lê (BL167.3: geometria
de rotação e escala do gizmo): esses arquivos são ownership explícito da S4a e ela só começa com a base W1 integrada.**

**Tracer bullet mínimo.** Um bundle do corpus (`valid-cut-instance-offsets`) com uma transição `crossDissolve` `center`
D=12 `hold` (corte em C = 40, janela [34,46)). **Fim da S1b (lib, pela entrada interna):** o plano do quadro 37 tem
`frameInWindow` 3, `primary: "from"`, vizinho `shot-b` com `heldInstance` e `heldCamera` verdadeiros, **`progress` 0,099248**
(após easing) e **peso efetivo 0,027595** (após smoothstep, `intensity=1`, nunca guardado no plano), e o teste de corpus
confere janela, `progress` e a mistura de pixel de duas cores constantes. **Fim da S2A (público):** o mesmo quadro por
`tessel-headless` real em `scene.fx.frame`. Antes disso nenhum processo público vê a transição (8.4).

## 14. Gates, ambiente, mbx, Python e disco

Gates **separados** e nenhum implica o outro:

| Gate | O que prova | Não prova |
|---|---|---|
| **Windows** | `fmt`, `check`, `clippy --all-targets -D warnings`, testes **focados** de cada fase, `build -p tessel-editor`, runtime após `./scripts/runtime setup`, provas de janela pela `scripts/ui` (D3D11) | Mac, WKWebView, ponteiro físico, desempenho |
| **Mac** | O mesmo e a janela real (WKWebView/Metal), provas Python | Windows |
| **Review** | Revisão independente do head (coordenador) | Execução |
| **CI** | Checks do GitHub no head | Janela real |
| **Merge** | Decisão do coordenador, sem *force* | |

**Ambiente Windows (H12).**

- **Invocação.** `scripts/cargo`, `scripts/ui` e `scripts/headless` são `#!/bin/sh` (BL167.4 registra a dependência de Git
  Bash). Em PowerShell, localizar o **Git Bash real**, por exemplo
  `& 'C:\Program Files\Git\bin\bash.exe' ./scripts/cargo <subcomando>` (também para scripts/ui e scripts/headless).
  Não usar bash não qualificado: o PATH desta máquina encontra o Microsoft Bash Launcher/WSL. Conferir a origem
  do executável, versão e o caminho real instalado; o texto Cargo só por ./scripts/cargo vale pela invocação explícita. Alvo padrão; **nunca** `CARGO_TARGET_DIR`, `--target-dir`, `build.target-dir` ou
  `TESSEL_CARGO_LANE`; sem `clean`/`gc`/limpeza de cache; artefatos/cache em `tmp` > 500 MB proibidos; reserva de disco
  ≥ 16 GiB para trabalho pesado (piso 12); evitar `--workspace`/release concorrentes (há outras frentes Windows).
- **Python real.** Na leitura inicial desta máquina, `python`/`python3` resolviam para o **atalho da Microsoft Store** (stub de
  `WindowsApps` que responde "Python was not found"): **não havia interpretador identificado** naquela leitura de 14/11/2026. Isto **não
  é** disponibilidade de Python. A S0 executa em **outro clone Windows**, com preparo de Python oficial por usuário
  autorizado pelo coordenador. O relato posterior da PR61 informa Python real3.13.17 via winget e161 testes normal/-O;
  setup e corpus continuam sob revisão, e o atalho do PATH não substitui esse intérprete. Isso **não altera** a descoberta de ferramentas de produção e **não
  admite falso verde**: o corredor e a spec **recusam** o stub (saem com erro, `SKIP` não é passe) e imprimem `sys.version`.
  Scripts de Windows (`bash ./scripts/cargo`) e o Python real são **coisas diferentes**; nada é feito por alias do Store.
- **BL167.5 fica parcial.** Só a preparação do interpretador, **antes** do corpus. Build *release* e as provas vivas
  (`scene-fx-transition-proof.py`, janela D3D11, mutantes com build) ficam como **futuras** e dependem de BL167.1–.4; sem
  esses pré-requisitos a célula é **A validar**, nunca verde.
- **O Mac não é host de execução da S0 nesta rodada** (relato do coordenador: a reserva de disco do Mac, 15 GiB, está
  abaixo do piso de 16): o coordenador lê objetos Git no Mac, mas não roda `cargo`/engine lá. A decisão de qual host executa cada fase seguinte é
  do coordenador; um host sem os pré-requisitos declara a lacuna.
- **Windows hoje não é verde** no workspace inteiro (BL167: gizmo BL167.3, ledger BL167.6, helper BL167.7). A PR de cada
  fase registra a **lista vermelha de base** da `main` (nomes de teste) antes da primeira mudança e afirma "sem falha nova",
  **nunca** "workspace verde". Os testes Rust do corpus **só leem JSON** e rodam em qualquer lugar; o `--check` do corpus
  e a spec da S0, as provas `W/P` da S4b e os mutantes da S5 exigem Python real (e, as duas últimas, janela/D3D11).
- **W1 (BL167.3/.7).** A PR [#59](https://github.com/journey-studios/tessel-editor/pull/59) (branch
  `w1/windows-portability-bl167-3-7`, head `a174c9b`) está **aberta, não mesclada**; relato do coordenador: revisão
  independente no Windows sem bloqueador (com nits), Mac e UI pendentes. As guardas da S4a só tocam os arquivos coincidentes
  **depois** que a base W1 estiver integrada e validada.
- **Lock da GPU só para desempenho.** BL082.4.8 (custo do quadro de dois decks e do par de splats) fica separado; sem o
  lock, desempenho **não** é declarado aceito. Provas funcionais não o tomam.
- **Origem não executada.** `ORIGIN_NOT_EXECUTED` e `ORIGIN_GOLDEN_NOT_COMPARABLE` continuam no manifesto.

| Risco | Mitigação |
|---|---|
| Pin da origem sem leitura direta da autora | Reproduzir os 21 blobs no manifesto da S0 (2.1); diferença de `fx-schema.ts` (comentário) e de `render-bundle.schema.ts` (narração) registradas; sem alegar paridade |
| Dupla modelagem do plano (`other`) | `other` é `ScenePlan` validado por `validate_deck`, com tabela de recursos **própria**; a união é derivada por `assetId` |
| Colisão de ids de nó entre decks | Nenhum identificador cruzando decks; `used_by` qualificado; teste com o corpus de nós colidentes (T10) |
| Export que falha no quadro 34 ou corte seco silencioso | Portão de admissão (8.4) e preflight do intervalo na S2A |
| Entrada hostil (`other` recursivo, bomba) | Profundidade 1 sem recursar e tetos nos dois lados (5.3, T28) |
| Overflow ou underflow na janela | `D ≤ edição` antes da conta, `i64` verificado, recusa tipada; `max(0, …)` só onde é semântico |
| Digest dos planos existentes mudar | Chave aditiva ausente quando não usada; T08 fixa os bytes do corpus |
| Cache servir pixel velho do vizinho ou da mistura | Chave por deck e chave da mistura com `progress`/`amount`; T19, T31 e mutantes |
| Meia mistura em erro | Erro da camada, nunca desenho parcial; T18 |
| `intensity = 0` tratado como identidade ou progresso sem easing | D07 e T07 explícitos, vetores 162/165/168; usar `curve=smooth` |
| Gesto aplicado a um deck que não se vê | Predicado único, guarda de gesto aberto, equivalência hit×press (9.6, T26) |
| Arquivo acima de 1000 linhas crescer | Módulos novos; ganchos com delta ≤ 0; BL082.55/.62 |
| Prova só da lib vendida como UI | Cada superfície tem a sua coluna; sintético ≠ físico |
| Python do Store tratado como interpretador | Corredor e spec recusam o stub; célula fica A validar |

## 15. Linhas propostas ao backlog, decisões registradas e entrega

**IDs** (o coordenador **reservou** BL082.4.1 a BL082.4.10 e F064 para a P4b; **mantêm a identidade**, nada é renumerado,
e **nenhum ID novo** é criado por esta frente; `BL082.4` é o agregador existente, Pendente). Leia `BL082.4.9` e `BL082.4.10`
com o ponto: não são `BL082.9` nem `BL082.40`. A divisão em subfases (S1a/S1b, S2/S2A, S3a/S3b, S4a/S4b) é **de PR dentro do
mesmo ID**.

| ID | Pendência | Subfases | Depende de |
|---|---|---|---|
| BL082.4.1 | S0 — contrato F064, manifesto de 21 blobs e corpus independente | S0 | BL082.4 |
| BL082.4.2 | S1a — documento, janela e validação tipada (portão fechado) | S1a | .1 |
| BL082.4.3 | S1b — avaliação por quadro e plano de dois decks (portão fechado) | S1b | .2 |
| BL082.4.4 | Runtime, renderer e **ativação** do portão | S2, S2A | .3 |
| BL082.4.5 | Edição, gesto e persistência (core) | S3a (depende de .3), S3b (depende de S2A em .4) | .3; S3b também .4 |
| BL082.4.6 | Guarda de overlay e UI Kit com automação | S4a (depende de .4, S3a e **W1/PR #59**), S4b | .4, .5, W1 |
| BL082.4.7 | S6 — dissolve legado, expansão de contrato com faixa nova | S6 | **.10** e Q2 |
| BL082.4.8 | Desempenho do quadro de dois decks (lock da GPU) | — | .4 |
| BL082.4.9 | `keys` em transições (D10) | — | .5 |
| BL082.4.10 | S5 — mutantes, codec, fechamento e relatório | S5 | .4, .5, .6 |

Dependência corrigida: **`.7` depende de `.10` (S5)**, não de `.6`. A cadeia é `.1 → .2 → .3 → {.4, .5a} → .4 (S2A) → .5b →
.6 → .10 → .7`.

Matriz: **F064** (reservada), `docs/features/fx-transitions.md`; hoje a última é F063. O **patch de linha de backlog e de
matriz** é entregue ao coordenador no corpo da PR e na mensagem de handoff; esta frente **não** edita `docs/backlog.md`,
`docs/features/README.md` nem `docs/README.md`.

**Decisões registradas** (coordenador, 14/11/2026). Nenhuma delas volta ao usuário como decisão rotineira.

- **Q1 (D16).** Recusar gizmo e alças (`transition_window`) durante a janela, por **um** predicado do core (9.6).
  Reversível por construção (trocar o corpo do predicado e inverter T26).
- **Q2.** S6 separada, com **faixa de oráculo nova** e registro de divergência; **sem** editar o acervo antigo nem o hash
  de `invalid-dissolve-transition`. S0–S5 mantêm a recusa legada.
- **Q3.** `UNSUPPORTED_ALPHA` **fora da P4b**, com **divergência de veredito explícita** (o piloto recusa o documento; o
  Tessel aceita e desenha opaco), registrada em F016 e F064 e válida também para os três looks. T33 fixa o comportamento.
- **Q4.** Aviso `TRANSITION_OVER_SPEECH` **adiado** até existir narração (BL017/BL026); fica N/A registrado, sem controle.
  Quando houver narração, o dissolve legado também conta.

**Entrega a quem implementa:** a primeira ação de S0 é reproduzir os 21 blobs no manifesto e fixar tolerâncias e tetos como
contrato; a última de cada fase é o relatório com a tabela da seção 12 preenchida por célula (*executado*, *A validar* ou
*N/A* com motivo). **Plano pronto não é feature implementada, nada daqui foi executado, e nenhum controle novo existe
antes da S4b.**

## 16. Rastreabilidade da revisão de 14/11/2026

Revisão independente (somente leitura) do head `5e64dd9`; `ab4cc69` reparou B1, fatos e vetores. Esta versão resolve o
resto **no corpo**, não por adendo.

| Achado | Resolução | Onde |
|---|---|---|
| B1 progresso × peso | Já reparado em `ab4cc69` (colunas `progress`/`weight`, tracer 0,099248 e 0,027595); mantido e reforçado | 6.2, 13 |
| B2 recursos do `other` | Tabela própria validada; união derivada por `assetId` ordenada; `used_by` por deck; teste com nós e assets colidentes; nenhum identificador cruzando decks | 7.3, T09, T10 |
| B3 estratégia do estado intermediário | Portão de admissão público até a S2A, controles negativos por consumidor, sem `Domain` novo; S1a/S1b/S2/S3a integráveis isoladamente | D19, 8.4, T32, 13 |
| B4 dissolve legado e alpha | Acervo antigo intacto; S6 é expansão de contrato com faixa nova; divergência de veredito de `straight`; decisões #35/#49 e origem `scene-fx-plan.md` l.147–175 citadas | D04, D20, 13 (S6), 15 (Q2, Q3), T33 |
| H1 relógio do vizinho | Três relógios, relógio de cena no quadro real; S0 confirma na fonte; T06 | D17, 6.1, 7.2 |
| H2 `disabled_reason` | Motivos legítimos vêm do core; `continuous`, handles, upsert e inércia não desabilitam; rótulo "Suavização" | 11.1, 9.3, D12 |
| H3 `intensity = 0` | D07 corrigido; vetores 162/165/168 e a nota de `linear` | D07, 6.2, T07, T29 |
| H4 `WRONG_FAMILY` | Adotado, com o portão fechado mantendo o comportamento de hoje | D18, 5.1 |
| H5 `other` recursivo e hostil | Profundidade 1 sem recursar, tetos de bytes e itens, Rust e JS, testes | D21, 5.3, T28 |
| H6 segurança de faixa | `D ≤ edição` antes da conta, `i64` verificado, recusa tipada, inteiro estrito, `seed` uint32, `max(0, …)` semântico | D03, 5.1, 5.2, 6.1, 7.2 |
| H7 referência e tolerâncias | Decks de 8 bits do runtime, viés médio, codec importado de F016 sem pior caso observado | D08, 7.3, 12 |
| H8 splats e dois decks | Leases e `continuous`, dispose de textura/material/geometria, cache da mistura, T30 e T31, mutantes | 8.3, T18, T29–T31 |
| H9 D16 e Q1 | Predicado único no core, hit × press, gesto que entra na janela, alvo do deck vizinho | D16, 9.6, T26 |
| H10 caminhos e tamanho | `directed_scene/temporal/…`, parser em `commands.rs`, módulos novos, delta ≤ 0 em arquivos > 1000 linhas | 4, 13 |
| H11 cobertura por fase | Matriz célula a célula com dono; S3 dividida em S3a/S3b; S4 em S4a/S4b; S2A | 12, 13 |
| H12 Windows | `bash ./scripts/cargo`, Python real em outro clone, stub recusado, BL167.5 parcial, Mac sem execução | 14 |
| H13 W1 | S4a só com a base W1 integrada; arquivos como ownership explícito | 13, 14 |
| B5, B6 do roadmap | Reparados em `ab4cc69`; spacing e W1 atualizados no roadmap | `phased-roadmap.md` §9 |
| Nit: IDs e dependência de S6 | `.7` depende de `.10`; subfases explicadas | 15 |
| Nit: pipes em tabelas | Escapados em `ab4cc69`; mantidos | 3, 4 |
| Nit: S0 e matriz | A fase **entrega o patch** da linha ao coordenador | 13 |
| Nit: texto de erro | Teste enumera todos os códigos novos nas 3 línguas | 9.4, T20 |
| Nit: alpha sem regra no core | Divergência registrada em F016 e F064 | D20 |
| Evidência do pin | Relato do coordenador reproduzido, com a diferença de narração | 2.1 |

## 17. Fechamento documental da rechecagem R2

R2 em2395719 liberou S0/S1a/S1b e pediu R1 antes de S2/S2A. O coordenador corrigiu o corpo:
os quatro carregadores do host usam resource_union e têm caso com asset exclusivo do vizinho; S2A
depende de S3a, T32 cobre as novas ops e R; hello aditivo não integra a promessa de bytes estáveis.
Hold misto tem oráculo analítico distinto de frames texturizados equivalentes; os três arquivos
grandes restantes entram na regra de módulos/delta; Git Bash é identificado explicitamente.
As células T04/T10/T18/T32 e os aceites das fases foram reconciliados. Isso é verificação de
contrato e documento, não execução de Rust/runtime/export. BL082.4.11 rastreia esta correção
no backlog do coordenador; não é fase de produto adicional e não renumera .1–.10.

Decisão após a revisão S0: no documento de transição hidden aceita true ou ausência, igual ao
schema do piloto; false explícito é recusa estrutural tipada na S1a. Tornar visível por operação
remove a chave, em vez de gravar false. Não muda o contrato de hidden dos looks existentes.
