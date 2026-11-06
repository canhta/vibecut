# Edit mapping

`EditMapping::new` validates an ordered list of `(SourceRange, output_start)`
pairs; each pair retains source duration at unit speed. Empty mappings, gaps,
repeated source spans, signed ticks and overlapping output spans are supported.
Output endpoint overflow returns a typed time error. Segment order identifies
occurrences and need not match temporal order; the mapping has no persisted schema.

`map_tick` returns every retained occurrence as a `MappedTick` (segment index and
output tick). `map_range` returns independent `MappedRange` intersections with both
source and output coordinates. All bounds are half-open. `inverse_tick` uses the
segment identity to recover the source exactly, even for overlapping output spans;
unknown occurrences and output ticks outside the selected segment have no inverse.

`map_cues` rebuilds a `CueTrack` from an original source track, drops fully cut
cues and clips partial intersections without merging gaps. It stable-sorts output
starts and preserves text, language, edit flags and translation references; it
never shifts overlaps or calls subtitle repair. Source IDs must be unique. Each
output cue receives a deterministic fragment ID encoding the mapping occurrence
and complete original ID. `CueFragmentId::from_id` recovers both; `to_id` encodes
an occurrence index followed by `/` and the unchanged source ID. This remains
unambiguous when the original ID contains slashes or looks like another fragment.

After changing cuts, build a new mapping and map the original source track again;
never map previously clipped output. Normalize stream timestamps to source-asset
time before mapping, applying the audio start offset once at that boundary. This
module neither knows nor reapplies stream offsets. Frame-snapped cuts use the
existing time crate conversions; retiming, media IO and project commands belong
to their respective consumers. Overlapping/negative output cues remain exact here;
interchange export separately applies its documented repair policy below.
