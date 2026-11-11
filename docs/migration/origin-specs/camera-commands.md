# Camera commands

How to change a camera's movement, speed or timing, from the Studio, a shell or a prompt. Every
route uses the same commands (`packages/scene-engine/src/camera/camera-commands.ts`), validates
them against the same rules and answers with the new clip or with errors that say what to do.

## Clips hold their last pose

A shot plays its camera clip from `cameraInFrame` for as long as the shot lasts. A clip that is
shorter than that is fine: past its last movement it holds its final pose, at rest, like a property
after its last keyframe. `analyzeCameraClip` reports `heldFrames` (how long the shots rest on it) and
a negative `slackFrames`. `fitToShots` still adds an explicit hold when you want the length written
down.

What a shorter clip does change is when the later movements happen. Making one movement faster
(fewer frames) pulls everything after it earlier in the shot, unless the freed frames go somewhere.
`absorb` says where:

- `"hold-end"` (default for `setDuration`/`setSpeed`): a hold at the end of the clip. Later
  movements keep their frames.
- `"next"`: the following movement takes them.
- `"none"`: nobody; the clip gets shorter or longer, and the movements after it shift.

Speed of a movement = its amount (scene units or degrees) / its seconds. Faster = same amount in
fewer frames (`setSpeed` with `factor: 2`), or more amount in the same frames (`setParam`).

Continuous cuts have a rule of their own: the camera's speed must not jump across the join
(`CONTINUOUS_VELOCITY_JUMP`). Ease the movement that ends at the join, or move the join.

## Tracks: movements at the same time

A clip's `segments` are its main track: one movement after another, from the clip's start pose.
`layers` (up to 3) are more tracks that play at the same time, each with its own timing. What a layer
has done by a frame (degrees of orbit, distance pushed in or slid sideways, field of view changed) is
added on top of the main track's pose: the camera turns around its target first, then slides and
pushes along the turned view, then zooms. So a dolly on the main track and an orbit on a layer give
a camera that turns while it moves in.

A layer has no start pose of its own: its movements always start from `previousEnd`
(`CAMERA_LAYER_FROM_INITIAL` otherwise). To start a layer later, begin it with a hold (`addTrack`
with `startFrame` does that). Past its last movement a layer keeps what it did, like the main track.
The clip lasts until the last of its tracks ends.

Every command that names a movement takes `track` too: 0 (the default) is the main track, 1 to 3
the layers.

## Keyframes: a movement drawn by its keys

Besides the recipes, a main-track movement can be `keyframes`: `{ recipe: "keyframes", durationFrames,
from, keys: [{ frame, position, target, fovDeg, easing }] }`. A key is a whole pose at a frame of the
movement (1 to `durationFrames - 1`, strictly increasing, the last key on the last frame). Frame 0 is
the pose the movement starts from (where the one before it ended, or the clip's first pose), so the
movement reaches `keys[0]` at `keys[0].frame`, then `keys[1]`, and so on. A key's `easing` is the
easing of the span that arrives at it. Poses are mixed key to key (position, target and field of view
each linearly, shaped by the easing); the rules above still hold: the clip holds its last pose, and a
continuous join checks the speed across it.

- **Main track only.** A layer adds offsets and cannot hold a pose (`KEYFRAMES_NOT_FOR_LAYER`,
  `CAMERA_LAYER_KEYFRAMES` in a bundle).
- **`toKeyframes`** turns a recipe movement into the keys that draw the same frames. `dollyIn`,
  `dollyOut`, `zoomIn`, `zoomOut`, `truckLeft`, `truckRight` and `hold` become one key at the last
  frame carrying the movement's easing: the frames are identical. `orbit` is an arc, so it becomes a
  key every 2 frames, linear: within 0.5% of the radius of the arc, not bit for bit. Converting what
  is already keys does nothing, so a batch can convert every movement it touches.
- **`moveKey`** changes one key's `position`, `target`, `fovDeg` or `frame`. Moving a key's frame
  past its neighbours is `CAMERA_KEY_FRAME_ORDER`; outside the movement is `KEY_FRAME_OUT_OF_RANGE`
  (the hint says to lengthen the movement with `setDuration`). Changing a movement's duration
  rescales its keys. `setRecipe` turns it back into a recipe, and drops the keys.

`analyzeCameraClip` gives each movement a `keys` list (`null` for a recipe) and `explainCameraClip`
refuses a bundle whose keys break the rules above.

## Commands

| op | fields | what it does |
|---|---|---|
| `setSpeed` | `segment`, `factor`, `absorb?` | duration / factor, frames kept per `absorb` |
| `setDuration` | `segment`, `frames`, `absorb?` | |
| `setParam` | `segment`, `param`, `value` | `distanceSceneUnits`, `fovDeltaDeg`, `yawDeg`, `pitchDeg` |
| `setEasing` | `segment`, `easing` | `linear`, `easeInOutCubic`, `easeOutCubic` |
| `setFrom` | `segment`, `from` | `previousEnd` (default) or `initial` (first movement only) |
| `setRecipe` | `segment`, `recipe` | `hold`, `dollyIn`, `dollyOut`, `zoomIn`, `zoomOut`, `truckLeft`, `truckRight`, `orbit` |
| `addSegment` | `segment`, `at?`, `absorb?` | |
| `removeSegment` | `segment`, `absorb?` | |
| `setStart` | `position?`, `target?`, `fovDeg?` | the clip's first pose |
| `fitToShots` | | add a hold so the clip lasts as long as the shots play it |
| `addTrack` | `segment`, `startFrame?` | stack a layer: a hold of `startFrame` frames, then the movement |
| `removeTrack` | `track` | remove a layer (1 to 3) |
| `moveSegment` | `track?`, `segment`, `toTrack`, `startFrame` | carry a movement to another track (one past the last layer makes a new one) so it starts at clip frame `startFrame`: its old place becomes a hold, it takes the room of a hold on the target (or goes after a movement it lands on), and a layer left with holds only is removed. The answer's `placed` says where it ended up |
| `toKeyframes` | `segment`, `track?` | turn one recipe movement of the main track into keys (see above); one command per movement |
| `moveKey` | `segment`, `key` (index or `"last"`), `position?`, `target?`, `fovDeg?`, `frame?`, `easing?` | change one key of a `keyframes` movement |

A batch is all or nothing and is validated at the end, so a batch may pass through a state that
would be refused on its own (shorten, then `fitToShots`).

## Routes

- **Code:** `applyCameraCommands(bundle, clipId, commands)` and `analyzeCameraClip(bundle, clipId)`
  from `@tessel/scene-engine/core`. Pure: nothing is written.
- **Shell:** `pnpm scene camera <bundle.json> analyze [clipId]` and
  `pnpm scene camera <bundle.json> apply <clipId> '<json array>' [--write]` (see the skill `scene-cli`).
  Exit code 0 ok, 1 refused, 2 usage; the output is JSON.
- **Studio server:** `PUT /bundle-camera-commands` with `{ clipId, commands }`. A refusal is a 400
  with `{ error, errors: [{ code, message, hint }] }`.

`analyzeCameraClip` is the first thing to read before editing: total frames, what the shots need,
the slack, and each movement's frames, amount and speed per second.

## Errors

`{ code, message, hint?, command? }`. Codes from the commands: `INVALID_COMMAND`, `UNKNOWN_CLIP`,
`SEGMENT_OUT_OF_RANGE`, `SEGMENT_TOO_SHORT`, `CANNOT_ABSORB`, `PARAM_NOT_FOR_RECIPE`,
`LAST_SEGMENT`, `TRACK_OUT_OF_RANGE`, `TOO_MANY_TRACKS`, `CANNOT_MOVE_HOLD`, `KEYFRAMES_NOT_FOR_LAYER`,
`KEYFRAMES_REQUIRED` (`moveKey` on a recipe), `KEY_OUT_OF_RANGE`, `KEY_FRAME_OUT_OF_RANGE`. From the bundle's own rules: `CAMERA_SEGMENT_TOO_SHORT`,
`FOV_OUT_OF_RANGE` (checked on the composed camera when there are layers),
`CAMERA_DIRECTION_DEGENERATE`, `UNKNOWN_CAMERA_EASING`, `CAMERA_SEGMENT_RESET`,
`CAMERA_LAYER_FROM_INITIAL`, `CAMERA_KEY_FRAME_ORDER`, `CAMERA_KEYS_END_SHORT` (the last key must be on
the movement's last frame), `CAMERA_LAYER_KEYFRAMES` and the `CONTINUOUS_*` join codes. `command` is the index of the failing command in the batch.

## In the Studio

Selecting the camera shows its path on the viewport and in a top-down plan. The camera on the
timeline has one row per track (the main one, then each layer) and one bar per movement. Drag a
bar's right edge to change how long that movement lasts (the same `setDuration` command, so the same
`absorb` rule and the same errors). Click a bar to pick that movement: the inspector shows only its
controls, and the tools over the rows step between the track's movements, change its kind, insert a
movement after it and remove it. "New track" stacks a layer with an orbit that starts at the
playhead. A hatched bar past a track's last movement is its held pose.

### Handles on the path

The path drawn on the viewport has a small ring at the start of the clip and at each joint between
two main-track movements (a joint is where a movement ends). Drag one to move the camera there:

- The ring follows the pointer on the plane at its own depth, as the light handles do, and the picture
  shows the change as you drag. Releasing saves it. A click without moving only picks the movement
  that ends there, and the inspector shows it.
- The first drag of a joint turns its movement into keys and moves the last key, in one undo step.
  The movement after the joint becomes keys too, from the moved joint to where it ended, so the path
  after it keeps going where it was going.
- The start handle moves the clip's first pose. The first movement becomes keys first (the same
  conversion) so the path after the start keeps its place instead of travelling with it.
- A change the engine refuses (a `CONTINUOUS_*` join, a key out of its movement) is not applied: the
  reason is shown where camera errors are, and a drag that ends on a refused point puts the clip back
  to the last saved one.
- The rings of a joint show where the camera is (layers included); what they write is the main
  track's own pose.

Handles are on the main track only. They write through the properties below (`studio-properties.md`),
which run the commands above and save the clip as every other camera change does.

Lights, weather, the floor line and the camera's start, ends and keys are edited by property path, see
`studio-properties.md`.

Bars move like clips on a video timeline: drag a movement's bar up or down to another track, or
sideways to start it at another frame (`moveSegment`). The dashed row below the tracks takes a bar
and makes a new track with it, so an orbit dragged there plays at the same time as the main track's
dolly. The faint bar is the one being carried; the dashed outline is where it will land.
