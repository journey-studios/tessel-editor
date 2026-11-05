# Spark sob concorrência — BL091 (subcontrato de F030 / F021)

## Objetivo e limites

`runtime/test/splat.test.mjs` falhava de forma intermitente quando a suíte do runtime rodava com a
concorrência padrão do `node --test` (um processo por arquivo, `availableParallelism() - 1`, aqui 10
Chromes ao mesmo tempo). Este contrato registra **a causa**, a correção e as provas que separam três
classes de falha que o sintoma não distinguia:

| Classe | Significa | Resultado deste trabalho |
|---|---|---|
| Corrida de harness | o teste assume uma ordem de eventos que a máquina não garante | **era esta** |
| Falha de runtime | o runtime descumpre o contrato (resposta errada, recurso vazado, pixels diferentes) | **não observada** em 920 iterações por modo controlado nem em 12 suítes completas |
| Limite de memória/GPU | contexto perdido, `resource` recusado, página caída | **não observada** (nenhuma resposta `other:*`) |

Não há código novo no motor: `src/splat-stage.js`, `assets.js`, `index.js`, compositor e protocolo
**não foram alterados**; a causa não está neles. Não é feature de edição: lib/CLI headless/UI do
domínio não mudam (N/A, ver Evidências).

## Causa

O passo 6 de `splat.test.mjs` ("liberar um asset enquanto o load está em voo cancela o load") fazia

```js
await rig.send({ type: 'asset.load', ... });     // page.evaluate #1
await rig.send({ type: 'asset.release', ... });  // page.evaluate #2
```

O runtime está correto nos dois sentidos (`AssetStore.load`/`release`, `runtime/src/assets.js`): um
`release` recebido **antes** de o load terminar responde `failed/cancelled`; um recebido **depois**
deixa o asset `ready`, que então é liberado (nenhuma resposta adicional). O teste só aceitava o
primeiro desfecho. Um load de um splat por loopback leva ~1,5–2,5 ms; o intervalo medido entre os dois `evaluate`
(`gapMs`, dentro da página) teve mediana de 0,7 ms com 1 Chrome e de 1,7–1,9 ms com 10 (p95 ≈ 8 ms, máximo 45 ms). Quando o load termina antes de o
segundo `evaluate` chegar, a resposta é `ready` e a asserção `'ready' !== 'failed'` (linha 341 do teste
antigo) falha. A suíte inteira tem esse perfil: 2 de 6 execuções com concorrência padrão falharam, as
duas exatamente nessa linha, sem nenhum outro modo de falha.

## Correção

- `test/support.mjs`: `holdFetches(page, match)` segura, por interceptação de requests do Puppeteer, o
  `fetch` do asset até `open()`. O load **comprovadamente** está em voo (`gate.held()`) quando o release
  é enviado; depois `open()`/`close()`. Nada além do fetch casado é alterado.
- `test/splat.test.mjs` passo 6 usa o gate. Mudar para execução serial, `skip` ou timeout maior **não**
  é a correção e não foi usado.
- Os encoders `.splat/.ply/.spz`, `splatScenePlan`, as câmeras e o rig de página saíram do arquivo
  de teste para `support.mjs` (um só lugar, usado também pelo teste novo e pela bancada).
- `splat.test.mjs` passa a remover seu diretório temporário (`finally`).

## Contrato testável (sem UI nem Cargo)

`runtime/test/splat-concurrency.test.mjs` (5 testes, 1 Chrome, 4 páginas simultâneas, assets de 1 splat):

1. release com o fetch retido (load em voo) responde `cancelled` e nada fica carregado, em 4 páginas ao mesmo tempo, 5 rodadas cada;
2. release **depois** de `ready` é uma liberação simples: nenhuma resposta de estado, nada fica carregado (é o desfecho que o teste antigo recebia e que **não** é bug);
3. controle: com o fetch liberado antes do release a mesma sequência termina `ready` (o gate é quem decide);
4. `load` + `release` na mesma tarefa (um `evaluate`) são `cancelled`, 10 rodadas em 4 páginas;
5. 4 páginas desenhando o mesmo splat ao mesmo tempo produzem o mesmo hash que uma página sozinha.

## Bancada de experimento

`runtime/scripts/splat-concurrency.mjs` — concorrência **dentro** do processo, sob o lock da bancada:

```sh
python3 scripts/directed-scene-proof/with-gpu-lock.py \
  node runtime/scripts/splat-concurrency.mjs --browsers 10 --iterations 40 --seed 1 --out /tmp/te-r7/bench.json
```

Orçamento explícito: no máximo 12 Chromes (`--browsers`), até 500 iterações, assets de um splat, arquivos
temporários só em `te-r7-splat-*` sob o tmpdir (KiB), removidos em `finally`; fecha todo browser e servidor
que abriu. Modos: `racy` (o teste antigo, o bug como estava escrito), `same-task` (controle de harness) e
`gated` (a forma determinística). Cada iteração é classificada por um **log dentro da página** (a ordem em
que `receive` recebeu e `__tesselPost` enviou; `performance.now()` só mede duração, pois empata):

- `cancelled` — o release chegou primeiro e o load respondeu `cancelled`;
- `harness-race` — `ready` foi postado antes de o release ser recebido (liberação legítima; não é falha de runtime);
- `runtime-fault` — `ready` depois do release recebido, `cancelled` sem release, ou asset ainda carregado;
- `other:<code>` — contexto perdido, limite, decode: mantido à parte.

Código de saída 1 se houver `runtime-fault`, `other:*` ou qualquer resultado não `cancelled` em
`same-task`/`gated`. `racy` só reporta (a corrida é o achado).

## Evidências e status

| Superfície | Estado | Evidência |
|---|---|---|
| Runtime (Node + Chrome headless/SwiftShader) | **V local** | abaixo |
| Lib / CLI headless / CLI da janela / UI | N/A | nenhuma regra de domínio ou UI mudou; a prova é do harness do runtime. WKWebView nativo **não** foi exercido |

**Reprodução (antes):** 6 execuções de `node --test test/*.test.mjs` com concorrência padrão: 2 falhas, ambas
`splat.test.mjs:341 'ready' !== 'failed'`. Sozinho o teste passa em 6,8 s.

**Dose-resposta, mesma carga ambiente alta (load average 13–17 por outros agentes):**

| Chromes simultâneos | iterações por modo | `racy`: release perdeu a corrida | `same-task` | `gated` |
|---|---|---|---|---|
| 1 | 40 | 1 (2,5%) | 40/40 cancelled | 40/40 cancelled |
| 2 | 80 | 2 (2,5%) | 80/80 | 80/80 |
| 4 | 160 | 17 (10,6%) | 160/160 | 160/160 |
| 6 | 240 | 11 (4,6%) | 240/240 | 240/240 |
| 10 | 400 | 124 (31,0%) | 400/400 | 400/400 |

Total: 920 iterações por modo; `racy` 155 corridas perdidas (16,8%), **0 `runtime-fault`**, 0 `other:*`;
`same-task` e `gated` 920/920 `cancelled`. A taxa não é monotônica (6 < 4) porque a carga ambiente de
outros agentes variou entre as medições; o que se afirma é o contraste dentro de cada nível.
Uma primeira rodada de 10 Chromes classificou 5 empates de timestamp como `runtime-fault`: o
classificador comparava `performance.now()` (resolução grossa); passou a usar a ordem do log, e a
rodada refeita não tem nenhum.

**Depois:** 12 execuções consecutivas da suíte completa com concorrência padrão (85 testes: os 80 de antes
mais os 5 novos), 12/12 verdes, `TESSEL_REQUIRE_RUNTIME=1`, sob lock, com a máquina em load average 16–20.
Com a taxa de falha anterior (~1/3), 12 sucessos seguidos por acaso teriam probabilidade ≈ 0,8%; ainda assim isso é
evidência estatística, e o argumento de correção é o determinístico (gate + classificação por ordem de log).

## Limites

- A intermitência é função da carga: 12/12 não prova ausência de outra falha rara; novas falhas de `splat` seguem
  o BL091 (reabrir com seed, `--out` do bench e o log da suíte).
- A janela entre o load responder `ready` e o teste olhar é fechada por construção apenas no passo 6; outros
  passos de `splat.test.mjs` esperam por mensagens e não dependem de ordem entre dois `evaluate`.
- O gate segura o `fetch`; a fase de decodificação do Spark não é retida. Um release **durante a decodificação**
  (depois do fetch, antes do `ready`) não tem prova determinística aqui: `same-task` libera antes do fetch e `gated`
  durante o fetch. O código (`AssetStore.load` confere o token depois do loader) cobre os dois, mas a prova é só de leitura.
- Diretórios `tessel-splat-*` de execuções anteriores do teste (antes do `rmSync`) podem existir no tmpdir; não
  foram apagados por não ser possível provar quais são desta frente.
