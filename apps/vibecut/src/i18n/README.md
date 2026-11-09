# Localization

One catalog in `src/i18n/text.rs` owns every user-visible string. The `Text` enum is the key set, with English and Vietnamese rows beside each key, so a missing row is a compile error. `text` returns a row, `format` fills named placeholders, `format_count` picks the locale's plural row and groups the number, and `number`/`decimal`/`date` apply the locale's separators and date order. Project dates are UTC display only. Timecode stays raw ticks at the display boundary and is never localized. Export measurements and blend-mode labels are catalog helpers.

`Locale::from_system` reads the OS preference (`sys-locale`). A missing preference file starts from it; `Preferences::default` stays English for deterministic tests. The header language menu switches at runtime and the choice persists.

Catalog decision: an enum table keeps call sites typed and en/vi coverage complete with no runtime formatter, which Project Fluent (`fluent-bundle`, `fluent-templates`) and `rust-i18n` (string keys, runtime fallback) do not. `cargo xtask i18n` checks what types cannot: empty or duplicate rows, unused keys and string literals passed to egui widgets. `skrifa` (dev-only) asserts the bundled Inter font covers every Vietnamese character in the catalog.
