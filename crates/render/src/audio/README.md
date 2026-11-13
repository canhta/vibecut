# Audio clock and headroom

`audio/` owns mix, cleanup, retime, loudness, limiter and oversampling.

`mix_audio(project, absolute_start_sample, frames, channels, sources, hooks)` is the preview and export owner. Project rate defaults to 48 kHz and admits 8 to 192 kHz. Output is mono or stereo, default stereo. Every source and item tick boundary rounds once to the absolute output sample grid using Nearest. Silence fills gaps and half-open inactive intervals. A block admits at most 65,536 samples per channel and 4,096 audible spans. Stereo stays L/R, mono duplicates at unity, stereo to mono averages L/R. Other source layouts are unsupported.

Gain is f32 with per-item dB gain and mute evaluated through project's item-local animation evaluator at each absolute output sample, so block size and other tracks cannot change it. Video source audio uses the same volume and mute controls. `source_audio_enabled=false` excludes it regardless of picture visibility. Nonfinite input or overflow refuses the mix. Clip fades (`project::Fades`) multiply each item's gain per absolute sample, after volume keyframes and before summing and the master chain. Preview and export are bit-identical for any block partition.

Media resamples each continuous source from its original clock, keeping filter phase across block partitions, splits and trims. Seeking backwards restarts that source cursor. Source-rate decode and AAC priming are media-owned; render never reapplies priming.

## Retime

A retimed item plays source position `n * speed` at output sample `n` of its original processing window. A split keeps that window and advances the output offset, so resampling phase and WSOLA grains do not restart at the cut. Sample rounding and the implicit edge envelope use the retained window; explicit fades keep their project-owned split offsets.

A reversed item reads its window's samples in reverse order before any rate conversion, so reversal never changes pitch; it always goes through the retimer, even at 1x. With `maintain_pitch` off, a streaming resampler reads the position with linear interpolation, area-averaging ceil(speed) taps above 1x; pitch follows speed. With it on, WSOLA keeps pitch: 40 ms squared-Hann grains at a 20 ms output hop, source offsets searched within +/-8 ms, summed with window-weight normalization so item edges keep unity gain and source outside the trimmed window never leaks in.

Output depends only on the item, speed and source, so block partitions and seeks give identical audio, and source reads only move forward. The mixer keeps one bounded, leased retimer per active retimed item; the lease covers maximum-block 5x source read-ahead, pull buffers, overlap-add state and output staging, and a budget that cannot hold it refuses before synthesis. WSOLA checks cancellation between grains, including after a source pull. Cancellation discards the mixer cursor so retry replays deterministically.

Stream copy is refused for retimed items. Video frames use the nearest earlier source frame at the mapped source time. Perceptual quality on speech or music is unmeasured.

## Source cleanup

An audio or video item's `cleanup` chain (`project::AudioCleanup`, at most 8 effects) runs on its source samples after retime and before item volume, fades and automation, so preview and export share it. Source media is never changed. Effects run in list order; each keeps `wet_percent` of its processed signal and the rest of its input. A disabled effect or a zero mix is skipped, and an all-bypassed chain is bit-identical to no chain.

Kinds: hum notches at the mains frequency's harmonics (1 to 10, Q 1 to 100, harmonics at or above 0.45 times the sample rate omitted); Butterworth high-pass and low-pass (20 to 20 000 Hz); low shelf at 200 Hz and high shelf at 4 kHz (-24 to +24 dB); ten octave-spaced peaking bands at 31.5 Hz to 16 kHz (-24 to +24 dB); and up to 8 parametric bands (peaking, low or high shelf, notch; 20 to 20 000 Hz, -30 to +30 dB, Q 0.1 to 30). Coefficients are RBJ cookbook designs in f64 transposed direct form II, flushed below 1e-30.

Every section is second order, zero latency and sample-recursive, so the state after a block depends only on the samples before it. A block that continues the previous block of the same item keeps the state; any other start (a seek, or a gap while the item is muted or outside the timeline) restarts it. The mixer clears the chain state with its other per-item state on replay, so preview, export and any block partition produce identical samples. The limiter's delay is unchanged.

Not yet implemented: spectral noise reduction (needs a latency design, since frame delay must be compensated in A/V timing), click removal, and keyframed cleanup parameters (parameters are static per project snapshot).

## Master chain

`AudioMixer::new(snapshot, channels, fade_ms, hooks)` keeps bounded DSP state for sequential realtime or offline blocks. The linked master chain is a -1 dBFS threshold, 20:1 hard knee, one-pole 1 ms attack / 120 ms release, 5 ms lookahead plus eight samples of FIR detector alignment, and a final hard sample clamp. A Kaiser-windowed polyphase detector estimates intersample peaks; this is not a certified true-peak ceiling. Post-limiter gain is 0.98, applied to below-threshold signals too. Every path applies the chain, including quiet offline exports.

`latency_samples` reports the full delay. It is removed by priming and feeding ahead, so returned PCM stays on the requested sample grid with no leading delay or lost final samples. Gaps and empty content stay bit-exact zero.

Retain one mixer per immutable snapshot. A discontinuous seek replays from timeline zero in bounded, cancellable blocks to rebuild the release envelope, so cost grows with seek position. `mix_audio` is a convenience for independent windows over the same graph. Playback owns one mixer per request epoch, export one per operation. Sources and DSP leases are caller-owned. A failed read invalidates the graph cursor before retry.

## Loudness

Normalization (EBU R128 / BS.1770-4) is a two-pass planner over the same graph. `measure_loudness(project, sources, hooks)` reads the project's clip and master targets.

Pass one mixes each audible clip alone with its volume, mute and automation, without master fader, mute, limiter or other gains, and meters it. Clip gain is `min(target - integrated, ceiling - true peak)`. Pass two meters the whole pre-limiter mix with those clip gains applied, and derives the master gain the same way against `min(true_peak_db, -1 dBFS)`.

The meter is K-weighted at the project rate, 400 ms blocks on a 100 ms hop, absolute gate -70 LUFS, relative gate -10 LU, true peak from the limiter's 4x/2x polyphase detector. Measurement is stereo (mono duplicates at unity), so a mono export of stereo-measured material reads up to 3 dB lower. Silence, clips under 400 ms and gains within 0.05 dB are exact unity. Gains clamp to +/-60 dB. An unreachable target lands below target rather than clipping.

The planner reports progress through `Stage::Mix`, honours cancellation and returns `LoudnessReport` (gains plus measured numbers; the engine owns its key). `project.settings.loudness.measured` holds the gains. `AudioMixer` and `mix_audio` apply the clip gain beside item volume and the master gain to the summed mix ahead of the limiter, whose 0.98 post gain stays in the chain, so final loudness is 0.18 dB under target. Gains apply sample-accurately and do not change timing.

## Export audio

`ExportSettings::audio_cut_fade_ms` optionally applies linear ramps at audible item edges on the absolute sample grid (0 disables, maximum 1000 ms, capped at half item duration). Clip batch exports use 10 ms. All included export audio runs through the graph and encodes AAC at 192 kbit/s for MP4/MOV or Opus for WebM, even when video packets copy. Media owns codec priming and FIFO.
