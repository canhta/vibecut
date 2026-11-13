# Frame route

Interface: `Editor::timeline_panel`, `timeline_scenes`, `apply_timeline`,
`cancel_interaction` and `zoom_timeline`.

Paints the timeline with the read-only context, then applies what it returns:
`timeline::apply` takes the actions it can through the port and `route_selection`
resolves the ones that write the selection scopes or open a gesture.
