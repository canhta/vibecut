# captions

Headless source-timed transcript and multilingual cue storage, BCP 47 language
profiles, pure source-to-output edit mapping and SRT/WebVTT interchange.
Excludes segmentation,
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

## Edit mapping

`EditMapping::new` validates an ordered list of `(SourceRange, output_start)`
pairs; each pair retains source duration at unit speed. Empty mappings, gaps,
repeated source spans, signed ticks and overlapping output spans are supported.
Output endpoint overflow returns a typed time error. Segment order identifies
occurrences and need not match temporal order; the mapping has no persisted schema.

`map_tick` returns every retained occurrence as a `MappedTick` (segment index and
output tick). `map_range` returns independent `MappedRange` intersections with both
source and output coordinates. All bounds are half-open. `inverse_tick` uses the
segment identity to recover the source exactly, even for overlapping output spans;
unknown occurrences and output ticks outside the selected segment have no inverse.

`map_cues` rebuilds a `CueTrack` from an original source track, drops fully cut
cues and clips partial intersections without merging gaps. It stable-sorts output
starts and preserves text, language, edit flags and translation references; it
never shifts overlaps or calls subtitle repair. Source IDs must be unique. Each
output cue receives a deterministic fragment ID encoding the mapping occurrence
and complete original ID. `CueFragmentId::from_id` recovers both; `to_id` encodes
an occurrence index followed by `/` and the unchanged source ID. This remains
unambiguous when the original ID contains slashes or looks like another fragment.

After changing cuts, build a new mapping and map the original source track again;
never map previously clipped output. Normalize stream timestamps to source-asset
time before mapping, applying the audio start offset once at that boundary. This
module neither knows nor reapplies stream offsets. Frame-snapped cuts use the
existing time crate conversions; retiming, media IO and project commands belong
to their respective consumers. Overlapping/negative output cues remain exact here;
interchange export separately applies its documented repair policy below.

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
Mapping tests exercise cut boundaries, repeated occurrences, disjoint fragments,
rebuild from source, 29.97 frame cuts, one-time audio offset normalization, extreme
ticks, source identity/edit preservation and SRT accuracy within 1 ms. Property
tests verify exact inverses for each retained occurrence and no inverse for gaps.
No media, network provider or renderer oracle is needed for this pure crate.
