# Text correspondence

`NormalizedText` owns raw-to-NFC `TextSpan` correspondence at whole extended grapheme granularity. `raw_range` and `normalized_range` expand partial byte spans to enclosing graphemes, including tokens that split UTF-8 scalars. Input and map overhead each cap at 16 MiB. Spans are correspondence, never a source/target word alignment.
