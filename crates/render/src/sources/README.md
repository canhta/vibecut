# Sources, bounds and cancellation

`FileSources` holds only active item cursors and bounded signature metadata.
Full content hashes and live probe checks admit required sources; frame-time
checks use file identity/length/mtime and final publication rehashes sources.
This detects observed changes; it is not an OS filesystem snapshot or protection
against a concurrent adversary restoring metadata. Missing/changed sources return
asset/item/source-range diagnostics. Strict final output refuses by default;
`allow_degraded` explicitly permits initially missing/changed assets as
black/silence with a receipt. New source failure during execution still refuses.
`SourceProvider` is the seam for an already validated source owner or test fixture;
it must uphold requested timing, color and audio contracts.
`video_window` may return transparency outside an explicitly requested horizontal
sampling range while retaining full image geometry. Its default calls ordinary
`video`. File-backed cover/crop renders request the shared transform's footprint;
media converts an aligned region with a reconstruction halo, and ingress writes
it into its original source coordinates. Fractional sampling does not move.
Half reduction, background blur and runtime dimension changes use full frames.
Preview and export use this same path; no second compositor or cache is involved.


`ExecutionHooks::parallelism` defaults to one and bounds row kernels and native
conversion to at most eight workers. `run_tasks` defaults to sequential execution;
engine supplies joined borrowed tasks. Render reserves disjoint destination
buffers and small task metadata before dispatch; kernels check cancellation per
row. An engine worker's unwind is typed, and every borrowed task finishes before
the operation releases buffers or returns an error. Native conversion reserves
an additional conservative 8 MiB per extra worker for each active converter or
encoder; engine separately reserves each extra row worker's 512 KiB stack.
Native admission lowers the worker count before allocating when additional
workers do not fit, retaining the original one-worker minimum and typed byte
refusal. Engine uses sequential row tasks when extra stacks do not fit. These
are bounded concurrency/memory tradeoffs, not an OS RSS sandbox.

`ExecutionHooks` checks cancellation and reserves lifetime-bound bytes before
allocation under Decoded, Compositor or Export ownership. Engine's implementation
uses its one aggregate budget and cache reclamation; adapters must not charge the
same bytes twice. Decoded cursor ceilings include aligned native frames and a
conservative 96 MiB packet/probe allowance, plus bounded PCM/resampler expansion.
Plan expansion caps active visual layers and audible spans at 4,096. Image and
packing buffers are separately leased. A frame exceeding the budget returns a
typed error with requested bytes before allocation. Native/OS codec allocations,
fixed transfer tables, metadata and allocator overhead are not an RSS sandbox;
measure peak RSS separately. Sources and renderer release at completion/failure.
