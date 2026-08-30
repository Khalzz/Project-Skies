# Animation tracks (`level_planning.ron`)

Every scene folder under `assets/scenes/<scene>/` can have a `level_planning.ron`
file, deserialized into `EventSystem` (`src/game/play/event_handling.rs`) and
driven every frame from `GameLogic::update` (`src/game/play/scene.rs`). It holds
two related but distinct things:

- **`event_list`** — one-shot triggers ("play this audio clip once we're 5
  seconds in"). Existed before this doc; unchanged.
- **`object_3d_tracks` / `ui_tracks` / `camera_tracks`** — continuous, keyframed
  parameter animation ("lerp this object's position from A to B between 2s and
  6s"). New; this doc covers these three.

All four live in the same file/struct, because the whole point of a timeline is
seeing everything that happens at a given moment together — see "One file per
scene" below.

## The keyframe model

Every track is a list of keyframes, each holding a **full value** (never a
partial diff — if you only want to move on `x`, still write `y`/`z` the same at
both ends). Between two consecutive keyframes, the value is linearly
interpolated. Before the first keyframe and after the last one, the value holds
flat — same convention every keyframe animation tool (Blender, Unity, After
Effects) uses.

```ron
Position(
    target: "player",
    keyframes: [
        (time: 0,    value: (x: 0.0, y: 6000.0, z: 0.0)),
        (time: 3000, value: (x: 0.0, y: 6200.0, z: 120.0)),
    ],
),
```

### `start_time`: keyframe times are relative, not absolute

Every track has an optional `start_time` (milliseconds since scene start,
defaults to `0`). Every keyframe's own `time` is relative to that — so
`time: 0` always means "the instant this track begins," not "scene start."

```ron
Position(
    target: "player",
    start_time: 3000,           // track begins 3000ms into the scene
    keyframes: [
        (time: 0,    value: (x: 0.0, y: 6000.0, z: 0.0)),    // -> absolute 3000ms
        (time: 1000, value: (x: 0.0, y: 6200.0, z: 120.0)),  // -> absolute 4000ms
    ],
),
```

This makes a whole animation movable by editing one field instead of rewriting
every keyframe, and lets a track sit dormant (target left completely untouched)
until its `start_time` arrives, instead of snapping the target to keyframe 0's
value from scene start.

## `object_3d_tracks` — 3D scene objects

Targets a `GameObject`'s `id` (from `data.ron`) via `app.renderizable_instances`
— the same map physics writes into every frame. **A track targeting a
physics-driven object (has `metadata.physics` in `data.ron`) will fight physics
every frame rather than composing with it** — stick to static/kinematic objects
(like `ground`, `fellow_aviator`, `sun`) unless that's specifically what you want.

| Variant | Value type | Notes |
|---|---|---|
| `Position` | `(x, y, z)` | World-space, same units as `data.ron`'s `position:`. |
| `Rotation` | `(x, y, z)` | Euler **degrees**, same as `data.ron`'s `rotation:` — converted to a quaternion each frame. |
| `Scale` | `(x, y, z)` | |

```ron
object_3d_tracks: [
    Position(
        target: "fellow_aviator",
        start_time: 2000,
        keyframes: [
            (time: 0,    value: (x: 50.0,  y: 50.0, z: 0.0)),
            (time: 4000, value: (x: 150.0, y: 50.0, z: 0.0)),
        ],
    ),
],
```

Not supported yet: targeting a sub-mesh of a model (e.g. just the aileron of a
plane) — that's a separate, per-model mutation path (`app.game_models[...]
.model.mesh_lists[...]`) used today for gameplay-driven control surfaces, not
wired into tracks. Would be a natural extension if a scene ever needs it.

### Scripting a physics-driven object (e.g. the player plane)

The "don't target a physics-driven object" rule above assumes physics is
still running. If you pause physics for that object first (see
`PhysicsCommand::TogglePause`/`SetTransform` in
`src/engine/physics/physics_handler.rs`), a track *can* be its position for
as long as physics stays paused — that's how `test_chamber`'s intro cinematic
scripts a straight-line flight path for `"player"` (see the worked example
below). Two things that setup needs:

1. **`end_time`** (optional, all three variants) — makes the track release
   control once `game_time` reaches it, instead of the default "hold the last
   keyframe forever." Needs `Some(...)` in RON since it's an `Option`, e.g.
   `end_time: Some(9000)`. Essential here: once physics resumes and starts
   writing this object's transform again every frame, a track that kept
   re-asserting its last keyframe would permanently fight it right back and
   freeze the object in place.
2. **Pause before, teleport-and-resume after, from Rust** — nothing in RON
   pauses physics on its own; `GameLogic::update` does this by watching
   `EventSystem::is_cinematic_camera_active` (see `camera_tracks`/`LookAt`
   below) and sending `PhysicsCommand::TogglePause` on the rising edge, then
   `SetTransform` (teleporting the rigidbody to wherever the track left the
   object) followed by `TogglePause` again on the falling edge — so physics
   resumes from exactly where the scripted motion ended, with no snap.

## `ui_tracks` — 2D / UI nodes

Targets a UI node via the same `"parent/child"` path syntax `Ui::get_ui_node`
uses everywhere else (see `assets/ui/*.ron` for node ids).

| Variant | Value type | Notes |
|---|---|---|
| `Position` | `(x, y)` | Raw screen pixels — same as `UiNode::move_to`. Mostly meaningful for top-level nodes; children get repositioned by their parent container's layout every frame regardless. |
| `Alpha` | scalar `0.0..1.0` | Fades text/image content; no-op on containers (they have no single color of their own — see `UiNode::set_alpha`'s doc comment). |
| `Color` | `(r, g, b, a: u8, default 255)` | Sets `background_color`. |
| `Active` | `bool` | Shows/hides the node and everything under it (`UiNode::set_active`) — a discrete step, not a lerp: holds `a` right up to the next keyframe's own timestamp, then switches. A single keyframe (`keyframes: [(time: 0, value: true)]`) is the common case — nothing happens until `start_time`, then it flips once. |

```ron
ui_tracks: [
    Alpha(
        target: "data_box/framerate",
        keyframes: [
            (time: 0,    value: 0.0),
            (time: 1000, value: 1.0),
        ],
    ),
],
```

Not supported yet: `Size`. Same shape as the others (`(width, height)`,
`Vec2Value` again) — add it if/when a scene actually needs it.

## `camera_tracks` — named cameras

Targets a camera already registered in `CameraHandler` by name
(`app.camera.get_mut(target)`, `src/engine/rendering/camera.rs`). **By default
only `"main"` exists** — the one `GameLogic::camera_control` drives every frame
from the plane's transform + `CameraState` (Normal/Cockpit/Cinematic/etc.).
Registering additional named cameras (`CameraHandler::create_camera`) is a
Rust-side step; RON can't do it on its own yet.

| Variant | Value type | Notes |
|---|---|---|
| `Position` | `(x, y, z)` | World-space camera position. |
| `Fov` | scalar (degrees) | Vertical FOV. |
| `LookAt` | `target`, `look_at` (a `GameObject` id), `position`, optional `fov`, `start_time`, required `end_time` | A bounded cinematic shot, not a keyframed value — see below. |

```ron
camera_tracks: [
    Fov(
        target: "main",
        start_time: 0,
        keyframes: [
            (time: 0,    value: 70.0),
            (time: 1500, value: 40.0),
            (time: 3000, value: 60.0),
        ],
    ),
],
```

**Two things to know before targeting `"main"`:**

1. Tracks apply *after* `camera_control` each frame (see
   `GameLogic::update`), so a `main`-targeting track visibly overrides the
   flight follow-cam for as long as it's running — that's the point, for a
   scripted push-in/zoom beat.
2. For `Position`/`Fov`, there's no "hand control back" step. Once a track
   starts, it holds its last keyframe's value **forever** (same as every other
   track), not just for the beat you intended. End a `main`-targeting track on
   a value the follow-cam would produce anyway, or target a dedicated camera
   you register separately instead.

### `LookAt` — a bounded cinematic shot

`LookAt` is the one exception to "holds forever": it's not keyframed at all,
just a fixed `position` plus a live `look_at` target (a `GameObject` id,
looked up in `app.renderizable_instances` fresh every frame — so it tracks a
moving object, e.g. the player plane, automatically). It only does anything
while `game_time` is inside `[start_time, end_time)`; outside that window it's
a complete no-op, so `target` (typically `"main"`) just reverts to whatever
else drives it (`camera_control`'s flight follow-cam) with nothing extra to
release or hand back.

```ron
camera_tracks: [
    LookAt(
        target: "main",
        look_at: "player",
        position: (x: 60.0, y: 15.0, z: 100.0),
        fov: Some((65.0, 35.0)),  // (start_fov, end_fov) - a push-in over the shot
        start_time: 4400,
        end_time: 9000,
    ),
],
```

`fov` is `(start_fov, end_fov)`, lerped over the shot's own progress through
`[start_time, end_time)` — a built-in zoom, not a flat value. Omit it for a
fixed FOV.

**A `LookAt` running underneath an opaque UI element is invisible.** The
first version of `test_chamber`'s intro had this bug: the mission-intro
backdrop (see the worked example below) stayed fully opaque until 8500ms and
the `LookAt` ended at 9300ms, so the cinematic ran entirely behind a black
screen — visible for a sliver of a second at best. `start_time`/`end_time`
controls when the shot is *active*, not when it's *on screen* — if something
opaque covers the viewport for part of that window, budget the window (or
retime whatever's covering it) so there's an actual gap for it to be seen.

`GameLogic::update` also uses this same window for two other things, both
derived from it rather than separately authored (so there's exactly one place
— the `LookAt`'s `start_time`/`end_time` — controlling all three; they can't
drift out of sync):

1. **Player input.** While *any* `LookAt` is active,
   `EventSystem::is_cinematic_camera_active` returns true and `Plane::update`
   (which reads live flight-stick/keyboard input) is skipped for that frame -
   so the player has no control for the length of the shot.
2. **Physics pause/resume.** On the window's rising edge, `GameLogic::update`
   sends `PhysicsCommand::TogglePause` - meant to pair with an
   `Object3DTrack::Position` (with a matching `end_time`, see above) scripting
   whatever object the shot is about for the same window. On the falling edge,
   it reads that object's current transform, sends `SetTransform` to teleport
   the rigidbody there, then un-pauses - so physics resumes exactly where the
   script left off. If nothing's paused (no rigidbody to fight), this is
   harmless and the target object just needs to already be moving how you want
   (e.g. coasting on `data.ron`'s own `initial_velocity`) - pausing is only
   necessary when a track is actively repositioning something physics also
   drives.

## One file per scene, not one per element

Everything above lives together in one `level_planning.ron` per scene, not
split per object/node/camera. The value of a timeline is seeing correlated
timing in one place — "camera cuts as dialogue ends, right before the plane
banks" needs all three visible together, not scattered across files you have
to cross-reference by timestamp. If a scene ever needs multiple genuinely
independent sequences (e.g. a mission-intro cutscene vs. ambient looping
animation), split by **sequence** (a second RON file, e.g.
`mission_intro.ron`), not by element.

## Worked example: the mission-intro sequence

`assets/scenes/test_chamber/level_planning.ron`'s `ui_tracks` fully replace what
used to be a hardcoded `MissionIntroPhase` state machine in
`GameLogic`/`scene.rs` (five phases, five timing consts, five helper
functions): black screen → mission title card → flight HUD. What's left in Rust
(`GameLogic::finish`) is only the one-time, per-selected-level setup that can't
be data (the backdrop/title nodes' dynamic text content, and hiding the HUD
before the first frame ever renders so there's no one-frame flash) — the entire
timeline (when things fade, for how long, in what order) is the `ui_tracks`
list. That's the dividing line to use elsewhere too: dynamic construction stays
in Rust, the "what happens when" stays in RON.

## Extending this

Adding a new animatable property is one new enum variant in
`src/game/play/animation_tracks.rs` (mirrors how `EventType` already works for
`event_list`) — pick the value type it needs (`Vec3Value`/`Vec2Value`/`f32`/
`ColorValue`, or a new one), write the `apply` arm using whatever mutation
method that target type already exposes, and add a `normalize` arm to keep its
keyframes sorted. No changes needed anywhere else — `EventSystem::apply_tracks`
just iterates whatever's in the three `Vec`s.
