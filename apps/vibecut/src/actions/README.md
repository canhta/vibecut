# Action registry

The app action registry in `src/actions.rs` is the single source for key, menu and button
trigger ids and executable routes. `ActionTarget::Engine(EngineAction)` chooses
an engine-owned `CommandKind`; the same chosen kind supplies its id to execution
and its engine availability predicate. App payload construction and batch
selection updates stay in the frontend. `ActionTarget::Ui { owner, action }`
names an explicit app workflow, including dialog opening and the combined app /
engine undo history. Parameterized scene/item widgets resolve to registered
families before activation; unregistered ids refuse before state changes.
`Editor::action_enabled` adds app selection requirements to engine availability
and gates both generic action widgets and direct/key dispatch.
