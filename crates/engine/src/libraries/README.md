# Libraries

Interface: `fonts`, `sounds`.

External assets for a project: fonts are resolved and verified for rendering; sounds are searched, saved, previewed and inserted as project media. Each submodule owns its remote transport, local cache and admission limits. Neither edits project data except through the registered insert/edit commands.

| Module | Responsibility | Contract |
|---|---|---|
| `font_catalog/` | Resolves font assets for project rendering. | [Font catalog](font_catalog/README.md) |
| `sound_library/` | Provides licensed sound assets for projects. | [Sound library](sound_library/README.md) |
