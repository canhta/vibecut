# Keybindings

The shortcuts dialog reads `actions()`, `action(id)`, `ActionSpec` metadata/defaults/availability (`bindable()` excludes parameterized widget families), effective bindings and conflicts on `KeybindingPreferences` / `Keymap`, and writes through `Editor::set_keybinding`, `reset_keybinding` and `reset_keybindings`. The default table is generated in [keyboard](../../../../../../docs/product/agent-control/keyboard.md). Dispatch rules: [key input](../../../input/README.md).

Shift bypasses snapping during a drag; scrubbing uses the same seek path. Overrides and `isCustomized` markers persist in the preference store; unknown actions and conflicts are reported and unsupported import profiles are refused with a typed diagnostic.

The header Shortcuts action opens a four-tab dialog grouped by action category. Rows show platform glyph chips; click-to-record captures the next chord; a conflict names the existing action and leaves both bindings untouched; unbound actions stay listed; per-action and all-action reset remove the override and marker. Every change is the app-local `ui.keybindings.set` / `ui.keybindings.reset` command, so undo and redo restore the map without touching Project JSON. Import/export controls are visible but disabled.
