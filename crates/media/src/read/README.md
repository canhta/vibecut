# read

Container reading, packet decoding and bounded thumbnails over the one shared
FFmpeg backend.

| Stage | Responsibility | Contract |
|---|---|---|
| `demux` | Opens containers through bounded I/O and delivers timed packets. | [Demux](#demux) |
| `decode` | Decodes packets into frames and reads video sequentially. | [Decoding](#decoding) |
| `thumbnail` | Decodes bounded first-frame images. | [Thumbnails](#thumbnails) |

## Demux

`probe(path)` and `PacketReader` open native `File` paths through seekable AVIO
(32 KiB buffers), detect the format, and report optional presentation duration,
stream start/time base, codec parameters, dimensions/color, audio
format/priming and optional counts. Unknown frame counts and VFR stay unknown;
no filename-based capability guesses are made. Probing reads at most 32 MiB.
The cancellation token interrupts probing and every packet read; a packet read
after exhaustion returns `MediaError::TooLarge("input read bytes")`.
`PacketReader::seek` seeks backward to the keyframe at or before a presentation
time and resets consumers. Packets are capped at 64 MiB.

## Decoding

`VideoDecoder` and `AudioDecoder` take whole access units in decode order and
drain with `flush`. `push_packet`, `pull_frame` and `finish_input` support
single-frame consumption without collecting a decoder burst; batch APIs retain
the 64-frame cap. Decoder native image/sample ceilings are installed before
opening codecs. `VideoFrame` retains CPU/GPU buffers; CPU access requires an
explicit `download`. `with_pts` assigns an output tick without copying pixels.
`from_rgba8` checks allocation and assigns straight-alpha metadata;
`RgbaConverter` preserves frame transfer/primaries/alpha convention for the
compositor boundary. `PixelFormat` describes layout, and plane access supplies
checked indexes/strides. Negative strides require normalization. No raw
hardware pointers leave the crate.

`VideoReader` provides hardware-preferred sequential frames, a
preceding-keyframe seek plus decode-forward selection, and one software recovery
after a decode error. Recovery seeks and suppresses already emitted PTS rather
than duplicating frames. `open_with_cancel` interrupts buffered frame delivery
as well as packet reads and recovery; cancellation is terminal. Only native
decoder errors trigger the one software retry; I/O, cancellation, bounds and
metadata errors retain their original category.

## Thumbnails

`thumbnail` and `thumbnail_at` decode the first frame at a requested position
(`seek` plus decode forward for a later position) through the existing
`VideoReader` and `RgbaConverter::convert_scaled`, converting directly to a
320 px maximum edge without a full-resolution RGBA intermediary. This is the one
frame decoder shared with the reader; no second decoder exists.
`Thumbnail::encode`/`decode` is the bounded cache payload (1024 px ceiling), and
`thumbnail_at_with_read_bytes` reports how many source bytes the first frame
needed for streaming evidence.

All errors use `MediaError`; logs omit paths and media/user text. FFmpeg's default
logger is disabled because it can disclose filenames. After a codec/mux error,
discard that execution; flush is idempotent after successful completion.
