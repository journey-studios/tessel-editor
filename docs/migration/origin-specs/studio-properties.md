# Studio properties

Everything the Studio's inspector edits for a scene (lights, ambient, shadows, weather, floor line)
and the camera's start, joints and keys is a **property** with a path, and one function changes it: `setProperty(path, value, { commit })`
in the editor store (`apps/scene-studio/src/engine/editorStore.ts`). A panel control, a spec and a
prompt all go through it, so they get the same checks, the same saving and the same undo step.

Code: `apps/scene-studio/src/engine/properties/` (`Property` = `{ path, kind, read, validate, write }`).
The camera's commands (`camera-commands.md`) are still what the inspector's movement controls run; a
camera property is a path in front of the same commands.

## What `setProperty` does

1. Finds the property for the path; a path that is not one answers with an error.
2. Validates the value (shape and range, the same bounds the bundle schema enforces), then computes
   the scene's new value. Pure: nothing changes if it is refused.
3. Shows it at once. With `commit` (the default) it is also saved to the bundle file and recorded
   as one undo step. `commit: false` only shows it, which is what a slider drag does; the drag's
   last value is then committed once.
4. A refused save puts the last saved value back (and drops the undo step), as for any panel change.

## Errors

`{ ok: false, error: { code, message, path } }`. It never throws.

| code | when |
|---|---|
| `UNKNOWN_PATH` | not a property path, or the scene has no such property |
| `UNKNOWN_SCENE` | `scene/<id>` is not a scene definition of the bundle |
| `UNKNOWN_LIGHT` | the scene has no light with that id |
| `UNKNOWN_LAYER` | `parent` names a layer (node) the scene does not have |
| `INVALID_VALUE` | wrong shape or out of range (`message` says which). For keys, `message` starts with the engine's code: `KEYFRAME_ORDER` (frames not strictly increasing) or `UNKNOWN_KEYFRAME_EASING` |
| `NOT_APPLICABLE` | the path exists but cannot take the value now (no weather to scale, settling without snow, no lights yet, no image plane for a floor) |
| `NOT_READY` | no scene is loaded yet |
| `UNKNOWN_CLIP` | `camera/<id>` is not a camera clip of the bundle |
| `CAMERA_REFUSED` | the engine refused the change (a `CONTINUOUS_*` join, a key out of its movement...); `error.errors` is its list of `{ code, message, hint }` |

On success: `{ ok: true, path, value }`, with the value now stored.

## Paths

`scene/<sceneId>/...`; ids are URI-encoded (an id with a `/` is written `%2F`).

| path | kind | range / value |
|---|---|---|
| `scene/<s>/lighting/ambient` | number | 0 to 1.5; needs lights |
| `scene/<s>/shadows/contact` | number | 0 to 1; needs shadows on |
| `scene/<s>/light/<l>/intensity` | number | 0 to 20 |
| `scene/<s>/light/<l>/radius` | number | at least 0.01 |
| `scene/<s>/light/<l>/flicker` | number | 0 to 1 |
| `scene/<s>/light/<l>/hidden` | boolean | `true` hides; `false` removes the key |
| `scene/<s>/light/<l>/color` | color | `{ r, g, b }`, each 0 to 1 |
| `scene/<s>/light/<l>/position` | vec3 | `{ x, y, z }` |
| `scene/<s>/light/<l>/position/x` (`y`, `z`) | number | any finite number |
| `scene/<s>/weather` | weather | a name (`"rain"`, `"storm:0.7"`) or `null`/`"none"` to clear |
| `scene/<s>/weather/intensity` | number | 0 to 1.5; needs a weather that is one of the panel's choices |
| `scene/<s>/weather/puddles` (`settle`, `covered`) | boolean | `settle` and `covered` need snow |
| `scene/<s>/ground/top` | number | 0.2 to 0.95; the floor line of the scene's ground plane |

### Keys and parent (lights and weather)

A light's intensity, colour and position, and the scene's weather intensity, can hold **keys**
`{ frame, value, easing }` (frames on the edit timeline, strictly increasing; the easing applies to
the span arriving at the key). The engine resolves them with `resolveKeyframes`, a pure function of
the edit frame, so a frame is the same image however it is reached. With no keys the static value
applies and the bundle is the old one.

| path | kind | value |
|---|---|---|
| `scene/<s>/light/<l>/intensity/keys` (`color/keys`, `position/keys`) | keys | the whole list; `[]` or `null` clears the track |
| `scene/<s>/light/<l>/intensity/keys/<k>/frame` (`.../value`, `.../easing`) | number, value of the channel, easing name | one key; `k` is its index |
| `scene/<s>/weather/intensity/keys` and `.../keys/<k>/{frame,value,easing}` | keys | same, for the weather's intensity (needs rain or snow, else `NOT_APPLICABLE`); saved as the definition's `weatherKeys`, which replaces the number in the name |
| `scene/<s>/light/<l>/parent` | layer | a layer (node) id, or `null` to clear |

Stopwatch semantics: while a channel has keys, `intensity`, `color`, `position` (and its axes) and
the weather `intensity` read the value **at the playhead**, and setting one puts a key there
(adds it, or updates the key already on that frame). Without keys they set the static value.
Adding a key (`addKey`) copies the value shown at the playhead; removing the last key takes the
track off (the static value stays). The value of a key is exact on its own frame.

`parent`: the light's `position` becomes an offset from the layer's animated position (translation
only), so the light follows the layer as it moves. Position paths read and write the offset, and
the viewport overlay shows the resolved world position; dragging stores the offset. A light has no
other light as parent and a layer has no light for a child, so a cycle cannot be written. The
camera is not parented in this step.

The plan called the floor line `ground/y`; it is `ground/top` because that is what the bundle
stores (`weatherGround.top`, from the top of the plane).

### Camera

`camera/<clipId>/...`, the clip's id URI-encoded. These write the clip through the camera commands
(`toKeyframes`, `moveKey`, `setStart`), so a refusal is `CAMERA_REFUSED` with the engine's errors, and
the clip, its saving and its undo step are the ones of any camera change.

| path | kind | value |
|---|---|---|
| `camera/<c>/start/position` | vec3 | the clip's first pose; the first movement is made keys first so the path after it stays |
| `camera/<c>/track/<t>/segment/<n>/end/position` | vec3 | where movement `n` ends: a recipe is made keys, its last key moves, and the next movement is made keys so it keeps its end. Main track (`t` = 0) only |
| `camera/<c>/track/0/segment/<n>/key/<k>/position` (`target`) | vec3 | a key of a `keyframes` movement |
| `camera/<c>/track/0/segment/<n>/key/<k>/fov` | number | above 0 and below 180 degrees |
| `camera/<c>/track/0/segment/<n>/key/<k>/frame` | number | strictly between its neighbours, inside the movement |

Key paths of a recipe movement are `NOT_APPLICABLE` (write the `end` first). The path overlay's handles
are these same paths (`camera-commands.md`, "Handles on the path"): a drag is `setProperty(path,
position, { commit: false })` for each move and a last `setProperty` on release. One drag, whether it
converts a movement or not, is one undo step: the saved clip before and after.

Adding and removing a light and switching a layer's shadow on or off change the shape of the
lighting, not a value, and stay actions of the Light panel.

## From a spec or a prompt

In the Studio page, `window.__sceneStudio` has:

- `setProperty(path, value)`: commits, returns the result above.
- `getProperty(path)`: `{ ok: true, path, value }` (value is `undefined` when what it points at is
  not set, e.g. the contact shadow of a scene without shadows) or the error.
- `listProperties()`: `[{ path, kind, min, max }]` for every path that can be changed now.
- `addKey(path, key?)`: adds a key to a `.../keys` path (default: the value shown, at the playhead).
- `removeKey(path, index)`: removes the key at that index.

Both go through `setProperty`, so they are one undo step and are saved like any change.

```js
await page.evaluate(() =>
  globalThis.__sceneStudio.setProperty("scene/scene-1/light/luz-1/intensity", 6),
)
```
