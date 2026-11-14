# P4b S1b — avaliação de dois decks e `ScenePlan` de `crossDissolve` (plano de execução)

Detalhe de **BL082.4.3** (não é fila paralela, não cria IDs nem estados). Contrato canônico:
[p4b-cross-dissolve-plan.md](p4b-cross-dissolve-plan.md) (D01–D21/R1, §§5.3, 6–8, 12–14, matriz T01–T33).
Base: `origin/main e3c20cb7c8d3e32692719a9c71adb1b65eb3c392`. Ramo `feat/p4b-s1b-evaluation`.
S1a foi aprovada só para o escopo interno; não vale como prova de S1b.

## Objetivo

Dado um quadro de edição dentro de uma janela de transição, o core:

1. avalia o shot sob a agulha e o shot vizinho no **quadro real de edição**, com clocks de
   instância/câmera presos (clamp) separados (D14) e luz/cena pelo quadro real (D17);
2. resolve o quadro em três estágios (primário, transição, saída) com `progress` já suavizado e
   `amount = clamp01(intensity)`;
3. serializa o resultado em `ScenePlan.transition` (chave opcional, ausente sem efeito) com um
   segundo `ScenePlan` (`other`), recursos por deck, união derivada e capacidade `fxTransition`.

**Não** abre a admissão pública: `Fx.transitions` continua vazio por `TransitionAdmission::Refuse`
em import/`scene.attach`/open/persist; `Accept` existe só no caminho interno (`import_*_admitting`)
e não é usado em produção.

## Decisões

- Top = o shot sob a agulha: antes de C, primário = `from` e `other` = `to`; depois de C, primário = `to`.
- O clip vizinho usa `clamp(f, shot.start, shot.end-1)` (D15); um clip no meio do shot não entra
  (§5.3). A semente de ruído do vizinho usa o quadro **real** (exceção registrada).
- `progress = ease((f − start + 0.5)/D)`; D=1, ímpar, bordas e frame 0/último não viram A/B puro por atalho.
- Saída uma vez; não é duplicada em `other`, nem peso/intensidade.
- Um plano sem transição mantém bytes/digests (chave ausente, sem novo limite nem versão).
- `PLAN_VERSION` 1, `FX_CATALOG_VERSION` 1, `Domain::ALL` 13. `Capability` ganha `FxTransition` (14ª).
  `fxTransition` **não** entra em `SCENE_CAPABILITIES` do JS (S2).
- `FX_PASS_BUDGET` conta saída uma vez + passes de cada deck (inclui clips de borda) + 1 blend; só
  aviso, nunca recusa. Nada de FPS/desempenho/GPU.

## Marcos e ownership

| Marco | Entrega | Arquivos (core) |
|---|---|---|
| M0 | Baseline sem transição na base **sem alterações** + negativo público | `tests/scene_plan_transition_baseline.rs` + JSON |
| M1 | Relógios e pureza: `evaluate_shot_at_offset` | `directed_scene/temporal/shot_offset.rs` (+ gancho `frame.rs`, `mod.rs`) |
| M2 | `resolve_frame` (primário/transição/saída) e orçamento | `directed_scene/fx/{frame,budget}.rs` (+ `fx/mod.rs`) |
| M3 | `ScenePlan.transition`, parser defensivo, tetos | `scene_plan/transition{,_json,_validate}.rs` + ganchos |
| M4 | Recursos por deck, união, `requires` | `scene_plan/{transition,validate,build,timeline}.rs` |
| M5 | Corpus, mutantes, relatório | `tests/pilot_dissolve_corpus.rs`, `tests/scene_fx_transition/*` |

Extensões mecânicas de ownership (registradas no relatório): `fx/transition_explain.rs` (alias de
tipo para `ShotReach`), `scene_project/fx.rs` (chave aditiva `transition` em `scene.fx.frame`),
constante de capacidades em `tests/scene_fx_shot_scope/plan.rs`.

## Estado

M0–M5 executados; status **concluída parcial interna, a validar review**. Resultados, mutantes e divergências:
[p4b-s1b-evaluation-report.md](p4b-s1b-evaluation-report.md).

## APIs/tipos propostos

- `FrameEvaluator::evaluate_shot_at_offset(shot, edit_frame) -> OffsetEvaluation` (mesmo
  `FrameEvaluation`, mais as flags de held por clock).
- `Fx::resolve_frame(edit, frame) -> FxFrame` com `FxTransitionFrame { side, progress, amount, … }`.
- `PlanTransition { other: ScenePlan, effect, progress, amount, window, … }` com `decks()` e
  `resource_union()` (BTreeMap por `assetId`, `used_by` qualificado por deck).
- `ScenePlan::from_json_str` aceita `transition` com profundidade 1; tetos 4194304 bytes, 20000 itens.

## Comandos, rejeições, compat

Sem novo `Command` de UI (S1b não tem controle). Rejeições são `PlanError` inválido com caminho
(`/transition/...`); `assetId` repetido com conflito = inválido em `/transition/other/resources`.
Compat: plano antigo idêntico em bytes; JS ignora a chave extra porque o runtime recusa por capacidade.

## Riscos e prova

- Deriva de ponto flutuante entre perfis: baseline por perfil (debug/release).
- Oráculo compartilhado com o teste não é independente: os vetores de §6.2 são calculados à mão.
- Matriz de mutantes (aplicar/rodar/reverter) em M5; células não executadas ficam "A validar".
- Não declarar pixels reais, `ORIGIN_NOT_EXECUTED` e `GOLDEN_NOT_COMPARABLE` mantidos.
