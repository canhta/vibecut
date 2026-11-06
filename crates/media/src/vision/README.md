# vision

The media half of local vision analysis: exact-time frame decoding, face and
scene tensor preparation and MFCC audio features. Detector inference, shot
tracking and job orchestration are other owners.

| Stage | Responsibility | Contract |
|---|---|---|
| `frame` | Decodes frames at exact presentation times as packed RGBA8. | [Frames](#frames) |
| `face` | Crops faces and prepares identity and detector tensors. | [Face preparation](#face-preparation) |
| `scene` | Prepares the CLIP scene tensor. | [Scene tensor](#scene-tensor) |
| `mfcc` | Computes bounded MFCC features from source audio. | [MFCC](#mfcc) |
| `limits` | Bounds one preparation request. | [Limits](#limits) |

## Frames

`decode_frame_at` returns the frame whose presentation interval contains an exact
source tick. `decode_shot_frames` streams the frames intersecting a source range
in presentation order, including the frame covering the range start (its PTS may
precede that start). Variable-frame-rate sources are selected by decoded PTS, not
by a nominal rate. Frames are converted to packed straight RGBA8 only when
downloaded; no GPU pointer escapes. Cancellation is checked before every decoded
frame and callback, so interruption latency is at most one frame. The decoded
frame budget covers the keyframe preroll and any replay, not just emitted frames.

`VisionFrame::identity_tensor` prepares SFace input from five source-pixel YuNet
landmarks: a least-squares orientation-preserving similarity fit to the pinned
112x112 template, bilinear sampling with black border, then raw CHW RGB pixels.
The output is fixed at 37,632 floats. Landmarks outside the source, eye distance
below 8 pixels or a degenerate transform return `None`, preserving unavailable
evidence; nonfinite landmarks return a typed error. No learned operator or
descriptor matching belongs here. Tests cover RGB order, translation and hostile
geometry; media's `identity_prepare` example prepares local evaluation tensors
through this same public method.

## Face preparation

`VisionFrame::face_crop` applies the Light-ASD recipe: expand the box by 0.40, pad
pixels outside the decoded frame with gray 110, convert to raw 0-255 grayscale,
resize to 224x224 with linear interpolation and keep the centre 112x112. A crop
that extends past the frame is valid. Crop coordinates, area and geometry are
checked; hostile boxes return typed errors. `FaceCrop` carries the exact source
tick, size and raw pixels.

`VisionFrame::face_tensor` prepares YuNet CHW BGR 640x384 with bilinear resize,
zero bottom/right letterbox and the exact rounded resize dimensions needed to map
boxes back to source pixels.

## Scene tensor

`VisionFrame::scene_tensor` prepares CLIP CHW RGB 224x224: resize the shorter edge
to 224 with cubic interpolation (a=-0.75), centre crop, then normalize with the
pinned CLIP mean/std. Both detector and scene conversions are pure bounded
conversions; no model, scheduler or AI dependency is involved. Extreme aspect
ratios allocate only the output/crop, not an intermediate resized panorama.

## MFCC

`mfcc_range` emits 13-coefficient MFCC frames from mono 16 kHz audio with a
25 ms window and 10 ms hop, 26 mel filters, a 512-point FFT, pre-emphasis 0.97,
cepstral lifter 22 and `c0` equal to log frame energy. The analysis grid is
anchored at the start of the source audio stream, so a subrange is the exact
suffix of a full-range call and pre-emphasis uses the sample before the first
window. Samples are interpreted as 16-bit full-scale PCM. Decoding records exact
sample anchors on the `SourceAsset`, shared by clones and excluded from asset
identity, serialization and schema. The table retains at most 4,096 anchors,
thinning uniformly as it grows. New imports, deserialization and relinks start
cold; callers reuse the same asset for successive ranges. A late request seeks
before the nearest recorded anchor and matches packet byte position, decoded
frame ordinal, PTS and sample count before restoring its sample cursor. PTS is
only a seek hint, never a rounded sample-position estimate. Unknown packet
positions cannot establish a reusable anchor; an unavailable exact match is a
typed refusal. No cache lock is held during I/O, decoding or callbacks.

A cold distant request automatically starts a first-analysis preparation pass.
It uses one reader/decoder in source order, records exact anchors while discarding
samples before the requested preroll, then continues directly into the requested
windows. It never restarts from zero at a checkpoint and retains no source-length
PCM or coefficient buffer. Later ranges reuse the anchors; jobs should submit
ranges in source order and share the same asset (or its clones).

`mfcc_range_with_progress` exposes `MfccProgress::Preparing` checkpoints with the
exact decoded source cursor, followed by `Analyzing` before feature callbacks.
`mfcc_range` runs the identical automatic path without a progress observer.
Preparation is real source-length work: its total cost is linear in the unindexed
prefix, not constant-time for a first late shot. The preparation pass is divided
into bounded work slices so cancellation and job progress remain responsive;
users do not retry limit errors or raise budgets to initialize a valid source.

The requested analysis range, and each individual preparation slice, charge all
decoded audio against `max_audio_decode_duration` (default 660 s, maximum one day),
input reads against `max_audio_bytes` (default 256 MiB, maximum 4 GiB), and packets
including other streams against a one-million-packet ceiling. During preparation,
checkpoints occur before the sample/packet ceiling or after half the read window;
they renew the next slice's budget while preserving demux/decoder state. Input
reads still enforce a hard native byte ceiling, including initial probing. A
single frame or native packet/probe that cannot fit the budget is a typed limit
failure; small or hostile-input refusals are not silently retried. Once analysis
starts, budget exhaustion remains `TooLarge` for that requested range. Callback
errors and cancellation stop immediately; already learned anchors remain reusable.
No source cache lock spans a checkpoint callback or I/O. Frames before resampler
preroll are discarded without conversion; the resampler starts on a stream-global
16 kHz grid phase at least one second before the window. Memory remains bounded
and cancellation is checked throughout. At end of input the final window count
comes from the actual received samples, so unknown-duration audio is padded only
at its real last window, never extended with synthetic windows. Only windows
starting inside the requested range are emitted; a range shorter than one window
still yields the windows that start inside it.

Observed on the 180 s excerpt: all 17,999 MFCC frames were bit-exact in f32
against the upstream speech-features reference; face crops matched the saved
reference tensors with a mean absolute difference of 1.84 gray levels (maximum 9)
in the final 112x112 crop, because the production FFmpeg hardware
decode/color conversion is not OpenCV `VideoCapture`'s decode path. With
identical input pixels the committed synthetic crop fixture matches within two
levels; OpenCV's one-channel 8-bit `INTER_LINEAR` path is not bit-exact
fixed-point interpolation.

## Limits

`VisionPrepLimits` bounds one request to ten minutes, 100,000 decoded video
frames (including seek preroll and replay), 60,001 MFCC frames and a 16 Mi-pixel
face-crop rectangle by default; callers may tighten these.
`max_frame_pixels` caps packed frame area before RGBA conversion (default
64 Mi-pixels; the engine analysis job tightens this to 16 Mi-pixels). The same
packet/frame cancellation and native allocation ceilings as the rest of this
crate apply. Video preparation passes the shared cancellation flag through
opening, packet reads, decoded frame delivery and software recovery, as well as
its callbacks.
