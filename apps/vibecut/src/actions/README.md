# Action registry

Interface: `actions()`, `action(id)`, `ActionSpec`, `ActionCategory`, `ActionTarget`,
`UiAction`, `BookmarkDirection`, `EngineAction`, and the keybinding types re-exported from
[key input](../input/README.md).

`actions()` is the single source of key, menu and button trigger ids and their
routes; it assembles per-owner tables and a new window adds one. `ActionSpec::label`
is a `Text` key, so every action has an English and Vietnamese label.
`ActionTarget::Engine(EngineAction)` names an engine `CommandKind`, which also
supplies the id and the engine availability predicate; `ActionTarget::Ui { owner,
action }` names an app workflow, including dialogs and the combined app/engine undo
history.

`MENUS` lays out the native menu bar (File, Edit, View, Window, Help) as registry action ids; a row shows the action's chord and runs the same route. Menu chords must include Cmd so a menu never steals plain typing.

Parameterized scene and item ids resolve to a registered family before activation;
an unregistered id refuses before any state change. `Editor::action_enabled` adds app
selection requirements to engine availability and gates widgets and key dispatch.
Parameterized dispatch commands (`playback.seek`, `preview.set_zoom`,
`ui.prefs.set`, the two keybinding commands) are non-bindable rows, so every accepted
id is in the registry.

## Bookmark navigation

`bookmark_target` is this module's internal lookup for `playback.jump_bookmark`,
action enablement and both bookmark menus. It compares each marker's start clamped to
the scene duration with the current playhead, then picks the minimum or maximum
original `(start, id)` strictly after or before that playhead. Empty, zero-duration
and already-at-the-edge searches return `None`, so dispatch performs no seek or
metadata work; an actual jump uses the existing Seek path.

`Chord`, `KeybindingPreferences`, `Keymap` and `Dispatch` live in
[key input](../input/README.md).

`edit.copy`, `edit.cut` and `edit.paste` run the scope-specific clipboard workflows through
typed Session commands: selected item snapshots when Items is the active scope, selected-key
snapshots when Keyframes is, the selected anchors with their tangents when MaskPoints is
(`clipboard.copy_mask_points` / `clipboard.paste_mask_points`). `edit.cut` is `clipboard.cut`
for the same scope. `edit.copy_attributes` is `clipboard.copy` of the one selected item and
`edit.paste_attributes` is `timeline.paste_attributes` over the selection; groups the target
kind cannot hold are skipped and named in the toast. Paste requires one
entry in that editor's Session. Eligibility and draft preflight belong to
[frame input](../editor/frame/input/README.md#clipboard-actions).
