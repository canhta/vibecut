# Subtitles

Interface: `Subtitles`, `Options`, `TaskEvents`.

Presents the captions panel: cue editing, translation staging and export requests over
engine's subtitle owners. Requests the panel cannot apply itself are queued for the frame.

The spoken-language select lists Auto plus every language the configured ASR adapters
accept. Auto is the default; a project's explicit choice is stored in
`Options::project_sources`, so another project starts from Auto. Auto runs the local
language detector first and passes the detected tag explicitly to every recognition
request.

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
values through the shared overlay. No project preset bank or word highlighting is offered.
