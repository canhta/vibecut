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

`preview_guide` and `preview_overlays` store the preview guide choice (none, or a grid with
rows and columns 1–24, validated on load and set) and the per-overlay visibility. Both are presentation only;
an edit is one `ui.prefs.set` step in app history. Version 11 introduced them with no
migration: older documents are dropped.

`adjustment_presets` is the bounded app-owned bank of named colour grades (at most 32,
64-byte unique names, exactly the current `color_grade` parameter set with each value
checked against its definition). Saving, applying and deleting a preset are preference
and project-command paths; the bank never enters project JSON or project history.

`sticker_recents` is the Stickers tab's recent ids (flag sticker ids and shape command ids), newest first and deduplicated, at most `MAX_RECENT_STICKERS` (50). It is written without a history entry and never enters project JSON.

`default_fps` is one of the project frame rates the settings dialog offers (24, 25, 30,
60, 120; default 30). `hardware_acceleration` is `auto`, `on` or `off`. `cache_limit_bytes`
is the one aggregate byte budget the app opens the engine with, bounded to 256 MiB–8 GiB
(default 768 MiB, the previous fixed value). The limit and acceleration apply at the next
start; an open project is never changed by either.
