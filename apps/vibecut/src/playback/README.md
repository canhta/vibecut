# Playback and limits


Two engine Preview jobs own decode-ahead and audio mixing. They retain no second
scheduler. Audio's fixed atomic stereo ring is engine-budgeted; the callback only
copies samples, zero-fills underruns, advances monotonic device sample counters and
reports callback-to-playback latency. Video follows the latency-corrected audio
clock and skips past frames under load. An absent, failed or one-second-stalled
audio device refuses/pauses playback with an unavailable state.

Device negotiation requires stereo f32 at the project's render-owned sample rate
(default 48 kHz); unsupported devices report unavailable. The app never resamples,
remaps channels or substitutes a wall clock. Gaps/underruns play silence and retain
the device timeline; they are counted. Pausing and seeking invalidate old ring
samples by generation. Shutdown cancels and joins the engine workers.

Preview fits within 640×360, preserving aspect ratio. At most four queued frames,
one shown frame and the upload's retained frame survive at once. Render leases its
working buffers; app leases cover its fixed ring, CPU ColorImage/upload/texture
retention. All leases share the one injected engine budget. The native default is
768 MiB, including render's conservative concurrent probe/decode allowances; this
is an admission ceiling, not resident usage. Pressure or a frame larger than the
available budget returns a typed error before allocation and shows unavailable.
Healthy cache clear preserves position/play state and invalidates queued output.
A terminated preview job pauses transport at the current device-clock position,
fences old output and cancels its companion. Retry keeps playback paused while
engine polling joins finished jobs, then resubmits both jobs with the same source
factory, audio port and aggregate budget. It never blocks the UI waiting for an
adapter; a noncooperating adapter delays replacement and normal shutdown until it
returns. Playback can resume explicitly after the jobs are available. Old work
cannot publish into the new generation, and partial resubmission failure cancels
any admitted replacement. OS codec/driver allocations and
allocator overhead require separate RSS measurement.
