# Kit

`kit.rs` exposes the app palette, measured geometry, regular icons and the
shared controls without an engine dependency. All callers import that interface
directly. The [token contract](../../../../docs/product/ui/tokens.md)
indexes the one executable value owner. The measured editor density (12/14 px type,
22 px controls, 4 px corners) is installed by `kit::install_editor_density` from the
editor frame only; every other window keeps the toolkit defaults. No standalone kit
demo is installed.
