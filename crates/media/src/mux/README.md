# Muxing

- `Muxer`: MP4/MOV/WebM over a seekable `MuxSink` (plain File or the project's
  bounded streaming writer), stream-copy parameters or encoded
  parameters, timestamp rescaling and trailer/fsync error propagation. Use a
  temporary output and promote it only after successful `finish`; render/engine
  own cancellation, final-file replacement and restart policy.

`Container::from_extension` resolves MP4, MOV and WebM case-insensitively for
CLI and control requests; render still takes an explicit container setting.
