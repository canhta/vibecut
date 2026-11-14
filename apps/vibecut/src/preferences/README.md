# Preferences

Interface: `Preferences`, `PreferenceStore`, `PreferenceIssue`, `PREFERENCES_VERSION`,
`PREFERENCE_KEYS`, `MAX_PREFERENCES_BYTES`, `MAX_SECTIONS`, `MAX_TIMELINE_LANES`,
`PANEL_MIN`, `PANEL_MAX`, `PREVIEW_MIN`, `PREVIEW_MAX`, `TIMELINE_MIN`, `TIMELINE_MAX`,
`DEFAULT_PANELS`, `drag_panel`, `validate_panels`, `WindowGeometry`,
`FontPickerPreferences`, `FontTab`, `AdjustmentPreset`, `MAX_ADJUSTMENT_PRESETS`,
`MAX_ADJUSTMENT_PRESET_NAME`, `validate_adjustment_presets`, `EasingPreferences`, `EasingTab`,
`SavedEasing`, `MAX_SAVED_EASINGS`, `SubtitleOptions`, `SOURCE_LANGUAGES`.

The composition root of the one versioned local preference document, persisted in a
bounded JSON file. It names feature-owned section types rather than redefining them:
export options, caption options, media view and sort, panel tabs and sections, preview
fit, zoom, preview axis and snapshot directory, projects view and sort (`home`), keybinding deltas
(`input`), window geometry and panel ratios.

The document is validated on load and before save. A damaged layout falls back to
defaults with a `PreferenceIssue` warning; an unreadable document restores system
defaults without rewriting the bytes. Reads and writes share a 32 KiB budget (including all remote-model choices and recent IDs), unknown
keys are refused, and the lane map, colour list, section map, window map (64 entries)
and panel ratios are bounded. `ui.prefs.set` edits carry app-owned undo and redo and
never touch project JSON. Ripple mode and keyframe lane visibility persist outside
undo history, so entering an editing mode or opening its lanes cannot shadow the
project edit's next Undo.

`EasingPreferences` stores the keyframe graph popover's tab (`presets` or `saved`) and up to
64 saved easing curves, each a unique name of at most 64 bytes with a normalized
`[x1, y1, x2, y2]` whose `x` values lie in 0..1. A damaged section falls back to its
defaults with the layout warning. Saving a curve is a `ui.prefs.set` edit of the `easing`
key, so agents save presets the same way the popover does and it joins preference undo.

`AiPreferences` is the one persisted remote provider/feature model map. It stores
bounded custom IDs and four recent IDs per pair, using AI's shipped table for defaults.
The Captions and Shorts provider pickers resolve their transient request snapshots
from this map. Downloaded local weight choices remain in their local owner. Keys,
fetched lists and price acknowledgements are never serialized. A document whose version is not
`PREFERENCES_VERSION` is refused through the version refusal; there is no migration.
Version 14 removed `recent_colors`; the colour popover's recents live in the project.

The text-task provider is one persisted choice, `subtitles.text_provider`, shared by
translation, `fillers.classify` and `chapters.generate`. The models are not shared: each task takes its own
`AiPreferences` row for that provider, the Translate row into the transient
`subtitles.translation_model`, the ClassifyFillers row into `subtitles.fillers_model` and the
GenerateChapters row into `subtitles.chapters_model`.
Version 13 replaced the translation-only `translation` and `translation_model` fields with
these; there is no migration.

`preview_guide` and `preview_overlays` store the preview guide choice (none, a grid with
rows and columns 1–24, validated on load and set, one of the platform guides TikTok, Instagram Reels and YouTube Shorts, or a custom set of four margins) and the per-overlay visibility. Both are presentation only;
an edit is one `ui.prefs.set` step in app history. Version 11 introduced them, version 12 added the platform guides and version 16 the custom margins, with no
migration: older documents are dropped.

`preview_lines` is `GuideLines`: the player rulers on or off, the guide lines shown or hidden and locked, and the vertical and horizontal guide positions (canvas pixels from the canvas centre, x right and y up; at most 16 per axis, within +/-100000). It is presentation only: an edit is one `ui.prefs.set` step in app history and the `preview.rulers` and `preview.guides.*` actions write it. Version 18 added it, with no migration.

`adjustment_presets` is the bounded app-owned bank of named colour grades (at most 32,
64-byte unique names, exactly the current `color_grade` parameter set with each value
checked against its definition). Saving, applying and deleting a preset are preference
and project-command paths; the bank never enters project JSON or project history.

`sticker_recents` is the Stickers tab's recent ids (flag sticker ids and shape command ids), newest first and deduplicated, at most `MAX_RECENT_STICKERS` (50). It is written without a history entry and never enters project JSON.

`catalog_favorites` is `CatalogFavorites`: starred catalogue ids for `effects`, `transitions`, `filters`, `animations` and `stickers`, keyed by family under `ids`, oldest star first. Starred `sounds` are a separate `sounds` list of `SoundFavorite` (provider id, title, duration, preview URL), so the Favorites list shows them without a provider lookup; at most 32, title at most 128 bytes, preview URL at most 256 bytes. Ids are at most 256 across every family, each at most 64 bytes. Unknown families, empty or duplicate ids and an overflowing document are refused; `with` refuses `sounds`, which only `with_sound` stars. Starring writes through the preference store without a history entry and never enters project JSON. Version 15 added the map and version 17 the sound metadata, with no migration. `catalog.favorite.set` (`AppCommand::SetFavorite`) stars or unstars one id; a sound stars only while it is in the Sounds results or the saved library (the command takes its metadata from there, else `Error::SoundNotListed`), and unstar drops the stored star by id. `catalog.favorite.list` is the typed read `Editor::catalog_favorites(family)`. A favourite outside the results previews from its stored `preview_url` and shows a disabled Save/Add slot with the reason. Both rows are parameterized, so the registry refuses them as plain activations and they are not bindable.

`default_fps` is one of the project frame rates the settings dialog offers (24, 25, 30,
60, 120; default 30). `hardware_acceleration` is `auto`, `on` or `off`. `cache_limit_bytes`
is the one aggregate byte budget the app opens the engine with, bounded to 256 MiB–8 GiB
(default automatic: `None` sizes it from host memory, [budget rule](../../../../crates/engine/src/runtime/README.md#host-sized-default-budget)). The limit and acceleration apply at the next
start; an open project is never changed by either.
