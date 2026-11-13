# Kit

Interface: `install`, `install_editor_density`, `install_fonts`, `install_theme`, the
semantic colour functions, `radius`, `size`, `space`, `type_size`, `label_font`, `Icon`,
`icon` and `Look` (the locale and dark-theme copy every window draws with).

Exposes the app palette, measured geometry, regular icons and shared controls without an
engine dependency; callers import it directly. [Tokens](../../../../docs/product/ui/tokens.md)
index the executable value owner. The measured editor density (12 and 14 px type, 22 px
controls, 4 px corners) is installed by `install_editor_density` from the editor frame
only; other windows keep toolkit defaults. Shared controls: [widgets](widgets/README.md).
