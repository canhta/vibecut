# media

Headless streaming media I/O through dynamically linked LGPL-only FFmpeg. Owns
probe, demux, codec parameters, decode/encode, hardware selection, conversion,
resampling and muxing. Rust timeline semantics, compositing, captions, text,
provider policy, jobs and output promotion belong to their respective owners.

## Entry points and timing

- `probe(path)` / `PacketReader`: native `File` paths through seekable AVIO,
  detected format, optional presentation duration, stream start/time base,
  codec parameters, dimensions/color, audio format/priming and optional counts.
  Unknown frame counts/VFR stay unknown. No filename-based capability guesses.
- `TimeBase`: positive rational seconds per unit, converted through `time` with
  checked integer arithmetic and explicit rounding. FFmpeg applies container
  edits, codec priming and packet/frame reordering once. Never reapply an offset.
- `Packet`: retained FFmpeg buffers with original PTS/DTS/duration. `try_clone`
  retains buffers without copying payloads. `CodecParameters` owns its extradata.
- `VideoDecoder` / `AudioDecoder`: whole access units in decode order; drain with
  `flush`. Source-rate audio retains its channel layout and presentation clock.
- `VideoReader`: hardware-preferred sequential frames, preceding-keyframe seek
  plus decode-forward selection, and one software recovery after a decode error.
  Recovery seeks and suppresses already emitted PTS rather than duplicating frames.
- `VideoFrame`: retains CPU/GPU buffers; CPU access requires explicit `download`.
  `with_pts` assigns an output tick without copying pixels. `PixelFormat` describes
  layout, and plane access supplies checked indexes/strides. Negative strides
  require normalization. No raw hardware pointers leave the crate.
- `VideoEncoder`: constant-rate frames on the requested output grid, explicit
  canvas/rate/bitrate/quality/color, H.264 or VP9. Reject missing/discontinuous
  output timing. Hardware frames pass directly to a compatible encoder; otherwise
  transfer/conversion is explicit in the encoder. Frame-aware swscale uses Lanczos,
  accurate rounding and frame color/chroma metadata when conversion is required.
- `AudioResampler`: explicit continuous-span rate/layout/sample-format conversion.
  Admits output rates of 8–192 kHz and up to 24× expansion. Keeps the source origin and counts output samples in integer time; drain `flush`.
  `convert` returns bounded frames and drains buffered output without finalizing
  the filter. A discontinuity requires a new converter. `AudioEncoder` retains source rate,
  converts sample format, and uses a bounded FFmpeg FIFO for AAC/Opus frame sizes.
  FFmpeg owns codec delay and final padding.
- `Muxer`: MP4/MOV/WebM over a seekable file, stream-copy parameters or encoded
  parameters, timestamp rescaling and trailer/fsync error propagation. Use a
  temporary output and promote it only after successful `finish`; render/engine
  own cancellation, final-file replacement and restart policy.

All errors use `MediaError`; logs omit paths and media/user text. FFmpeg's default
logger is disabled because it can disclose filenames. After a codec/mux error,
discard that execution; flush is idempotent after successful completion.

## Input and quality boundaries

Input formats/codecs come from the resolved FFmpeg build. The development recipe
includes built-in decoders, native AAC, libvpx and libopus; dav1d/LAME admission and
all distribution requirements are owned by the
[open-source policy](../../docs/engineering/open-source.md#ffmpeg-distribution).
Missing encoders/decoders return typed capability failures. AV1 availability is
conditional; no AV1 software decoder is promised by this build recipe.

| Path | Observed fixture evidence |
|---|---|
| MP4 H.264 + AAC | Probe, reorder, software/hardware decode, source-rate audio, seek, stream copy, hardware H.264/AAC encode |
| MP4 HEVC 10-bit PQ | Software decode, color/bit-depth metadata and HDR stream copy |
| WebM VP9 | Lanczos resize, BT.709 tags, mux and CLI decode |
| WebM Opus | AAC-to-Opus frame-size conversion, padding/duration and CLI decode |
| MOV | FFmpeg muxer admitted; dedicated runtime fixture remains unverified |

Unknown SDR color tags resolve to the project's BT.709 limited-range default.
Known color/range/chroma tags are retained. SDR encoding refuses HDR, high bit
depth and implicit color changes; explicitly tone-map/convert upstream or stream
copy. Tone mapping itself is not implemented here. H.264 defaults request 85/100
quality and a 12 Mb/s bitrate target; Windows backends map that same quality
request to their native controls and retain the bitrate target on fallback.
Those mappings are not evidence of equal perceptual quality across platforms.

Hardware preference uses VideoToolbox on macOS and D3D11VA/QSV on Windows.
Encoding tries platform hardware and then the platform software path (VideoToolbox
`allow_sw`, or Media Foundation software), retaining requested settings. Missing
capability is an error. Windows device/quality behavior and live device-loss
recovery remain unverified; they require the Windows hardware ticket.

## Bounds and verification

Individual native FFmpeg allocations are capped at 256 MiB before media I/O.
AVIO uses 32 KiB buffers; probing reads at most 32 MiB. Stream count is capped at
256, extradata at 1 MiB, packets at 64 MiB, decoded frame area at 2^26 pixels,
audio frames at 65,536 samples/channel and 32 channels, and one receive batch at
64 frames. Audio encoding admits up to eight channels and caps its FIFO at
131,072 samples/channel. Native FFmpeg allocation/codec limits still apply;
these checks do not establish safety for every hostile C-library input. The native
build backports upstream PSSH iterative cleanup and linear append fixes under
[ADR 0002](../../docs/adr/0002-desktop-media-and-time.md). A generated 20,000-box
PSSH regression probes on a 512 KiB worker stack in a subprocess with a five-second
deadline. Full-size same-rate and upsampled PCM and AVI-to-MP4 H.264 stream copy
have dedicated public API regressions.

`cargo xtask ffmpeg` resolves/builds the current LGPL SDK in the external user
cache. `cargo xtask ci` configures that SDK for workspace checks. Direct Cargo
invocations need `FFMPEG_DIR` and the platform's runtime library search path; use
the xtask commands for the configured development path. See
[development workflow](../../docs/engineering/workflows.md#upgrade-policy).

Tests generate synthetic inputs with an independently installed ffmpeg, then
exercise public packet/decode/encode/resample/mux interfaces. Subprocess ffprobe
checks counts, durations, codecs and tags. The CLI and production backend share
FFmpeg algorithms: these are interface/pipeline oracles, not independent codec
implementations. Missing tools are explicit skips; `VIBECUT_REQUIRE_ORACLES=1`
requires them. A found tool that fails is a failure.

Observed on macOS: Unicode/space paths, cancellation, deterministic truncation and
bit-flip cases, and a 2,500,002,687-byte sparse MP4 with only 65,536 bytes read while
opening and reading its first packet. Source-rate AAC processing produced 48,000
samples; 48 kHz stereo -> 16 kHz mono produced 16,000 samples, bit-exact against
the CLI resampling fixture. Native codec behavior on other machines is unverified.

## Observed benchmark

Apple M1 Pro, 16 GiB, macOS 27.2; release profile, two runs on a generated
60 s 1920x1080/30 H.264 BT.709 video-only fixture. This is a media transcode pass
at the same canvas size; it excludes a Rust compositor, subtitle burn and audio.
FFmpeg 9.0.2 was resolved for this observation; no FFmpeg version is committed as
a provisioning pin. Measured media code revision: `2bb02f0`. Reproduce with `cargo xtask media-bench`.

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
