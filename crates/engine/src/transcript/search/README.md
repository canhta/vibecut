# Transcript search index

| Module | Responsibility | Contract |
|---|---|---|
| `folding.rs` | Folds text for matching: compatibility forms, combining marks, Vietnamese đ and case. | [Folding](#folding) |
| `index.rs` | Builds, encodes and checks the per-asset index entry and its fingerprint. | [Index entry](#index-entry) |

## Folding

`fold` applies NFKC, then NFD, drops the combining marks in the Unicode combining blocks,
maps đ/Đ to d/D and lowercases. Queries and indexed text pass through the same function, so
`chao`, `Chào` and a decomposed `Chào` match one another. Han text has no marks and
passes through unchanged.

## Index entry

One entry per asset, keyed by the artifact cache's `TranscriptIndex` kind, so it lives in the
cache beside waveforms and never in the project. The stored header carries the format and
folding version, the media content hash and a fingerprint of the source-language transcript.
A header that no longer matches the current media or transcript is replaced and reported as
stale. Unreadable bytes and corrupt entries are rebuilt. A store refused for size or budget
still answers the query from memory.
