# Decoding

- `VideoDecoder` / `AudioDecoder`: whole access units in decode order; drain with
  `flush`. `push_packet`, `pull_frame` and `finish_input` support single-frame
  consumption without collecting a decoder burst. Batch APIs retain the 64-frame
  cap. Decoder native image/sample ceilings are installed before opening codecs.
- `VideoFrame`: retains CPU/GPU buffers; CPU access requires explicit `download`.
  `with_pts` assigns an output tick without copying pixels. `from_rgba8` checks
  allocation and assigns straight-alpha metadata; `RgbaConverter` preserves frame
  transfer/primaries/alpha convention for the compositor boundary. `PixelFormat` describes
  layout, and plane access supplies checked indexes/strides. Negative strides
  require normalization. No raw hardware pointers leave the crate.
