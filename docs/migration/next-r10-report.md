# R10 report: independent export corpus and harness (BL008, BL019)

Branch `feat/directed-scene-next-delivery`, base `914c830`, dirty tree (no Git write by this front).
Feature doc: [scene-export-proof.md](scene-export-proof.md). Evidence record of the last full run:
`scripts/directed-scene-export-proof/results.json` (paths repo-relative; every hash below comes from it).

## Verdict

`python3 scripts/directed-scene-proof/with-gpu-lock.py python3 scripts/directed-scene-export-proof/proof.py --gpu-lock-held --report ...`
→ **exit 0, 23/23 phases PASS** against R1's delivered `scene.export` (CLI process and window socket) and the
`flow.export_scene` dialog (semantic protocol), with release `tessel-editor` and debug `tessel-render` rebuilt
from the working tree through the serialized helper during this run. The old `target/release/tessel-editor`
(before R1) was **not** used as evidence: it answered `unknown_operation` and the phase was BLOCKED (exit 3)
until the rebuild.

What this does NOT show: that a person can see or reach the control (no pixels, no pointer, BL043); heard audio
(BL046, nothing listens, no device); photons or seek-to-photon (BL032); other GPUs/platforms (BL054; SwiftShader on one
Apple-silicon machine); bit-exact frames (lossy codec, judged by probes and tolerances fixed beforehand); light/weather/
effects/rig/path/data-map/splat domains (the corpus does not use them); the header routing (`node:16`) and the preflight
report of the dialog (not driven by this harness).

## What was built (all under my ownership; no production source touched)

| Path | Role |
|---|---|
| `scripts/directed-scene-export-proof/export_corpus.py` | deterministic generator of corpus + `expected.json` from the independent model (`directed-scene-proof/model.py`) and exact fractions; `--check`, `--materialize` |
| `scripts/directed-scene-export-proof/corpus/` | `bundle.{main,ntsc,audio,detail}.json`, 3 PNGs, `audio/mix.wav`, `expected.json`, `provenance.json` (0.35 MB; the 33 MB 4K PNG is regenerated and hash-checked) |
| `export_verify.py` | verifier: ffprobe/ffmpeg container, exact rational fps/pts grid, BT.709-limited decode, flat-interior probes, dense 160x90 picture identity, 1/2/4 px detail stripes, Goertzel audio windows |
| `export_controls.py` | positive (4) and negative (20 injected defects) controls that prove the verifier can fail |
| `adapters.json` | contract adapter: `scene.export {scene,preset,destination,from,frames,wait?}` (+ `replace`, `audio`), result paths, async job key, accepted error codes, UI prefixes |
| `proof.py` | 23 phases, exit semantics 0/1/3/2, provenance report |
| `proof.spec.py` | 16 self-tests: determinism, tampered corpus fails, lying reports fail, BLOCKED paths |
| `crates/render/tests/scene_export_proof.rs` | 5 lib/headless tests + 3 `#[ignore]` tests that run the harness (fail on BLOCKED) |
| `docs/migration/scene-export-proof.md`, this report | docs |

### Seams to integrate (adapters, no phase code changes)

- `adapters.json` `ops.export.args` is the request; `$name` placeholders, a null drops the key.
- `ops.export.result.fields`: paths into the answer; present fields are compared with what ffprobe measured
  (a report that lies fails), absent ones are listed as `unreported`. R1's fields are confirmed:
  `destination`, `frames`, `fps` as **`{num,den,fps}`** (compared as the exact fraction `num/den`), `duration_seconds`,
  `output.width/height`, `audio {included,declared,omitted}`, `warnings` (`audio_omitted`), `verification`, and
  **`resources`** `[{asset,sha256,revision,bytes}]`, which the harness checks against the bundle's own
  `contentHash`/`revisionId`: the earlier adapter note saying `resources` was not reported was wrong and is corrected.
- `ops.export.async.job_key` = `job` (`wait:false` and the window host answer a job; result in `job.get.result`).
- `error_codes`: refusals must be typed; `audio_refusal` leads with `unsupported`; `audio_omitted_warning_codes` = `audio_omitted`.
- `ui.export_node_prefixes`: `flow.export_scene` first. Dialog fields `scene, preset, from, frames, audio (yes|no), destination`
  as `dialog.field.<name>`, `dialog.submit`.

## Measured

| Phase | Result | Notes |
|---|---|---|
| `corpus`, `model_vs_origin`, `identity_coverage` | 20/20, 1/1, 14/14 | corpus equals generator; model equals origin oracle; neighbouring frames are told apart (46-48 of 48 in the weakest case) |
| `verifier_controls_scene/audio/detail` | 13/13, 5/5, 6/6 | duplicated/swapped/black/short/25 fps/NTSC-rounded/BT.601/no-tags/truncated/extra-audio/proxy/flipped/shifted all detected |
| `baseline_render`, `baseline_detail`, `baseline_encode` | 20/20, 3/3, 36/36 | existing renderer and the harness's own encode vs the model; 4K original drawn 1:1 (contrast ≥ 0.98) |
| `export_main_1080_event` / `_cut` / `export_main_4k_cut` / `export_ntsc_1080` | 29/29 each | 1080p 24 and 16 frames, UHD 8 frames across the shot cut, 30000/1001; ~6 s, ~5 s, ~7 s, ~5 s wall (budgets 240/240/600/240 s) |
| `export_detail_4k` | 31/31 | original 4K PNG, no proxy: 1/2/4 px stripes survive, colour blocks within 12 |
| `export_invalid_requests`, `export_destination_exists` | 15/15, 18/18 | typed refusals leave no file and no mutation; `replace:true` replaces |
| `export_cancel` | 9/9 | progress observed, `job.cancel`: `cancelled`, folder listing unchanged (sentinel intact), destination reusable; final state in 2.4 s |
| `export_snapshot` | 26/26 | scene edited mid-job (930 probes would differ): file holds the request-time scene; a render afterwards shows the edit |
| `export_audio_policy` | 35/35 | observed: default `unsupported` + no file; `audio:false` exports with `audio_omitted` and `declared/omitted` stated |
| `export_restored_package`, `export_restored_managed` | 67/67, 32/32 | only the harness's own origin copy deleted; render and export from the restored project match the independent pixels |
| `socket_export` | 29/29 | the same export through the window socket of an isolated instance (own HOME/socket; the user's app untouched) |
| `ui_export_flow` | 74/74 | dialog by semantic ids; non-numeric frames → `invalid_value`, dialog open, no job, no file; valid submit → one job, verified file; scene with audio: `yes` → `unsupported` (no job/file), `no` → export + `audio_omitted` |

Tolerances (fixed in `expected.json` before any export existed): PNG probe ≤ 6; video probe ≤ 12 on every probe
(libx264 crf ≤ 18, 4:2:0, BT.709 limited decode; BT.601 fails); fps and pts exact; duration ±1 ms; picture agreement ≥ 75% of the
160x90 grid where the model keeps neighbours apart; 4K stripe retention ≥ 0.6/0.8/0.9 (1/2/4 px). Audio policy: a scene without audio has no
audio stream and claims none; a scene with a mix must carry it (measured), be refused typed with no file, or be video-only with a
typed warning; a silent drop fails.

Evidence record (`results.json`): harness `proof.py` c506c498…, `export_corpus.py` 20bbed37…, `export_verify.py` 6f0432c0…,
`export_controls.py` 275f9c69…, `adapters.json` 0f39203e…; `expected.json` 3fd0c377…; `tessel-render` 111e7edc… (19,547,616 B, 20:22Z),
`tessel-editor` release aee07e76… (35,833,456 B, 20:27Z); runtime `runtime.js` 125c678f…; ffmpeg/ffprobe 7.1.1; SwiftShader GL; 178 s for the full run;
git head `914c830`, 88 dirty files. Both binaries are flagged `older_than_sources: true`: other fronts kept editing after the build, so the
coordinator's final gate must rebuild and re-run this harness (3 minutes).

## Tests run and not run

Run: `proof.spec.py` 16/16; `scene_export_proof` (cargo, serialized helper) 5 passed, 3 ignored; full `proof.py` 23/23 (exit 0);
`--ignored export_proof_restored_projects` passed and `--ignored export_proof_window_flows` failed as designed with BLOCKED while the old editor binary was current
(before the rebuild); the harness scripts were run directly after.
Not run: the global gate (`pnpm`-style full test/clippy/build) is the coordinator's; `export_proof_headless_phases` through Rust was not repeated
(same phases passed directly); no pointer, header `node:16`, preflight report or file picker flows; no real GPU (`--gl gpu`).

## Limitations and findings

- The first corpus put the PCM-payload hash in `audio.sourcePcmHashSha256`; the resource gate rejected it (`hash_mismatch`): the field is the hash of the **whole WAV file**. Corrected and documented.
- `scene.export` answers `backend_unavailable` on a host with no renderer; the harness treats that and `unknown_operation` as BLOCKED, never PASS or FAIL.
- The identity check is deliberately conservative (flat probes + a masked 160x90 grid): it catches duplicated, swapped, frozen and shifted frames but not sub-pixel drift; budgets are first measurements on an idle machine, not promises.
- The verifier cannot catch a regression it has no model for (domains listed above); the corpus must grow with each new domain.
- An editor that reads the live scene instead of a snapshot is detected only when the edit changes enough probes (930 here); smaller edits are not proof of the snapshot.

## Patches and suggestions for the coordinator (matrix, backlog and indices are yours)

- **BL008** (Em andamento): evidence now independent of R1 at lib, CLI, socket and semantic-UI level: exports verified for 1080p/UHD/NTSC, cancel, snapshot, audio policy, restored projects (23/23, `results.json`). Remaining before "V": rebuild from the integrated tree and re-run; pointer-level/visual dialog proof (BL043/BL087); budgets revised explicitly.
- **BL019** (Parcial → proposed "V no export"): `scripts/directed-scene-export-proof/` is the export corpus and harness; add `scripts/directed-scene-export-proof/proof.spec.py` and `cargo test -p tessel-editor-render --test scene_export_proof` to the gate; the full proof takes ~3 min under the GPU lock.
- **BL011**: restored project (package and managed base) renders and exports to the independent pixels with only the harness's own origin copy deleted (`export_restored_*`).
- **BL086**: the export dialog (`flow.export_scene`) proven semantically (fields, refusals, one job, audio choice); the visible/pointer part stays open.
- **BL052**: exports declare and decode as BT.709 limited (file tags + decode by probes); BT.601 mix-up has a negative control. Transfer/range beyond that is not claimed.
- **BL090**: not addressed (alpha policy, minification vs origin); the 4K detail case only proves 1:1 minification-free drawing.
- **BL092**: untouched (lifecycle); R8's phase is independent.
- Suggested new item: "grow the export corpus per domain" (light, weather, effects, rig, path, data maps, splat) and a `--gl gpu` baseline (BL054).
- The message that asked for a report at `docs/migration/next-r1-report.md` named R1's file; I left it untouched and put this report in `next-r10-report.md`.
