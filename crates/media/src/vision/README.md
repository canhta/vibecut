# vision

The media half of local vision analysis: exact-time frame decoding, face and scene tensor preparation and MFCC audio features. Detector inference, shot tracking and job orchestration belong to other owners.

## Frames

`decode_frame_at` returns the frame whose presentation interval contains an exact source tick. `decode_shot_frames` streams the frames intersecting a source range in presentation order, including the frame covering the range start (its PTS may precede it). VFR sources are selected by decoded PTS, not nominal rate. Frames convert to packed straight RGBA8 only when downloaded; no GPU pointer escapes. Cancellation is checked before every decoded frame and callback, so interruption latency is at most one frame. The decoded-frame budget covers keyframe preroll and replay, not just emitted frames.

`VisionFrame::identity_tensor` prepares SFace input from five source-pixel YuNet landmarks: a least-squares orientation-preserving similarity fit to the pinned 112x112 template, bilinear sampling with black border, then raw CHW RGB pixels. The output is 37,632 floats. Landmarks outside the source, eye distance below 8 pixels or a degenerate transform return `None`; nonfinite landmarks return a typed error. No learned operator or descriptor matching belongs here.

## Face preparation

`VisionFrame::face_crop` applies the Light-ASD recipe: expand the box by 0.40, pad pixels outside the decoded frame with gray 110, convert to raw 0-255 grayscale, resize to 224x224 with linear interpolation and keep the centre 112x112. A crop extending past the frame is valid. Hostile boxes return typed errors. `FaceCrop` carries the exact source tick, size and raw pixels.

`VisionFrame::face_tensor` prepares YuNet CHW BGR 640x384 with bilinear resize, zero bottom/right letterbox and the exact rounded resize dimensions needed to map boxes back to source pixels.

## Scene tensor

`VisionFrame::scene_tensor` prepares CLIP CHW RGB 224x224: resize the shorter edge to 224 with cubic interpolation (a=-0.75), centre crop, then normalize with the pinned CLIP mean/std. Extreme aspect ratios allocate only the output and crop, not an intermediate resized panorama.

## MFCC

`mfcc_range_with_progress` emits 13-coefficient MFCC frames from mono 16 kHz audio with a 25 ms window and 10 ms hop, 26 mel filters, a 512-point FFT, pre-emphasis 0.97, cepstral lifter 22 and `c0` equal to log frame energy. Samples are 16-bit full-scale PCM. The grid is anchored at the start of the source audio stream, so a subrange is the exact suffix of a full-range call, and pre-emphasis uses the sample before the first window.

Decoding records exact sample anchors on the `SourceAsset` (see [sample anchors](../source/README.md#sample-anchors)). The table keeps at most 4,096 anchors, thinning uniformly as it grows. Callers reuse the same asset for successive ranges. A late request seeks before the nearest recorded anchor and matches packet byte position, decoded frame ordinal, PTS and sample count before restoring its sample cursor. PTS is only a seek hint. Unknown packet positions cannot establish a reusable anchor; an unavailable exact match is a typed refusal. No cache lock is held during I/O, decoding or callbacks.

A cold distant request automatically runs a preparation pass with one reader and decoder in source order. It records anchors while discarding samples before the requested preroll, then continues into the requested windows. It never restarts from zero at a checkpoint and holds no source-length PCM or coefficient buffer. Later ranges reuse the anchors, so jobs should submit ranges in source order on the same asset. Preparation cost is linear in the unindexed prefix. It runs in bounded slices so cancellation and progress stay responsive.

`mfcc_range_with_progress` reports `MfccProgress::Preparing` checkpoints with the exact decoded source cursor, then `Analyzing` before feature callbacks.

The requested analysis range, and each preparation slice, charge decoded audio against `max_audio_decode_duration` (default 660 s, maximum one day), input reads against `max_audio_bytes` (default 256 MiB, maximum 4 GiB) and packets, including other streams, against a one-million-packet ceiling. During preparation, checkpoints occur before the sample or packet ceiling or after half the read window, and renew the next slice's budget while keeping demux and decoder state. Input reads keep a hard native byte ceiling, including initial probing. A single frame, packet or probe that cannot fit the budget is a typed limit failure and is not retried. Once analysis starts, budget exhaustion is `TooLarge` for that range. Callback errors and cancellation stop immediately; learned anchors stay reusable.

Frames before resampler preroll are discarded without conversion; the resampler starts on a stream-global 16 kHz grid phase at least one second before the window. At end of input the final window count comes from the samples actually received, so unknown-duration audio is padded only at its real last window. Only windows starting inside the requested range are emitted; a range shorter than one window still yields the windows that start inside it.

Observed on the 180 s excerpt: all 17,999 MFCC frames bit-exact in f32 against the upstream speech-features reference. Face crops matched the saved reference tensors with a mean absolute difference of 1.84 gray levels (maximum 9) in the final 112x112 crop, because FFmpeg hardware decode and color conversion is not OpenCV `VideoCapture`'s path. With identical input pixels the synthetic crop fixture matches within two levels, since OpenCV's one-channel 8-bit `INTER_LINEAR` is not bit-exact fixed-point interpolation.

## Limits

`VisionPrepLimits` bounds one request by default to ten minutes, 100,000 decoded video frames (including seek preroll and replay), 60,001 MFCC frames and a 16 Mi-pixel face-crop rectangle; callers may tighten these. `max_frame_pixels` caps packed frame area before RGBA conversion (default 64 Mi-pixels; the engine analysis job tightens it to 16 Mi-pixels). The crate's packet and frame cancellation and native allocation ceilings apply. Video preparation passes the shared cancellation flag through opening, packet reads, frame delivery, software recovery and callbacks.
