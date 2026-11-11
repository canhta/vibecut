# Keybindings

Interface: `AppCommand` (`SetKeybinding`, `ResetKeybindings`, `JumpBookmark`),
`PreviewZoom`, and the
`Editor` methods `set_keybinding`, `reset_keybinding`, `reset_keybindings`,
`open_shortcuts`, `shortcut_tab`, `set_shortcut_tab` and `record_shortcut`.
`JumpBookmark` delegates the playhead search to the
[action registry owner](../../../actions/README.md#bookmark-navigation).

The shortcuts dialog reads `actions()`, `action(id)`, `ActionSpec` metadata and defaults,
`bindable()` availability, effective bindings and conflicts on `KeybindingPreferences` /
`Keymap`; `bindable()` excludes parameterized widget families. Defaults are generated in
[keyboard](../../../../../../docs/product/agent-control/keyboard.md); dispatch rules are
[key input](../../../input/README.md).

Overrides and `isCustomized` markers persist in the preference store. Unknown actions and
conflicts are reported, and unsupported import profiles are refused with a typed
diagnostic.

The header Shortcuts action opens a four-tab dialog grouped by action category. Rows show
platform glyph chips; click-to-record captures the next chord; a conflict names the
existing action and leaves both bindings untouched; unbound actions stay listed;
per-action and all-action reset remove the override and marker. Every change is the
app-local `ui.keybindings.set` / `ui.keybindings.reset` command, so undo and redo restore
the map without touching Project JSON. Import and export controls are visible but
disabled.

## Clipboard actions

Copy requires a current single-item key selection. Paste requires a single selected
item with compatible existing channels and an entry in its Session. The shared current
eligibility check runs before draft flushing or selection/view writes, and before the
timeline temporarily takes area state. Busy/save/transaction/inline edits, dirty or
focused Properties/caption drafts, recording and blocking modals refuse. A menu popup
alone does not block its action. Accepted Paste reads the current output playhead once,
uses one typed command and clears nested scopes only on change; refusal/no-op preserves
them. Engine [transfer semantics](../../../../../../crates/engine/src/session/README.md#keyframe-clipboard)
remain authoritative for all transports.
