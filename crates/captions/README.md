# captions

Headless source-timed transcript and multilingual cue storage, BCP 47 language profiles, readable cue segmentation, pure source-to-output edit mapping and SRT, WebVTT, plain TXT and ASS interchange. Translation providers and subtitle rendering are excluded. The complete caption presentation value and its sparse override patch live here so project, render and the app share one persisted style vocabulary.

`clip_track` clips a cue track to one source range: retained cues keep their source IDs and text, partial cues clamp at the half-open edges, and output uses the range's coordinates. `Transcript` owns stable `Segment` IDs, optional source `Word` ranges, speaker and validated confidence, optional `TimingSpans`, and a provider/model/settings `Fingerprint`. Timing spans keep source unit identity and provenance; tokens, characters and syllables never become fake words. `Cue` owns a stable ID, a validated `SourceRange`, the source language, language-keyed `CueText` with independent edit flags and translation references, and measured `word_ranges` from native ASR timing (empty for imported or estimated cues). `CueTrack` groups cues and owns their shared style table: equal styles are stored once and cues reference an entry by `CueStyleId`, so a document with many cues and few styles stays compact. The table admits at most `MAX_CUE_STYLES` distinct entries. ASS import interns into that same table while parsing. `set_text` protects edits unless explicitly overridden. `Cue::source_text` selects the cue's source-language text, never a translation; `Cue::words` pairs its UAX #29 tokens index-for-index with `word_ranges` and returns nothing when the counts disagree, but count equality is not alignment proof. Verified display painting instead consumes `Cue::word_timing`, whose individual native/forced correspondence is owned by [Timing spans](src/timing/README.md). `detect_fillers` pairs a cue's text with its measured `word_ranges` and returns stable whole-word `FillerCandidate` spans from a validated `FillerLexicon`, with filled pauses by default and discourse markers only on request; shipped lexicons are reviewed data files under `data/fillers/`. All models support serde. `NfcText` normalizes stored text on construction and deserialization, and its Debug is redacted. BCP 47 tags validate syntax and standardize casing with no fixed language allowlist. The crate has no project store or schema.

| Module | Responsibility | Contract |
|---|---|---|
| `filler.rs` | Finds whole-word filler candidates through per-language lexicon data over cue word timings. | [Filler detection](src/filler/README.md) |
| `interchange/` | Round-trips SRT and WebVTT subtitle files and exports plain TXT through cue tracks. | [SRT, WebVTT and TXT](src/interchange/README.md) |
| `interchange/ass/` | Parses ASS documents into typed cues and maps styles. | [ASS import and style mapping](src/interchange/ass/README.md) |
| `language.rs` | Owns BCP 47 language tags and editable subtitle profiles. | [Language profiles](src/language/README.md) |
| `mapping.rs` | Maps source ticks to output ticks through edit cut lists and clips a cue track to a source range. | [Edit mapping](src/mapping/README.md) |
| `segment.rs` | Generates subtitle cues from transcripts. | [Segmentation](src/segment/README.md) |
| `span.rs` | Owns raw-to-NFC text correspondence at grapheme granularity. | [Text correspondence](src/span/README.md) |
| `style.rs` | Owns the complete persisted caption presentation style and its sparse field overrides. | - |
| `timing.rs` | Owns source timing identity and explicit raw/NFC/display correspondence. | [Timing spans](src/timing/README.md) |

The crate root re-exports the same public names. `cargo test -p captions` needs no media, network or renderer oracle.
