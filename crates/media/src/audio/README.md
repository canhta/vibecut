# audio

Speech extraction, bounded speech energy measurement and explicit audio resampling over the shared decode path.

## Extraction

`decode_speech` opens one `PacketReader` and `AudioDecoder`, seeks at most one second before the requested range, resamples every decoded frame to mono 16 kHz with one global phase, and pushes contiguous absolute sample indexes to the caller. Frames before the requested range are converted to establish that phase but are not pushed. Exact source ticks come from the anchor tick plus the sample count, never from a rounded PTS. Cancellation is checked between packets and resampled frames. A range without audio returns `MediaError::NoAudioStream`; an audio stream shorter than the request is trimmed to its real end.

## Speech windows

`extract_speech_windows_with_cancel` takes a `SourceAsset` and streams bounded mono 16 kHz f32 PCM windows; it writes no file and adds no encode/decode round trip. Defaults are 10 minutes and 2 s overlap; `SpeechExtractSettings` bounds the window duration and the audio repeated at the start of the next window. The callback receives complete windows only, each with a sample count matching its exact source tick duration on the 16 kHz grid, and a callback error stops extraction.

Window boundaries are quantized to the 16 kHz output sample grid. Each `SpeechWindow` reports its exact source ticks and the ticks shared with the preceding window. Splits search up to the last 2 s before the duration target for the lowest-energy 20 ms frame, so the duration bound holds. `speech_detected` is an RMS energy hint, not voice-activity detection; silent input still produces windows. Decoding streams at most one window's PCM plus its split search and overlap, independent of input length.

## Speech energy windows

`analyze_speech_energy(asset, source_range, cancel)` uses the same streaming mono 16 kHz decode and resample path without AAC encoding or file writes. It returns contiguous 20 ms `SpeechEnergyWindow`s (a final partial window is kept) with exact source ranges and RMS amplitude ratios. Requests are capped at two hours and 360,001 windows; cancellation is checked between packets and frames. Shorts boundary snapping consumes this measurement. RMS is an energy hint: background noise and unvoiced speech need transcript protection. The caller sets the quiet threshold.

## Source peaks

`peak_summary(asset, cancel)` decodes one source's audio stream and returns a bounded `PeakSummary`: for every 128-sample bucket the minimum and maximum linear amplitude of each channel, anchored at the first decoded sample PTS. The bucket size is 128 samples (`PEAK_BUCKET_SAMPLES`). Mono and stereo source layouts are admitted; other channel counts are `Unsupported`. Decoding reuses `AudioDecoder` and the explicit `AudioResampler` planar-f32 conversion at the source rate and layout, one frame at a time, so memory follows the summary rather than the duration. `PeakSummary::over` flags a bucket whose any sample leaves `[-1, 1]` (above 0 dBFS). `bucket_span` and `bucket_source_range` are the only tick/sample mapping helpers. `encode`/`decode` use a compact binary envelope; `decode` rejects hostile headers, wrong counts, non-finite values and inverted bounds. One encoded summary is capped at 64 MiB, about 3.1 hours of 128-sample buckets at 48 kHz stereo; a longer source fails with `TooLarge`, and a source without audio is `NoAudioStream`. Cancellation is checked between packets, frames and resampled outputs.

## Audio conversion

`AudioResampler` performs explicit continuous-span rate, layout and sample-format conversion. It admits output rates of 8-192 kHz and up to 24x expansion, keeps the source origin and counts output samples in integer time; drain with `flush`. `convert` returns bounded frames and drains buffered output without finalizing the filter. A discontinuity requires a new converter.

`AudioEncoder` keeps the source rate, converts sample format and uses a bounded FFmpeg FIFO for AAC/Opus frame sizes. FFmpeg owns codec delay and final padding.
