# Entry points and timing

- `probe(path)` / `PacketReader`: native `File` paths through seekable AVIO,
  detected format, optional presentation duration, stream start/time base,
  codec parameters, dimensions/color, audio format/priming and optional counts.
  Unknown frame counts/VFR stay unknown. No filename-based capability guesses.
  Source-rate audio retains its channel layout and presentation clock.
- `VideoReader`: hardware-preferred sequential frames, preceding-keyframe seek
  plus decode-forward selection, and one software recovery after a decode error.
  Recovery seeks and suppresses already emitted PTS rather than duplicating frames.

All errors use `MediaError`; logs omit paths and media/user text. FFmpeg's default
logger is disabled because it can disclose filenames. After a codec/mux error,
discard that execution; flush is idempotent after successful completion.
