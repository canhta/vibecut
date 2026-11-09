# Preferences

`src/preferences.rs` is the composition root of the one versioned local preference document, persisted in a bounded JSON file. It names feature-owned section types rather than redefining them: export options, caption options, media view/sort, panel tabs and sections, preview fit/zoom and snapshot directory, projects view/sort (`home`), keybinding deltas (`input`), window geometry and panel ratios.

The document is validated on load and before save. A damaged layout falls back to defaults with a `PreferenceIssue` warning; an unreadable document restores system defaults without rewriting the bytes. Reads and writes share a 16 KiB budget, unknown keys are refused, and the lane map, colour list, section map, window map and panel ratios are bounded. `ui.prefs.set` edits carry app-owned undo/redo and never touch Project JSON.
