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
