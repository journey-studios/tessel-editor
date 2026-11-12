# Scene engine API: library, Node module, CLI, thin preview

How a render bundle is edited, voiced and rendered, as built from
[scene-engine-api-plan.md](scene-engine-api-plan.md). One set of rules, three layers, no copy of a rule in
any shell. For using it, read the skill `.claude/skills/scene-cli/SKILL.md`.

## Architecture

```
@tessel/scene-engine/core   pure rules, no I/O          editBundle, finalizeBundle, properties, audio and caption plans
@tessel/scene-engine/node   files, TTS, ffmpeg, Chrome  openBundle, makeTakes, cutTakesFromMix, renderBundle, ...
shells                      arguments in, JSON out      bin/scene.mjs (CLI), scripts/studio-harness.mjs (preview), scripts
```

- **Core** never reads a file or the network. Every function is `(bundle, input) -> result` and returns
  `{ ok: true, ... }` or `{ ok: false, errors }`; nothing throws for a refusal.
- **Node** is the only place that writes a bundle: `BundleFile.update` reads the file fresh, runs the
  change, prunes the outline, validates with the schema and the camera joins, then replaces the file in
  one step (tmp file + rename), one write at a time per file in the process. The preview server and a
  script in the same process can no longer overwrite each other from stale copies.
- **Shells** parse arguments or HTTP, call the two modules and print what they answer. The Studio's
  preview harness has no WAV, TTS, take, narration, outline or lighting logic; the CLI's only own logic
  is argument parsing and the exit code.
- **One plan, two players.** The Studio's playback and the MP4 render take what sounds from the same
  functions (`playableTakes`, `mixSilences`; `audioMixPlan(bundle)` puts them together: the mix cut to the
  edit with its silent spans, every take that sounds and where) and the captions from `captionLayout`
  (the overlay's geometry in pixels), so the export matches what the Studio plays.

## API reference

### Core (`@tessel/scene-engine/core`)

| Function | Does |
|---|---|
| `editBundle(bundle, edits, { editFrame? })` -> `{ ok, bundle, notes } \| { ok: false, errors: BundleEditError[] }` | Applies a list of edits (`set`, `camera`, `narration`, `outline`, `rig`, `path`) in order on the bundle as changed so far, then `finalizeBundle`. Pure. |
| `finalizeBundle(bundle)` -> `{ ok, bundle } \| { ok: false, message }` | `pruneOutline` + schema + `validateEditJoins`: what any write must pass. |
| `describeBundleProperties(bundle, { editFrame?, prefix? })` | Every property path with kind, bounds, options and value. |
| `listBundleProperties`, `getBundleProperty(bundle, path, { editFrame? })` | Property objects / one value. |
| `setProperties(bundle, [{ path, value }], { editFrame? })` | Writes several property paths, all or nothing. |
| `applyPropertyChange(s)(bundle, change(s), path)` | Writes `PropertyChange`s (lighting, weather, weatherKeys, effects, ground, camera, rig, path) and checks the result. |
| `parseSceneSave(kind, body)` | Validates a whole-object scene save (`weather`, `weatherKeys`, `effects`, `ground`, `lighting`, `rig`, `path`) into `{ sceneId, change, answer }` or `{ ok: false, message }`. |
| `SCENE_EFFECTS`, `describeSceneEffects()` -> `SceneEffectDescription[]` | The catalog of post-production looks (`ink`, `dream`, `film`): kind, title, summary, technique, cost, limits and order in the pass, as plain data. The kinds (`SCENE_EFFECT_KINDS`), the schema of a scene's `effects` (`sceneEffectsSchema`), the property paths `scene/<id>/effects/<kind>` (`effectPath`) and the shader assembly derive from it. See [scene-effects.md](scene-effects.md). |
| `applyCameraCommands`, `analyzeCameraClip` | Camera layer (see [camera-commands.md](camera-commands.md)). |
| `applyNarrationCommands`, `captionsAt`, `mixGainAt` | Narration layer. |
| `applyOutlineCommands`, `pruneOutline`, `effectiveFlags` | Outline (groups) layer. |
| `parseSrt(text)`, `srtNarrationCommands(bundle, cues, trackId?)` | SRT to narration lines on the edit (cue times minus `audio.editStartSeconds`, cut to the edit, `fromMix` span). |
| `audioMixPlan(bundle)` -> `{ durationSeconds, mix, takes }` | The audio of the edit: what plays, from where, where silenced. `playableTakes`, `mixSilences` are its parts. |
| `captionLayout(captions, size)`, `placeCaptionBoxes(layout, measured, size)` | The caption boxes (text split into shown / coming, sizes in px) and where they sit. |
| `linesMissingTakes`, `linesInMix` | Which lines still need a TTS take / can be cut from the mix. |
| `summarizeBundle`, `showLayer(bundle, layer)`, `parseShowLayer` | What `scene show` prints. `SHOW_LAYERS` are `camera`, `narration`, `outline`, `lights`, `weather`, `effects`, `rig`, `paths`, `fx`; the `effects` layer is `[{ sceneId, effects }]` for every scene (`null` when it has none) and the summary counts `scenesWithEffects`; the `fx` layer is the bundle's `fx` whole (`null` when there is none) and the summary has an `fx` entry only when the bundle has one. |
| `applyFxCommands`, `explainFx`, `expandPreset`, `listFxEffects`, `suggestTransition`, `resolveFxFrame` | The fx layer (clips, transitions and presets over the shots; see [scene-fx-plan.md](scene-fx-plan.md)). It is not the scene's looks (`effects`, above): `scene.effects` is the base look of a scene, drawn first, and `fx` is applied after it. `editBundle` takes `{ fx: FxCommand }`; a preset is `expandPreset` -> `BundleEdit[]`, applied all-or-nothing by `editBundle`. Properties: `fx/<clipId>/...`, `fx/track/<trackId>/...`, `transition/<transitionId>/...`. |

### Node (`@tessel/scene-engine/node`)

| Function | Does |
|---|---|
| `openBundle(file, { assetsDir?, onSaved? })` -> `BundleFile` | `{ path, dir, assetsDir, bundle, read(), save(bundle), update(change) }`. |
| `changeBundle(file, change, { write })` | The same change as dry run (`write: false`) or write; adds `written`. |
| `createTtsClient({ baseUrl, model, voice })`, `ttsConfigFromEnv(env)` | OpenAI-compatible `/audio/speech` client (`synthesize`, `listVoices`); reads the `X-Timings` header. |
| `makeTakes(file, { clipIds?, missing?, tts, onProgress? })` | A take per line, one after another, each set as soon as made; a failure stops the batch there. |
| `cutTakesFromMix(file, clipIds)`, `linesToCutFromMix(bundle)` | Cuts each line's span out of the mix into its own take, all in one write. |
| `checkBundleFolder(file)` | Schema + joins + every file the bundle names exists; `warnings` (effects layer advice) only when there are some. |
| `renderFrames(path, { frames, outDir, contactSheet?, force? })` | Chosen edit frames as PNGs (and a contact sheet) through the same viewer page as `renderBundle`. |
| `renderFxPreview({ effectId, params?, frames?, outDir, force? })` | One effect over the test plates as one PNG strip; needs no bundle. |
| `renderBundle(path, { outDir, label?, captions?, sampleFrames?, onProgress? })` | Renders every frame in Chrome, encodes, builds the audio from `audioMixPlan`, draws captions, muxes, verifies video and audio against the plan. |
| `audioGraphOf`, `audioArgsOf`, `createCaptionPainter` | The ffmpeg filter graph of a plan; the sharp/Pango caption painter. |
| `wavChunks`, `sliceWav`, `wavDurationSeconds`, `SceneError` | WAV helpers (also streaming headers); the error type with `.code` and `.details`. |

`SceneError` details use the engine's shape: `{ code, message, hint?, index?, layer?, path? }`.

### CLI (`bin/scene.mjs`)

See the skill for every command. JSON on stdout, exit 0 ok / 1 refused / 2 usage, nothing written without
`--write`. `scene effects` prints the catalog of looks and needs no bundle; `scene show --layer effects` and
`scene props|get|set ... scene/<id>/effects/<kind>` read and write a scene's looks.

Effects (`scene fx`), from the effects plan's F3:

```
scene fx catalog [--family f] [--tag t] [--mood m]   scene fx show <effectId>   scene fx describe [--locale l] [--max-chars N]
scene fx preview <effectId> [--params '{...}'] [--frames N] --out <dir> [--force]
scene fx <bundle> list | explain | apply '<FxCommand[]>' [--write]
scene fx <bundle> preset <presetId> --start N --frames N [--scope '{...}'] [--set k=v ...] [--write]
scene fx <bundle> transition list | set <from> <to> <effectId> --frames N [--align a] [--set k=v ...] [--write] | suggest <from> --mood m
scene show <bundle> --layer fx      scene validate <bundle>   (adds `warnings` when the layer has advice)
scene render <bundle> --out <dir> --frames 120,240 [--contact-sheet] [--force]
```

The catalog verbs take no bundle. `--out` is the opt-in for what writes images and never replaces a file
without `--force`. `scene fx gallery` comes with the effects plan's F6.

### Pipeline example (`examples/narrated-scene.mjs`)

`narrateScene({ bundlePath, srtPath | linesPath, workDir, ttsUrl, render })`, also a script: copies the
bundle, puts the lines on the timeline (SRT cues cut from the mix, or a list of lines voiced by the TTS),
groups the voice track, brightens a light, validates and renders. It imports only `core` and `node`.
Its spec (`src/node/narrated-scene.spec.ts`) runs everything but the render on a fixture with the fake
TTS and logs that the render is left to the demo run: a render needs the generated art (not in git) and
Chrome.

### Studio preview server (`scripts/studio-harness.mjs`)

HTTP only: `PUT /bundle-edit` (`{ edits, editFrame? }`, every narration, outline and camera command goes
through it), `/bundle-camera`, `/bundle-camera-commands`, `/bundle-narration-take`,
`/bundle-narration-from-mix`, and the whole-object scene saves `/bundle-weather`, `/bundle-weather-keys`,
`/bundle-effects`, `/bundle-ground`, `/bundle-lighting`, `/bundle-rig`, `/bundle-path`. A refusal is a 400 with
`{ error, code, errors }`.

## Migration table

| Before | Now |
|---|---|
| `scripts/camera-command.mjs`, `pnpm camera:command` | `scene camera <bundle> analyze\|apply` |
| `scripts/narration-command.mjs`, `pnpm narration:command` | `scene narration <bundle> show\|apply\|import-srt` |
| `scripts/outline-command.mjs` | `scene outline <bundle> show\|apply` |
| Inline SRT parser in `narration-command.mjs` | `parseSrt`, `srtNarrationCommands` (core) |
| Harness `PUT /bundle-narration`, `PUT /bundle-outline` | Removed: `PUT /bundle-edit` with `{ narration }` / `{ outline }` entries |
| Harness `synthesize`, `listVoices`, WAV helpers | `createTtsClient`, `wavChunks`, `sliceWav` (node) |
| Harness `makeNarrationTake` | `makeTakes(file, { clipIds: [id], tts })` |
| Harness `cutTakesFromMix` | `cutTakesFromMix(file, clipIds)` |
| Harness per-route validation of weather, lighting, rig, path saves | `parseSceneSave` + `applyPropertyChanges` |
| `scripts/viewer-harness.mjs` browser and page code | `createViewerHarness`, `openViewerBrowser`, `renderViewerFrame` (node) |
| `scripts/render-pilot.mjs` render, mux and checks | `renderBundle` (node); the script is a call |
| Render without captions; audio = bundle mix only | Captions drawn from `captionLayout`; audio from `audioMixPlan` (mix with silent spans + every take) |
| Hand-written `render-bundle.json` edits | `scene set\|edit --write`, `file.update` |
| `FxPresetEdit` (effects plan F1) | `BundleEdit[]` from `expandPreset`, resolved by `editBundle` (all or nothing) |
| Plates and golden bundles inside `scripts/fx-lab-bundles.mjs` | `writePlates`, `goldenBundle` (node, `src/node/fx-lab-bundles.ts`); the script re-exports them |
| Editing the effects layer by hand in the bundle | `fx/...` and `transition/...` properties, `{ fx }` edits, `scene fx ...` |

## Decisions where the plan met the code

- Engine files are kebab-case (`bundle-file.ts`, `mix-plan.ts`) like the rest of the package.
- Subpath exports: `./core`, `./node`, `./testing` (a fixture bundle for specs).
- Property reads are `listBundleProperties` / `getBundleProperty`; `describeBundleProperties` is the plain
  data version for shells. A rig change carries its `rigId` in `PropertyChange`.
- `finalizeBundle` also runs `validateEditJoins`, because the old server ran both checks before every write.
- A path command naming a path the scene lacks answers `UNKNOWN_PATH_ID` (`UNKNOWN_PATH` stays for property paths).
- Error entries use `index` and `layer` (where in the list, which layer) in place of the old `command`.
- `esbuild`, `playwright-core` and `sharp` stay devDependencies and are `external` in tsup: the Node module
  is for the monorepo's own tools, not a published package.
- The Node specs (`src/node/**`) run only under the package's vitest config, not the root jsdom one: the root
  setup mocks `fetch`, and the TTS client specs need a real HTTP server.
- The harness is split in three files (`studio-harness.mjs` 317 lines, `studio-http.mjs`,
  `studio-project-api.mjs`): the project registry routes and the HTTP helpers are not rules, but they are
  not small either; the engine-facing part is the first file.
- `scene preview` starts `scripts/scene-studio-preview.mjs` as a child process; the preview is a long-lived
  server with its own build step and the CLI keeps to the engine's two entry points.
- Caption font: `sans-serif Medium` through Pango/fontconfig (Helvetica-like on macOS, the distro's default
  sans on Linux); set `TESSEL_SCENE_ENGINE_CAPTION_FONT` to pin one. Pixels differ from the Studio's DOM
  text only by font rasterisation; the box geometry and colours are the overlay's.
- Studio browser specs need art under `output/`, which is not in git: they ran against copies of the demo
  art (`/tmp`), and the transport spec against fixtures generated with `pnpm fixture:generate` and
  `pnpm fixture:transport-audio`.

- A property error can carry a `hint` (what would have worked); `editBundle` passes it on in the entry's error. A
  look path that names no look answers `UNKNOWN_PATH` with `Known paths under scene/<s>/effects/: ink, dream, film.` (any scene area: the paths under it, or the scene's areas).
- The post-production looks are described once, in `src/effects/catalog.ts`; the GLSL of each look is its own module
  under `src/runtime/post-effects/` and the pass is assembled in the catalog's order. Splitting the shader moved no
  pixel: `scripts/effects-reference.mjs` hashes the frames before and after (see [scene-effects.md](scene-effects.md)).
- The runtime's `setSceneEffects` is the Studio's live preview override, not a bundle edit; scripts set looks with
  `editBundle` / `setProperties`, which is what the render reads.

## Out of scope

Reparenting scene nodes in groups (world-transform preservation), per the plan.
