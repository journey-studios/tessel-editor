# Emenda A1 — semântica de `audio.editStartSeconds` (BL016.5, BL019)

Registro **imutável** do que existia antes da emenda. Nenhuma fase do harness lê esta pasta. O texto completo
(motivo, hipóteses, impacto, resultado depois) está em `docs/features/scene-acceptance.md`, seção "Emenda A1".

## O que foi emendado

A suposição **A1** do oráculo (`oracle.py` em `0d95be1`: "`mix[0]` ↔ `editStartSeconds`", o arquivo de mix começa em
`editStartSeconds` da linha do tempo e antes disso há silêncio) contradiz a origem. A decisão do coordenador foi **manter a
origem**; o oráculo e o gerador do corpus foram corrigidos, o contrato do editor (S9) **não** foi alterado.

Origem fixada `d6adac15c61f4ef582a7ec259397b2e025d1f059` (`/Users/wallyssonnunes/www/journeystudios/tesselapp`, `git show`, só leitura):

| Arquivo | Blob | Linhas | Regra |
|---|---|---|---|
| `packages/scene-engine/src/transport/audio-window.ts` | `721554a14a1ffd4ee032d81cbfce87b5b775321d` | 10–15 | `editSecondsToSourceTime(t) = editStartSeconds + t`: o WAV é a fonte inteira; o instante de edição `t` toca o arquivo em `editStartSeconds + t` |
| `packages/scene-engine/src/node/render.ts` | `123e9039c7546a7a21578b73603b49d46caa5b7a` | 246–248 | recusa `AUDIO_SHORT` quando `duration(fonte) + 0,001 < editEndSecondsExclusive` (a fonte precisa cobrir `[0, editEnd)`) |
| idem | idem | 250–252 | recusa `AUDIO_VIDEO_DURATION` quando `editEnd − editStart` difere da duração da edição por mais de 1 ms |

## Antes (commit `0d95be13f9a99805ae816225134a580268576b65`, harness `c03b41e`/`0d95be1`)

| Artefato | SHA-256 antes |
|---|---|
| `tolerances.json` (**inalterado**, também depois) | `696c3cd112dbfbefecd23e2849162be8df3a783d512a6f407b1369fe2ed1c722` |
| `oracle.py` | `59240b24db0523a0a14518198550112d3f30e433a0135e6126b8ea82f8537ffb` |
| `corpus.py` | `4681351226d9790937ead9c06d5e6e1c7c623662b703588d6f25449f9672621a` |
| `verify.py` | `2aea5a286de6441a5201e725295861aeaf4785890fc5f3afffa9347d2ecc16b9` |
| `controls.py` | `cabd701b64bae7783e6113451bddc5adbfd76172bb5a6aed694bf48171296f0f` |
| `proof.py` | `7cb153d32b86bbd5ce01195538fd0e89e434190ee9ab681c80fe926b7987ed9d` |
| `proof.spec.py` | `96a5667ba2a624748cc44c4f65a5a1afdc3b1ba74777aa9d05a927380aa6b3c9` |
| `corpus/expected.json` (cópia em `before/`) | `71c9508b16d46ecb338b3e707f47d26235511fc986adf46e0b89381a91f4bad2` |
| `corpus/provenance.json` (cópia em `before/`) | `b48cd207c199224aa3062af18f55ad0a568b3c03dd246b7f0685986d753958f5` |
| `corpus/bundle.audio24.json` (cópia em `before/`) | `1b181b3f080526ef8b9cb99453a2a03060fdbb013ebfbe7e542b8af6229439b6` |
| `corpus/bundle.audio-ntsc.json` (cópia em `before/`) | `3919bb6238c633aef31e696506a043b8aac444fb3a2b0a926699363b2618c767` |
| `corpus/bundle.full.json` (cópia em `before/`) | `32ef214b974c93b2cab3f121c41a075981b5309eb7ab30f9d1618f9e9838cede` |
| `corpus/audio/mix.wav` (8 s; **mantido byte a byte** como `corpus/audio/mix-short.wav`) | `cc252a2b0f8ec9695772e0ef51b01b7700d242b72af9a1e77ca10f0eeaffba75` |
| `results.json` (primeira execução completa, `1b5953e`; **permanece no lugar, não reescrito**) | blob `b787b48cc617ebace5ca840ade0400a8d52140b4` em `0d95be1` |

Janelas declaradas antes: `audio24` `[1,0; 9,0)` para uma edição de 12 s, `audio_ntsc` `[0; 8,0)` para 10,01 s, `full` `[0; 8,0)` para
2 s, sobre um WAV de 8 s com os tons em "tempo de mix" (A1). Pela origem as três janelas são inválidas (`AUDIO_SHORT` e
`AUDIO_VIDEO_DURATION`), e o `audio24_window_b` lê o arquivo até 9 s.

## FAIL anterior (proveniência)

`before-A1.report.json` e `before-A1.log`: o harness em `0d95be1` (árvore limpa, nenhuma edição) contra o binário da S9
original `t3-205dc567/target/debug/tessel-render` (sha256 `c5122a4e7f2b3735587eda5fc11a7ab9624312d88f75c56980ca382d56e2c86a`, worktree da S9 em
`20604c462613f47bd27d6044678031735e59ff92`, árvore limpa, nenhum fonte mais novo que o binário), sob o lock de GPU, SwiftShader:

| Fase | Resultado antes | Causa |
|---|---|---|
| `tolerances_frozen` / `corpus` / `models` / `controls_audio` / `baseline_audio` / `audio_off` | PASS 6 / 32 / 16 / 26 / 46 / 70 | — |
| `audio_export` | **FAIL** 75/92 | 17 checks: `audio24_window_a` com correlação ≈ 0 e atraso no limite da busca (+7200 amostras; o export tocou o arquivo em `editStart + t`, que é a regra da origem, e o oráculo A1 esperava outro trecho); `audio24_window_b` recusado com `audio_source_short` (`file_frames 352800`, lê até o quadro `396899`); a fase abortou em seguida por `destination_exists` no probe `seam-probe/probe.mp4` deixado pelo primeiro probe, de modo que `audio_ntsc_window` **nunca rodou** |
| `audio_package_export` | **FAIL** (0 checks) | o mesmo `destination_exists`: o probe da fase anterior deixou `seam-probe/probe.mp4`; defeito de isolamento do harness, **não** do editor |

Duas causas independentes, ambas do harness: (1) a suposição A1; (2) as fases dividiam o diretório `seam-probe`.
