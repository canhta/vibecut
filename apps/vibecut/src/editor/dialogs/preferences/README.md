# AI preferences dialog

`State` retains draft input and provider catalogs. `show` reads the editor context
and returns one validated `Models` edit for the frame's preference history; it
never edits the project. `preferences.open` opens this dialog from the Window menu.
The layout and localized states belong to [AI preferences](../../../../../../docs/product/ui/editor.md#ai-preferences).
Selection and refresh limits belong to the [shared model service](../../../services/remote_models/README.md).

The Settings modal (`preferences.open`, 480 px, Draft, Edit, Performance and General) edits one stored preference per row through `Set` actions, so each change joins the preference history. Clear cache requests the one cache-maintenance clear the Properties cache rows also run (`Request::Cache(CacheAction::Clear)`) and shows its bytes freed and entries still in use. The modal uses the kit dialog frame, title and footer button. The cache limit and hardware choice apply at the next start; the rows say so. Shortcut export and import raise `EditorAction::Keybindings*`, which the window answers with the native picker. Import merges or replaces through `KeybindingPreferences::apply_profile`; a refused file changes nothing. AI models open from General as the modal above.
