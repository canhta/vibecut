# Captions

Interface: `Subtitles`, `TaskEvents`, `show`, `queue_requests`. The persisted `Options` live in
[preferences](../../../../preferences/README.md) as `SubtitleOptions`.

The Captions tab is part of the assets area: its panel state is `assets::State.captions`,
built on first use, and `show` mounts it in the tab body. Cue editing, translation staging
and export requests run over engine's subtitle owners. Opening a picker constructs no adapter
and downloads no model. Settings persist in preferences; approval, transcripts and the session
glossary do not. Requests the panel cannot apply itself (a seek, opening the model manager) stay
`Request` values that `queue_requests` hands to the frame, which drains them after the tab returns.

Cue times seek in output coordinates. Shift extends a range from a cue identity (after
a merge the surviving selection is the anchor); Cmd/Ctrl toggles. Text edits keep
independent language edit and provenance flags and normalize through captions. Drafts
commit on leaving a field, switching tabs, and on cue, page, item, language and timing
actions; unrelated project edits keep them, and a concurrent change to the same
caption keeps a copyable draft until explicit reload. Focused text never triggers
timeline shortcuts. All edits are registered engine commands in the shared history.

Cue, import, text and glossary limits belong to [captions](../../../../../../../crates/captions/README.md).
Design: [captions workflow](../../../../../../../docs/product/ui/editor.md#captions-workflow).

The spoken-language select lists Auto plus every language the configured ASR adapters
accept. Auto is the default; a project's explicit choice is stored in
`SubtitleOptions::project_sources`, so another project starts from Auto. Auto runs the local
language detector first and passes the detected tag explicitly to every recognition
request.

Generate declares the [engine transcript working ceiling](../../../../../../../crates/engine/src/transcript/README.md)
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

The filler cut list ([fillers.rs](fillers.rs)) is a section after the cue list. Find calls
`timeline.filler_candidates` for the caption item's first cue language and refuses with a
word-timing message when no cue has measured words. Find with AI runs `fillers.classify` on a
worker thread with the configured text provider (`translation` in the subtitle options), because
`submit` requests consent synchronously. The worker's [`TaskEvents`](../../../../services/ai_task.rs)
is the captions panel's own upload sheet, so approval, refusal and cancellation use the same
`approve_upload` path as every other upload; a refusal shows the declined line and sends nothing,
and a provider failure shows its own line. The desktop text opener
([`ai_access`](../../../../services/ai_access.rs)) builds the access with `choose_filler` and
the process credentials; no fallback exists. Remove selected passes exactly the checked candidate
ids to `timeline.remove_fillers` as one command, with `confirm_edited` set only after the inline
confirmation of an edited caption. After a removal the list clears and shows `Removed N`; a fresh
Find reads again. There is no applying state: the removal is synchronous, so nothing is in flight
to show. Its state lives in `Subtitles`, resets when the caption item changes, and never mutates
the project itself. Lexicon and limits belong to the [captions crate](../../../../../../../crates/captions/README.md).

Import accepts SRT and VTT. Export offers SRT, VTT, TXT and burned MP4; TXT is one cue
per line with no timings, serialized by captions.

The Style chooser applies Outline or Box to the current caption item, independently of cue
selection, through `captions.apply_style`. Custom and Mixed describe saved cue values rather
than a preset identity. A saved project style is selected live: the item stores the preset ID and
a sparse field patch, so the chooser shows the preset name (marked edited when a patch exists)
and an Overrides editor exposes the complete font, size, colour, outline, box, position, safe-area
and max-line values with Apply, Reset and Cancel. Locked items, active work/transactions and
focused or changed text drafts disable style actions before any draft flush. Changing the item,
project snapshot, playhead, tab, bank or selection invalidates an open chooser or override draft.
Applying a look is one undo step; export reads the same live style through the shared overlay.
Save captures a selected item's patched full style, or a uniform authoritative cue style, while
bank management works without a caption target. Deleting a selected style refuses until another
look is materialized. The snapshot-bound name editor confirms on Enter/blur and cancels on Escape
or stale project, item, time, tab, lock or readiness. Editor-owned Properties/inline/save/import/
modal admission is shared with direct actions; caption draft readiness remains here. Names and
limits belong to the [project model](../../../../../../../crates/project/src/model/README.md#saved-caption-values).

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

The kit-sized spoken-word toggle uses `subtitles.style.word_highlight` and the real selected-item command, with the same readiness-before-draft-flush boundary. Safe localized warnings come from render’s immutable shared reader and clear on repaired/undone/disabled state. Timing is never generated by showing the panel.

The captured warning state distinguishes the player’s source display from selected export languages. Equal diagnostics are shown once; differing player/export conditions are labelled by context. Preflight resource failures retain their concrete source in the UI cache.
