# Sources, bounds and cancellation

`FileSources` holds only active item cursors and bounded signature metadata. Full content hashes and live probe checks admit required sources. Frame-time checks use file identity, length and mtime, and final publication rehashes sources. This detects observed changes; it is not an OS filesystem snapshot and does not stop an adversary restoring metadata.

Missing or changed sources return asset, item and source-range diagnostics. Strict final output refuses by default. `allow_degraded` permits initially missing or changed assets as black and silence with a receipt. A new source failure during execution still refuses.

`SourceProvider` is the seam for a validated source owner or test fixture and must uphold the requested timing, color and audio contracts. `video_window` may return transparency outside an explicitly requested horizontal sampling range while keeping full image geometry; its default calls `video`. File-backed cover and crop renders request the shared transform's footprint, media converts an aligned region with a reconstruction halo, and ingress writes it at its original source coordinates. Fractional sampling does not move. Half reduction, background blur and runtime dimension changes use full frames. Preview and export share this path.

## Execution

`ExecutionHooks::parallelism` defaults to one and bounds row kernels and native conversion to at most eight workers. `run_tasks` defaults to sequential; engine supplies joined borrowed tasks. Render reserves disjoint destination buffers and task metadata before dispatch, and kernels check cancellation per row. A worker unwind is typed, and every borrowed task finishes before the operation releases buffers or returns an error.

Native conversion reserves an extra 8 MiB per extra worker for each active converter or encoder. Engine reserves each extra row worker's 512 KiB stack. Native admission lowers the worker count before allocating when extra workers do not fit, keeping the one-worker minimum and typed byte refusal. Engine falls back to sequential row tasks when extra stacks do not fit.

`ExecutionHooks` checks cancellation and reserves lifetime-bound bytes before allocation under Decoded, Compositor or Export ownership. Engine's implementation uses its one aggregate budget and cache reclamation; adapters must not charge the same bytes twice. Decoded cursor ceilings include aligned native frames, a 96 MiB packet and probe allowance, and bounded PCM and resampler expansion. Plan expansion caps active visual layers and audible spans at 4,096. Image and packing buffers are leased separately. A frame over budget returns a typed error with the requested bytes before allocation. Sources and renderer release at completion or failure.

Native and OS codec allocations, fixed transfer tables, metadata and allocator overhead are outside the budget. These limits are not an RSS sandbox; measure peak RSS separately.
