# write

Frame encoding and container writing for one export.

## Encoding

`VideoEncoder` accepts constant-rate frames on the requested output grid with explicit canvas, rate, bitrate, quality and color, as H.264 or VP9. It rejects missing or discontinuous output timing. Hardware frames pass directly to an encoder that keeps the same frame pool; otherwise transfer and conversion happen explicitly in the encoder. CPU recovery frames are converted to the retained pool's software format and uploaded through FFmpeg. A failed pool allocation or upload is a typed native error; encoding is never reopened midstream against stale mux parameters. Conversion uses frame-aware swscale with Lanczos, accurate rounding and frame color and chroma metadata.

`VideoEncoderSettings.profile` selects automatic, Main or High for H.264 through the encoder's `profile` option; VP9 refuses explicit H.264 profiles. `export_container_available` admits only installed H.264/AAC or VP9/Opus encoder pairs. Device-specific open failures keep the typed encoder failure rather than changing the requested profile.

Input formats and codecs come from the resolved FFmpeg build. The development recipe includes built-in decoders, static zlib for PNG, native AAC, libvpx and libopus. dav1d, LAME and distribution requirements are owned by the [open-source policy](../../../../docs/engineering/open-source.md#ffmpeg-distribution). Missing encoders or decoders return typed capability failures. AV1 availability is conditional; no AV1 software decoder is promised.

Unknown SDR color tags resolve to the project's BT.709 limited-range default. Known color, range and chroma tags are kept. SDR encoding refuses HDR, high bit depth and implicit color changes; tone-map or convert upstream, or stream copy. Tone mapping is not implemented here.

H.264 defaults request 85/100 quality and a 12 Mb/s bitrate target. Windows backends map the same quality request to their native controls and keep the bitrate target on fallback. NVENC/QSV use a positive CQ/ICQ level at the maximum quality request. These mappings do not establish equal perceptual quality across platforms.

Hardware decode preference uses VideoToolbox on macOS and D3D11VA/QSV on Windows. Windows encode defaults to `EncodePreference::Software`; other platforms default to hardware. Explicit hardware preference tries platform hardware, then the platform software path (VideoToolbox `allow_sw`, or Media Foundation software), keeping the requested settings. Missing capability is an error. `EncodePreference::Software` requires the platform software path (VideoToolbox `require_sw` on macOS) and keeps the requested bitrate.

VideoToolbox tries QSCALE on arm64, with `allow_sw=0` to require hardware; a successful open proves the encoder accepts Quality. On rejection it opens a fresh bitrate-mode session with `allow_sw=1`. Intel and explicit software use bitrate mode directly. No Quality property is sent to a software encoder, and the bitrate target is never reduced. An encoder open rejection returns `MediaError::Ffmpeg` with its native cause; `EncoderUnavailable` is reserved for an absent explicit software H.264 encoder.

The CPU compositor explicitly downloads GPU frames when it needs pixels. Direct decode/encode sharing avoids that transfer only for a compatible retained pool; it is not a zero-copy claim for composited or vertical output.

Media Foundation always receives CPU NV12 input, including with its hardware encoder explicitly preferred. FFmpeg's [D3D11 sample wrapper](https://github.com/FFmpeg/FFmpeg/blob/n9.0.2/libavcodec/mfenc.c#L357) does not retain the decoded AVFrame's pool lease while the asynchronous MFT may still read it, so direct D3D11 pool sharing is excluded for MF; hardware decode stays available.

`RgbaConverter::with_threads` and `VideoEncoderSettings::conversion_threads` admit 1-8 FFmpeg conversion workers, default one. The execution owner chooses and charges that bound. No quality, color, chroma or transfer setting changes with the worker count. Workers live only as long as that converter or encoder.

`RgbaConverter::convert_horizontal` accepts a nonempty source-column range and returns CPU RGBA plus its integer source-column origin. NV12/YUV420P at even widths use an aligned integer crop with a 64-pixel chroma-reconstruction halo and 128-pixel alignment. Existing crop metadata or another layout falls back to full conversion. Only the requested interior replaces full-frame conversion; halo-edge pixels carry no resampling contract. PTS, color and alpha convention are kept.

## Muxing

`Muxer` writes MP4, MOV and WebM over a seekable `MuxSink` (plain File or the project's bounded streaming writer), accepts stream-copy or encoded parameters, rescales timestamps and propagates trailer and fsync errors. Write to a temporary output and promote it only after successful `finish`; render and engine own cancellation, final-file replacement and restart policy. Packet buffers are shared by reference and only metadata is rescaled into the final mux time base. FFmpeg owns codec padding and edit-list creation.

`Container::from_extension` resolves MP4, MOV and WebM case-insensitively for CLI and control requests; render takes an explicit container setting.
