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

`#[serde(default)]` fields on `ItemParams` exist because it is also the `timeline.insert` wire input, where callers omit them; they are not for reading old project files.

`VideoParams.fit` is the item's [fit mode](../visual/README.md#fit-mode), a required persisted value (the wire default `fill` exists only because `ItemParams` is also the `timeline.insert` input). `VideoParams.background` is a `ClipBackground`: `none`, `solid` (a colour) or `blur` (an intensity, 0..=500). The kinds are exclusive, and the value fills the canvas behind a clip that does not cover it. `VideoParams.crop` is the optional source region a video or image item shows: `x`/`y` are the top-left corner and `width`/`height` the size, as fractions of the whole source frame, finite, positive and contained in `0..=1`. Absent means the full frame. The crop follows the item through trim, retime, split, undo/redo and persistence because it is item content, not render state.

`VideoParams.reduce_noise` is the per-item switch for the noise-reduction pass: a Boolean with no strength levels (the reference shows only a toggle), off at creation. It is persisted always; the wire default `false` exists only because `ItemParams` is also the `timeline.insert` input. It is item content, so it follows trim, retime, split, undo/redo and persistence; `timeline.set_item_param` writes it as field `reduce_noise` on video items. Render owns the pass ([effect passes](../../../render/src/scene/effects/README.md#reduce-noise)).

Missing referenced asset IDs are invalid; missing files are valid and carry media-owned relink state ([media source](../../../media/src/source/README.md)). Source paths are absolute native paths; working-directory changes never reinterpret them.

Audio items carry `AudioParams::source`, and both audio and video items carry an `AudioCleanup` chain for their source audio (`cleanup`: at most 8 typed effects with unique ids, validated by `AudioCleanup::validate`). The chain never changes source media; its kinds and parameter bounds are described in the [render cleanup](../../../render/src/audio/README.md#source-cleanup) section. The default `upload` references a project asset. `library` records the provider identity, attribution, licence, source page and exact preview URL of the cached bytes, validated (positive provider id, HTTPS URLs, bounded text) and persisted so export can write attribution and licence metadata.

### Music provenance

`music` retains one `LibraryMusic` record with its audio item. `LibraryMusic::validate`
is its admission interface for project construction and ingestion; its public docs
own the text, URL, identifier and millisecond limits. `MusicLicence::parse` admits
only matching recognized global dedication/attribution identities and deed URLs;
its public docs own the exact recognized spellings. It has no persisted shape of
its own. Loading or validating a project never performs a source-service lookup or
repairs restrictive, contradictory or unknown provenance.

## Settings

The default canvas is 1920×1080 at 30 fps, black background, 16:9 preset, 1920×1080 last-custom size, 48 kHz output audio. Canvas frames must have exact positive frame ticks. `Settings` owns output sample rate, master volume and mute, the clip and master loudness targets and the last measured gains. Presentation preferences stay in the app.

## Saved caption values

`ProjectData.caption_presets` is a required project-wide bank of at most 64 `SavedCaptionPreset` values with stable `CaptionPresetId` identities. Every entry must have a valid complete `captions::CaptionStyle` and a unique ID and name, even without captions. Names are NFC, case-sensitive, 1..=64 extended graphemes and at most 256 UTF-8 bytes, without controls or leading/trailing whitespace; invalid input is refused, never trimmed. Equal styles under different names are valid. Forks and scene snapshots retain the whole bank.

`CaptionParams.preset` is an optional live reference to one bank entry; `CaptionParams.overrides` is a sparse `CaptionOverrides` patch applied field by field over that value. Overrides require a selected preset, and every referenced ID must exist after complete validation. The effective style is the bank value with the patch folded in; deleting a referenced entry therefore refuses rather than leaving a dangling look. A caption item without a selection keeps its imported cue styles and the transient render settings.

## Saved text styles

`ProjectData.text_presets` is a required project-wide bank of at most 64 `SavedTextPreset` values with stable `TextPresetId` identities and the same closed naming rule as caption presets. Each `TextStyle` holds font family, size, weight, italic, alignment, decoration, letter spacing, line height, colour, optional stroke, optional shadow, glow and optional background box; the item's words and animation keys are never stored. Every entry must be valid on its own and have a unique ID and name, even without text items. Forks and scene snapshots retain the whole bank. Applying copies values into ordinary `TextParams`; no selected preset reference is saved.

A text item's optional `TextStroke` is an outer stroke in project-canvas pixels (0..=64 width); `TextShadow` stores colour, an offset up to 1024 pixels on each axis and a 0..=64-pixel blur; `glow` is a switch whose halo uses the fill colour at 15% of the font size, capped at 64 output pixels. These are authored appearance values, not measured reference defaults. The reference exposes a keyframe diamond on the stroke colour only, so `stroke.color` is the one added built-in animation channel; width, shadow and glow stay static and are copied whole by presets.

## Generated graphics

`ItemParams::Graphic` stores only rectangle and ellipse definitions, with explicit sRGB RGBA8 fill/stroke, author-pixel stroke width (0..64), inside/center/outside alignment, and rectangle corner radius (0..50 percent). The author square is512 pixels; these generated Video-track items have no source asset or media retime. Appearance paths `params.fill`/`stroke` require straight linear RGBA channels; `params.strokeWidth` and rectangle-only `params.cornerRadius` require bounded scalar bases and keys; `params.strokeAlign` requires held text inside/center/outside. Other `params.*` paths refuse. Present Params, including zero-key bases, override static fallbacks. Ordinary key-time propagation applies.

`CaptionParams.word_highlight` is a required persisted per-item opt-in, false for new/imported items. It is independent of copied caption styles and the named preset bank. Cues retain captions-owned word correspondence; project validation bounds its stored structure. `CaptionParams.emphasis` maps a cue ID to at most three ascending word ids (indices into that cue's verified word records); validation bounds only its structure, and readers use `captions::resolve_selection`, so ids that outlived their words are ignored rather than refused. `CaptionWordWarnings` is transient preflight data, not project state.

## Stickers

`ItemParams::Sticker` stores the bundled sticker id (`provider:value`, ASCII letters, digits, `-`, `_`, `.`, at most 128 bytes) and the intrinsic pixel size captured at insert (1..=16384 per side). It lives on `TrackKind::Sticker` tracks, takes transform, blend, effects and flip, and carries no masks, media asset or retime. The catalog that resolves ids belongs to [libraries](../../../libraries/src/sticker_library/README.md); render rasterises the asset.
