# Editor frame

Interface: `State`, `show`, `Action`, `AppCommand`, `PreviewZoom`.

Composes the editor window from the top bar, draggable panel layout, input routing and
shortcut dialog, and routes the actions areas return to their owners. Its state owns the
resolved keymap, the app preference history, the shortcut dialog and the automation
records, so areas return what they painted instead of writing those. The header menu's dialogs (`header_dialogs`) are
frame chrome. The Delete confirmation's Confirm hands the intent to the lifecycle
owner, which alone saves, leaves and trashes the project. In the About dialog `State::about` holds its open page (main or licences), `app.about`, `app.licences` and `app.about.close` move it, and Terms and Privacy raise `EditorAction::OpenTerms` and `OpenPrivacy` for the window to open. The view and its facts are the shared [services](../../services/README.md) `views::about` module (Home hosts the same view); `header_dialogs` only routes the clicks. While it is open it counts as an overlay for dispatch.

`agent` docks the [agent chat](../panels/agent/README.md) in the timeline row (the layout hands it the right part of that row and the timeline keeps the rest) or, expanded, in a full-height right column (`layout::Dock`; the other areas share the width left of it), draws the button that opens it, and marks a previewed change: while it is on, `context` lends every area the fork's session, `display_project` feeds the player the fork, `edit_ready` is false, shortcuts are muted and the media, properties and timeline areas are disabled.

`lend` builds a `Port` from the `Editor` fields (`with_assets`, `with_player`, `with_timeline`, `with_properties`, `with_dialogs`, `with_agent`) and applies the notes an area leaves. An area handles its own actions; the frame routes only the cross-area `Request`s (`Seek`, `OpenModel`, `Action`, `ExportClips`, `Lifecycle`, `AddEffect`, `InsertShape`, `ArmKeyColor`, `ReplaceMedia`, `PruneSelection`, cache and proxy intentions) in `route`. `bench` mounts one area alone in an offscreen harness for area tests and routes its actions through the same frame entry.

A timeline fade gesture rolls back when its audio target disappears or becomes locked,
the window loses focus or fullscreen hides the timeline. A gesture closed externally
drops its pointer state before another frame can publish it.
