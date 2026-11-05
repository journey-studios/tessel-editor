# Directed scene: synthetic corpus and end-to-end proof harness (BL019, front P)

Status integrado: **408/408 checks** no conjunto selecionado da rodada M/N/O/P (ver [relatório](round-mnop-report.md) e [hashes](scene-integration-results.json)). Plano, pixels headless/nativos, pacote restaurado sem origem e árvore semântica foram desbloqueados. `context_loss` e `export_directed` permanecem não executados. As seções abaixo descrevem a entrega original de P; os adapters atuais estão no código e em F040/F041.

## What is proved today, and on which surface

Evidence is kept apart per surface. One surface never stands in for another.

| Surface | Phase | Result (2026-11-05, head `192e766`, dirty tree) |
|---|---|---|
| data | `corpus`, `model_vs_origin` | PASS 13/13 and 1/1: the corpus equals what the generator writes; the reference model reproduces the origin oracle |
| headless process (`tessel-headless`, JSONL, no renderer) | `headless_flow` | PASS 80/80 |
| native window socket (own isolated editor instance) | `socket_flow` | PASS 80/80: the same flow, same checks, through `ui.sock` |
| Rust lib, no process | `crates/core/tests/scene_flow.rs`, `crates/render/tests/scene_flow_resources.rs` | PASS 6/6 and 3/3; 1 + 1 `#[ignore]` BLOCKED (BL063, BL011) |
| everything that draws, packages or exports | the other phases | BLOCKED, see below |

The flow (`run_scene_flow`, shared by the headless and socket phases so the two cannot drift):
attach the corpus bundle, select, evaluate frames **out of order**, compare camera and world
matrices with the independent model, gesture edit (begin, two updates, commit) on the hero,
edit the camera fov, typed rejections (out-of-range fov, unknown scene) that leave history and
frame untouched, undo/redo (a gesture is one history step), save, check the document
(schema 2, `scenes` block, assets by `assetId` and relative path, never absolute), reopen,
re-check the same frames and the committed edits, save again and require an identical scene block.

## Rust tests (lib level, no process, no GPU)

- `scene_flow.rs` (core): corpus attaches clean; frames asked out of order match the model;
  the `ScenePlan` of every frame matches the model in any order and names resources by `assetId`;
  a frame outside the edit is a typed error that changes nothing; selection, gesture, edit, undo,
  save and reopen form one flow; the swap bundle changes content identity, not geometry.
- `scene_flow_resources.rs` (render, uses media `preflight`/`materialize`): plan and manifest agree on
  identity per frame; resources resolve by `assetId` from the transport root after the origin
  folder is deleted; a revision-2 swap lands beside revision 1, and an in-place swap is refused
  (`resources_invalid` with the problem `destination_conflict`, file untouched).
- The two `#[ignore]` tests fail on purpose with `-- --ignored` and carry the blocker text
  (BL063: `scene.plan` is `unknown_operation`; BL011: package preflight answers
  `not_packaged`/`unsupported`). When the API lands, remove the `#[ignore]` and they become the
  proof.

## Where the expected values come from (not from the destination)

`model.py` is an independent implementation, written from the render-bundle 1.0 contract and the
origin: camera clips (dollyIn/hold/truckRight, easings, progress `f/(duration-1)`), Euler XYZ,
cycle/event/static channels, sockets, column-major matrices, painter order and a pinhole
ray-caster for the pixel probes. It raises `Unsupported` instead of guessing.
`model.spec.py` runs it against `docs/migration/fixtures/expected` (the origin oracle,
`packages/scene-engine@d6adac15`): camera on `valid-minimal-continuous`, matrices and sockets on
`valid-graph-sockets-performance`. A destination render is never copied into `expected.json`.

Pixel probes are chosen only where the pixel and its +-2 px neighbours agree, so texture
filtering and an sRGB/linear round trip can move a channel by a few units but never change the
layer. `proof.spec.py` also checks that each asset has a colour of its own on every frame.

## Corpus (`scripts/directed-scene-proof/corpus/`)

Drawn from scratch by `corpus.py`; `provenance.json` has the SHA-256 of every file and of the
generator and model; `private_media` is false. PNGs use stored deflate blocks, so the bytes do
not depend on a zlib build. `python3 corpus.py --check` fails on any difference or stray file.

- Assets: `sky` 64x36 opaque, `mid` 48x48 straight alpha with a magenta alpha-0 fringe (a
  premultiply/matte bug shows up as pink), `hero` 32x32 with an alpha-0 corner,
  `assets-rev2/hero.png` (same `assetId`, different content, for the swap).
- Bundle `proof-p-corpus` (rev-1) and `bundle.swap.json` (rev-2): 96 frames at 24/1, 320x180.
  Nodes `root`, `sky` (z -4), `mid`, `stage` (event channel, frames 12-36), `hero` (cycle on Z, period
  48, phase 5, child of `stage`), `echo` (same asset as `hero`, static scale 0.6). Camera clip
  `cam-main`: dollyIn 32 frames easeInOutCubic, hold 16, truckRight 48 easeOutCubic; two
  continuous shots (0-48, 48-96).
- Frames `[0,11,12,20,31,36,47,48,60,95]` cover both sides of the event boundary, the shot cut and
  the last frame. `SEEK_ORDER=[95,0,48,12,47,36,11,60,20,12,31,0]` repeats frames 12, 0 and
  revisits across the cut.

## Tolerances (recorded in every report)

| Quantity | Rule |
|---|---|
| camera | `abs(a-e) <= 1e-9 + 1e-12*abs(e)` |
| world matrix | `abs(a-e) <= 1e-10 + 1e-12*abs(e)` |
| pixel | `max channel delta <= 6`, 8-bit sRGB, flat blocks only |
| gesture edit | exact (binary64 round trip) |

The report also records git head/branch/dirty count, the sha256, size and mtime of each binary
(with `older_than_sources`: a stale binary is flagged, not hidden), the GL backend, the corpus
bundle hash and the harness version. A pixel claim without these is not a claim.

## Commands

```bash
cd /Users/wallyssonnunes/www/tessel-editor
python3 scripts/directed-scene-proof/model.spec.py      # model vs origin oracle (4 tests)
python3 scripts/directed-scene-proof/proof.spec.py      # harness self-tests (10 tests)
python3 scripts/directed-scene-proof/corpus.py --check  # corpus is what the generator writes
python3 scripts/directed-scene-proof/proof.py --list
python3 scripts/directed-scene-proof/proof.py --report /tmp/proof.json
python3 scripts/directed-scene-proof/proof.py --phases headless_flow,socket_flow
# binaries: --bin-headless/--bin-render/--bin-editor; GPU: --gl swiftshader|angle-metal
# Rust (serialized; never run cargo directly):
python3 /Users/wallyssonnunes/.cache/tessel-editor/cargo-serial.py test -p tessel-editor-core --locked --test scene_flow
python3 /Users/wallyssonnunes/.cache/tessel-editor/cargo-serial.py test -p tessel-editor-render --locked --test scene_flow_resources
# Show the blocked Rust phases (fail on purpose until the API exists):
... test -p tessel-editor-core --locked --test scene_flow -- --ignored
... test -p tessel-editor-render --locked --test scene_flow_resources -- --ignored
```

Exit codes of `proof.py`: 0 every selected phase PASS; 1 some FAIL; **3 some BLOCKED and none FAIL
(not a pass)**; 2 usage. Negative control: `--corpus <copy with a tampered file>` exits 1.

The harness takes `/tmp/tessel-scene-gpu.lock` (`fcntl.flock`, held until the process exits)
only when a phase needs Chrome/GPU, so phases that only talk to `tessel-headless` or the socket
never wait on it. Its own temporaries stay under 2 GiB, with 6 GiB free required, and only its
own files are removed. Each editor instance gets a short isolated `HOME` and socket; the
user's app is never touched.

## Interfaces needed (blocked phases)

The harness does not hold any editor rule. Operations that do not exist yet are described in
`interfaces.json` (`op` and `args` are proposals); the phase sends the real request and
`unknown_operation` makes it BLOCKED with the backlog item. After integration the coordinator
edits the names in that file; no phase code changes.

| Phase | Needs | Backlog |
|---|---|---|
| `scene_plan` | `scene.plan {scene, frame}` through `ops::parse_op`, answering `ScenePlan::to_json`, behind the admission gates | BL063 |
| `render_headless_pixels`, `render_seek_out_of_order`, `resource_swap`, `socket_render_pixels` | `scene.render {scene, frame, width, height, out?, probes?, resources_root?}`: resources by `assetId` from an approved root, answered only after the frame is drawn, `probes` read back (`inspected=true`), identity of the resources used | BL007 |
| `context_loss` | the scene renderer goes through the same `frame.render` page path that `scripts/viewport-lifecycle-proof.py` already loses and restores | BL007 |
| `package_restore_no_origin` | package/restore of a project with a scene carries its resources (today: `not_packaged` / `unsupported`) | BL011 |
| `ui_semantic` | a `scene`/`hero` node in the live UI tree and a selection that round-trips | BL014 |
| `export_directed` | `scene.export` reusing the plan and resource binding of `scene.render` | BL008 |

When the renderer exists, the pixel phases already encode: frames out of order (`SEEK_ORDER`),
a `hero` rev-1 to rev-2 swap while keeping geometry (`bundle.swap.json`), and context loss in
the middle of a sequence. They compare against `expected.json` probes with the tolerance above.

## Not claimed

- Physical pointer input (BL043): no coordinate clicks; gestures are semantic commands.
- Seek-to-photon latency (BL032): rAF/hash timing is not a photon; nothing here measures one.
- Any pixel, package or export result: those phases are BLOCKED, so none exists.
- Hero/echo draw order: both use one texture and never overlap in the corpus, so their relative
  order is not discriminated (sky/mid/hero order is).
- Evidence from `tessel-headless`/lib stands for neither the native window nor the renderer.

## Known limits

- The binaries found at the default paths were older than the sources (`older_than_sources`);
  build them through the serialized helper before using the numbers as release evidence.
- `ui_semantic` blocks on the current UI tree, so UI evidence is limited to the socket `scene.*`
  flow.
- The pixel model covers planes with straight-alpha textures, nearest sampling, painter order.
  Anything else (filtering, post effects, depth) raises `Unsupported`.

## Suggested backlog / matrix lines (coordinator)

- BL019: Parcial. Corpus, independent model and harness delivered; headless and socket flows PASS;
  the pixel/package/export phases wait for BL007/BL011/BL008/BL063.
- BL063, BL007, BL011, BL008, BL014: add "unblocks `proof.py` phase X; edit `interfaces.json`".
- BL021 (socket evidence): `socket_flow` 80/80 on an isolated native instance (not a pixel claim).
