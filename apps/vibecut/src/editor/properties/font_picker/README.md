# Font picker

One popover over the engine font catalog, owned by the properties area because the text Font field is its only entry. `state.rs` owns tabs, category/search filtering, favourites and download projections; `backend.rs` is the seam to `fonts.list`, `fonts.load`, `fonts.download` and `job_cancel`; `view.rs` draws and returns actions for the properties owner. Tests inject a backend through `Editor::set_font_backend`.

All fonts, My fonts and Favorites always show. My fonts is empty until font import supplies families. Favorites and the last tab persist through `ui.prefs.set`. The System/Google filter and the disabled Custom category live inside All fonts. Rows have fixed star, name, state and action columns. A name uses its own typeface once the shared text registry resolves the family (bundled faces, loaded downloads); otherwise the UI font.

Loading, failed previews with Retry, no result, not downloaded, downloading with Cancel and download failed are explicit states. A failed registration falls back to the next available family.
