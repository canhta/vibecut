# Assets view

Interface: `show`.

Renders the assets surface from read-only projections: the media browser, deletion
confirmation, the Rename dialog (`rename.rs`), the Media tab's Yours pages (`yours/`: Favorites, My presets, Brand assets), effect cards, filter tiles (`filters.rs`), feedback, the Audio tab pages (`audio/`), the transition grid and the Captions sub-navigation (`captions.rs`). It reads
state and returns `Action` values; it writes no project.
