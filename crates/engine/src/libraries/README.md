# Libraries

Interface: `fonts`, `sounds`.

`libraries` provides external assets to a project without turning the engine into
an asset marketplace: fonts are resolved and verified for rendering, and sounds
are searched, saved, previewed and inserted as ordinary project media. Both
submodules own their remote transport, local cache and admission limits; neither
edits project data except through the registered insert/edit commands.

| Module | Responsibility | Contract |
|---|---|---|
| `font_catalog/` | Resolves font assets for project rendering. | [Font catalog](font_catalog/README.md) |
| `sound_library/` | Provides licensed sound assets for projects. | [Sound library](sound_library/README.md) |
