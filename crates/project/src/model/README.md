# Model

Project, scene, track, item and settings data with whole-project validation.

## Project

`Project::empty` creates a 1920×1080, 30 fps, black-canvas project with the 16:9 preset, a 1920×1080 last-custom size and 48 kHz output audio. `Project::new(ProjectData)` validates a complete candidate. `data()` borrows an immutable snapshot; `into_data()` yields an editable candidate for an engine command, revalidated before commit. Serde construction also validates. Use `decode`/`load` for untrusted JSON so allocation and nesting budgets apply first.

Project, scene, track, item and bookmark IDs are separate `u64` newtypes; media owns asset IDs. IDs are unique within their type across the project. Zero is permitted; engine owns allocation. The main track's name is editable like secondary track names.

`ProjectData.original_canvas_size` is the optional first visual media size, used by reset-to-original. It starts null and obeys the canvas area bound. [Engine import policy](../../../engine/src/session/edit/README.md#import-policy) owns its initialization and FPS ratcheting.

`Project::duration` uses the active scene. `Project::scene_ids(SceneScope)` resolves Active, All or an explicit nonempty, unique Selected list; unknown or duplicate IDs and more than 256 selections refuse. `for_scene` creates a validated immutable snapshot of one scene; preview, mixing and export consume it.

## Scenes

`ProjectData.scenes` is a nonempty array (`Scenes`) with one protected main scene and up to 255 secondary scenes. `current_scene_id` identifies an existing scene. A candidate with no main, multiple mains, duplicate IDs or a missing active scene is refused; loading never repairs it. Item, track, animation and bookmark validation and aggregate budgets span every scene, including inactive ones.

`Scene` owns its NFC name, `is_main`, timeline and bookmarks. Assets, canvas (width, height, fps, background) and output audio settings are project-level. `ProjectData::scene` / `scene_mut` access the current scene; incomplete candidates read main when the current ID is invalid, but `Project::new` still refuses them. `Scene::duration` derives content duration; `Scenes::main().duration()` gives main-scene duration without changing the active scene.

## Tracks

`Timeline` owns `overlays` (topmost first), one mandatory `main` Video track named `Main Track`, `audio` (topmost first), and persisted `magnet` state (default on). `iter` / `iter_mut` traverse bottom-to-top render order; UI reverses it. `insert_track` places a secondary track at the highest position of its group. An engine edit never removes main.

Track kinds are Video, Audio, Text, Sticker, Effect, Filter and Adjustment. Captions share Text tracks. Generated solid-colour `ItemParams::Overlay` uses Video tracks. Sticker, Effect, Filter and Adjustment accept empty tracks.

## Items

Items within a track are sorted by output start, nonnegative and nonoverlapping; touching half-open intervals are valid. Item end uses checked arithmetic. An active scene's output duration is the maximum item end, excluding bookmarks.

Each item's typed `ItemParams` determines its track kind. Video and audio items require an asset with a corresponding stream and cannot exceed its known presentation duration. Caption items keep source asset identity and captions' `CueTrack` (source ranges, languages, edited flags, translation references, and one shared style table that cues reference by `CueStyleId`); cues must fit the item's source range and every style id must name a valid table entry. `Item::word_spans` reads a caption item's native ASR words as `WordSpan` values carrying the item, retained cue, index, and clipped source and output ranges; a word is clipped to its cue and the item's source window, so a trim never duplicates a word range. Text and solid-overlay items have no media reference and their source range uses local content ticks.

`VideoParams.crop` is the optional source region a video or image item shows: `x`/`y` are the top-left corner and `width`/`height` the size, as fractions of the whole source frame, finite, positive and contained in `0..=1`. Absent means the full frame. The crop follows the item through trim, retime, split, undo/redo and persistence because it is item content, not render state.

Missing referenced asset IDs are invalid; missing files are valid and carry media-owned relink state ([media source](../../../media/src/source/README.md)). Source paths are absolute native paths; working-directory changes never reinterpret them.

Audio items carry `AudioParams::source`. The default `upload` references a project asset. `library` records the provider identity, attribution, licence, source page and exact preview URL of the cached bytes, validated (positive provider id, HTTPS URLs, bounded text) and persisted so export can write attribution and licence metadata.

## Settings

The default canvas is 1920×1080 at 30 fps, black background, 16:9 preset, 1920×1080 last-custom size, 48 kHz output audio. Canvas frames must have exact positive frame ticks. `Settings` owns output sample rate, master volume and mute, the clip and master loudness targets and the last measured gains. Presentation preferences stay in the app.

## Saved caption values

`ProjectData.caption_presets` is a required project-wide bank of at most 64 `SavedCaptionPreset` values with stable `CaptionPresetId` identities. Every entry must have a valid complete `CueStyle` and a unique ID and name, even without captions. Names are NFC, case-sensitive, 1..=64 extended graphemes and at most 256 UTF-8 bytes, without controls or leading/trailing whitespace; invalid input is refused, never trimmed. Equal styles under different names are valid. Forks and scene snapshots retain the whole bank. Applying copies values into ordinary cue styles; no selected preset reference or override map is saved.
