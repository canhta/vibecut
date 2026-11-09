# Preferences

`src/preferences.rs` composes the one versioned local preference document from
feature-owned section types and persists it in the same bounded JSON file. The
aggregate sits above the windows (the composition root); it
names each feature's own type rather than redefining it: export options,
caption options, media view/sort, panel tabs and sections, preview fit/zoom and
snapshot directory, projects view/sort (`home`), keybinding deltas (`input`),
window geometry and panel ratios.

Every document is validated on load and before save; a
damaged layout falls back to defaults with a `PreferenceIssue` warning, while an
unreadable document restores system defaults without rewriting the bytes. Editing and writes share the 16 KiB read budget, unknown
keys are refused, and the lane map, colour list, section map, window map and
panel ratios are bounded. `ui.prefs.set` edits carry app-owned undo/redo and
never touch Project JSON.
