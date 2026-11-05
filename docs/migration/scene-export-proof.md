# Directed scene export: independent corpus and acceptance harness (BL008, BL019; front R10)

Folder: `scripts/directed-scene-export-proof/`. Rust lib surface: `crates/render/tests/scene_export_proof.rs`.
Report of the delivery (results, hashes, limitations, patches for the coordinator): [next-r10-report.md](next-r10-report.md).

This proof judges `scene.export` (front R1, F042) **without trusting it**. It reuses nothing from R1's tests
or helpers, and it never compares an output with a copy of itself:

- expectations (probes, frame counts, rational rates, durations, audio windows, 4K detail geometry) are written
  by `export_corpus.py` from the independent reference model (`scripts/directed-scene-proof/model.py`, itself
  checked against the origin oracle by `model.spec.py`) and from exact fractions of each bundle's own fps;
- the verifier (`export_verify.py`) decodes the exported file with ffmpeg/ffprobe and judges it against
  `corpus/expected.json`; it is proven able to fail by positive and negative controls (below) before it judges
  anything real;
- the destination is whatever the operation wrote; the harness never fabricates it.

## Evidence surfaces (never mixed)

| Surface | Phases | What it proves | What it does not |
|---|---|---|---|
| data / harness | `corpus`, `model_vs_origin`, `identity_coverage`, `verifier_controls_*` | corpus is the generator's deterministic output; the model agrees with the origin; the verifier detects every defect injected into a real file | anything about the editor |
| lib (Rust, no process) | `scene_export_proof.rs` (5 tests) | core schedule (frames, exact rate, duration, size) equals the independent number for every case; invalid ranges are typed errors that change nothing; declared audio is never silently dropped; the adapter's request parses in the core | pixels, codec, files |
| renderer baseline | `baseline_render`, `baseline_detail`, `baseline_encode` | the corpus holds against the renderer that exists (`scene.render`, 320x180 probes and a 3840x2160 1:1 detail frame); the verifier judges real renderer pixels encoded by this harness's own ffmpeg | `scene.export`: these phases never call it |
| headless process | `export_*` | `scene.export` itself through the `tessel-render` JSONL process | the window |
| native socket | `socket_export` | the same op through an isolated editor instance's `ui.sock` (release `tessel-editor`, window host with `with_scene_renderer`) | a visible dialog |
| UI flow (semantic) | `ui_export_flow` | the `flow.export_scene` dialog driven by `get`/`set`/`invoke` only: fields, typed refusals that keep the dialog open and start no job, a submitted export judged like any other, and the audio choice (`yes` refused, only an explicit `no` omits, with the typed warning) | that a person can see or reach the control, the header `node:16` routing, the preflight report, the native file picker; no pixels, no pointer (BL043) |

Nothing here hears sound or measures a photon (BL046, BL032). No coordinate click is used. The user's app and
project are never touched; the window phases start a separate instance with a short `HOME` and socket.

## Exit semantics

`0` every selected phase ran and passed. `1` at least one FAIL. `3` no FAIL but at least one phase BLOCKED:
a missing operation, binary or window control. **BLOCKED is never a pass.** `2` harness usage error. A phase
that needs `scene.export` first sends an invalid-on-purpose request (0 frames): `unknown_operation` makes the
phase BLOCKED with the backlog id; any typed answer proves the op exists.

## Corpus (`corpus/`, 0.35 MB committed; the 33 MB 4K PNG is regenerated, hash-checked)

| Bundle | Role |
|---|---|
| `bundle.main.json` | 4 layers (sky, mid, hero, echo) over two shots with a cut at frame 48; a hero event at frame 12; stage slide and hero sway strong enough that neighbouring frames differ in dozens of probe cells |
| `bundle.ntsc.json` | same geometry at `30000/1001` |
| `bundle.audio.json` | same geometry plus `audio.mixRelativePath` (440 Hz for 2 s, then 880 Hz), 4 s mono WAV |
| `bundle.detail.json` | one plane filling a 16:9 frame exactly at 3840x2160 (1 texel = 1 pixel): 1, 2 and 4 px stripes, colour blocks, corner markers |

`sourcePcmHashSha256` is, despite its name, the hash of the **whole WAV file** (header included); the first
version of this corpus used the PCM payload hash and the resource gate rejected it with `hash_mismatch`.

| Case | Bundle | Preset | Frames (from + n) | Rate | Budget |
|---|---|---|---|---|---|
| `main_1080_event` | main | fullhd | 12 + 24 (event + sway) | 24/1 | 240 s |
| `main_1080_cut` | main | fullhd | 40 + 16 (across the shot cut) | 24/1 | 240 s |
| `main_4k_cut` | main | uhd4k | 44 + 8 (straddles the cut) | 24/1 | 600 s |
| `ntsc_1080` | ntsc | fullhd | 20 + 12 | 30000/1001 | 240 s |
| `detail_4k` | detail | uhd4k | 0 + 6 | 24/1 | 600 s |
| `restored_1080` | main | fullhd | 12 + 8 | 24/1 | 240 s |
| `snapshot_1080` | main | fullhd | 0 + 48 | 24/1 | 300 s |
| `audio_1080` | audio | fullhd | 36 + 24 | 24/1 | 240 s |

Budgets are wall clock of the op on an idle machine with SwiftShader GL. They are an initial budget, not a
promise of speed; the coordinator revises them explicitly.

## Tolerances (fixed in `expected.json`, before any export exists)

| What | Value | Why |
|---|---|---|
| rendered PNG vs model (flat interior) | max channel delta 6 | same as `scripts/directed-scene-proof`; no codec |
| decoded video probe vs model | max channel delta 12, 0 failed probes | libx264 crf≤18 + 4:2:0 at flat interiors; decode is **BT.709 limited**, and decoding the reference as BT.601 fails (negative control) |
| frame count | decoded frames (`-count_frames`) equals requested; `nb_frames` agrees when present | |
| rate | `r_frame_rate` and `avg_frame_rate` equal the bundle fps as an exact fraction; 30000/1001 never accepted as 30 or 29.97 | |
| duration / pts | last pts + one period equals `frames*den/num` within 1 ms; every pts delta is one exact period | |
| picture identity | each decoded frame must agree with the model's frame at the **same index** on ≥75% of the 160x90 grid (edge cells masked), where the model keeps neighbours apart by ≥24 cells; ≥50% of frames must be judgeable | catches duplicated, swapped, shifted and frozen frames |
| 4K detail | contrast retention of 1/2/4 px stripes ≥ 0.6 / 0.8 / 0.9, colour blocks within 12 | a proxy at ≤ 3/4 scale drops 1 px contrast below 0.2 |

## Audio policy (recorded, not assumed)

A directed scene has no audio unless its bundle names a mix. No directed audio is promised unless mixed.

- Bundle **without** `audio`: zero audio streams and the result must not claim audio.
- Bundle **with** `audio`: one of three outcomes is accepted: the mix is in the file (one audio stream whose
  Goertzel spectrum holds 440 Hz in the first half and 880 Hz in the second, duration equal to the video's),
  or the default request is refused with a typed code (`unsupported`) and leaves no file, **and** the
  explicit video-only request (`audio: false`) works and its result states `included: false`, what was
  `declared` and `omitted`, and carries the typed warning `audio_omitted`; or the file is video-only with a typed
  warning from `adapters.json`. A silent drop fails. R1 today: `unsupported` by default, `audio_omitted` with
  `audio:false`; in the window the same through the dialog (`audio` `yes` refused, only an explicit `no` omits).

## Phases

`python3 scripts/directed-scene-export-proof/proof.py --list` is the source of truth. GPU phases always run under
`scripts/directed-scene-proof/with-gpu-lock.py ... --gpu-lock-held`.

| Phase | Checks |
|---|---|
| `corpus` | every committed byte equals the generator's output; bundle hashes pinned; 4K PNG regenerated byte-for-byte; the cases cross the cut and the event |
| `model_vs_origin` | `model.spec.py` (origin oracle) |
| `verifier_controls_scene` | positive: reference at 24/1 and 30000/1001. Negative: 7 of 8 frames, 25 fps, 24 vs 30000/1001, duplicated frame, swapped frames, black frame, BT.601 decode, wrong size, no colour tags, truncated file, unrequested audio stream |
| `verifier_controls_audio` | positive window; wrong offsets, silent stream, no stream |
| `verifier_controls_detail` | positive: 4K original; negative: proxy, 3/4 proxy, flipped, shifted, BT.601 |
| `identity_coverage` | the model tells neighbouring frames apart for every case (the identity check is not vacuous) |
| `baseline_render` | real `scene.render` probes at 320x180 equal the model (7 main, 2 NTSC, 1 audio-bundle frames) |
| `baseline_detail` | the renderer draws the original 3840x2160 PNG 1:1 (stripes, blocks, markers) |
| `baseline_encode` | real renderer pixels at 1080p encoded by this harness's ffmpeg pass the verifier (24/1 and 30000/1001) |
| `export_main_1080_event`, `export_main_1080_cut`, `export_main_4k_cut`, `export_ntsc_1080` | the export's own file: container, rate, frames, pts, BT.709 tags, probes, identity, budget, result fields vs measured |
| `export_detail_4k` | original PNG, no proxy, stripes 1/2/4 px |
| `export_invalid_requests` | each corpus range error, unknown scene, unknown preset: typed code, no file, project unchanged |
| `export_destination_exists` | an existing destination is refused (or replaced only with `replace: true`) and the original survives |
| `export_cancel` | `wait:false`, progress observed, `job.cancel`: job ends `cancelled`, destination folder listing unchanged (sentinel file intact), history unchanged, destination reusable afterwards |
| `export_snapshot` | the scene is edited (hero position, camera fov) while the job runs; the file matches the unedited model and not the edited one; a render after the job shows the edit |
| `export_audio_policy` | the three audio outcomes above |
| `export_restored_package` | package, delete **only the harness's own origin copy**, restore, resolve, render matches the independent pixels, export matches |
| `export_restored_managed` | same through the managed base |
| `socket_export` | the op through the isolated window instance: same verification as the headless exports |
| `ui_export_flow` | BLOCKED while the build has no node with a prefix of `adapters.json` `ui.export_node_prefixes`; otherwise: open `flow.export_scene`, field nodes, `audio` offers `yes/no` defaulting to `yes`, a non-numeric frame count is `invalid_value` and the dialog stays open with no job and no file, a valid submit creates exactly one job whose file is verified, an audio-declaring scene with `yes` is refused (`unsupported`) with no job and no file, with `no` it exports with the `audio_omitted` warning |

Run order for an acceptance:

```sh
python3 scripts/directed-scene-export-proof/proof.spec.py                       # harness self-tests, CPU + ffmpeg
python3 scripts/directed-scene-proof/with-gpu-lock.py \
  python3 scripts/directed-scene-export-proof/proof.py --gpu-lock-held --report /tmp/te-r10-report.json
python3 /Users/wallyssonnunes/.cache/tessel-editor/cargo-serial.py test -p tessel-editor-render --test scene_export_proof --locked
python3 /Users/wallyssonnunes/.cache/tessel-editor/cargo-serial.py test -p tessel-editor-render --test scene_export_proof --locked -- --ignored   # same phases from Rust; fails on BLOCKED
```

## Adapter (`adapters.json`): where the real fields plug in

`op`/`args` of `export` are the frozen contract: `scene.export {scene, preset, destination, from, frames, wait?}`.
`replace` and `audio` are additive and omitted (null drops the key) except where a phase needs them. Result paths
are optional: a present field is **compared with what ffprobe measured in the file** (a report that lies fails);
an absent one is listed as `unreported` in the evidence and never assumed.

| Adapter path | Meaning | Matches R1's code today |
|---|---|---|
| `destination` | file written | yes (`job.rs` report) |
| `frames`, `fps`, `duration_seconds` | what the file holds. `fps` is an object `{num, den, fps}`: compared as the exact fraction `num/den`; the float is ignored (also accepted: `"n/d"`, number) | yes |
| `output.width/height` | file size | yes |
| `audio` | `{included, declared, omitted}` | yes |
| `warnings` | typed list `{code, message}`; `audio_omitted` when the declared audio was left out by `audio:false` | yes |
| `verification` | `{ok, ...}` the export's own check | yes (not trusted: judged separately) |
| `resources` | `[{asset, sha256, revision, bytes}]`; must name exactly the bundle's assets with the bundle's own `contentHash`/`revisionId` | yes |
| `async.job_key` = `job` | `wait:false` answers `{job, ...}`; result in `job.get.result` | same shape as `export.start` |

Error codes accepted for refusals are listed in `adapters.json` (`error_codes`); a refusal must be typed and
non-empty. Names are read from R1's working tree, which was uncommitted when this was written; if R1 changes
a name, edit `adapters.json`, no phase code changes.

UI node prefixes for the window flow: `flow.export_scene` (real id), `scene.export`, `export.scene`. Dialog fields: `scene`, `preset`, `from`, `frames`, `audio` (`yes`/`no`), `destination`, set as `dialog.field.<name>`, submitted with `dialog.submit`.

## Provenance of every evidence record

`proof.py --report` writes: harness file hashes, git head/branch/dirty count, corpus file hashes and the
`expected.json` hash, tolerances, hashes/sizes/mtime of the binaries the selected phases use (`tessel-render`,
`tessel-editor`), the runtime bundle hashes (`runtime.js`, `index.html`) and Node version, ffmpeg/ffprobe/python/OS
versions, GL mode, the work-directory budget and every phase's checks with metrics. `older_than_sources: true`
means a source file is newer than the binary: rebuild before trusting a result. Evidence without those hashes is
not evidence.

A missing operation (`unknown_operation`) and a surface with no renderer attached (`backend_unavailable`, e.g. the
window host before `MediaHost::with_scene_renderer` is wired there) are both BLOCKED, never FAIL and never PASS.

## Limits

See `NOT_CLAIMED` in `proof.py` and the report: no heard audio, no photons, no other GPU or platform, no bit-exact
frames (the codec is lossy, pixels are judged by probes and dense picture agreement), no light/weather/effects/
rig/path/data-map/splat domains (the corpus does not use them), and a lib/CLI result never stands in for the
window.
