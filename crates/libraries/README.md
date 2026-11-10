# libraries

External assets for a project: fonts are resolved and verified for rendering; sounds are searched, saved, previewed and inserted as project media. Each module owns its remote transport, local cache and admission limits. Neither edits project data except through the registered insert/edit commands.

Use `fonts::FontStore` for discovery, verified loading and explicit downloads, and `sounds::{SoundClient, SoundLibrary, SoundDownloads, SoundPreview}` plus the `add_to_timeline`/`add_saved` helpers for licensed audio. Callers compose the engine `JobRunner`, the project writer lease and the app-owned preview sink; this crate owns no UI, scheduler or project mutation outside the registered edit.

| Module | Responsibility | Contract |
|---|---|---|
| `font_catalog/` | Resolves font assets for project rendering. | [Font catalog](src/font_catalog/README.md) |
| `sound_library/` | Provides licensed sound assets for projects. | [Sound library](src/sound_library/README.md) |

Follow [verification](../../docs/engineering/verification.md#checks) for tests and evidence.
