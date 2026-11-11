# AI preferences dialog

`State` retains draft input and provider catalogs. `show` reads the editor context
and returns one validated `Models` edit for the frame's preference history; it
never edits the project. `preferences.open` opens this dialog from the Window menu.
The layout and localized states belong to [AI preferences](../../../../../../docs/product/ui/editor.md#ai-preferences).
Selection and refresh limits belong to the [shared model service](../../../services/remote_models/README.md).
