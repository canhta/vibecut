# Captions panel


The assets-panel Captions tab owns `subtitles` presentation. It reuses engine's
capability catalog, ASR/checkpoint workflow, translation windows and export jobs;
no adapter is constructed by opening a picker and no model downloads implicitly.
Local Whisper is the initial explicit choice. Gipformer appears only for
Vietnamese. OpenAI and ElevenLabs are explicit remote ASR choices; OpenAI and
Anthropic are translation choices. Settings persist in preferences; approval,
transcripts and the session glossary do not.

SRT/VTT imports retain the file's content identity as an internal SourceAsset and
insert the asset and caption item in one undoable batch. Generate targets one
selected media item, clips source cues to its source range and inserts at its
output start. Translation stages a snapshot and applies one content command only
against the original Session revision. A changed provider/language setting cancels
the pending run; denial, cancellation, stale results and failures apply no partial
project changes. `ai_task` coalesces progress and holds one exact upload request;
its modal remains available when switching tabs. Disclosures show provider/model,
languages, payload size, duration, cost and the provider's own HTTPS policy links.
The Models action on a missing bundle opens the existing model manager.

Cue times seek in output coordinates. Shift extends a range from a cue identity;
after merging away that cue, the surviving selection supplies the anchor.
Cmd/Ctrl toggles.
Text edits retain independent language edit/provenance flags and normalize through
captions. Leaving a field or switching tabs commits drafts. Cue, page, item,
language and timing actions also commit drafts before changing the view or content;
unrelated project edits retain
them, and concurrent changes to the same caption retain a copyable draft until
explicit reload. Focused text never
triggers timeline shortcuts. Split uses the chosen playhead time and whole
graphemes; merge requires adjacent cues with matching languages. One-frame nudges
refuse overlaps or item-boundary crossings atomically. All edits use registered
engine commands and the shared history; no subtitle-specific undo stack exists.
Lists page at 50 cues, imports cap at 16 MiB/10,000 cues, text at 64 KiB per field
and glossary entries at 128. Reading-speed warnings follow LanguageProfile data.
The existing bundled common-Chinese font subset is the egui fallback.

`tests/it/subtitles_ui.rs` uses real widgets plus offline adapters for editing,
undo/redo, all vi/en/zh directions and Hans/Hant targets, bilingual SRT/VTT,
consent denial/cancellation/provider changes, stale output and oversized imports.
`cargo xtask ui-audit captions-empty captions-empty-vi captions-editor
captions-editor-vi` captures the real minimum-size panel in both themes at 1x/2x;
these audit scenes open no native window and make no provider requests.
