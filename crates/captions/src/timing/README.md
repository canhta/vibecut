# Timing spans

`TimingSpans::new` validates ordered, independently timed layers of `SpanTiming`
with ordinal identity, raw byte spans, optional source range, native/forced
provenance, speaker and confidence. Different layers and normalized grapheme
correspondences can overlap; each layer's source time and byte endpoints remain
monotonic. Token fragments can cover the same NFC grapheme without losing their
individual raw endpoints. Serialization stores the original raw text and inputs,
then reconstructs and validates maps on read. Original provider text is retained
only as explicit raw correspondence data; normalized and display text use NFC.
Debug redacts all text and raw bytes. Total text, map and span overhead is capped
at 16 MiB, including aggregate input to segmentation.

`with_display(DisplayVariant)` rebuilds from the original raw source using
caller-resolved NFC `DisplayReplacement` phrase ranges, permitting many-to-many
Han orthography changes without shifting timestamps by bytes. The variant records
its language; segmentation refuses a change of primary language (translations
must use `segment_translation`) and emits the selected display language while
retaining the recognition language as cue source. No conversion lexicon or
automatic Han choice is bundled. Replacement endpoints must preserve graphemes,
and replacements cannot be empty or compose across their neighbors. A phrase
whose graphemes change together must be supplied as one correspondence.

`groups(profile)` selects the preferred available layer, then tokens, graphemes,
characters, syllables, words or segments. Grouping keeps Latin runs intact and
joins candidate boundaries crossed by provider spans or converted display phrases;
it never divides an existing source range into finer timestamps. Token grouping
uses whole display graphemes protected by actual token boundaries. Uncovered
visible raw bytes or missing native ranges yield an untimed group. Segmentation
reuses its existing UAX #14/dictionary boundary policy on these groups; translation
execution never accepts these source timing spans.
## Span timing performance

Observed on Apple M1 Pro, 16 GiB RAM, macOS 27.2, release profile: the ignored
`timing_span_benchmark` creates raw/NFC/display maps and segments 11,600 raw bytes
into 400 cues using 11,600 one-byte provider tokens, including NFD Vietnamese,
Han/Latin, full-width text and emoji. Twenty same-process calls preserved all
visible NFC text and had zero visible end-tick drift. First call: 4.944 ms;
19 warm calls: 4.335–6.520 ms (median 5.116 ms); process peak RSS: 11,927,552 bytes.
The synthetic token clock is one millisecond per byte; the final formatting space
is retained in the transcript but excluded from visible cue timing. These figures
measure pure correspondence/segmentation, not ASR accuracy or speech alignment.
Build with `cargo test -p captions --release --test it timing:: --no-run`, then run the
emitted binary under `/usr/bin/time -l` with
`--exact timing::timing_span_benchmark --ignored --nocapture`. Other hardware is unverified.
