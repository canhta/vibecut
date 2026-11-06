# Localization


One app-owned catalog in `src/catalog.rs` owns every user-visible string. The
`Text` enum is the stable key set; both English and Vietnamese rows live beside
each key, so an omitted row is a compile error. `catalog::text` returns a row,
`format` fills named placeholders, `format_count` selects the locale's plural row
and groups the number, and `number`/`decimal`/`date` apply the locale's separators
and date order; project dates are UTC display only. Timecode remains raw ticks at
the display boundary and is never localized.

`Locale::from_system` reads the OS preference (`sys-locale` 0.3.2, MIT OR
Apache-2.0); a missing preference file starts from that locale, while
`Preferences::default` stays English for deterministic tests. The header language
menu control switches the catalog at runtime and the existing bounded preference store
persists the choice.

Catalog format decision: Project Fluent (`fluent-bundle` 0.16.0, Apache-2.0 OR
MIT; `fluent-templates` 0.15.1, MIT OR Apache-2.0) has strong plural/gender
syntax and compile-time FTL loading, but its message-id lookup and formatter stack
are heavier than this two-locale API and still do not type-check keys at call
sites. `rust-i18n` 4.2.4 (MIT) generates `t!` lookups from YAML/TOML and is simple,
but missing keys fall back at runtime and keys are strings no compiler checks. The
enum table keeps every call site typed, gives complete en/vi coverage for free and
uses no added runtime formatter for two CLDR-simple locales; `cargo xtask i18n`
adds the checks the type system cannot see (empty/duplicate rows, unused keys and
direct string literals passed to egui widgets). `skrifa` (dev-only, MIT OR
Apache-2.0) asserts that the bundled Inter font has a glyph for every Vietnamese
character in the catalog; the offscreen diacritics screenshot is audit evidence,
not a test.

property-section open state, the active assets tab, per-type properties tabs and
timeline snapping and the preview fit/zoom (with the last snapshot directory) use one bounded, versioned preference store;
`ui.prefs.set` edits have app-owned undo/redo and never touch Project JSON.
Tab navigation is presentation state: it persists through the same validated
store but stays out of app undo history so it cannot shadow a project edit. A
damaged layout restores its defaults with a visible warning, while an unreadable
document restores the system defaults. Unversioned and unsupported versions are
refused without rewriting their bytes on load or save. Edits and writes share the 16 KiB
read budget, so a saved document always remains readable.
