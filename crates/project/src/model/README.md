# Model

Project, scene, track, item and settings data with whole-project validation.

| Module | Responsibility | Contract |
|---|---|---|
| `project` | Owns the validated snapshot, its editable candidate and whole-project invariants. | [Project](#project) |
| `scene` | Owns scenes, scene selection and bookmarks. | [Scenes](#scenes) |
| `track` | Owns track kinds, ordering and magnet state. | [Tracks](#tracks) |
| `item` | Owns item identity, typed parameters, timing and media references. | [Items](#items) |
| `settings` | Owns canvas, output audio and loudness settings. | [Settings](#settings) |

## Project

`Project::empty` creates a 1920×1080, 30 fps, black-canvas project with the
16:9 preset, a 1920×1080 last-custom size and 48 kHz output audio.
`Project::new(ProjectData)` validates a complete candidate;
`data()` borrows an immutable snapshot and `into_data()` yields an editable
candidate for an engine command. Revalidate before committing. Serde
construction also validates; use `decode`/`load` for untrusted JSON ingestion so
allocation and nesting budgets apply before typed deserialization.

Project, scene, track, item and bookmark IDs are separate `u64` newtypes; media owns
asset IDs. IDs are unique within their type across a project. Zero is permitted;
engine owns allocation. Groups, main kind and unique IDs are validated on every
candidate. The main track's name is editable like secondary track names.

`ProjectData.original_canvas_size` is the optional first visual media size, not a
second user canvas setting. It starts null and retains the first inserted media's
width/height for reset-to-original. Its dimensions obey the canvas area bound.
[Engine import policy](../../../engine/src/session/edit/README.md#import-policy) owns automatic
initialization and FPS ratcheting in import/insert commands.

`Project::duration` uses the active scene. `Project::scene_ids(SceneScope)`
resolves Active, All or an explicit nonempty, unique Selected list before any
export scheduling. Unknown/duplicate IDs and more than 256 selections refuse.
`for_scene` creates a validated immutable snapshot for the selected scene;
preview, mixing and export consume that same active-scene snapshot. Export
settings UI owns output scope controls and paths.

## Scenes

`ProjectData.scenes` is a nonempty JSON array, represented by `Scenes` with one
protected main scene and up to 255 secondary scenes. `current_scene_id` identifies
an existing scene. Scene IDs are unique; track, item and bookmark IDs remain
unique across the entire project, including inactive scenes. All existing item,
track, animation and bookmark validation and aggregate budgets span every scene.
A candidate with no main, multiple mains, duplicate IDs or a missing active scene
is refused; loading never repairs it.

`Scene` owns its NFC name, `is_main`, timeline and bookmarks. Assets, canvas
(width, height, fps, background) and output audio settings remain project-level.
`ProjectData::scene` / `scene_mut` access the current scene; incomplete candidates
read main when their current ID is invalid, but `Project::new` still refuses those
candidates. `Scene::duration` derives content duration. `Scenes::main().duration()`
supplies main-scene duration to project cards without changing the active scene.

## Tracks

`Timeline` owns `overlays` (topmost first), one mandatory `main` Video track named
`Main Track`, `audio` (topmost first), and persisted `magnet` state (default on).
`iter` / `iter_mut` traverse bottom-to-top render order; UI reverses this traversal.
Secondary insertion uses `insert_track` at the highest position of its group. Main
is never removed by an engine edit. Track kinds are Video, Audio, Text, Sticker,
Effect, Filter and Adjustment. Captions share Text tracks; generated solid-colour
`ItemParams::Overlay` uses Video tracks. Sticker, Effect, Filter and Adjustment
accept empty tracks until their item owners land.

## Items

Items within each track are sorted by output start, nonnegative and nonoverlapping.
Touching half-open intervals are valid. Item end uses checked arithmetic. An active
scene's output duration is the maximum item end, excluding bookmarks.

Each item's typed `ItemParams` determines its track kind. Video/audio require an
asset with a corresponding stream and cannot exceed its known presentation
duration. Caption items retain source asset identity and captions' `CueTrack`,
including source ranges, languages, edited flags and translation references.
Cues must fit the item's source range. Text/solid-overlay items have no media
reference; their source range uses local content ticks. Missing referenced asset
IDs are invalid; missing files are valid. After loading, use
`SourceAsset::availability()` to show relink state; explicit relinking verifies
content before retaining the asset ID. Source paths are absolute native paths;
working directory changes never reinterpret them.

Audio items carry `AudioParams::source`. The default `upload` source references a project asset; `library` records the provider identity, attribution, licence, source page and exact preview URL that produced the cached bytes. The record is validated (positive provider id, HTTPS URLs, bounded text) and round-trips in the project file, so export can write attribution and licence metadata without guessing.

## Settings

The default canvas is 1920×1080 at 30 fps with a black background, the 16:9
preset, a 1920×1080 last-custom size and 48 kHz output audio. Canvas frames must
have exact positive frame ticks. `Settings` owns output sample rate, master
volume/mute and the clip/master loudness targets plus the last measured gains;
presentation preferences stay in the app.
