# Localization

Interface: `Text`, `text`, `Locale`, `format`, `format_count`, `number`, `decimal`,
`date`, `signed`, `parse_decimal`, `Plural`, `blend_label`, `export_duration`,
`export_range`.

One catalog owns every user-visible string. `Text` is the key set with English and
Vietnamese rows beside each key, so a missing row is a compile error. `text` returns a
row, `format` fills named placeholders, `format_count` picks the locale's plural row and
groups the number, and `number`, `decimal` and `date` apply the locale's separators and
date order. Project dates are UTC display only. Timecode stays raw ticks at the display
boundary and is never localized.

`Locale::from_system` reads the OS preference (`sys-locale`). A missing preference file
starts from it; `Preferences::default` stays English for deterministic tests. The header
language menu switches at runtime and the choice persists.

`cargo xtask i18n` checks what types cannot: empty or duplicate rows, unused keys and
string literals passed to egui widgets. A dev-only `skrifa` test asserts the bundled
Inter font covers every Vietnamese character in the catalog.
