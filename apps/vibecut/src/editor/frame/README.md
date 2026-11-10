# Editor frame

Composes the editor window: top bar, draggable panel layout, input routing and
the shortcut dialog, returning actions to their owners. Its state owns the
resolved keymap, the app preference history, the shortcut dialog and the
automation records, so areas return what they painted instead of writing those.
