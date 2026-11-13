# Brand kit

`libraries::brand_kit::BrandKit` stores the user's brand kits under one app-data root. A kit belongs to the user, not to a project: it holds colours, font families, media files and preset values. A project never references a kit. Using a kit item copies its value into the project, so a project opens the same on another machine and later kit edits leave it unchanged.

## Contents

| Kind | Stored as | Value rule |
|---|---|---|
| Colour | `name`, `rgba` bytes | ordered by insertion; names unique within the kit |
| Font | family name | names a family in the font store; the kit does not verify availability |
| Media | file copied in, named by its SHA-256 | recorded with name, size and hash; verified on every listing |
| Text preset | `project::SavedTextPreset` | the type and its name rule belong to `project`; the kit stores it unchanged |
| Caption preset | `project::SavedCaptionPreset` | likewise |

Adjustment presets are not stored yet: their type lives in the desktop app's preferences, which this crate cannot import.

## Operations

`create`, `rename`, `delete`, `set_active` manage kits; `kits()` lists them with the active flag. `add_color`, `remove_color`, `add_font`, `remove_font`, `add_media`, `remove_media`, `add_text_preset`, `remove_text_preset`, `add_caption_preset` and `remove_caption_preset` edit one kit. Removal by name returns `NotFound` when the name is absent. `active_view()` returns the active kit as the engine's `BrandKitView`, the value `brand.list` reports.

Every change is validated as a whole, written to `brand_kits.json` through a `.partial` file and renamed into place, and only then published. A failed change leaves the previous state; media files that no kit cites are removed. One `project::WriterLease` on `brand_kits.vibecut` serialises writers across processes.

## Media and verification

`add_media` streams the source into `media/.partial-*` while hashing, renames it to `media/<sha256>` and re-hashes the stored copy. A file already stored with the same bytes is kept. Source must be a regular file of at most `MAX_FILE_BYTES`.

Listings check each stored file against its recorded size and hash. A missing file is reported `missing`, a file that differs is `corrupt`, and the entry stays in the kit either way. Nothing is dropped silently.

## Archive

`export_kit(id, dest)` writes an empty folder with `kit.json` (schema, then the kit contents without local identity) and one `media/<sha256>` file per media entry. It refuses when any stored file is missing or corrupt.

`import_kit(src)` reads `kit.json` (at most 4 MiB), checks the schema and every name and bound, then requires each `media/<sha256>` to match its recorded size and hash before copying it in. The hash is the only path component an archive can name, so an entry such as `../x` is refused as invalid. The kit is added under a new local identity; a name clash is refused.

Export followed by import followed by export produces byte-equal `kit.json` and media files.

## Limits

| Limit | Value | Refusal |
|---|---|---|
| Kits in a store | `MAX_KITS` = 16 | `Limit` |
| Entries of each kind in a kit | `MAX_ENTRIES` = 64 | `Limit` |
| Stored media file | `MAX_FILE_BYTES` = 256 MiB | `Limit` |
| Name (kit, colour, font, media, preset) | 1 to `MAX_NAME_BYTES` = 128 bytes, no controls, no `/` or `\`, no leading or trailing space | `Invalid` |
| Index or archive record read from disk | 4 MiB | `Limit` |

These bounds are this crate's choice; no CapCut evidence sets them.

## Errors

Failures are `engine::BrandKitError`: `NotFound`, `Limit`, `Invalid(what)`, `Integrity` (a damaged index or an archive file that fails verification), `Io` and `Project`. The engine maps them to wire codes; `brand.list` refuses `Unavailable` when no composition root lends the store.

## Control

`brand.list` is a read command. Composition roots lend the store as `Caller::brand` through `engine::control::BrandAccess`; the engine owns no store and copies nothing into a project. The control adapter is `control.rs`.
