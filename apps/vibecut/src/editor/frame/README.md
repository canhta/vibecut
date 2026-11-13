# Editor frame

Interface: `State`, `show`, `Action`, `AppCommand`, `PreviewZoom`.

Composes the editor window from the top bar, draggable panel layout, input routing and
shortcut dialog, and routes the actions areas return to their owners. Its state owns the
resolved keymap, the app preference history, the shortcut dialog and the automation
records, so areas return what they painted instead of writing those. The header Delete
confirmation (`project_delete`) is frame chrome: Confirm hands the intent to the lifecycle
owner, which alone saves, leaves and trashes the project.

`lend` builds a `Port` from the `Editor` fields (`with_assets`, `with_player`) and applies the notes an area leaves. An area handles its own actions; the frame routes only the cross-area `Request`s (`Seek`, `OpenModel`, `Action`, `ExportClips`, `AddEffect`, `InsertShape`, `ArmKeyColor`, cache and proxy intentions) in `route`.

A timeline fade gesture rolls back when its audio target disappears or becomes locked,
the window loses focus or fullscreen hides the timeline. A gesture closed externally
drops its pointer state before another frame can publish it.
