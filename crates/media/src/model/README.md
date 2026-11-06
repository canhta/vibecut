# Time base and packets

- `TimeBase`: positive rational seconds per unit, converted through `time` with
  checked integer arithmetic and explicit rounding. FFmpeg applies container
  edits, codec priming and packet/frame reordering once. Never reapply an offset.
- `Packet`: `shifted` rebases checked timestamps without changing payloads;
  retained FFmpeg buffers with original PTS/DTS/duration. `try_clone`
  retains buffers without copying payloads. `CodecParameters` owns its extradata;
  `display_transform_required` checks sample aspect ratio and display matrix so a
  pixel consumer can refuse unsupported presentation instead of losing it.
