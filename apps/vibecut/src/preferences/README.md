# Preferences

Interface: `Preferences`, `PreferenceStore`, `PreferenceIssue`, `PREFERENCES_VERSION`,
`PREFERENCE_KEYS`, `MAX_PREFERENCES_BYTES`, `MAX_SECTIONS`, `MAX_TIMELINE_LANES`,
`PANEL_MIN`, `PANEL_MAX`, `PREVIEW_MIN`, `PREVIEW_MAX`, `TIMELINE_MIN`, `TIMELINE_MAX`,
`DEFAULT_PANELS`, `drag_panel`, `validate_panels`, `WindowGeometry`,
`FontPickerPreferences`, `FontTab`.

The composition root of the one versioned local preference document, persisted in a
bounded JSON file. It names feature-owned section types rather than redefining them:
export options, caption options, media view and sort, panel tabs and sections, preview
fit, zoom, preview axis and snapshot directory, projects view and sort (`home`), keybinding deltas
(`input`), window geometry and panel ratios.

The document is validated on load and before save. A damaged layout falls back to
defaults with a `PreferenceIssue` warning; an unreadable document restores system
defaults without rewriting the bytes. Reads and writes share a 16 KiB budget, unknown
keys are refused, and the lane map, colour list, section map, window map (64 entries)
and panel ratios are bounded. `ui.prefs.set` edits carry app-owned undo and redo and
never touch project JSON.
