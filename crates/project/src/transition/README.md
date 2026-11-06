# Video transitions

## Video transitions

`Timeline.transitions` owns first-class `Transition` entities with project-wide
unique `TransitionId`, track, outgoing/incoming item IDs, kind, output start,
duration and `TransitionAlignment` (start, center, end, custom). The cut-adjacent items must
be adjacent and touching on the same video track and contain video or generated
solid overlays. `TRANSITION_DEFINITIONS` is the admitted catalog: cross dissolve,
dip to black and dip to white in the Dissolve family. Unknown kinds return
`UnsupportedTransition` with a bounded kind name, including during file ingestion.

A transition retains earlier outgoing IDs in `from_pieces` and later incoming
IDs in `to_pieces`, both in timeline order. Each side must form a contiguous chain
of video/overlay items on its track. `from`/`to` remain adjacent at the original
cut. `contributors(at)` resolves the appropriate piece on each side, using the
first/last piece's handles beyond the side extent. A split preserves the output
start, duration and alignment; it never renormalizes transition progress.

`Transition::bounds` intersects the two complete side extents with real source handles:
the incoming source in-point bounds the time before the cut and the outgoing
source's remaining known duration bounds the time after it. Unknown outgoing
source duration permits no extension after the cut. Generated overlays have no
media-handle constraint. `set_duration` clamps positive ticks to those bounds,
following alignment; an odd centered duration puts floor(duration/2) before the
cut. Custom duration changes keep the span's centre, clamped to stay over the cut.
Zero duration/handles and invalid spans refuse. Only one transition may name a cut, and spans on one track cannot overlap. Validation never repairs
untrusted persisted data. Edit-time revalidation belongs to [engine](../../../engine/src/session/edit/README.md#video-transitions).
`Timeline::rendered_items(from, to)` lists the visible items that draw in a range: those
overlapping it plus both clips of every overlapping transition, which render from their
handles outside their own extent. Range consumers (export copy and admission, HDR
detection, control previews) use it instead of filtering item extents.
