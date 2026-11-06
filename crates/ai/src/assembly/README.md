# Recognition and time

`AudioWindow` borrows finite mono f32 PCM with an exact sample count/rate and
absolute nonnegative source sample range. Resampling and source offset mapping
happen before this boundary. Results must use that source coordinate system,
not chunk-relative samples. `RecognitionSegment`/`TimedText` are adapter transport
records, not persisted project models. Every stored text uses captions' `NfcText`.
IDs are unique, timings are contained in the input window and segment, and each
unit layer is ordered by time and matching source text. Different timing layers
may describe the same audio; tokens and characters never become fake words.
`RecognitionSegment::raw_tokens` additionally carries provider-agnostic subword
bytes with optional absolute sample ranges, provenance and validated confidence.
Individual tokens may split UTF-8; only their full concatenation is decoded and
must normalize to the authoritative segment text. Tokens retain their per-segment
ordinal identity. Empty tokens, malformed/truncated UTF-8, duplicate token layers,
nonmonotonic/out-of-segment ranges and unadvertised token timing are errors.
Wholly untimed raw bytes can accompany an untimed segment without a timing
capability. A token range requires both segment timing and advertised token timing;
raw text alone does not advertise or establish recognition timing.
Token byte and record sizes are capped at 16 MiB per segment before assembly.

`transcribe` validates native timings, then calls an explicitly supplied admissible
aligner for segments missing segment timing or the requested timing unit. Missing
timing can also be applied later through `align(recognition, provider, ...)`: this
reuses the same validation/replacement owner, keeps the recognition provider/model
and payload fingerprint, retains native unit layers and accepts a local aligner's
empty-text template scope by binding the exact missing segment texts before
execution. A remote aligner must receive an already text-bound scope and its own
consent; a template never authorizes an upload. `Recognition::needs_timing(unit)`
reports whether alignment is required without exposing timing internals. Separate
alignment rejects audio whose source window or sample rate differs from the
recognition. Alignment is at segment granularity; providers can omit punctuation spans. Existing native
unit layers and segment boundaries remain authoritative. The aligner must preserve
text, IDs and ordering, return every requested segment and declare forced timing.
Wrong-language/unsupported aligners leave missing timing absent; malformed output,
consent/auth/quota/payload failures propagate. No timestamp interpolation occurs.
A raw-token layer satisfies requested token timing only when every token has a
range; absent or partially timed raw tokens request alignment. Forced token timing
fills missing native ranges only with identical token counts and bytes at each
ordinal. Native bytes, confidence and existing ranges remain authoritative; filled
ranges carry forced provenance. Retokenized output, incomplete requested timing
and conflicts with retained native boundaries fail with a typed error. Other
unit layers are available when present; providers may omit punctuation, and
quality/coverage acceptance for those layers remains adapter-specific.

`Recognition::is_text_only` indicates any missing segment timing. `to_transcript`
returns captions' existing `Transcript`, or `TimingUnavailable` when any segment
is untimed. Non-word and raw-token layers become captions-owned `TimingSpans`, preserving
raw-to-NFC many-to-many correspondence, ordinal IDs, unit identity, provenance,
confidence, speaker and missing timing. Existing word-only output keeps `Word`
storage; when spans coexist with words, the complete layers also remain available
in the span owner. No layer is silently relabelled or dropped. Captions owns
script grouping and same-language display conversion; no native Whisper wiring
or language conversion table is introduced by this generic seam. Source samples convert once to ticks using
floor for both absolute boundaries and checked arithmetic. Native fingerprint and
optional alignment fingerprint are available separately; engine must include both
in its job/cache key. The captions conversion preserves the recognizer provider/model and combines the
recognizer settings and complete aligner fingerprint with length-prefixed fields,
so changing alignment also changes the derived transcript fingerprint.

`assemble` requires matching source content, sample rate, language and recognition
fingerprints. Native-only chunks can merge with chunks using forced alignment;
the merged result retains the actual aligner identity. Two different nonempty
aligner fingerprints are incompatible. Fully timed segments sort by source start;
only completely identical observations (same segment ID, text, source range,
timing layers, raw-token bytes/timing/confidence and metadata) deduplicate. Every other overlap, including equal-text
speech with different IDs or ranges, returns a typed ambiguity error, leaving chunk-boundary
reconciliation for an explicit retry/user policy. Untimed segments retain chunk
order and cannot be deduplicated by invented coordinates. There is no fuzzy text
matching or splitting of words at chunk boundaries.
