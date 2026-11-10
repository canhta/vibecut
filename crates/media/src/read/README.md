# read

Container reading, packet decoding and bounded thumbnails over the one shared FFmpeg backend.

## Demux

`probe(path)` and `PacketReader` open native `File` paths through seekable AVIO (32 KiB buffers), detect the format, and report optional presentation duration, stream start and time base, codec parameters, dimensions and color, audio format and priming, and optional counts. Unknown frame counts and VFR stay unknown; capabilities are never guessed from filenames. Probing reads at most 32 MiB.

The cancellation token interrupts probing and every packet read. A packet read after exhaustion returns `MediaError::TooLarge("input read bytes")`. `PacketReader::seek` seeks backward to the keyframe at or before a presentation time and resets consumers. Packets are capped at 64 MiB.

## Decoding

`VideoDecoder` and `AudioDecoder` take whole access units in decode order and drain with `flush`. `push_packet`, `pull_frame` and `finish_input` support single-frame consumption; batch APIs keep the 64-frame cap. Native image and sample ceilings are installed before codecs open.

`VideoFrame` holds CPU or GPU buffers; CPU access needs an explicit `download`. `with_pts` assigns an output tick without copying pixels. `from_rgba8` checks allocation and assigns straight-alpha metadata. `RgbaConverter` preserves frame transfer, primaries and alpha convention for the compositor boundary. Plane access supplies checked indexes and strides; negative strides require normalization. No raw hardware pointers leave the crate.

`VideoReader` gives hardware-preferred sequential frames, a preceding-keyframe seek plus decode-forward selection, and one software recovery after a decode error. Recovery seeks and suppresses already emitted PTS rather than duplicating frames. `open_with_cancel` interrupts buffered frame delivery, packet reads and recovery; cancellation is terminal. Only native decoder errors trigger the software retry; I/O, cancellation, bounds and metadata errors keep their category.

## Thumbnails

`thumbnail` decodes the first frame at a position (`seek` plus decode forward for a later one) through `VideoReader` and `RgbaConverter::convert_scaled`, converting directly to a 320 px maximum edge without a full-resolution RGBA intermediary. `thumbnails_at` samples ascending positions from one reader: it opens and seeks once, decodes the range once and hands each position the first frame at or after it, the same frame one `thumbnail` call per position returns. A single position is that same loop, including the opening recovery thumbnail and the first frame's read-bytes report; each leg between positions keeps the single-thumbnail read allowance. `Thumbnail::encode`/`decode` is the bounded cache payload (1024 px ceiling).
