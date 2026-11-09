# Action registry

`src/actions.rs` is the single source of key, menu and button trigger ids and their executable routes. It assembles one registry from per-owner tables: `actions/shell.rs` (file, project, appearance, shortcuts) and `actions/editor.rs` (media, caption, timeline, transport, scene, inspector, preview). A new window adds a table.

`ActionSpec::label` is a `Text` catalog key, so every action has an English and Vietnamese label.

`ActionTarget::Engine(EngineAction)` picks an engine `CommandKind`; the same kind supplies the id and the engine availability predicate, and execution sends typed `engine::Command`s. App payload construction and batch selection updates stay in the frontend. `ActionTarget::Ui { owner, action }` names an app workflow, including dialogs and the combined app/engine undo history.

Parameterized scene/item widgets resolve to registered families before activation; unregistered ids refuse before any state change. `Editor::action_enabled` adds app selection requirements to engine availability and gates widgets and key dispatch. Parameterized dispatch commands (`playback.seek`, `preview.set_zoom`, `ui.prefs.set`, the two keybinding commands) are non-bindable rows, so every accepted id is in the registry.

`Chord`, `KeybindingPreferences`, `Keymap` and `Dispatch` live in [key input](../input/README.md) and are re-exported here.
