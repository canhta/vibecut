# libraries

External assets for a project: fonts are resolved and verified for rendering; sounds and music are searched, saved, previewed and inserted as project media. Each module owns its local cache and admission limits and shares the [`transport`](../transport/README.md) crate's bounded HTTPS client. Neither edits project data except through the registered insert/edit commands.

Use `brand_kit::BrandKit` for the user's brand kits, which live outside projects and reach a project only as copied values; use `fonts::FontStore` for discovery, verified loading, explicit downloads and user font imports, `sounds::{SoundClient, SoundLibrary, SoundDownloads, SoundPreview}` plus the `add_to_timeline`/`add_saved` helpers for licensed audio, and `music::{MusicClient, MusicLibrary, MusicDownloads, MusicPreview}` for openly licensed music. Callers compose the engine `JobRunner`, the project writer lease and the app-owned preview sink; this crate owns no UI, scheduler or project mutation outside the registered edit.

| Module | Responsibility | Contract |
|---|---|---|
| `font_catalog/` | Resolves font assets for project rendering. | [Font catalog](src/font_catalog/README.md) |
| `sound_library/` | Provides licensed sound assets for projects. | [Sound library](src/sound_library/README.md) |
| `music_library/` | Provides openly licensed music for projects. | [Music library](src/music_library/README.md) |
| `brand_kit/` | Stores the user's brand kits; projects copy values out of them. | [Brand kit](src/brand_kit/README.md) |
| `sticker_library/` | Searches bundled flag stickers and keeps recent ids outside projects. | [Sticker library](src/sticker_library/README.md) |

Follow [verification](../../docs/engineering/verification.md#checks) for tests and evidence.
