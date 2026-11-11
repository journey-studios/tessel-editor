# Scene effects: the post-production looks

A look is a treatment of the whole frame after the scene is drawn: painted strokes with ink outlines
(`ink`), a hazy glow (`dream`), a warm film grade with grain (`film`). A scene stores how strong each
look is, from 0 (off) to 1, and the engine, the Studio and every export draw the same frame from it.

Everything here is driven by the **catalog of looks** (`packages/scene-engine/src/effects/catalog.ts`),
the one list that says which looks exist, in what order they stack and what each one is. The schema of
a scene's `effects`, the property paths, the Studio's sliders, `scene effects`, the shader and the
list in this page all follow from it. Planned in [scene-effects-plan.md](scene-effects-plan.md); the
authoring skill is `.claude/skills/scene-effects/SKILL.md`, the CLI skill `.claude/skills/scene-cli/SKILL.md`.

## What a look is

- A scene definition holds `effects: { ink?: number, dream?: number, film?: number }`, each from 0 to 1.
  An absent look and a look at 0 are the same: off. When the last look goes off the `effects` key is
  removed, so a bundle with every look off is byte for byte the bundle that never had looks.
- Looks **stack in the order of the catalog**: ink first, then dream, then film. Each one sees the
  picture the looks before it made.
- A look has one number, its **amount**. There are no other parameters: what a look does between 0 and
  1 is part of its design (see each look below).
- The property of a look is `scene/<sceneId>/effects/<kind>`, a `number` from 0 to 1, like any other
  property of the engine (`scene props` lists it, `scene set` writes it).

## Where it runs

Post-production in the Three.js runtime, as one fullscreen pass **after the scene and before the canvas**:

1. `ThreeSceneRuntime.drawEditFrame` finds the looks of the scene (the editor's override if there is
   one, else the definition's `effects`). With no look on, the frame takes the direct path it always
   took, pixel for pixel.
2. With a look on, the scene is drawn into an offscreen target (half float, 4 samples, the size of the
   render profile) instead of the canvas.
3. `PostEffects.present` draws that target to the canvas through **one fragment shader**, assembled from
   the catalog (`runtime/post-effects/assemble.ts`): the shared helpers, then `main()`, which reads the
   pixel and runs, for each look in the catalog's order and only while its amount is above 0, that look's
   block (`<kind>.glsl.ts`).

The Studio's preview, `scene render` and the pixel tools all go through this one path: the render
draws every frame in headless Chrome with the same runtime, so the MP4 has the looks. Captions are laid
over the frame after the pass (the render paints them from the engine's caption layout, the Studio shows
them as an overlay), so a look never touches the text.

## The determinism contract

The same bundle and the same edit frame give the same pixels, in the Studio and in every export.

- Nothing in a look reads the clock or a random source. The film grain is an integer hash of the pixel
  position and the **edit frame**: it is different on every frame and identical every time a frame is
  drawn again, however the playhead got there.
- With every look off nothing changes: no pass, no offscreen target, no `effects` key in the bundle.
  `scripts/scene-studio-effects.spec.mjs` checks that turning every look off gives back the plain frame
  byte for byte.
- A look may not depend on the frame before it (see the note on ink's boiling below): a frame reached by
  a seek must equal the same frame reached by playing.

## The looks

The facts of each look (summary, cost, limits) are in the catalog and are what `scene effects` prints;
what follows explains them. Amounts are written `a`.

### Ink

*What it looks like.* Flat areas turn into painted patches, as if brushed, and the contours of shapes are
drawn as dark ink lines.

*How it is made.*
- **Kuwahara painting.** Around each pixel there are four square sectors (up-left, up-right, down-left,
  down-right, sharing the centre row and column), each `radius + 1` pixels wide. For each sector the
  shader takes the mean colour and its spread (the variance summed over red, green and blue). The pixel
  becomes the mean of the sector with the smallest spread. In a flat area every sector is alike and the
  pixel takes a local average (a stroke); at an edge, the sectors that cross it have a large spread
  and lose, so the pixel takes the mean of a sector on its own side and the edge stays sharp.
- **Ink outline.** A Sobel filter on the brightness (luma), reading the eight neighbours at a distance of
  `1.5 + 1.5a` px, gives how strong an edge runs through the pixel. Only strong edges count
  (`smoothstep(0.18, 0.45, edge)`): rain, splashes and the texture of the picture stay paint, the
  contours of shapes become lines. Under a line the paint is darkened by up to 70%.
- **Mix.** The painted, outlined result is mixed with the original by `smoothstep(0, 0.25, a)`.

*What the amount does.* Just above 0 the radius is 2 px and the result is almost the original. The mix
is complete at `a = 0.25`. After that only the brush grows: the radius is `floor(2.5 + 4a)`, which goes
2, 3, 4, 5, 6 px at `a` = 0, 0.125, 0.375, 0.625, 0.875, and the outline reads from 1.5 px (at 0) to 3 px
(at 1) away.

*Cost.* From 25 samples per pixel (radius 2) up to 169 (radius 6) for the painting, plus 8 for the
outline. The heaviest of the looks at its top amounts: the 13 x 13 loop runs in full whatever the radius.

*Known limits.*
- **It boils.** Each frame chooses its sectors on its own, so wherever the picture moves between frames
  the strokes flip from one sector to another and shimmer.
- The outline follows brightness only: two colours of the same brightness get no line.
- Radii are in pixels (made for 1280x720): at a higher resolution the strokes look finer.

*Ideas to refine it.*
- **Less boiling without history.** Blend the four sector means by their spread (weights such as
  `1 / (1 + spread^q)`) instead of taking the smallest: the choice becomes continuous and the flipping
  stops being a hard switch. This keeps the contract, because a frame still depends only on itself.
- **Temporal stability by sampling the previous output** (the usual fix) conflicts with the contract: a
  frame would depend on how it was reached, so a seek would draw another frame than playing does. It is
  only acceptable if the previous output is re-derived deterministically (for example, always drawing a
  fixed number of warm-up frames before the one asked for), which multiplies the cost.
- Stroke direction from a structure tensor (anisotropic Kuwahara), and an outline from colour difference
  instead of luma alone.

### Dream

*What it looks like.* A hazy, lens-like glow: bright parts bloom, the colours fringe toward the corners
and the borders of the frame go soft.

*How it is made.* Three effects, the last two sharing a pattern of 12 offsets (`GLOW_TAPS`, a rosette of
radius 0.35 to 1):
- **Colour fringes.** Red is read from `uv + shift` and blue from `uv - shift`, where `shift` is the
  vector from the centre of the frame times `0.012a`: none at the centre, 0.6% of the frame at the edge
  at `a = 1`.
- **Glow.** 12 samples within 18 px keep, for each channel, what is above 0.55; their mean is added back
  at `1.6a`.
- **Soft edges.** 12 samples within `6 px x (0.5 + centreDistance)` are averaged and mixed in by
  `smoothstep(0.35, 0.8, centreDistance) x a`, where `centreDistance` is how far the pixel is from the centre in
  frame heights: the centre stays sharp, the borders go soft.

*What the amount does.* It scales all three: the fringes, the glow and how soft the borders get. The
thresholds, the 18 px reach and the 1.6 gain are fixed.

*Cost.* 26 samples per pixel at any amount: 2 for the fringes, 12 for the glow, 12 for the soft edges.

*Known limits.*
- Only what is brighter than 0.55 glows, so a dark scene hardly glows.
- Sizes are in pixels (made for 1280x720): the glow reaches 18 px at any resolution.

*Ideas to refine it.* Sizes as fractions of the frame height, so the look is the same at any resolution;
a bloom from a chain of smaller copies of the frame (wider and cheaper than 12 samples); a soft knee on
the threshold instead of a hard 0.55.

### Film

*What it looks like.* A warm picture with a little more contrast, darker corners and a fine grain that
moves from frame to frame.

*How it is made.* In this order, on the colour the earlier looks left:
- **Warm tint.** Red x1.06, blue x0.9, and a small lift (+0.012 red, +0.008 green).
- **S-curve.** With `y` the brightness, `curved = y*y*(3 - 2y)` (smoothstep); the colour is multiplied by
  `mix(1, curved / y, 0.35)`, so 35% of the curve is applied: darks get darker and lights lighter.
- **Vignette.** The corners are darkened by up to 35%, from 0.35 to 1 frame heights from the centre.
- **Grain.** `grainAt(pixel, editFrame) - 0.5` (an integer hash of the pixel and the edit frame), times
  0.05, times a factor that is strongest in the mid tones and fades toward black and white. A flat grain
  would drown a night scene.
- The graded colour is mixed with the plain one by `a`.

*What the amount does.* A straight mix: at 0 the plain frame, at 1 the full grade, and every part of it
(tint, curve, vignette, grain) grows together.

*Cost.* No extra samples: arithmetic only, with one integer hash per pixel. The cheapest look.

*Known limits.*
- **It darkens night scenes.** The S-curve lowers everything below mid grey and the vignette lowers the
  corners, and the grade has no exposure compensation, so an already dark picture ends up darker.
- The grain is per output pixel, so it gets finer as the resolution grows.

*Ideas to refine it.* Exposure compensation: a gain that grows with the amount, or pivot the curve around
the frame's own average brightness instead of 0.5; a vignette that weakens as the frame gets darker;
grain size in fractions of the frame height.

## How to apply looks

- **Studio.** The *Effects* row of the Layers panel opens one slider per look (0 to 1, step 0.05). A
  drag shows the look at once (the editor's live override, `ThreeSceneRuntime.setSceneEffects`); letting
  go saves it into the bundle file and records the undo step. The override is the Studio's preview path
  and writes nothing by itself; it is not an edit of the bundle, so a script should not use it.
- **JavaScript.** One list of edits, whatever the layer; it is pure and writes nothing until you save.

  ```js
  import { editBundle, describeSceneEffects } from "@tessel/scene-engine/core"
  import { openBundle, renderBundle } from "@tessel/scene-engine/node"

  describeSceneEffects() // the catalog: kind, summary, technique, cost, limits, order in the pass
  const file = await openBundle("output/.../render-bundle.json")
  const saved = await file.update((bundle) => {
    const edited = editBundle(bundle, [
      { set: "scene/s1/effects/ink", value: 0.6 },
      { set: "scene/s1/effects/film", value: 0.4 },
    ])
    if (!edited.ok) return { ok: false, errors: edited.errors }
    return { ok: true, bundle: edited.bundle, notes: edited.notes }
  })
  await renderBundle(file.path, { outDir: "out", label: "scene-01" }) // the MP4 has the looks
  ```

  `setProperties(bundle, [{ path, value }])` does the same for properties alone. A value outside 0..1,
  not a number, or a look the catalog does not have is refused with the path and, for an unknown look, a
  hint that lists the ones that exist.
- **CLI.**

  ```bash
  scene effects                                         # the catalog, as JSON (no bundle)
  scene props render-bundle.json --prefix scene/s1/effects/
  scene set render-bundle.json scene/s1/effects/ink=0.6 scene/s1/effects/film=0.4 --write
  scene show render-bundle.json --layer effects         # the looks of every scene, null for none
  scene render render-bundle.json --out out/
  ```

  `scene show` with no layer counts the scenes with at least one look on (`scenesWithEffects`).
- **The preview server** (`scripts/studio-harness.mjs`, HTTP only): `PUT /bundle-effects` with
  `{ sceneDefinitionId, effects }` (the scene's looks as they are after the edit, `null` for none; what
  the Studio's saver sends) or `PUT /bundle-edit` with `{ edits: [{ set: "scene/<id>/effects/<kind>",
  value }] }`. Both land through the engine, so a refusal is the same 400 with the same error.

## How to add a look

A look is one GLSL module, one catalog entry and its names; the schema, the property, the CLI, the
Studio slider and the shader assembly follow. Checklist:

1. **GLSL module.** `packages/scene-engine/src/runtime/post-effects/<kind>.glsl.ts` exports a
   `LookShader` (`kind`, `declarations`, `apply`; the contract and what a block can read are in
   `look-shader.ts`; a block reads its amount as `amount` and never names a uniform) and is listed in
   `looks.ts`. Its constants and functions share one scope with the other looks: pick names no other
   module uses. Start the file with a comment that says the idea of the
   look and what its amount drives.
2. **Catalog entry** in `src/effects/catalog.ts`, at the position where it should run in the pass: `kind`
   (lowercase letters and digits), `title`, `summary`, `technique` (the math in words and what the amount
   does), `cost`, `limits`, `uniform` (`uSepia`, the amount in the shader).
3. **i18n.** `studio.effects.<kind>` in `apps/scene-studio/src/i18n/{pt-br,en,es}.json`. Adjust the
   `studio.effects.hint` sentence if it lists the order.
4. **Property and CLI come for free.** Check them: `scene effects` describes the look,
   `scene props <bundle> --prefix scene/<id>/effects/` lists it, `scene set` writes it, the Studio shows
   a slider.
5. **Specs.** The assembly refuses, and its specs fail, when the module and the entry disagree (a look
   with no module, a module with no entry, two looks on one uniform, a name declared twice, a block that
   names a uniform instead of reading `amount`); the Studio spec fails for a name
   missing in a locale. Add a spec for anything that is specific to the look.
6. **Reference hashes.** The existing looks must not move. Capture the reference *before* you start, and
   again after: `effects-reference.mjs` draws every look alone and two fixed stacks, so the new look gets
   its own images (`added`) and `--compare` must show every older image equal (see below). Do not touch
   the stacks in the script: a stack that changes renames the older images, and `--compare` then reports
   them `removed`.
7. **Browser spec.** `scene-studio-effects.spec.mjs` goes through the catalog: each look must change the
   frame, save, and repeat when the frame is reached again. Run it.
8. **This page.** Add a section for the look, in the same shape as the ones above.

## How to check a change

```bash
cd packages/scene-engine && pnpm build
node scripts/effects-reference.mjs --out /tmp/scene-effects-reference/before   # before touching a shader
# ...the change...
node scripts/effects-reference.mjs --out /tmp/scene-effects-reference/after
node scripts/effects-reference.mjs --compare /tmp/scene-effects-reference/before /tmp/scene-effects-reference/after
node scripts/scene-studio-effects.spec.mjs --bundle <a copy of a bundle>     # after building the Studio
pnpm test:e2e                                                                  # renders with ink through Chrome and ffmpeg (render-effects.e2e.spec.ts)
scene render <a copy of a bundle> --out /tmp/scene-effects-check             # look at the frames
```

- `effects-reference.mjs` draws edit frames 30, 60 and 120 of a bundle (default: the `local-art-sample`
  fixture) with every look off, each look alone, and two fixed stacks (`ink 0.5 + film 0.5` and `ink 0.3 +
  dream 0.6 + film 0.9`, named after their looks so they stay put when a look is added), through the viewer the render uses, and
  writes the PNGs with the sha256 of each in `hashes.json`. A change that must not move a pixel (a
  refactor, a split of a shader) leaves every hash equal; `--compare` says which differ, and which images
  are new. A change that is meant to move a look moves the hashes of the images that look is in, and no
  others: check that too.
- The reference needs Chrome (`TESSEL_SCENE_ENGINE_CHROME` overrides the path) and a built package
  (`pnpm build`). It never writes the bundle it draws.
- The browser spec works on a copy of a bundle; copy the bundle with its folder first so the assets
  resolve.
