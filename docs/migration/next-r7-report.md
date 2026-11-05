# R7 — Spark concorrente (BL091)

Contrato e evidências completos: [features/spark-concurrency.md](../features/spark-concurrency.md).
Estado de execução fica só no backlog; este relatório entrega o resultado e os patches para o coordenador.

## Resultado

**Reproduzido, causa encontrada e corrigida de forma demonstrável.** A causa é uma **corrida do harness**, não
falha do runtime nem limite de memória/GPU.

- Reprodução: 6 execuções de `node --test test/*.test.mjs` com a concorrência padrão (10 Chromes): 2 falharam, as duas em
  `splat.test.mjs:341`, `'ready' !== 'failed'` (passo 6). Sozinho o arquivo passa em 6,8 s.
- Causa: o passo 6 enviava `asset.load` e `asset.release` em dois `page.evaluate` separados e assumia que o load
  (~2 ms) ainda estava em voo. Se o load termina antes do segundo `evaluate` chegar, o runtime responde `ready` e depois
  libera: comportamento correto, mas o teste só aceitava `cancelled`.
- Prova da causa (bancada, concorrência dentro do processo, classificação por log da página):
  920 iterações por modo, mesma carga ambiente alta. `racy` (o teste antigo): 155 corridas perdidas (16,8%), de 2,5% com 1 Chrome a
  31% com 10; `same-task` e `gated`: 920/920 `cancelled`; **0 `runtime-fault`, 0 `other:*`**.
- Correção: o teste segura o `fetch` do asset (`holdFetches`) para o load estar comprovadamente em voo. Sem serial, skip nem
  timeout maior.
- Depois: 12/12 execuções consecutivas da suíte completa com concorrência padrão (85 testes), sob lock, load average 16–20.

`src/splat-stage.js` e o restante de `runtime/src` **não foram alterados** (causa não está neles). Nada de `dist`, deps,
assets.js/index.js/compositor/protocol tocado.

## Arquivos (todos sob o ownership de R7)

| Arquivo | Mudança |
|---|---|
| `runtime/test/support.mjs` | + `encodeSplat/encodePly/encodeSpz`, `SPLAT_ENCODERS`, `holdFetches` (gate de fetch com `held/open/arm/close`), `openSplatPage`, `splatScenePlan`, `FRONT/BACK/CENTER`. Só adições; outros testes não mudam |
| `runtime/test/splat.test.mjs` | passo 6 com gate; usa os helpers compartilhados (sem encoders/rig duplicados); remove o tmpdir no `finally` |
| `runtime/test/splat-concurrency.test.mjs` | novo, 5 testes (cancelamento determinístico em 4 páginas, release após `ready`, controle do gate, mesma tarefa, hash igual sob concorrência) |
| `runtime/scripts/splat-concurrency.mjs` | novo, bancada de experimento (`--browsers 1..12 --iterations 1..500 --modes --seed --out`), orçamento e limpeza em `finally` |
| `docs/features/spark-concurrency.md` | novo, contrato/evidência |
| `docs/migration/next-r7-report.md` | este relatório |

## APIs / seams para outros

- `holdFetches(page, match)` em `runtime/test/support.mjs` serve a qualquer teste que precise provar "em voo" de forma
  determinística (outros testes de lifecycle/cancelamento do runtime podem adotar; R8/R10 só leem, não precisam).
- Nada para o coordenador aplicar em arquivos alheios: **nenhum patch de código necessário**.

## Execução

Executado (todos sob `with-gpu-lock.py`, sem Cargo, sem GPU nativa, sem janela do usuário):

- `node --test test/splat.test.mjs` sozinho (baseline: passou; 6,8 s).
- Suíte `test/*.test.mjs` com concorrência padrão: 6× antes da correção (2 falhas), 12× depois (12/12 verdes, 85 testes).
- `test/splat-concurrency.test.mjs` + `test/splat.test.mjs` sozinhos (verdes, antes e depois de acrescentar o controle).
- Bancada: `--browsers` 1, 2, 4, 6, 10 (×2, a primeira com o classificador antigo por timestamp, descartada) × 40 iterações × 3 modos, seeds 1–2.
- `node --check` nos arquivos JS alterados.

Não executado:

- `./scripts/runtime test` isolado (equivalente ao `pnpm test`, já coberto pelos 12 runs completos).
- Cargo (nenhum código Rust), `pnpm build`/`tsc` do monorepo, gate global (é do coordenador).
- Runtime em WKWebView/janela nativa; GPU real (`TESSEL_GL=gpu`): os dados são SwiftShader.
- Release **durante a decodificação** do Spark (depois do fetch) sem prova determinística (limite no feature doc).
- Concorrência com mais de 12 Chromes ou assets grandes (fora do orçamento proposto).

Recursos: temporários próprios `te-r7-*` (KiB) removidos em `finally`; logs em `/tmp/te-r7/` (< 1 MiB). Piso de disco
(80 GiB livres) respeitado. Todo Chrome aberto foi fechado pela própria bancada/teste; nenhum processo alheio foi tocado.
Observação: restam `tessel-splat-*` no tmpdir de execuções antigas do teste; não apaguei por não poder provar autoria.

## Limites

- Estatística: 12/12 após 2/6 antes (prob. ≈ 0,8% de 12 sucessos por acaso à taxa antiga). O argumento de correção é a
  construção determinística (gate) mais o controle negativo `racy`, não a contagem.
- A bancada mede SwiftShader (CPU) com carga de outros agentes; as taxas absolutas mudam com a máquina, o contraste
  `racy` × `gated/same-task` é o resultado.
- Outra falha rara de `splat` não é descartada; ver "Limites" no feature doc.

## Patches para o coordenador (backlog e matriz)

**docs/backlog.md** — linha BL091:

```
| BL091 | Runtime/testes | Falha intermitente de splat.test.mjs em concorrência padrão | Validado localmente (fechar após o gate global) | Harness runtime | R7 | Causa: corrida do passo 6 (dois page.evaluate vs load de ~2 ms); runtime correto. 6 runs padrão antes: 2 falhas na mesma linha; bancada 920 it/modo: racy 16,8% de corridas, gated e same-task 920/920 cancelled, 0 runtime-fault; depois: 12/12 suítes completas (85 testes). Ver [spark-concurrency.md](features/spark-concurrency.md) e [next-r7-report.md](migration/next-r7-report.md) |
```

E na nota "Runtime final": `79/79 serializado` → `85/85 (80 + 5 de splat-concurrency) em concorrência padrão, 12 execuções`.
Pendência nova sugerida (ID novo, só se o coordenador quiser rastrear): **BL093** — prova determinística de release durante a
decodificação do Spark e rodada com `TESSEL_GL=gpu`; aceite: teste que retém a decodificação (não o fetch) e cancela, e
bancada com `--gl gpu` sob o lock; responsável: coordenador; sem dependência.

**docs/features/README.md** — linha F030: acrescentar à coluna de contrato/evidência o link
[Spark sob concorrência](spark-concurrency.md) (subcontrato; sem mudar V da linha). Linha nova sugerida (subcontrato):

```
| F030a | [Spark sob concorrência](spark-concurrency.md) | N/A | N/A | N/A | N/A | harness do runtime (`runtime/test/splat-concurrency.test.mjs`); lib/CLI/UI inalteradas, evidência só Node+Chrome headless (V local) |
```

**docs/README.md** — índice: adicionar `features/spark-concurrency.md` e `migration/next-r7-report.md`.
