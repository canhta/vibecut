# captions

Headless source-timed transcript and multilingual cue storage, BCP 47 language
profiles, readable cue segmentation, pure source-to-output edit mapping and
SRT, WebVTT and ASS interchange. Excludes
translation providers and subtitle rendering.

## Entry points and persistence

`Transcript` owns stable `Segment` IDs, optional source `Word` ranges, speaker and
validated confidence, optional `TimingSpans`, and provider/model/settings
`Fingerprint`. Timing spans retain source unit identity and provenance; tokens,
characters and syllables never become fake words. `Cue` owns a
stable ID, validated `SourceRange`, source language, and language-keyed `CueText`
with independent edit flags and translation references. `set_text` protects edits
unless explicitly overridden. `CueTrack` groups cues; all models support serde.
`NfcText` normalizes every stored text on construction/deserialization; its Debug
is redacted. BCP 47 tags validate syntax and standardize casing without a fixed
language allowlist. No project store/schema is introduced here.

| Module | Responsibility | Contract |
|---|---|---|
| `interchange/` | Round-trips SRT and WebVTT subtitle files through cue tracks. | [SRT and WebVTT](src/interchange/README.md) |
| `interchange/ass/` | Parses ASS documents into typed cues and maps styles. | [ASS import and style mapping](src/interchange/ass/README.md) |
| `language.rs` | Owns BCP 47 language tags and editable subtitle profiles. | [Language profiles](src/language/README.md) |
| `mapping.rs` | Maps source ticks to output ticks through edit cut lists. | [Edit mapping](src/mapping/README.md) |
| `segment.rs` | Generates subtitle cues from transcripts. | [Segmentation](src/segment/README.md) |
| `span.rs` | Owns raw-to-NFC text correspondence at grapheme granularity. | [Text correspondence](src/span/README.md) |
| `timing.rs` | Owns source timing identity and explicit raw/NFC/display correspondence. | [Timing spans](src/timing/README.md) |

The crate root re-exports the same public names at unchanged crate-root paths.

## Verification

`cargo test -p captions` exercises model persistence/NFC/edit protection, language
fallback, bilingual ordering, decoding, timing repair and malformed/overflow
inputs. Mapping tests exercise cut boundaries, repeated occurrences, disjoint fragments,
rebuild from source, 29.97 frame cuts, one-time audio offset normalization, extreme
ticks, source identity/edit preservation and SRT accuracy within 1 ms. Property
tests verify exact inverses for each retained occurrence and no inverse for gaps.
Segmentation tables cover diacritics, mixed Han/Latin/emoji, long words,
long sentences, caller measurements/dictionary spans, warning conflicts and target
timing provenance. Property tests cover canonical interchange stability, hostile
bytes and segmentation text/timing preservation. The ignored
`segmentation_benchmark` test runs a deterministic Vietnamese corpus in release
mode and checks exact visible-text preservation and zero end-tick drift.

No media, network provider or renderer oracle is needed for this pure crate.
