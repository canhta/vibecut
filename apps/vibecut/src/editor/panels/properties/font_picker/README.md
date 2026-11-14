# Font picker

Interface: `State`, `Action`, `Event`, `show`, `Backend`, `EngineBackend`, `Filter`,
`Download`, `Inspection`, `Status`.

One popover over the engine font catalog, owned by the properties area because the
text Font field is its only entry. `State` owns the chip filter, the collapsible Presets section, search filtering,
favourites and download projections; `Backend` is the seam to `fonts.list`,
`fonts.load`, `fonts.download`, `fonts.import` and `job_cancel`, and tests inject one through
`Editor::set_font_backend`.

The production runner borrows the current editor's playback accounting policy, including
its attached project scope. Import retains its512MiB reservation; downloads retain the
font catalog's existing work declaration. Both use the common parent before source or
provider work. Every workflow and poll rebinds a replaced policy, cancelling and joining
old work before discarding its job ids; replacing
the backend also resets picker/chooser state. No private budget or smaller reservation
can admit work outside the shared application limit.

The popover follows the reference dropdown: a search field, the All and Commercial chips
(Commercial keeps families with a recorded licence), a collapsible Presets section
(bundled, catalogue and imported families) and a System font section (operating-system
families). Favourites persist through `ui.prefs.set` and float to the top of their section.
Rows have a check gutter, the name, a state cell, a download icon (only for a family not
yet downloaded) and the star at the right edge. A name uses its own typeface once the
shared text registry resolves the family; otherwise the UI font.

Loading, failed previews with Retry, no result, not downloaded, downloading with Cancel
and download failed are explicit states. A failed registration falls back to the next
available family.

Design: [font picker](../../../../../../../docs/product/ui/editor.md#font-picker).

Import raises `EditorAction::FontUpload`; the window's native picker (TTF, OTF and WOFF2
filter) returns the file through `Editor::font_import_picked`, which imports it through the
same registry/store as CLI. The tooltip states usage-rights acknowledgement before
selection. Importing has Cancel and a localized failure state; completion refreshes the
list without choosing a family or adding history. The pending import and its job belong to
the project snapshot, item selection, playhead and picker lifetime; change, close,
transaction or gesture cancels them, and a pick that completes after such a change
imports nothing.
A selected family still uses the existing one-command field edit and undo/redo.
