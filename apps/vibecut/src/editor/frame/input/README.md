# Keybindings

The read surface for the shortcuts dialog remains `actions()`, `action(id)`,
`ActionSpec` metadata/defaults/availability (`bindable()` excludes parameterized
widget families), effective bindings and conflicts on
`KeybindingPreferences` / `Keymap`, and `Editor::set_keybinding` /
`reset_keybinding` / `reset_keybindings`. Target payloads are executable route
enums; consumers need not match them to list or edit shortcuts.
[The keyboard guide](../../../../../../docs/product/agent-control/keyboard.md) holds its
generated default table. Global dispatch runs in the capture phase, waits while
an overlay or dialog is open, a project is loading, recording is active or a text
field has focus, and consumes one exact chord once. Paste is intercepted only
when the in-app clipboard has an entry; otherwise the event remains for media
paste. Shift bypasses snapping during a drag. Scrubbing uses the same seek path.
Binding overrides and `isCustomized` markers persist in the one preference store;
unknown actions and conflicts are reported, and unsupported import profiles are
refused with a typed diagnostic. The header Shortcuts action opens a four-tab
dialog grouped by action category: rows show platform glyph chips, click-to-record
captures the next chord, a conflict names the existing action and leaves both
bindings untouched, unbound actions stay listed and bindable, and per-action or
all-action reset removes the override and its `isCustomized` marker. Every change
is the app-local `ui.keybindings.set` / `ui.keybindings.reset` command, so undo and
redo restore the previous map without touching Project JSON. Import/export JSON
controls are visible but disabled until settings import/export lands.
