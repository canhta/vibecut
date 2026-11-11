# Subtitles

Interface: `Subtitles`, `Options`, `TaskEvents`.

Presents the captions panel: cue editing, translation staging and export requests over
engine's subtitle owners. Requests the panel cannot apply itself are queued for the frame.

The spoken-language select lists Auto plus every language the configured ASR adapters
accept. Auto is the default; a project's explicit choice is stored in
`Options::project_sources`, so another project starts from Auto. Auto runs the local
language detector first and passes the detected tag explicitly to every recognition
request.

Generate declares the [engine transcript working ceiling](../../../../crates/engine/src/transcript/README.md)
in its outer job alongside the existing 32 MiB result reservation. Shared budget pressure
prevents the workflow from starting, and the synchronous pipeline adds no second charge.

A silent clip returns `NoSpeech`; a mean recognized-word confidence below the engine
floor returns `LowConfidence`. Both name the likely wrong language, and an
explicit-language run offers one click to retry the same source with Auto (Auto never
offers it).

`subtitles.split` cuts at the playhead. In a cue field, Enter cuts at the caret's nearest
word boundary, Shift+Enter inserts a line break, and Backspace at the start or Delete at
the end merges with the previous or next cue. Unspaced scripts cut on grapheme
boundaries; separated scripts only between words. With native ASR word timings the
source-language cut time is the end of the word before the caret; otherwise the span
divides at the caret's text fraction, and the playhead split snaps to the nearest native
word edge. Empty or zero-length results are refused. Split and each merge commit one
engine command, one undo step.

Import accepts SRT and VTT. Export offers SRT, VTT, TXT and burned MP4; TXT is one cue
per line with no timings, serialized by captions.

The Style chooser applies Outline or Box to the current caption item, independently of cue
selection, through `captions.apply_style`. Custom and Mixed describe saved cue values rather
than a preset identity. Locked items, active work/transactions and focused or changed text drafts
disable the chooser and direct activation before any draft flush. Changing the item or project
snapshot invalidates an open chooser. Applying a look is one undo step; export reads those saved
values through the shared overlay. Named project styles save the complete uniform authoritative cue style and apply explicit copies; no live selected-ID, overrides or word highlighting is offered. Save requires an eligible source, while bank management works without a caption target. The snapshot-bound name editor confirms on Enter/blur and cancels on Escape or stale project, item, time, tab, lock or readiness. Editor-owned Properties/inline/save/import/modal admission is shared with direct actions; caption draft readiness remains here. Names and limits belong to the [project model](../../../../crates/project/src/model/README.md#saved-caption-values).

Provider setup/refusal and direct Runtime cancellation are borrowed from the provider result
or its explicit engine composition. Missing-model setup retains its existing download action;
opaque provider failures stay failed, without source-chain or Failed-envelope searching.

Acquisition problems are borrowed from raw Transcript or engine Transcript composition for
NoSpeech, LowConfidence, direct cancellation and provider setup/refusal labels. Opaque Runtime
causes are not searched; mixed translation and Session failures keep their original roles.

Completed-output application exposes Kernel Edit failures directly; the blocking waiter
composes missing/stale results through Runtime Edit. Both explicit Session stale shapes
keep the existing stale-result presentation.

Translation failures retain the Transcript owner through staging. The existing explicit
failed-cause presentation also accepts Transcript Runtime Failed, preserving the original
inner cause and setup labels; Transcript Edit Session stale keeps the stale-result category.

Registered subtitle failures are borrowed from raw Subtitles or engine Subtitles composition,
then their explicit Transcript, Providers, Runtime or Edit cause. Existing failure presentation
and stale/setup/cancellation labels keep the original typed causes and scheduling opacity.
