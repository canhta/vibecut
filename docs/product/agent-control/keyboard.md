# Keyboard

`Ctrl` means Cmd on macOS and Ctrl on Windows/Linux; `Alt` means Option on
macOS. Menus and the shortcuts dialog show the platform glyphs. Defaults live in
the app action registry; this table is generated from it and a drift test keeps
both in sync.

<!-- BEGIN GENERATED KEYMAP -->
| Action | Category | Default keys | Engine command | UI owner |
|---|---|---|---|---|
| `media.view` — Toggle media view | View | — | — | media library preferences |
| `media.sort` — Toggle media sort order | View | — | — | media library preferences |
| `media.sort.name` — Sort media by name | View | — | — | media library preferences |
| `media.sort.type` — Sort media by type | View | — | — | media library preferences |
| `media.sort.duration` — Sort media by duration | View | — | — | media library preferences |
| `media.sort.size` — Sort media by size | View | — | — | media library preferences |
| `media.delete` — Request media deletion | Edit | — | — | media removal confirmation |
| `media.cancel_delete` — Cancel media deletion | Edit | — | — | media removal confirmation |
| `media.confirm_delete` — Confirm media deletion | Edit | — | — | media removal confirmation workflow |
| `media.reveal` — Reveal media | File | — | — | media library reveal |
| `text.insert` — Insert text | Edit | — | — | text insertion workflow |
| `subtitles.cue` — Select subtitle cue | Edit | — | — | subtitle cue selection |
| `subtitles.cancel` — Cancel subtitle operation | Edit | — | — | subtitle editor workflow |
| `subtitles.consent.approve` — Approve subtitle upload | Edit | — | — | subtitle editor workflow |
| `subtitles.consent.deny` — Deny subtitle upload | Edit | — | — | subtitle editor workflow |
| `subtitles.generate` — Generate subtitles | Edit | — | — | subtitle editor workflow |
| `subtitles.translate` — Translate subtitles | Edit | — | — | subtitle editor workflow |
| `subtitles.split` — Split subtitle cue | Edit | — | — | subtitle editor workflow |
| `subtitles.merge` — Merge subtitle cues | Edit | — | — | subtitle editor workflow |
| `subtitles.earlier` — Nudge subtitle earlier | Edit | — | — | subtitle editor workflow |
| `subtitles.later` — Nudge subtitle later | Edit | — | — | subtitle editor workflow |
| `subtitles.select_all` — Select all subtitle cues | Edit | — | — | subtitle editor workflow |
| `subtitles.models` — Manage subtitle model | Edit | — | — | subtitle editor workflow |
| `subtitles.bilingual` — Toggle bilingual subtitles | Edit | — | — | subtitle editor workflow |
| `subtitles.force` — Toggle subtitle generation override | Edit | — | — | subtitle editor workflow |
| `subtitles.dismiss` — Dismiss subtitle notice | Edit | — | — | subtitle editor workflow |
| `file.import` — Import media | File | — | — | workspace import picker |
| `file.export` — Export | File | — | — | export dialog |
| `project.save` — Save | File | — | — | workspace lifecycle |
| `project.save_as` — Save as | File | — | — | workspace lifecycle |
| `window.home` — Focus Home | File | — | — | workspace lifecycle |
| `project.rename` — Rename project | File | — | — | project rename input |
| `media.insert` — Add to timeline | File | — | `timeline.insert` | — |
| `import.cancel` — Cancel import | File | — | — | workspace import queue |
| `edit.copy` — Copy | Edit | Ctrl+C | — | clipboard (unavailable; `p1-parity/20`) |
| `edit.paste` — Paste | Edit | Ctrl+V | — | clipboard (unavailable; `p1-parity/20`) |
| `timeline.split` — Split at playhead | Edit | S, Ctrl+B | `split_at` | — |
| `timeline.split_left` — Split and remove left | Edit | Q | — | timeline commands (unavailable; `p1-parity/16`) |
| `timeline.split_right` — Split and remove right | Edit | W | — | timeline commands (unavailable; `p1-parity/16`) |
| `timeline.delete` — Delete selection | Edit | Backspace, Delete | `delete` / `ripple_delete` (ripple enabled) | — |
| `timeline.ripple_delete` — Ripple delete | Edit | Shift+Backspace, Shift+Delete | `ripple_delete` | — |
| `transport.play` — Play/Pause | Playback | Space | — | playback transport |
| `transport.pause` — Pause | Playback | K | — | playback transport |
| `transport.back` — Seek back 1 second | Navigation | J | — | playback transport |
| `transport.forward` — Seek forward 1 second | Navigation | L | — | playback transport |
| `transport.jump_back` — Jump back 1 second | Navigation | Shift+Left | — | playback transport |
| `transport.jump_forward` — Jump forward 1 second | Navigation | Shift+Right | — | playback transport |
| `transport.frame_back` — Step one frame back | Navigation | Left | — | playback transport |
| `transport.frame_forward` — Step one frame forward | Navigation | Right | — | playback transport |
| `transport.start` — Go to start | Navigation | Home, Enter | — | playback transport |
| `transport.end` — Go to end | Navigation | End | — | playback transport |
| `timeline.select_all` — Select all | Selection | Ctrl+A | — | timeline selection |
| `timeline.deselect_all` — Deselect all | Selection | — | — | timeline selection |
| `timeline.duplicate` — Duplicate | Selection | Ctrl+D | — | timeline commands (unavailable; `p1-parity/16`) |
| `selection.escape` — Cancel interaction or deselect | Selection | Escape | — | timeline selection |
| `edit.undo` — Undo | History | Ctrl+Z | — | app and engine history |
| `edit.redo` — Redo | History | Ctrl+Shift+Z, Ctrl+Y | — | app and engine history |
| `timeline.snap` — Toggle snapping | Timeline | N | — | timeline view |
| `timeline.ripple` — Toggle ripple editing | Timeline | — | — | timeline view |
| `timeline.zoom_in` — Zoom in | Timeline | — | — | timeline view |
| `timeline.zoom_out` — Zoom out | Timeline | — | — | timeline view |
| `timeline.add_track` — Add track | Timeline | — | `timeline.add_track` | — |
| `scene.open` — Open scenes | Timeline | — | — | scene switcher |
| `scene.close` — Close scenes | Timeline | — | — | scene switcher |
| `scene.create` — Create scene | Timeline | — | `scene.create` | — |
| `scene.select` — Select scenes | Timeline | — | — | scene switcher |
| `scene.delete` — Delete scenes | Timeline | — | — | scene delete confirmation |
| `scene.cancel_delete` — Cancel scene delete | Timeline | — | — | scene switcher |
| `scene.confirm_delete` — Confirm scene delete | Timeline | — | `scene.delete` | — |
| `appearance.theme` — Toggle theme | View | — | — | preference store |
| `appearance.locale` — Toggle language | View | — | — | preference store |
| `layout.reset` — Reset layout | View | — | — | preference store |
| `inspector.project` — Project properties | View | — | — | selection inspector |
| `inspector.details` — Element details | View | — | — | selection inspector |
| `preview.retry` — Retry preview | View | — | — | playback transport |
| `models.open` — Manage models | View | — | — | model manager |
| `export.run` — Run export | View | — | — | export dialog |
| `export.retry` — Retry export | View | — | — | export dialog |
| `export.mark_in` — Set in point | View | I | — | export range |
| `export.mark_out` — Set out point | View | O | — | export range |
| `export.in_out` — Export in/out range | View | Shift+X | — | export range |
| `scene.rename` — Rename scene | View | — | — | scene rename input |
| `scene.row` — Choose scene | View | — | — | scene switcher |
| `timeline.item` — Select item | View | — | — | timeline selection |
| `preview.snapshot` — Open snapshot dialog | View | — | — | preview snapshot dialog |
| `preview.fullscreen` — Toggle preview fullscreen | View | — | — | preview viewport |
| `preview.zoom.fit` — Fit preview | View | — | — | preview view |
| `preview.zoom.in` — Zoom preview in | View | — | — | preview view |
| `preview.zoom.out` — Zoom preview out | View | — | — | preview view |
| `preview.zoom` — Set preview zoom percentage | View | — | — | preview view |
| `shortcuts.open` — Keyboard shortcuts | View | — | — | shortcuts dialog |
<!-- END GENERATED KEYMAP -->

Every project mutation uses its engine command id (`vibecut-cli commands
--json` lists those schemas). UI-only actions stay app-owned, and actions marked
unavailable are registered for their owning ticket but do not run yet.

Bindings persist in the one versioned app preference store. A recorded chord is
refused when another action already uses it, and resetting an action removes its
`isCustomized` marker. Imported binding profiles with an unsupported format or
version are refused with a typed diagnostic.
