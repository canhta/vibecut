# Keyboard

`Ctrl` means Cmd on macOS and Ctrl on Windows/Linux; `Alt` means Option on macOS. Menus and the shortcuts dialog show platform glyphs. Defaults live in the app action registry; the table below is generated from it.

<!-- BEGIN GENERATED KEYMAP -->
| Action | Category | Default keys | Engine command | UI owner |
|---|---|---|---|---|
| `media.view` — Switch media view | View | — | — | media library preferences |
| `media.sort` — Change media sort order | View | — | — | media library preferences |
| `media.sort.name` — Sort by name | View | — | — | media library preferences |
| `media.sort.type` — Sort by type | View | — | — | media library preferences |
| `media.sort.duration` — Sort by duration | View | — | — | media library preferences |
| `media.sort.size` — Sort by size | View | — | — | media library preferences |
| `media.delete` — Delete media | Edit | — | — | media removal confirmation |
| `media.cancel_delete` — Keep media | Edit | — | — | media removal confirmation |
| `media.confirm_delete` — Delete media | Edit | — | — | media removal confirmation workflow |
| `media.reveal` — Show in folder | File | — | — | media library reveal |
| `graphic.insert.rectangle` — Rectangle | Edit | — | — | graphic insertion workflow |
| `graphic.insert.ellipse` — Ellipse | Edit | — | — | graphic insertion workflow |
| `graphic.insert.polygon` — Polygon | Edit | — | — | graphic insertion workflow |
| `graphic.insert.star` — Star | Edit | — | — | graphic insertion workflow |
| `text.insert` — Add text | Edit | — | — | text insertion workflow |
| `subtitles.cue` — Choose a caption | Edit | — | — | subtitle cue selection |
| `subtitles.cancel` — Cancel caption job | Edit | — | — | subtitle editor workflow |
| `subtitles.style.word_highlight` — Highlight spoken words | Edit | — | — | subtitle editor workflow |
| `subtitles.style.outline` — Use outlined captions | Edit | — | — | subtitle editor workflow |
| `subtitles.style.box` — Use boxed captions | Edit | — | — | subtitle editor workflow |
| `subtitles.style.save` — Save caption style | Edit | — | — | subtitle editor workflow |
| `subtitles.style.manage` — Manage caption styles | Edit | — | — | subtitle editor workflow |
| `subtitles.style.overrides` — Overrides | Edit | — | — | subtitle editor workflow |
| `subtitles.consent.approve` — Allow caption upload | Edit | — | — | subtitle editor workflow |
| `subtitles.consent.deny` — Keep captions on this computer | Edit | — | — | subtitle editor workflow |
| `subtitles.generate` — Create captions | Edit | — | — | subtitle editor workflow |
| `subtitles.translate` — Translate captions | Edit | — | — | subtitle editor workflow |
| `subtitles.split` — Split caption | Edit | — | — | subtitle editor workflow |
| `subtitles.merge` — Merge captions | Edit | — | — | subtitle editor workflow |
| `subtitles.earlier` — Move caption earlier | Edit | — | — | subtitle editor workflow |
| `subtitles.later` — Move caption later | Edit | — | — | subtitle editor workflow |
| `subtitles.select_all` — Select all captions | Edit | — | — | subtitle editor workflow |
| `subtitles.models` — Manage caption data | Edit | — | — | subtitle editor workflow |
| `subtitles.bilingual` — Toggle bilingual captions | Edit | — | — | subtitle editor workflow |
| `subtitles.force` — Regenerate captions | Edit | — | — | subtitle editor workflow |
| `subtitles.dismiss` — Dismiss caption notice | Edit | — | — | subtitle editor workflow |
| `media.insert` — Add to timeline | File | — | `timeline.insert` | — |
| `edit.copy` — Copy | Edit | Ctrl+C | — | clipboard |
| `edit.cut` — Cut | Edit | Ctrl+X | — | clipboard |
| `edit.paste` — Paste | Edit | Ctrl+V | — | clipboard |
| `edit.copy_attributes` — Copy attributes | Edit | Ctrl+Shift+C | — | clipboard |
| `edit.paste_attributes` — Paste attributes | Edit | Ctrl+Shift+V | `timeline.paste_attributes` | — |
| `timeline.split` — Split | Edit | Ctrl+B | `split_at` | — |
| `timeline.split_left` — Split and remove left | Edit | Q | `timeline.split_left` | — |
| `timeline.split_right` — Split and remove right | Edit | W | `timeline.split_right` | — |
| `timeline.split_all` — Split all | Edit | Ctrl+Shift+B | `timeline.split_all` | — |
| `timeline.delete` — Delete | Edit | Backspace, Delete | `delete` / `ripple_delete` (ripple enabled) | — |
| `timeline.freeze` — Freeze frame | Timeline | — | `timeline.freeze_frame` | — |
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
| `playback.jump_bookmark.next` — Next bookmark | Playback | — | — | playback transport |
| `playback.jump_bookmark.previous` — Previous bookmark | Playback | — | — | playback transport |
| `playback.loop` — Loop playback | Playback | — | — | playback transport |
| `timeline.select_all` — Select all | Selection | Ctrl+A | — | timeline selection |
| `timeline.deselect_all` — Deselect all | Selection | — | — | timeline selection |
| `timeline.duplicate` — Duplicate | Selection | Ctrl+D | `timeline.duplicate` | — |
| `timeline.set_hidden` — Hide or show selection | Edit | — | `timeline.set_hidden` | — |
| `timeline.set_muted` — Mute or unmute selection | Edit | — | `timeline.set_muted` | — |
| `timeline.extract_audio` — Extract audio | Edit | — | `timeline.extract_audio` | — |
| `timeline.recover_audio` — Recover audio | Edit | — | `timeline.recover_audio` | — |
| `selection.escape` — Cancel or deselect | Selection | Escape | — | timeline selection |
| `edit.undo` — Undo | History | Ctrl+Z | — | app and engine history |
| `edit.redo` — Redo | History | Ctrl+Shift+Z, Ctrl+Y | — | app and engine history |
| `timeline.snap` — Toggle snapping | Timeline | N | — | timeline view |
| `timeline.ripple` — Toggle ripple editing | Timeline | — | — | timeline view |
| `timeline.preview_axis` — Preview axis | Timeline | S | — | timeline view |
| `timeline.tool.select` — Select | Timeline | A | — | timeline tool |
| `timeline.tool.split` — Split mode | Timeline | B | — | timeline tool |
| `timeline.select_left` — Select leftward | Selection | [ | — | timeline selection |
| `timeline.select_right` — Select rightward | Selection | ] | — | timeline selection |
| `timeline.zoom_in` — Zoom in | Timeline | — | — | timeline view |
| `timeline.zoom_out` — Zoom out | Timeline | — | — | timeline view |
| `timeline.add_track` — Add track | Timeline | — | `timeline.add_track` | — |
| `timeline.bookmark` — Toggle bookmark | Timeline | M | `bookmark.toggle` | — |
| `timeline.crop` — Crop | Edit | — | — | crop dialog |
| `timeline.flip_horizontal` — Flip horizontal | Edit | — | `timeline.set_flipped` | — |
| `timeline.flip_vertical` — Flip vertical | Edit | — | `timeline.set_flipped` | — |
| `timeline.align_left` — Align left | Edit | — | `timeline.align_items` | — |
| `timeline.align_horizontal_center` — Align horizontal centre | Edit | — | `timeline.align_items` | — |
| `timeline.align_right` — Align right | Edit | — | `timeline.align_items` | — |
| `timeline.align_top` — Align top | Edit | — | `timeline.align_items` | — |
| `timeline.align_vertical_center` — Align vertical centre | Edit | — | `timeline.align_items` | — |
| `timeline.align_bottom` — Align bottom | Edit | — | `timeline.align_items` | — |
| `timeline.distribute_horizontal` — Distribute horizontally | Edit | — | `timeline.distribute_items` | — |
| `timeline.distribute_vertical` — Distribute vertically | Edit | — | `timeline.distribute_items` | — |
| `scene.open` — Open scenes | Timeline | — | — | scene switcher |
| `scene.close` — Close scenes | Timeline | — | — | scene switcher |
| `scene.create` — Create scene | Timeline | — | `scene.create` | — |
| `scene.select` — Select scenes | Timeline | — | — | scene switcher |
| `scene.delete` — Delete scenes | Timeline | — | — | scene delete confirmation |
| `scene.cancel_delete` — Cancel scene delete | Timeline | — | — | scene switcher |
| `scene.confirm_delete` — Confirm scene delete | Timeline | — | `scene.delete` | — |
| `inspector.project` — Project properties | View | — | — | selection inspector |
| `inspector.details` — Element details | View | — | — | selection inspector |
| `preview.retry` — Retry preview | View | — | — | playback transport |
| `preferences.open` — Settings | View | — | — | preferences dialog |
| `models.open` — Manage downloads | View | — | — | model manager |
| `voiceover.open` — Record voice-over | View | — | — | voice-over dialog |
| `timeline.split_scene` — Split scene | Edit | — | — | scene split dialog |
| `export.run` — Export | View | — | — | export dialog |
| `export.retry` — Retry export | View | — | — | export dialog |
| `export.mark_in` — Set in point | View | I | — | export range |
| `export.mark_out` — Set out point | View | O | — | export range |
| `export.in_out` — Export in/out range | View | Shift+X | — | export range |
| `scene.rename` — Rename scene | View | — | — | scene rename input |
| `scene.row` — Choose scene | View | — | — | scene switcher |
| `timeline.item` — Select item | View | — | — | timeline selection |
| `preview.snapshot` — Save a frame | View | — | — | preview snapshot dialog |
| `preview.copy_snapshot` — Copy frame | View | — | — | preview snapshot |
| `preview.fullscreen` — Toggle preview fullscreen | View | — | — | preview viewport |
| `preview.zoom.fit` — Fit preview | View | — | — | preview view |
| `preview.zoom.in` — Zoom preview in | View | — | — | preview view |
| `preview.zoom.out` — Zoom preview out | View | — | — | preview view |
| `preview.zoom` — Set preview zoom | View | — | — | preview view |
| `file.import` — Import | File | — | — | workspace import picker |
| `file.export` — Export | File | — | — | export dialog |
| `project.save` — Save | File | — | — | workspace lifecycle |
| `project.save_as` — Save as | File | — | — | workspace lifecycle |
| `window.home` — Go to Home | File | — | — | workspace lifecycle |
| `project.rename` — Rename project | File | — | — | project rename input |
| `project.delete` — Delete project | File | — | — | project delete confirmation |
| `project.cancel_delete` — Cancel project delete | File | — | — | project delete confirmation |
| `project.confirm_delete` — Move project to Trash | File | — | — | workspace lifecycle |
| `project.exit` — Exit project | File | — | — | project lifecycle |
| `import.cancel` — Cancel import | File | — | — | workspace import queue |
| `appearance.theme` — Toggle theme | View | — | — | preference store |
| `appearance.locale` — Toggle language | View | — | — | preference store |
| `layout.reset` — Reset layout | View | — | — | preference store |
| `shortcuts.open` — Keyboard shortcuts | View | — | — | shortcuts dialog |
| `app.help` — Help | View | — | — | help |
| `app.feedback` — Feedback | View | — | — | help |
| `playback.seek` — Seek to time | Playback | — | — | playback transport |
| `preview.set_zoom` — Set preview zoom | View | — | — | preview view |
| `ui.prefs.set` — Change a setting | View | — | — | preference store |
| `catalog.favorite.set` — Star or unstar a catalogue item | View | — | — | catalog favourites |
| `catalog.favorite.list` — List starred catalogue items | View | — | — | catalog favourites |
| `ui.keybindings.set` — Change a shortcut | View | — | — | shortcut dialog |
| `ui.keybindings.reset` — Restore shortcuts | View | — | — | shortcut dialog |
| `ui.keybindings.import` — Import shortcuts | View | — | — | shortcut dialog |
| `ui.keybindings.export` — Export shortcuts | View | — | — | shortcut dialog |
<!-- END GENERATED KEYMAP -->

Every project mutation uses its engine command id (`vibecut-cli commands --json` lists the schemas). UI-only actions stay app-owned.

With the main track magnet on (the default), Delete and Split and remove left or right (`Q`, `W`) on a main-track clip close the gap they leave on the main track in the same undo step; a linked clip on another track closes its own gap, and other tracks do not move. With the magnet off, and no ripple enabled, the gap stays.

Bindings persist in the app preference store. In the shortcuts dialog, a recorded chord that another action already owns opens a conflict prompt naming the owner: Cancel keeps the edit box recording, and Override moves the chord to the recorded action so the old owner loses it. Escape inside the edit box is recorded as a key, so it can conflict like any other chord. The dialog edits a working copy: Save stays disabled until it differs from the stored map, and Save commits it as one preference edit; Cancel, Escape and closing discard it. Reset (per action or all) has no confirmation and acts on the working copy. The `ui.keybindings.set` command refuses a conflicting chord; it does not prompt. Resetting an action removes its `isCustomized` marker. A binding profile with an unsupported format or version is refused with a typed diagnostic.
