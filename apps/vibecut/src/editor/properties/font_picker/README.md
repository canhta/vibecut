# Font picker

One popover over the engine font catalog, owned by the properties area because
the text Font field is its only entry. `state.rs` owns tabs, category/search
filtering, favourites presentation, the download projection and the inspection
snapshot; `backend.rs` is the seam to the registered `fonts.list`, `fonts.load`,
`fonts.download` and `job_cancel` commands; `view.rs` draws the popover and
returns actions for the properties owner to route.

All fonts, My fonts and Favorites always show. My fonts is empty until font
import (80) supplies imported families; Favorites and the last tab persist
through `ui.prefs.set`. The System/Google filter and disabled Custom category
live inside All fonts and are not source parity. Rows use fixed star, name,
state and action columns. A row name uses its own typeface once the shared text
registry can resolve the family (bundled faces and already loaded downloads);
not-yet-loaded, refused and missing families fall back to the UI font.

Loading, failed previews with Retry, no result, not downloaded, downloading with
Cancel and download failed are explicit states; a failed registration falls back
silently to the next available family. Tests inject a recording backend through
`Editor::set_font_backend`.
