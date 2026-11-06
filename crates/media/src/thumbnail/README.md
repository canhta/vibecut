# Thumbnails

`thumbnail` / `thumbnail_at` decode the first frame at a requested position
(`seek` plus decode forward for a later position) through the existing
`VideoReader` and `RgbaConverter::convert_scaled`, converting directly to a
320 px maximum edge without a full-resolution RGBA intermediary. This is
the one frame decoder shared with the reader; no second decoder exists.
`Thumbnail::encode`/`decode` is the bounded cache payload (1024 px ceiling),
and `thumbnail_at_with_read_bytes` reports how many source bytes the first frame
needed for streaming evidence.
