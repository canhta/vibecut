# Kit

Interface: `install`, `install_editor_density`, `install_fonts`, `install_theme`, the
semantic colour functions, `radius`, `size`, `space`, `type_size`, `label_font`, `Icon`,
`icon`, `Look` (the locale and dark-theme copy every window draws with), the panel
state, progress bar and toast (`panel_state`, `Tone`, `progress_bar`, `toast`) and the
dialog frame, title and footer button (`dialog_frame`, `dialog_title`, `dialog_button`).

Exposes the app palette, measured geometry, regular icons and shared controls without an
engine dependency; callers import it directly. [Tokens](../../../../docs/product/ui/tokens.md)
index the executable value owner. The measured editor density (12 and 14 px type, 22 px
controls, 4 px corners) is installed by `install_editor_density` from the editor frame
only; other windows keep toolkit defaults. Shared controls: [widgets](widgets/README.md).

`feedback.rs` draws what the app tells the creator while it works: a 24 px glyph over one
12 px line for a panel's empty, loading, offline or error face (the caller supplies at
most four words), a 2 px accent progress bar and a 320 px bottom-right toast. `dialog.rs`
draws the modal frame (12 px corners, 24 px inset) and the 90x34 footer buttons. Both
keep strings, glyphs and automation IDs with the caller.
