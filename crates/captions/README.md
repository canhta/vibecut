# captions

Headless source-timed transcript and multilingual cue storage, BCP 47 language
profiles, and SRT/WebVTT interchange. Excludes segmentation, edit mapping,
translation providers and subtitle rendering.

## Entry points and persistence

`Transcript` owns stable `Segment` IDs, optional source `Word` ranges, speaker and
validated confidence, and provider/model/settings `Fingerprint`. `Cue` owns a
stable ID, validated `SourceRange`, source language, and language-keyed `CueText`
with independent edit flags and translation references. `set_text` protects edits
unless explicitly overridden. `CueTrack` groups cues; all models support serde.
`NfcText` normalizes every stored text on construction/deserialization; its Debug
is redacted. BCP 47 tags validate syntax and standardize casing without a fixed
language allowlist. No project store/schema is introduced here.

`LanguageRegistry::standard` provides editable profiles for vi, en, zh-Hans and
zh-Hant. Lookup tries progressively shorter tag prefixes then the generic profile.
Limits are grapheme counts/rates, not byte counts; policies and font preferences
are initial defaults, not measured reading-speed or font-coverage guarantees.

## Interchange and deterministic repairs

`parse(bytes, format, source_lang)` accepts up to 16 MiB: UTF-8, UTF-8 BOM,
UTF-16LE/BE with BOM, or Windows-1252 fallback. BOM, encoding fallback, malformed
encoding replacements and CRLF/CR conversion warn. UTF-16 without BOM is not
detected. Missing SRT counters, comma/dot fractions (1–9 digits), omitted hours,
and missing blank separators are accepted. Bad timings/stray lines warn and skip;
empty text remains in the model. VTT header absence warns. VTT identifiers are
replaced with deterministic ordinal IDs; settings, header metadata and
NOTE/STYLE/REGION blocks are discarded (with warnings for metadata). Inline VTT
markup remains literal model text; this is text interchange, not a style renderer.

Zero/backward durations become 1 ms at the same start with a warning. The track
stable-sorts by start (ties retain input order), warns on nonmonotonic input,
clamps negative starts to zero, and shifts overlaps after the previous cue while
preserving duration. `SourceRange` rejects zero/backward persisted ranges; callers
cannot bypass validation through serde. Arithmetic overflow returns a typed error.
Repair positions refer to sorted cue ordinals; import syntax positions are lines;
position zero means a document-wide warning. IDs and language edit flags survive
track repair. Repair can return an error after modifying earlier cues.

`export(track, format, languages)` selects one tag or two distinct tags in display
order; missing/empty language text warns and is omitted. Output uses UTF-8/LF;
blank payload lines are removed with a warning to prevent block splitting. SRT
renumbers cues; VTT escapes &, < and > and decodes those entities on import.
SRT rejects payload lines resembling timestamps to prevent cue injection.
Ticks round to nearest millisecond, ties away from zero (integer arithmetic).
Collapsed durations expand to 1 ms and push following starts as needed; overflow
errors. After canonical export, SRT import/export is byte-stable. Sub-millisecond
source precision and file formatting/metadata are not preserved.

## Verification

`cargo test -p captions` exercises model persistence/NFC/edit protection, language
fallback, bilingual ordering, decoding, timing repair and malformed/overflow
inputs. Property tests cover canonical interchange stability and hostile bytes.
No media, network provider or renderer oracle is needed for this pure crate.
