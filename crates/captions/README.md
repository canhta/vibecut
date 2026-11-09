# captions

Headless source-timed transcript and multilingual cue storage, BCP 47 language profiles, readable cue segmentation, pure source-to-output edit mapping and SRT, WebVTT, plain TXT and ASS interchange. Translation providers and subtitle rendering are excluded.

`Transcript` owns stable `Segment` IDs, optional source `Word` ranges, speaker and validated confidence, optional `TimingSpans`, and a provider/model/settings `Fingerprint`. Timing spans keep source unit identity and provenance; tokens, characters and syllables never become fake words. `Cue` owns a stable ID, a validated `SourceRange`, the source language, language-keyed `CueText` with independent edit flags and translation references, and measured `word_ranges` from native ASR timing (empty for imported or estimated cues). `set_text` protects edits unless explicitly overridden. `CueTrack` groups cues. `detect_fillers` pairs a cue's text with its measured `word_ranges` and returns stable whole-word `FillerCandidate` spans from a validated `FillerLexicon`; shipped lexicons are reviewed data files under `data/fillers/`. All models support serde. `NfcText` normalizes stored text on construction and deserialization, and its Debug is redacted. BCP 47 tags validate syntax and standardize casing with no fixed language allowlist. The crate has no project store or schema.

| Module | Responsibility | Contract |
|---|---|---|
| `filler.rs` | Finds whole-word filler candidates through per-language lexicon data over cue word timings. | [Filler detection](src/filler/README.md) |
| `interchange/` | Round-trips SRT and WebVTT subtitle files and exports plain TXT through cue tracks. | [SRT, WebVTT and TXT](src/interchange/README.md) |
| `interchange/ass/` | Parses ASS documents into typed cues and maps styles. | [ASS import and style mapping](src/interchange/ass/README.md) |
| `language.rs` | Owns BCP 47 language tags and editable subtitle profiles. | [Language profiles](src/language/README.md) |
| `mapping.rs` | Maps source ticks to output ticks through edit cut lists. | [Edit mapping](src/mapping/README.md) |
| `segment.rs` | Generates subtitle cues from transcripts. | [Segmentation](src/segment/README.md) |
| `span.rs` | Owns raw-to-NFC text correspondence at grapheme granularity. | [Text correspondence](src/span/README.md) |
| `timing.rs` | Owns source timing identity and explicit raw/NFC/display correspondence. | [Timing spans](src/timing/README.md) |

The crate root re-exports the same public names. `cargo test -p captions` needs no media, network or renderer oracle.
