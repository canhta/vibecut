# write

Frame encoding and container writing for one export.

| Stage | Responsibility | Contract |
|---|---|---|
| `encode` | Encodes video and audio under explicit quality and timing boundaries. | [Encoding](#encoding) |
| `mux` | Writes MP4, MOV and WebM over a seekable sink. | [Muxing](#muxing) |

## Encoding

`VideoEncoder` accepts constant-rate frames on the requested output grid with an
explicit canvas/rate/bitrate/quality/color, as H.264 or VP9. It rejects missing
or discontinuous output timing. Hardware frames pass directly to an encoder
retaining the same frame pool; otherwise transfer/conversion is explicit in the
encoder. CPU recovery frames are converted to the retained pool's software
format and uploaded through FFmpeg. A failed pool allocation/upload remains a
typed native error; encoding is never reopened midstream against stale mux
parameters. Frame-aware swscale uses Lanczos, accurate rounding and frame
color/chroma metadata when conversion is required.

`VideoEncoderSettings.profile` selects automatic, Main or High for H.264 through
the encoder's `profile` option; VP9 refuses explicit H.264 profiles. The
`export_container_available` registry probe admits only installed H.264/AAC or
VP9/Opus encoder pairs. Device-specific open failures retain the typed encoder
failure path rather than silently changing the requested profile.

Input formats/codecs come from the resolved FFmpeg build. The development recipe
includes built-in decoders, static zlib for PNG, native AAC, libvpx and libopus;
dav1d/LAME admission and all distribution requirements are owned by the
[open-source policy](../../../../docs/engineering/open-source.md#ffmpeg-distribution).
Missing encoders/decoders return typed capability failures. AV1 availability is
conditional; no AV1 software decoder is promised by this build recipe.

| Path | Observed fixture evidence |
|---|---|
| MP4 H.264 + AAC | Probe, reorder, software/hardware decode, source-rate audio, seek, stream copy, hardware H.264/AAC encode |
| MP4 HEVC 10-bit PQ | Software decode, color/bit-depth metadata and HDR stream copy |
| WebM VP9 | Lanczos resize, BT.709 tags, mux and CLI decode |
| WebM Opus | AAC-to-Opus frame-size conversion, padding/duration and CLI decode |
| MOV | H.264/AAC export probed through the CLI, including High profile and exact output frame counts |
| M4A AAC speech | 16 kHz mono chunk extraction, exact tick tiling, silence hint and per-chunk ffprobe |

Unknown SDR color tags resolve to the project's BT.709 limited-range default.
Known color/range/chroma tags are retained. SDR encoding refuses HDR, high bit
depth and implicit color changes; explicitly tone-map/convert upstream or stream
copy. Tone mapping itself is not implemented here. H.264 defaults request 85/100
quality and a 12 Mb/s bitrate target; Windows backends map that same quality
request to their native controls and retain the bitrate target on fallback.
NVENC/QSV use a positive CQ/ICQ level at the maximum quality request, avoiding
automatic or disabled native quality. These mappings are not evidence of equal
perceptual quality across platforms.

Hardware decode preference uses VideoToolbox on macOS and D3D11VA/QSV on Windows.
Windows encode defaults to `EncodePreference::Software`; other platforms retain
hardware preference. Explicit hardware preference tries platform hardware and
then the platform software path (VideoToolbox `allow_sw`, or Media Foundation
software), retaining requested settings. Missing capability is an error.
`EncodePreference::Software` requires the platform software path (VideoToolbox
`require_sw` on macOS), retaining the requested bitrate. VideoToolbox attempts
QSCALE only on FFmpeg-supported arm64, with `allow_sw=0` to require hardware; a
successful open proves that encoder accepts Quality. On rejection it opens a
fresh bitrate-mode session with `allow_sw=1`. Intel and explicit software use
bitrate mode directly. No Quality property is sent to a software encoder, and
the bitrate target is never reduced. Other backends retain their existing native
quality mappings.

The CPU compositor explicitly downloads GPU frames when it needs pixels. Direct
decode/encode sharing avoids that transfer only for a compatible retained pool;
this is not a zero-copy claim for composited or vertical output. Physical
device-loss recovery remains unverified and requires its own runtime test.

Media Foundation always receives CPU NV12 input, including when its hardware
encoder is explicitly preferred. FFmpeg's
[D3D11 sample wrapper](https://github.com/FFmpeg/FFmpeg/blob/n9.0.2/libavcodec/mfenc.c#L357)
retains a texture/subresource but not the decoded AVFrame's pool lease, and
releases that frame after input acceptance. An asynchronous MFT may still read
it: [ProcessInput's contract](https://learn.microsoft.com/en-us/windows/win32/api/mftransform/nf-mftransform-imftransform-processinput)
allows the transform to retain the input sample. Reusing the decoder surface can
therefore replace a picture without changing its timestamp. The existing
download/conversion path reaches mfenc's owned sample copy, retaining requested
bitrate, quality, color and timing without an unbounded frame-retention queue.
This deliberately excludes direct D3D11 pool sharing for MF; hardware decode
remains available. `streaming_h264_preserves_every_picture_identity` streams
60 seconds of 1080p frames with an eleven-bit picture marker through default
software and explicit hardware encode, checking every decoded marker and PTS. It
reports a capability skip when no hardware decoder is available; a skip is not
Windows hardware acceptance evidence.

`RgbaConverter::with_threads` and `VideoEncoderSettings::conversion_threads`
admit 1-8 FFmpeg conversion workers, defaulting to one. The execution owner
chooses and charges that bound; no codec quality, color, chroma or transfer
setting changes with the worker count. Conversion contexts retain their native
workers only for that converter/encoder lifetime.

`RgbaConverter::convert_horizontal` accepts a nonempty source-column range and
returns CPU RGBA plus its integer source-column origin. NV12/YUV420P at even
widths use an aligned integer crop with a 64-pixel chroma-reconstruction halo and
128-pixel alignment. FFmpeg applies cropping with alignment enabled; existing
crop metadata, other layouts or a rounded crop retain full conversion. Only the
requested interior replaces full-frame conversion; halo-edge pixels are not a new
resampling contract. PTS, color and alpha convention are retained. Public oracle
tests compare every requested byte through hardware-preferred/software decode,
varied endpoints, alignment transitions and sequential/parallel conversion.

### Observed benchmark

Apple M1 Pro, 16 GiB, macOS 27.2; release profile, two runs on a generated
60 s 1920x1080/30 H.264 BT.709 video-only fixture. This is a media transcode pass
at the same canvas size; it excludes a Rust compositor, subtitle burn and audio.
FFmpeg 9.0.2 was resolved for this observation; no FFmpeg version is committed as
a provisioning pin. Reproduce with `cargo xtask media-bench`.

| Measurement | First run | Warm repeat |
|---|---:|---:|
| Hardware decoded frames | 1,800 | 1,800 |
| Decode fps | 280.623 | 216.495 |
| Encode API fps, including drain | 498.868 | 697.713 |
| Export wall time | 9.668 s | 9.313 s |

The repeat's process peak RSS was 143,458,304 bytes. First-run quality against the
source decoded pixels: SSIM 0.999483 and PSNR 56.681886 dB (not VMAF). Encode API
time excludes decoding/mux time; export wall time includes both. Cache state and
hardware scheduling affect results; these two samples are not a general latency
or quality guarantee. Fixture generation: ffmpeg `testsrc2`, 60 s at 1080p30,
libx264 ultrafast CRF 18, yuv420p with BT.709 tags. No media is committed.

The `benchmark` example prints the actual first decoded pixel format, hardware
frame count, selected encoder and requested quality/bitrate. Run it with
`INPUT OUTPUT --software` to measure the explicit platform software encoder with
the same settings and hardware-preferred decode. Compare both outputs against the
same source with SSIM/PSNR; retaining settings does not establish equal
perceptual quality. A missing SDK backend must be distinguished from an absent
GPU, and a working external FFmpeg encoder does not establish that the managed
SDK exposes it. Physical device loss and Windows N without Media Feature Pack
require their own runtime environment; fault injection proves only recovery
control flow.

The earlier software H.264 failure was caused by sending QSCALE/Quality to
VideoToolbox's software encoder, rather than an absent codec (since corrected).
Explicit software export uses the platform encoder with `require_sw=1` and the
requested bitrate; it never silently selects hardware or requires another codec.
Observed on Apple M1 Pro/macOS 27.2: the real export regression produces 30 H.264
frames plus AAC, with maximum decoded color-patch channel error <=6. Intel/Rosetta,
VM-only fallback and Windows runtime remain unverified. An encoder open rejection
returns `MediaError::Ffmpeg` with its native cause; `EncoderUnavailable` is
reserved for an absent explicit software H.264 encoder.

## Muxing

`Muxer` writes MP4/MOV/WebM over a seekable `MuxSink` (plain File or the
project's bounded streaming writer), accepts stream-copy or encoded parameters,
rescales timestamps and propagates trailer/fsync errors. Use a temporary output
and promote it only after successful `finish`; render/engine own cancellation,
final-file replacement and restart policy. Packet buffers are retained by
reference; only metadata is rescaled into the final mux time base. FFmpeg owns
codec padding and edit-list creation.

`Container::from_extension` resolves MP4, MOV and WebM case-insensitively for CLI
and control requests; render still takes an explicit container setting.
