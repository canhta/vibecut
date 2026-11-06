# Module structure

Structure is folders first, then crates. A module is where a responsibility lives, a folder groups siblings of one kind, and a crate edge is spent only where the compiler must enforce a boundary.

The studied reference gets its structure from crates, not folders: about 50 small crates in numbered layers L0 to L6, edges only downward, enforced by a layer check, with `speech`, `tts`, `captions`, `interchange`, `export` and the pure edit algebra each their own crate. Inside its engine and UI crates it is flat (100+ files, one `misc.rs`). We take the crate layering and the command registry and reject that flat folder style, because a flat crate cannot state which concept owns what and its imports cross every boundary that should have been a folder.

Folders first keeps the later crate cut mechanical: the compiler enforces a boundary only at a crate edge, internal crates are not API boundaries, and a flat `crates/<name>/` layout scales once the folders inside are sound. We promote a folder to a crate when it has a second consumer, an independent seam (a provider family, a codec, a platform), heavy dependencies the rest should not rebuild for, or a direction that keeps being violated, in that order prefer the feature layers above the engine kernel and the pure edit algebra out of `session/edit`. A kit crate follows when a second UI consumer justifies it.

Current contracts live in [structure](../engineering/structure.md); [architecture](../engineering/architecture.md#crate-map) owns the crate map and edges.
