# Audio clock and headroom

`audio/` mix, retime, loudness, limiter, oversample.

`mix_audio(project, absolute_start_sample, frames, channels, sources, hooks)` is
the preview/export owner. Project rate defaults to 48 kHz and admits 8–192 kHz;
output channels are mono or stereo, with stereo default. Every source/item tick
boundary rounds once to the absolute output sample grid using Nearest. Silence
fills gaps and half-open inactive intervals. A block admits at most 65,536 samples
per channel and 4,096 audible spans. Stereo stays L/R; mono duplicates at unity;
stereo to mono averages L/R. Other source layouts are explicitly unsupported.

A retimed item (project `retime`) plays source position `n * speed` at output
sample `n` of its original processing window. A split retains that source window
and advances the output offset, so neither resampling phase nor WSOLA grains
restart at the cut. Absolute sample rounding and the implicit edge envelope also
use the retained window; explicit fades keep their project-owned split offsets. With `maintain_pitch` off, a streaming resampler reads that
position with linear interpolation, area-averaging ceil(speed) taps above 1x so
fast playback does not alias; pitch follows speed. With it on, WSOLA time stretch
keeps pitch: 40 ms squared-Hann grains at a 20 ms output hop, source offsets
searched within +/-8 ms for the best continuation of the previous grain, summed with
window-weight normalization so item edges keep unity gain and source outside the
trimmed window never leaks in. Every output sample depends only on the item, the
speed and the source, so block partitions and seeks (which replay from timeline
zero like the limiter) give identical audio, and source reads only move forward.
The mixer keeps one bounded, leased retimer per active retimed item. WSOLA checks
cancellation between grains, including after a source pull; cancellation discards
the mixer cursor so retry replays deterministically. `tests/it/retime_audio.rs`
checks tone purity, chirp local pitch/duration, block boundaries and cancellation/retry.
The retimer lease covers maximum-block 5x source read-ahead (stereo plus mono),
pull buffers, overlap-add state and output staging; a constrained-budget regression
refuses that working set before synthesis.
Perceptual quality on speech or music remains unmeasured. Stream copy is refused for
retimed items, and video frames use the nearest earlier source frame at the
mapped source time.

Media resamples each continuous source from its original clock, retaining filter
phase across arbitrary block partitions, splits and trims. Seeking backwards
restarts that source cursor. Source-rate decode and AAC priming are media-owned;
render never reapplies priming. Accumulation is f32 with per-item dB gain and
held mute evaluation through project's item-local animation evaluator. Each
sample maps from the absolute output sample index to ticks before evaluation;
block size and other tracks ending cannot change its gain. Video source audio
uses the same volume/muted controls; `source_audio_enabled=false` excludes it
regardless of picture visibility. Nonfinite input/overflow refuses the mix.
Clip fades (`project::Fades`) multiply each item's gain per absolute sample, after
volume keyframes and before summing and the master chain; preview and export share
this path, so their samples are bit-identical for any block partition.

`AudioMixer::new(snapshot, channels, fade_ms, hooks)` retains bounded DSP state for
sequential realtime or offline blocks. Its linked master chain uses a -1 dBFS
threshold, 20:1 hard knee, one-pole 1 ms attack / 120 ms release, 5 ms lookahead
plus eight samples of FIR detector alignment, and a final hard sample clamp.
A Kaiser-windowed polyphase detector estimates intersample peaks; this is not a
certified output true-peak ceiling. Post-limiter gain is 0.98. Below-threshold
signals are unchanged by dynamics and still receive that post gain. All paths
apply the chain, including quiet offline exports; no block-dependent bypass.
`latency_samples` reports the full delay, removed by priming and feeding ahead,
so returned PCM stays on the requested output sample grid without leading delay
or lost final samples. Gaps/empty content stay bit-exact zero.

Loudness normalization (EBU R128 / BS.1770-4) is a two-pass planner over this
same graph. `measure_loudness(project, sources, hooks)` reads the project's
clip and master targets. Pass one mixes each audible clip alone with its volume,
mute and automation, without master fader, mute, limiter or other gains, and
meters it; the clip gain is `min(target - integrated, ceiling - true peak)`.
Pass two meters the whole pre-limiter mix with those clip gains applied, so
material is never normalized twice, and derives the master gain the same way
against `min(true_peak_db, -1 dBFS)`, the limiter threshold. The meter is
K-weighted at the project rate (shelf and RLB high-pass from the BS.1770-4
analogue prototype), 400 ms blocks on a 100 ms hop, absolute gate -70 LUFS,
relative gate -10 LU, with true peak from the limiter's 4x/2x polyphase
detector. Measurement is stereo (mono duplicates at unity), so a mono export of
stereo-measured material reads up to 3 dB lower. Silence, clips under 400 ms and
gains within 0.05 dB are exact unity; gains clamp to +/-60 dB, and an
unreachable target (peak-limited) lands below target rather than clipping. The
planner reports progress through `Stage::Mix`, honours cancellation and returns
`LoudnessReport` (gains plus the measured numbers; its key is the engine's).
`project.settings.loudness.measured` holds the gains: `AudioMixer`, `mix_audio`
and therefore preview and export apply the clip gain beside item volume and the
master gain to the summed mix ahead of the limiter, whose fixed 0.98 post gain
stays part of the master chain (final loudness is 0.18 dB under the target).
Gains apply sample-accurately and do not change timing. Tests compare the
meter with ffmpeg's `ebur128` on a tone, pink noise and a gated programme
(`cargo test -p render --test it loudness::`).

Retain one mixer for an immutable snapshot. Discontinuous seeks deterministically
replay from timeline zero in bounded, cancellable blocks to reconstruct the full
release envelope; cost grows with seek position. `mix_audio` is a convenience
for independent windows and uses this same graph. Playback owns one mixer per
request epoch; export owns one per operation. Sources and DSP leases remain
caller-owned; a failed read invalidates the graph cursor before retry.

`ExportSettings::audio_cut_fade_ms` optionally applies linear ramps at audible
item edges on the absolute sample grid (0 disables, maximum 1000 ms, capped at
half item duration). Clip batch exports use 10 ms. All included export audio
runs through the graph and encodes AAC at 192 kbit/s for MP4/MOV or Opus for
WebM, even when unchanged video packets copy. Media owns codec priming/FIFO.
