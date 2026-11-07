# Action registry

The app action registry in `src/actions.rs` is the single source for key, menu and button
trigger ids and executable routes. It assembles one registry from each owner's
table: `actions/shell.rs` owns file, project, appearance and shortcut entries and
`actions/editor.rs` owns the editor window's media, caption, timeline,
transport, scene, inspector and preview entries. Adding a window adds a table;
the registry itself stays one lookup. `ActionSpec::label` is a `Text` catalog
key, so every row is localized and no action can ship without an English and
Vietnamese label.

`ActionTarget::Engine(EngineAction)` chooses an engine-owned `CommandKind`; the
same chosen kind supplies its id to execution and its engine availability
predicate, and execution sends typed `engine::Command`s rather than rebuilding
JSON. App payload construction and batch selection updates stay in the
frontend. `ActionTarget::Ui { owner, action }` names an explicit app workflow,
including dialog opening and the combined app / engine undo history.
Parameterized scene/item widgets resolve to registered families before
activation; unregistered ids refuse before state changes. `Editor::action_enabled`
adds app selection requirements to engine availability and gates both generic
action widgets and direct/key dispatch. Parameterized dispatch commands
(`playback.seek`, `preview.set_zoom`, `ui.prefs.set` and the two keybinding
commands) are registered as non-bindable rows, so every id dispatch accepts is
visible in the one registry.

`Chord`, `KeybindingPreferences`, `Keymap` and `Dispatch` live in the
foundation [key input](../input/README.md) module; this registry re-exports them
for callers that only know action ids.
