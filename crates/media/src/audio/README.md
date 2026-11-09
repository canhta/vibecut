# audio

Speech extraction, bounded speech energy measurement and explicit audio resampling over the shared decode path.

## Extraction

`decode_speech` opens one `PacketReader` and `AudioDecoder`, seeks at most one second before the requested range, resamples every decoded frame to mono 16 kHz with one global phase, and pushes contiguous absolute sample indexes to the caller. Frames before the preroll are discarded without conversion. Exact source ticks come from the anchor tick plus the sample count, never from a rounded PTS. Cancellation is checked between packets and resampled frames. A range without audio returns `MediaError::NoAudioStream`; an audio stream shorter than the request is trimmed to its real end.

## Speech upload chunks

`extract_speech_audio` takes a `SourceAsset`. It and its `_with`/`_with_cancel` variants default to 24 MB, 10 minutes, 2 s overlap and 48 kb/s mono AAC-LC. The [OpenAI speech-to-text guide](https://developers.openai.com/api/docs/guides/speech-to-text) accepts files up to 25 MB in `mp3`, `mp4`, `mpeg`, `mpga`, `m4a`, `wav` or `webm`, so m4a is admissible and the default leaves 1 MB of decimal headroom.

Chunk boundaries are quantized to the 16 kHz output sample grid. Each `AudioChunk` reports its exact source ticks and the ticks shared with the preceding chunk. Files use chunk-local timestamps from zero and are promoted from a temporary file only after a successful trailer. Splits search the last 2 s before the duration target for the lowest-energy 20 ms frame, so both byte and duration bounds hold. `speech_detected` is an RMS energy hint, not voice-activity detection; silent input still produces chunks. Decoding streams: only the confirmed prefix plus a few seconds of PCM stay buffered, independent of input length.

`extract_speech_windows_with_cancel` shares the same splitter and reports each window as contiguous mono 16 kHz f32 PCM, with no second encode or decode. The callback receives complete windows only, each with a sample count matching its exact source tick duration on the 16 kHz grid. A callback error stops extraction.

Observed on Apple M1 Pro, macOS 27.2, debug test binary, generated 2 h 48 kHz stereo AAC: 13 chunks covering 7200.000 s in 26.5 s, 45.7–47.2 MB maximum RSS. Provider upload and transcription are unverified.

## Speech energy windows

`analyze_speech_energy(asset, source_range, cancel)` uses the same streaming mono 16 kHz decode and resample path without AAC encoding or file writes. It returns contiguous 20 ms `SpeechEnergyWindow`s (a final partial window is kept) with exact source ranges and RMS amplitude ratios. Requests are capped at two hours and 360,001 windows; cancellation is checked between packets and frames. Engine clip snapping consumes this measurement. RMS is an energy hint: background noise and unvoiced speech need transcript protection. The caller sets the quiet threshold.

## Source peaks

`peak_summary(asset, cancel)` decodes one source's audio stream and returns a bounded `PeakSummary`: for every 128-sample bucket the minimum and maximum linear amplitude of each channel, anchored at the first decoded sample PTS. The bucket size is the implementation reference's timeline waveform bucket (`DEFAULT_SOURCE_WAVEFORM_BUCKET_SIZE = 128` in `apps/web/src/media/waveform-summary.ts:7`), recorded here as `PEAK_BUCKET_SAMPLES`. Mono and stereo source layouts are admitted; other channel counts are `Unsupported`. Decoding reuses `AudioDecoder` and the explicit `AudioResampler` planar-f32 conversion at the source rate and layout, one frame at a time, so memory follows the summary rather than the duration. `PeakSummary::over` flags a bucket whose any sample leaves `[-1, 1]` (above 0 dBFS). `bucket_span` and `bucket_source_range` are the only tick/sample mapping helpers. `encode`/`decode` use a compact binary envelope; `decode` rejects hostile headers, wrong counts, non-finite values and inverted bounds. One encoded summary is capped at 64 MiB, about 3.1 hours of 128-sample buckets at 48 kHz stereo; a longer source fails with `TooLarge`, and a source without audio is `NoAudioStream`. Cancellation is checked between packets, frames and resampled outputs.

Observed on the Windows host, debug test binaries: a generated 0.5 s 8 kHz stereo file matched an independent FFmpeg `s16le` decode bucket-for-bucket with peak error 0 (stated tolerance one 16-bit step, 1/32768). A generated 2 h 48 kHz stereo AAC source produced 345,600,000 samples and 2,700,000 buckets (43,200,034-byte summary) in 16.0 s with a 65.9 MiB peak process working set. The local 30-minute 44.1 kHz stereo interview sample produced 79,380,480 samples and 620,160 buckets (9,922,594-byte summary) in 3.9 s with a 44.1 MiB peak process working set including import hashing. A two-hour generated source and that interview sample are ignored manual checks; run them with the documented environment variables.

## Audio conversion

`AudioResampler` performs explicit continuous-span rate, layout and sample-format conversion. It admits output rates of 8-192 kHz and up to 24x expansion, keeps the source origin and counts output samples in integer time; drain with `flush`. `convert` returns bounded frames and drains buffered output without finalizing the filter. A discontinuity requires a new converter.

`AudioEncoder` keeps the source rate, converts sample format and uses a bounded FFmpeg FIFO for AAC/Opus frame sizes. FFmpeg owns codec delay and final padding.
