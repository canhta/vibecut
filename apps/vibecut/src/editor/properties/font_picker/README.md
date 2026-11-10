# Font picker

Interface: `State`, `Action`, `Event`, `show`, `Backend`, `EngineBackend`, `Category`,
`Download`, `Inspection`, `Status`.

One popover over the engine font catalog, owned by the properties area because the
text Font field is its only entry. `State` owns tabs, category and search filtering,
favourites and download projections; `Backend` is the seam to `fonts.list`,
`fonts.load`, `fonts.download` and `job_cancel`, and tests inject one through
`Editor::set_font_backend`.

All fonts, My fonts and Favorites always show. My fonts is empty until font import
supplies families. Favorites and the last tab persist through `ui.prefs.set`. The
System/Google filter and the disabled Custom category live inside All fonts. Rows have
fixed star, name, state and action columns. A name uses its own typeface once the
shared text registry resolves the family; otherwise the UI font.

Loading, failed previews with Retry, no result, not downloaded, downloading with Cancel
and download failed are explicit states. A failed registration falls back to the next
available family.

Design: [font picker](../../../../../../docs/product/ui/editor.md#font-picker).
