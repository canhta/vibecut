# Text correspondence

`NormalizedText` owns raw-to-NFC `TextSpan` correspondence at whole extended
grapheme granularity. `raw_range` and `normalized_range` expand partial byte
spans to enclosing graphemes, including tokens splitting UTF-8 scalars. Input and
map overhead each have a 16 MiB ceiling. Spans are correspondence, never a
source/target word alignment.
