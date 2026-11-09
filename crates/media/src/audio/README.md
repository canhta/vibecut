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

## Audio conversion

`AudioResampler` performs explicit continuous-span rate, layout and sample-format conversion. It admits output rates of 8-192 kHz and up to 24x expansion, keeps the source origin and counts output samples in integer time; drain with `flush`. `convert` returns bounded frames and drains buffered output without finalizing the filter. A discontinuity requires a new converter.

`AudioEncoder` keeps the source rate, converts sample format and uses a bounded FFmpeg FIFO for AAC/Opus frame sizes. FFmpeg owns codec delay and final padding.
