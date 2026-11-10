# Bounds and verification

Individual native FFmpeg allocations are capped at 256 MiB before media I/O. AVIO uses 32 KiB buffers. Probing reads at most 32 MiB.

A first-frame thumbnail reads at most 64 MiB in total (opening plus seek and decode-forward), with a single software decoder worker, a native 2^21-pixel image and scratch ceiling, and small output RGBA planes plus the packed thumbnail. Inputs above that ceiling return a typed size refusal; video assets stay usable with a placeholder. The same bound applies when geometry must be recovered during opening. It is stricter than playback's 2^26-pixel ceiling. Thumbnailing never reads the file for hashing; the import job hashes separately.

Other caps: 256 streams, 1 MiB extradata, 64 MiB packets, 2^26 pixels decoded frame area, 65,536 samples per channel and 32 channels per audio frame, 64 decoded frames per receive batch.

Video encoding requests at most eight codec workers (also capped by available parallelism), independent of conversion workers. Each returned video packet batch admits at most 64 MiB of accounted payload, side data and metadata, including vector capacity slack; packet count is not a codec-delay limit. An oversized batch returns `MediaError::TooLarge`; discard that execution. Final video flush receives until EOF, and repeated successful flushes are empty, per [FFmpeg's send/receive contract](https://github.com/FFmpeg/FFmpeg/blob/master/libavcodec/avcodec.h).

Audio encoding admits up to eight channels and caps its FIFO at 131,072 samples per channel. These checks do not establish safety for every hostile C-library input. The decoder keeps the native 2^26-pixel ceiling instead of tightening it to the probed frame area, because FFmpeg also checks codec scratch buffers against it. Visible input and output dimensions are checked separately. Hardware decode preference does not imply a given codec uses a GPU.

The native build backports upstream PSSH iterative cleanup and linear append fixes under [ADR 0002](../../../../docs/adr/0002-desktop-media-and-time.md).

FFmpeg provisioning, including the direct Cargo environment, is owned by the [development workflow](../../../../docs/engineering/workflows.md#ffmpeg-provisioning).
