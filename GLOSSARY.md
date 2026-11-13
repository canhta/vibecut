# VibeCut

**Imported LUT** is a user-owned display-referred sRGB colour table copied into a durable project resource bundle. Its `LutId` identifies canonical cube content; source bytes, name and optional user-supplied licence remain provenance. Render owns interpolation and working-pixel conversion.

Language for the editor's privacy and execution boundaries.
- **Local-first**: Editing and project data stay on the user's computer by default; remote AI processing requires explicit consent.
- **Headless core**: The editing capabilities that run without a graphical interface.
- **AI provider**: A service or local model offering AI processing through an adapter.
- **Explicit consent**: The user's affirmative approval to send specified media or transcripts to a remote provider.
- **Tick**: An integer time unit at 254_016_000_000 ticks per second, exact for admitted rational frame rates and audio sample rates.
- **FrameRate**: An exact rational frames-per-second rate, stored as a positive numerator and denominator rather than a floating value.
- **SourceRange**: A nonempty half-open `[start, end)` range in source-media ticks.
- **Speed**: A video or audio item's constant playback rate, 0.01x to 5x on an exact 0.001 grid. The item plays its source window for `floor(source / speed)` output ticks, with split pieces retaining their slices of that original playback; its pitch policy chooses between pitch following speed and time-stretching with pitch preserved.
- **SourceAsset**: A source-media identity and its location, content fingerprint and probed stream information; timeline items refer to it without copying its media.
- **Proxy**: A low-resolution intra-frame copy of a source's video in the app cache, keyed by the source content hash and never named by a project; preview reads it while the project's choice is on, and export always reads the source ([engine artifacts](crates/engine/src/artifacts/README.md#preview-proxies)).
- **Take**: One voice-over capture. It becomes a project-owned mono AAC file only when encoding finishes, then one SourceAsset and one audio item at the playhead in a single undo step; a cancelled or failed take leaves no file and no item.
- **Transcript**: Source-language recognition result with stable segments and a provider/model/settings fingerprint.
- **Cue**: Identified source range with source language and per-language text, independent edit flags and translation references.
- **CueStyle**: Saved static caption appearance shared by cues in a cue track, including its font, colours, alignment and margins.
- **Caption preset**: A built-in caption look applied as saved cue-style values to one caption item; it has no saved preset identity or project bank.
- **Filler candidate**: A whole-word source span of filler speech in one cue's timed words, with a stable ID and the cue's edit flag.
- **Chapter**: A titled topic boundary at a transcript cue start, with the transcript language; a versioned chapter document also carries the video title, description and optional translations for sidecar publication.
- **EditMapping**: The correspondence between selected source ranges and their positions in an output timeline; one source span can appear more than once.
- **LanguageProfile**: Segmentation, reading-speed, punctuation and font-fallback rules for a content language, with a generic fallback for unknown tags.
- **Subtitle workflow**: The engine composition from source speech through explicit provider choice, checkpointed recognition, optional alignment/translation, cue segmentation and subtitle export/burn.
- **ASR checkpoint**: One independently complete recognition window stored under its exact source content, sample range, language, provider/model/settings and aligner identity; a restarted workflow reuses valid checkpoints and recomputes missing ones.
- **Provider catalog**: Offline capability data (operation, models, languages, local or remote, native timing) used to list and filter explicit provider choices without constructing an adapter or making a network call.
- **Upload disclosure**: The exact remote request scope shown before consent: provider/model, operation, languages, audio duration, payload bytes, per-invocation cost ceiling and the provider's official policy links.
- **Job**: A tracked editing operation with progress, cancellation, failure and checkpoint information.
- **CacheRoot**: The discovered per-user cache location with named model, font and media artifact paths; [engine runtime](crates/engine/src/runtime/README.md#default-cache-location) owns its discovery and failures.
- **Session**: The active project together with its command execution and undo/redo history.
- **Segment**: Stable identified span of transcript text in source-media time.
- **Timing unit**: Unit of recognized speech timing: segment, word, syllable, character, grapheme or token. A token can be part of a character or word.
- **Display variant**: Same-language spelling or script rendering of recognized text, retaining its source speech timing.
- **Word**: Recognized source text with optional source range, speaker and confidence.
- **Verified word correspondence**: An exact pairing of individually timed native or forced source words with the displayed source text.
- **Word projection**: The portion of a timed source word present in one edited cue occurrence; a fully trimmed word has no active projection.
- **CueTrack**: A collection of timed cues for subtitle validation and export.
- **Selection**: The timeline items chosen as the targets of an editing action.
- **Viewport**: The screen region displaying a project's canvas, fitted with letterboxing and adjusted by preview zoom and pan.
- **Handle**: A point on an item's selection box used to change its scale; its displayed position follows the item's rotation.
- **Reflection (flip)**: Reversal of an item's local horizontal or vertical axis about its placed centre, applied before clockwise rotation and preserving positive scale magnitudes. It is a held Boolean item parameter, never a negative scale.
- **Preview guide**: A framing overlay the player draws over the preview (currently a grid); the app paints it and export never includes it.
- **Snap guide**: A line drawn across the preview canvas while a transform gesture holds a snap to the canvas centre or edge, or to another item's box edge; it disappears on release and never changes the project before the drop.
- **Selection scope**: The current level of selection: timeline items, keyframes within items, or points within a mask. Clearing a selection acts on the most specific occupied scope first.
- **Face track**: Observations of one visible face within a single shot; its identifier is local to that shot.
- **Cross-shot identity**: A confident link from a face track to an earlier face track serving as its anchor. An uncertain link stays Unknown; it is not a claim about a person's name.
- **Transition**: A video-only timeline entity joining adjacent clips over their shared cut; [project](crates/project/src/visual/README.md#video-transitions) owns its kinds, alignment and source-handle bounds.
- **Scene-change candidate**: A possible hard cut or gradual transition in source media, with a source time and confidence, awaiting review before it becomes a timeline split.
- **Speech-activity span**: One source-time interval classified as speech or pause by local frame energy, with a heuristic confidence; pause removal consumes these spans without claiming speech recognition.
- **Project**: The editable document: its identity and name, canvas, assets, scenes, active scene and output settings; [project](crates/project/README.md) owns its validation, persistence and writer lease.
- **Project library**: The managed set of projects that Home lists, searches and trashes, with each project's cover image and metadata; the store owns it, not the file system.
- **Scene**: One timeline inside a Project, with a name, a main flag, tracks and bookmarks; a project has one main scene and up to 255 secondary scenes, while assets, canvas and output audio stay project-level.
- **Track**: An ordered lane of items of one kind: Video, Audio, Text, Sticker, Effect, Filter or Adjustment; the main Video track always exists.
- **Item**: One object on a timeline track: content or a source reference, start, duration, animated parameters, effects and masks, with an identity unique across the project.
- **Clip**: A video or audio item, the kinds the timeline draws with a filmstrip and waveform; [project](crates/project/README.md) calls every timeline object an item, and clip names those two kinds.
- **Short**: A suggested vertical clip: a source range with a title, role and play order that a user accepts for batch export from Long video to shorts.
- **Effect**: One entry in an item's ordered effect stack, with a kind, typed parameters and an enabled flag; the effect owner declares it, `project` stores and validates it, and rendering executes it.
- **Filter**: A named look from the filter catalogue, stored as one `color_grade` effect instance; a filter clip is an effect item (adjustment layer) on an Effect track holding that instance, and its Intensity is the item's opacity.
- **Tone curve**: One monotone map from a 0..1 colour channel to 0..1, stored as 2..=32 ordered points on the exact 1/4096 input grid with fixed endpoints; the colour-grade effect owns red, green, blue and luma curves.
- **Param**: An animated parameter override for one animation path, holding an explicit base value and its keyframes even when no keys remain.
- **Keyframe**: One timed value inside a Param, with an interpolation segment and optional in/out handles.
- **Clip animation**: A built-in In, Out or Combo look expanded once into ordinary editable item-relative Params and Keyframes on a visual item, with no stored identity; the slots are read back from exact key matches. In and Out sit on the first and last edge and never overlap; Combo spans the whole clip and excludes both.
- **Mask**: An item-relative shape that limits where the item draws, with feather, expansion, stroke and an ordered add/subtract list.
- **Mask path**: A mask's closed cubic contour: anchors in normalized mask-box coordinates with anchor-relative incoming and outgoing tangents; saved exactly and flattened only for rendering.
- **Mask point**: One indexed anchor of a mask path, identified by the mask's stable id and its position in that path.
- **Revision**: An opaque Session-local token that advances on every accepted mutation and that effectful control requests must supply as `base_rev`.
- **Receipt**: The stored terminal outcome of an effectful control request, keyed by principal, Session and idempotency key, replayed instead of re-executing a retry.
- **Writer lease**: An exclusive OS lock on the sidecar beside a project file, held from load through save so only one process may write that project.
- **Window**: One top-level OS window: Home, Editor or Long video to shorts, each with its own size, top bar and chrome.
- **Panel**: One of the four resizable regions of the Editor: assets, player, properties or timeline.
- **Feature tab**: A tab inside the assets panel that switches its feature: Media, Audio, Text, Stickers, Effects, Transitions, Captions, Filters, Adjustment, Templates or AI.
- **Area**: An editor module that owns one panel's or dialog's state and returns the actions it wants; its interface is `State` and `show`.
- **Kit**: The shared UI foundation of tokens, icons and widgets used by all three windows; it knows nothing about the engine.
- **Automation id**: A stable identifier on an interactive widget that survives locale and layout changes, through which agents and tests address input.

- **Saved caption preset**: A named project-owned full `CaptionStyle` value with a stable `CaptionPresetId`. A caption item may select it live and carry a sparse override patch; delete refuses while selected, and rename never changes appearance. This ID differs from a cue table's `CueStyleId`.

- **Graphic**: A saved generated rectangle or ellipse with fill/stroke appearance and ordinary item-relative animation; a Video-track item with no source asset. Other shape kinds remain outside the admitted scope.

- **Music track**: An openly licensed audio record from the aggregator, carrying title, author, canonical licence name and deed URL, attribution, source page and audio URL. Its licence identity is admitted by [project Music provenance](crates/project/src/model/README.md#music-provenance); only public-domain-dedication and attribution-only families are shown.

**In-app clipboard** - A bounded immutable snapshot of selected items or selected
animation keys held by one Session, separate from project/history and system text.
One slot holds the latest copy; transfer and lifetime:
[Session](crates/engine/src/session/README.md#clipboard).
