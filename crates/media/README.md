# media

Pure Rust MP4/MOV containers for the editor core: probe, streamed packet reading
and progressive H.264/AAC muxing.

## Purpose and exclusions

Owns ISO Base Media File Format (ISO/IEC 14496-12) reading for MP4 and QuickTime
MOV, the packet view of a source asset, and a progressive muxer that writes one
H.264 (`avc1` + `avcC`) and one AAC (`mp4a` + `esds`) track with an edit list for
encoder priming.

Excludes Matroska/WebM, fragmented MP4 (read or write), transcoding/decoding,
encrypted or compressed movie headers, subtitles and timecode tracks beyond
reporting their presence, and every platform or FFI dependency. FFmpeg is only a
test oracle.

## Public entry points

- `probe(path) -> Result<Probe>`: movie brand, timescale and duration in ticks,
  plus one `StreamInfo` per track with codec fourcc, media timescale, duration in
  ticks, sample and keyframe counts, the VFR flag, video size, audio rate and
  channels, the `avcC`/`esds` configuration where present, and raw edit-list
  entries.
- `PacketReader::open(path)`, `PacketReader::probe()` and
  `next_packet() -> Result<Option<Packet>>`: samples interleaved by decode
  timestamp, each `Packet` carrying `stream`, `pts`, `dts`, `duration` (all
  `time::Tick`), `keyframe` and the stored bytes.
- `Muxer::create(path, MuxOptions)` and `Muxer::new(Write + Seek, MuxOptions)`
  with `add_video(VideoTrackConfig)`, `add_audio(AudioTrackConfig)`,
  `write_packet(track, WritePacket)` and `finish()`.
- `AacConfig` (AudioSpecificConfig) and `AvcConfig` (SPS/PPS) for passing a
  probed stream's configuration to the muxer.
- `MediaError` is the single typed error enum; no public operation panics.

## Supported inputs and limits

- Box scan reads only top-level headers and the `moov` box (at most 1 GiB);
  sample payloads are read on demand. The reader uses positioned reads
  (`pread` on Unix, `seek_read` on Windows): random access without a whole-file
  buffer or a shared cursor.
- Sample tables are capped at 2²⁶ samples or one sample per four file bytes,
  whichever is smaller, and at least one million samples for short files. Entry
  counts are checked against the box length before allocation.
- `PacketReader` allocates at most `MAX_PACKET_BYTES` (64 MiB) per packet and
  errors with `TooLarge` above it. A sample whose offset or size runs past the
  file end is `Truncated`.
- Timestamps convert from each track's media timescale to ticks with nearest
  rounding, which is exact for admitted frame rates and audio sample rates.
  `pts`/`dts` are source-media coordinates; edit lists are reported raw and a
  caller maps them to presentation time.
- The VFR flag is set for a video stream when its sample durations are not all
  equal; audio streams are never reported as VFR.
- Only the first sample description of a track is modeled; tracks that switch
  codecs mid-file would be reported with their first entry.
- The muxer writes a large-size `mdat` (64-bit length), `moov` at the end and
  a single chunk run per contiguous sample run. Durations are 32-bit media units;
  composition offsets must fit `i32`. AAC priming is in media units and must be
  smaller than the track duration. Sample data must be stored as AVCC
  length-prefixed NAL units for H.264 and as raw AAC frames.
- Paths are `Path` values and are never converted to text; Unicode and spaces
  work.

## Tests

`cargo test -p media` runs unit tables for box headers and readers, sample-entry
and configuration round trips, muxer configuration errors, a two-track muxing
round trip, VFR detection and truncation/bit-flip inputs under `catch_unwind`
without panics. A sparse, 2.5 GB MP4 is probed and read through a counting byte
source that asserts the header scan reads less than 1 MiB.

Oracle tests generate fixtures with external ffmpeg into `target/fixtures/media`
and compare against ffprobe:

```
ffmpeg -y -f lavfi -i testsrc2=size=1920x1080:rate=30000/1001:duration=10 \
  -f lavfi -i sine=frequency=440:sample_rate=48000:duration=10 \
  -c:v libx264 -preset ultrafast -crf 30 -pix_fmt yuv420p \
  -c:a aac -b:a 96k -ac 2 -shortest h264_aac_1080p_2997.mp4
ffmpeg -y -f lavfi -i testsrc2=size=640x360:rate=30:duration=3 \
  -vf "setpts='if(lt(N,45),N/30/TB,1.5+(N-45)/15/TB)'" -fps_mode vfr \
  -c:v libx264 -preset ultrafast -crf 30 -pix_fmt yuv420p vfr_30_15_640x360.mp4
```

The tests check stream parameters, frame counts, packet durations, VFR detection,
a full remux of the source, AAC priming in the edit list, and that ffmpeg decodes
the remuxed file without errors. Fixtures skip with a message when ffmpeg or
ffprobe is absent; `VIBECUT_REQUIRE_ORACLES=1` turns that into failure. Tool
version used while developing: ffmpeg/ffprobe 9.0.2.
