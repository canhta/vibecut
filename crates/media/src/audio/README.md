# audio

Speech extraction, bounded speech energy measurement and explicit audio
resampling over the shared decode path.

| Stage | Responsibility | Contract |
|---|---|---|
| `extract` | Streams a source range as mono 16 kHz PCM. | [Extraction](#extraction) |
| `chunk` | Splits a range into bounded upload chunks and PCM windows. | [Speech upload chunks](#speech-upload-chunks) |
| `energy` | Measures 20 ms energy windows over a source range. | [Speech energy windows](#speech-energy-windows) |
| `resample` | Converts sample rate, layout and format over one clock. | [Audio conversion](#audio-conversion) |

## Extraction

`decode_speech` opens one `PacketReader` and `AudioDecoder`, seeks at most one
second before the requested range, resamples every decoded frame to mono 16 kHz
with one global phase, and pushes contiguous absolute sample indexes to the
caller. Frames before the preroll are discarded without conversion. Exact source
ticks come from the anchor tick plus the sample count, never from a rounded PTS.
Cancellation is checked between packets and resampled frames. A range without
audio returns `MediaError::NoAudioStream`; an audio stream shorter than the
request is trimmed to its real end.

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
decode, so ASR sees the decoder output directly. A callback error stops
extraction; the caller owns the returned window and may process it synchronously.
The callback receives complete windows only; a window's sample count matches its
exact source tick duration on the 16 kHz grid.

Observed on Apple M1 Pro, 16 GiB, macOS 27.2 with the debug test binary and a
generated 2 h 48 kHz stereo AAC fixture: 13 chunks covering 7200.000 s in 26.5 s
of extraction and 45.7-47.2 MB maximum RSS across runs. This covers extraction
only; provider upload and transcription remain unverified.

## Speech energy windows

`analyze_speech_energy(asset, source_range, cancel)` exposes the same streaming
mono 16 kHz decode/resample path used by speech extraction, without AAC encoding
or file writes. It returns contiguous 20 ms `SpeechEnergyWindow`s (a final
partial window is retained), with exact source ranges and measured RMS amplitude
ratios. Requests are capped at two hours/360,001 windows and cancellation is
checked between packets and frames. Engine clip snapping consumes this
measurement; there is no separate decoder or voice-activity model. RMS is an
energy hint: background noise and unvoiced speech need transcript protection. The
caller sets the quiet threshold when interpreting windows.

## Audio conversion

`AudioResampler` performs explicit continuous-span rate/layout/sample-format
conversion. It admits output rates of 8-192 kHz and up to 24x expansion, keeps
the source origin and counts output samples in integer time; drain with `flush`.
`convert` returns bounded frames and drains buffered output without finalizing the
filter. A discontinuity requires a new converter. `AudioEncoder` retains source
rate, converts sample format, and uses a bounded FFmpeg FIFO for AAC/Opus frame
sizes. FFmpeg owns codec delay and final padding.
