# Font picker

One popover over the engine font catalog. It renders only engine-owned data:
`fonts.list` supplies the families and their typed available/not-downloaded
state, `fonts.load` registers a chosen family, and `fonts.download` starts one
explicit, cancellable catalog download. Nothing here reads the cache or the
network directly, and selecting a family mutates the project only through the
properties owner's registered `text_font` patch.

All fonts, My fonts and Favorites mirror the source tabs. My fonts stays hidden
until font import (80) supplies imported families; Favorites is app behaviour
persisted through `ui.prefs.set` (`font_picker.tab`, `font_picker.favorites`).
The System/Google filter and the disabled Custom category live inside All fonts
and are not source parity. Loading, failed previews with Retry, no result, not
downloaded, downloading with Cancel and download failed are explicit states; a
failed family registration falls back silently to the next available family.

`Backend` is the seam: production is `EngineBackend`, the shared `FontStore`
behind the registered commands; `tests/it/panels.rs` injects a recording fake
and asserts the command ids and an unchanged project for read-only actions.
