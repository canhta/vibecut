# media

Headless streaming media I/O through dynamically linked LGPL-only FFmpeg. Owns
probe, demux, codec parameters, decode/encode, hardware selection, conversion,
resampling and muxing. Rust timeline semantics, compositing, captions, text,
provider policy, jobs and output promotion belong to their respective owners.

## Source identity

`SourceAsset::import` probes an absolute native path and streams its complete
bytes through BLAKE3. `AssetData` validates persistable identity/path/hash and
`SourceProbe` metadata, without serializing runtime codec handles. Metadata
retains rational time bases/rates, optional duration/count/VFR evidence, audio
start offsets/priming and video color/bit depth. Stream indexes are unique and
capped at 256; paths require lossless Unicode serialization. Paths/hashes are
redacted in asset debug output. Callers schedule imports and keep source files
stable while probing/hashing; this is not a filesystem snapshot operation.

`availability` checks file presence without hashing; missing media is a relink
state, not a malformed project. `relink` accepts a new absolute path only if its
full content hash matches, preserving asset identity and probed metadata.
`ContentHash::from_bytes` hashes already bounded content using the same BLAKE3
identity as `ContentHash::read`, reused by project publication receipts.
Presence alone does not prove unchanged content. `ContentHash::read_with_cancel`
checks cooperative cancellation throughout streaming full-content hashing.
Project persistence retains
these media-owned contracts; it does not recreate probe or asset models.

## Entry points and timing

- `probe(path)` / `PacketReader`: native `File` paths through seekable AVIO,
  detected format, optional presentation duration, stream start/time base,
  codec parameters, dimensions/color, audio format/priming and optional counts.
  Unknown frame counts/VFR stay unknown. No filename-based capability guesses.
- `TimeBase`: positive rational seconds per unit, converted through `time` with
  checked integer arithmetic and explicit rounding. FFmpeg applies container
  edits, codec priming and packet/frame reordering once. Never reapply an offset.
- `Packet`: `shifted` rebases checked timestamps without changing payloads;
  retained FFmpeg buffers with original PTS/DTS/duration. `try_clone`
  retains buffers without copying payloads. `CodecParameters` owns its extradata;
  `display_transform_required` checks sample aspect ratio and display matrix so a
  pixel consumer can refuse unsupported presentation instead of losing it.
- `VideoDecoder` / `AudioDecoder`: whole access units in decode order; drain with
  `flush`. `push_packet`, `pull_frame` and `finish_input` support single-frame
  consumption without collecting a decoder burst. Batch APIs retain the 64-frame
  cap. Decoder native image/sample ceilings are installed before opening codecs.
  Source-rate audio retains its channel layout and presentation clock.
- `VideoReader`: hardware-preferred sequential frames, preceding-keyframe seek
  plus decode-forward selection, and one software recovery after a decode error.
  Recovery seeks and suppresses already emitted PTS rather than duplicating frames.
- `VideoFrame`: retains CPU/GPU buffers; CPU access requires explicit `download`.
  `with_pts` assigns an output tick without copying pixels. `from_rgba8` checks
  allocation and assigns straight-alpha metadata; `RgbaConverter` preserves frame
  transfer/primaries/alpha convention for the compositor boundary. `PixelFormat` describes
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
- `Muxer`: MP4/MOV/WebM over a seekable `MuxSink` (plain File or the project's
  bounded streaming writer), stream-copy parameters or encoded
  parameters, timestamp rescaling and trailer/fsync error propagation. Use a
  temporary output and promote it only after successful `finish`; render/engine
  own cancellation, final-file replacement and restart policy.
- `extract_speech_audio`: decodes a `SourceRange` from a `SourceAsset`, resamples to mono 16 kHz and
  writes AAC-LC m4a chunks for transcription upload. Bounds every file by bytes
  and duration, overlaps adjacent chunks, prefers low-energy split frames and
  reports exact source ticks plus an energy-based `speech_detected` hint. Input
  without an audio stream returns `MediaError::NoAudioStream`.
- `extract_speech_windows_with_cancel`: the same decode, resample and split policy,
  but streams bounded mono 16 kHz f32 PCM `SpeechWindow`s to a callback instead of
  writing m4a. The callback runs before decoding continues, so only one window is
  retained; source ticks, overlap and `speech_detected` have the same meaning.
  Engine uses this for local/remote ASR invocations without an encode/decode round trip.

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
| M4A AAC speech | 16 kHz mono chunk extraction, exact tick tiling, silence hint and per-chunk ffprobe |

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
capability is an error. `EncodePreference::Software` requires the platform software
path (VideoToolbox `require_sw` on macOS), retaining the requested bitrate.
VideoToolbox attempts QSCALE only on FFmpeg-supported arm64, with `allow_sw=0`
to require hardware; a successful open proves that encoder accepts Quality. On
rejection it opens a fresh bitrate-mode session with `allow_sw=1`. Intel and
explicit software use bitrate mode directly. No Quality property is sent to a
software encoder, and the bitrate target is never reduced. Other backends retain
their existing native quality mappings.
Windows device/quality behavior and live device-loss
recovery remain unverified; they require the Windows hardware ticket.

`RgbaConverter::with_threads` and `VideoEncoderSettings::conversion_threads`
admit 1–8 FFmpeg conversion workers, defaulting to one. The execution owner
chooses and charges that bound; no codec quality, color, chroma or transfer
setting changes with the worker count. Conversion contexts retain their native
workers only for that converter/encoder lifetime.

`RgbaConverter::convert_horizontal` accepts a nonempty source-column range and
returns CPU RGBA plus its integer source-column origin. NV12/YUV420P at even
widths use an aligned integer crop with a 64-pixel chroma-reconstruction halo
and 128-pixel alignment. FFmpeg applies cropping with alignment enabled; existing
crop metadata, other layouts or a rounded crop retain full conversion. Only the
requested interior replaces full-frame conversion; halo-edge pixels are not a
new resampling contract. PTS, color and alpha convention are retained. Public
oracle tests compare every requested byte through hardware-preferred/software
decode, varied endpoints, alignment transitions and sequential/parallel conversion.

## Speech upload chunks

`extract_speech_audio` takes a `SourceAsset`; it and the `_with`/`_with_cancel`
variants default to 24 MB, 10 minutes, 2 s overlap and 48 kb/s mono AAC-LC. The
[OpenAI speech-to-text guide](https://developers.openai.com/api/docs/guides/speech-to-text)
(source-documented 2026-11-04) accepts files up to 25 MB in `mp3`, `mp4`, `mpeg`,
`mpga`, `m4a`, `wav` or `webm`, so m4a is admissible and the default leaves 1 MB
of decimal headroom. Chunk boundaries are quantized to the 16 kHz output sample
grid; each `AudioChunk` reports the exact source ticks and the ticks shared with
the preceding chunk. Files use chunk-local timestamps starting at zero and are
promoted from a temporary file only after a successful trailer. Splits search
the last 2 s before the duration target for the lowest-energy 20 ms frame, so
both the byte and duration bounds hold. `speech_detected` is an RMS energy hint,
not voice-activity detection; silent input still produces chunks. Decoding is
streaming: only the confirmed prefix plus a few seconds of PCM stay buffered,
independent of input length.

`extract_speech_windows_with_cancel` shares the same bounded splitter and reports
each window as contiguous mono 16 kHz f32 PCM. It performs no second encode or
decode, so ASR sees the decoder output directly. A callback error stops extraction;
the caller owns the returned window and may process it synchronously. The callback
receives complete windows only; a window's sample count matches its exact source
tick duration on the 16 kHz grid.

Observed on Apple M1 Pro, 16 GiB, macOS 27.2 with the debug test binary and a
generated 2 h 48 kHz stereo AAC fixture: 13 chunks covering 7200.000 s in 26.5 s
of extraction and 45.7–47.2 MB maximum RSS across runs. This covers extraction
only; provider upload and transcription remain unverified.

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

The task /09 software H.264 failure was caused by sending QSCALE/Quality to
VideoToolbox's software encoder, rather than an absent codec (corrected by /31).
Explicit software export uses the platform encoder with `require_sw=1` and the
requested bitrate; it never silently selects hardware or requires another codec.
Observed on Apple M1 Pro/macOS 27.2: the real export regression produces 30 H.264
frames plus AAC, with maximum decoded color-patch channel error <=6. Intel/Rosetta,
VM-only fallback and Windows runtime remain unverified. An encoder open rejection
returns `MediaError::Ffmpeg` with its native cause; `EncoderUnavailable` is reserved
for an absent explicit software H.264 encoder.

## Speech energy windows

`analyze_speech_energy(asset, source_range, cancel)` exposes the same streaming
mono 16 kHz decode/resample path used by speech extraction, without AAC encoding
or file writes. It returns contiguous 20 ms SpeechEnergyWindows (a final partial
window is retained), with exact source ranges and measured RMS amplitude ratios.
Requests are capped at two hours/360,001 windows and cancellation is checked
between packets and frames. Engine clip snapping consumes this measurement;
there is no separate decoder or voice-activity model. RMS is an energy hint:
background noise and unvoiced speech need transcript protection. The caller sets
the quiet threshold when interpreting windows.
