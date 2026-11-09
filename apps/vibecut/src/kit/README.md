# Kit

`kit.rs` exposes the app palette, measured geometry, regular icons and shared controls without an engine dependency; callers import it directly. [Tokens](../../../../docs/product/ui/tokens.md) index the executable value owner. The measured editor density (12/14 px type, 22 px controls, 4 px corners) is installed by `kit::install_editor_density` from the editor frame only; other windows keep toolkit defaults.
