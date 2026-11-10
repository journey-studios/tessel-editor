# Smoke de integração antes de P2 e depois de P2 (2026-11-10)

Registro de evidência de [F016](../features/effects.md)/[F057](../features/fx-inspector.md). Os números são
de execuções reais em macOS (Apple Silicon, SwiftShader no Chrome headless, **sem GPU real**). Uma prova de
Mac **não prova Windows**. O status e a atribuição ficam em [`docs/backlog.md`](../backlog.md).

## Lacunas preservadas (nada disso foi fechado)

`ORIGIN_NOT_EXECUTED` (a origem nunca foi instalada nem executada), `ORIGIN_GOLDEN_NOT_COMPARABLE` (sem golden
da origem), nenhum ponteiro físico (BL043) e nenhum áudio ouvido. JSONL sem janela (`./scripts/headless`,
`tessel-headless`) e instância própria da janela (`./scripts/ui`) são **superfícies distintas**: uma não
substitui a outra.

## Antes de P2: `origin/main` em `3e48861`, sem edição

Nenhuma falha real apareceu; não houve correção de integração.

| Superfície | Resultado |
|---|---|
| Janela própria, só `op`/`get`/`set`/`invoke` (`scripts/scene-fx-ui-proof.py`) | 116/116 |
| Histórico (`scripts/fx-history-smoke-proof.py`: congelar/selecionar/`exportJSON`) | 42/42 |
| Specs Python das provas | 24/24 (normal e `-O`) |
| Corpus do piloto (`scripts/pilot-fx-corpus.py verify`) | OK |
| Core: `scene_fx` 25, `scene_fx_gesture` 19 (processo `tessel-headless` real), `pilot_fx_corpus` 5, `scene_interaction` 24 | passaram |
| GPUI | 60 passaram |
| Pixels (render): `scene_fx` 3, `scene_fx_export` 1, `scene_fx_gesture_frames` 1 | passaram |
| Runtime (`output-fx`) | 11/11 |

Cobertura do fluxo: FX begin/update/preview/commit/cancel/undo, recusa que mantém o último candidato legal,
seleção/troca de cena com gesto ou quadro em voo, persistência, export curto.

Hashes (sha256) do binário de `3e48861`: `tessel-editor` `f05823bb723faaf7a65d68422b0c2171da1a7524858cf451f79a2e8bb7aebf3c`,
`tessel-headless` `c70d02b7beb2e9f0598f85323e3e52609a340b59fef8ae3b78aabb819b182e4b`, runtime
`runtime/dist/runtime.js` `a703245a76d61189ff0b728bea1fddaf03f3e351e907c3f34c2905321c923b49`.

## Depois de P2: branch `t3/fx-inspector-fade-p2`

| Superfície | Resultado |
|---|---|
| Janela própria, comandos semânticos (`scene-fx-ui-proof.py`, com fade, reset, pilha vinheta+fade e `scene.fx.inspect`) | 153/153 |
| Histórico (`fx-history-smoke-proof.py`) | 42/42 |
| Specs Python (`scene-fx-ui-proof.spec.py`) | 28/28 (normal e `-O`) |
| Corpus do fade (`scripts/pilot-fade-corpus.py verify --origin <checkout>`) | OK contra os blobs; vinheta `pilot-fx-corpus.py verify` OK |
| Runtime (Node, `runtime/test/*.test.mjs`, com Chrome) | 216 passaram, 0 falhas |

Os gates do head final (fmt, check, clippy, workspace, build, árvore do core) e os hashes do binário atual
estão no relatório de entrega e no PR; este arquivo não os repete para não envelhecer.

A prova da janela achou, na primeira execução com fade, um defeito de reset de componentes (ver F057); foi
corrigido antes dos números acima.
