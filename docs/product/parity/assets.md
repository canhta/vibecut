# Assets panel: media, audio, text, stickers, templates

Snapshot of main `66512fec` on 2026-11-14. Legend, evidence names and update rule: [parity](README.md).

## Media (MEDIA)

### Import and pool
| ID | Item | Function | UI/UX | Evidence | Gap / note |
|---|---|---|---|---|---|
| MEDIA-01 | Import button | Done | Proposed | 110316 | |
| MEDIA-02 | Import menu: From device | Done | Differs | 110322 | Direct picker, no menu |
| MEDIA-03 | Import menu: From library | Out of scope | — | 110322 | Online assets |
| MEDIA-04 | Drag and drop import | Done | Proposed | web 12-editor-media-import.md | |
| MEDIA-05 | Import progress overlay | Done | Proposed | [ui/index.md](../ui/index.md) | |
| MEDIA-06 | Corrupt file toast | Done | Proposed | 110341 | Width unmeasured |
| MEDIA-07 | Empty media state | Done | Proposed | 221513 | |
| MEDIA-08 | Media search box | Done | Proposed | 110316 | |
| MEDIA-09 | Media tile | Done | Proposed | 221635 | |
| MEDIA-10 | Added badge on tile | Missing | Not captured | 221635 | |
| MEDIA-11 | Add to timeline (plus) | Done | Proposed | 221635 | |
| MEDIA-12 | Tile drag to timeline | Done | Proposed | 221635 | |
| MEDIA-13 | Grid/list, sort, filter | Ours | Proposed | | |
| MEDIA-14 | Text button in media | Ours | Proposed | [ui/editor.md](../ui/editor.md) | |
| MEDIA-15 | AI clipper promo | Out of scope | — | 110316 | Generated media |

### Missing media and tile menu
| ID | Item | Function | UI/UX | Evidence | Gap / note |
|---|---|---|---|---|---|
| MEDIA-16 | Media lost tile state | Done | Matches | 095537; [ui/editor.md](../ui/editor.md) | |
| MEDIA-17 | Link media dialog | Done | Matches | 095459 | |
| MEDIA-18 | Tile menu: Delete | Done | Not captured | 103259 | |
| MEDIA-19 | Tile menu: Rename | Missing | Not captured | 103259 | |
| MEDIA-20 | Tile menu: Open file location | Done | Not captured | 103259 | |
| MEDIA-21 | Tile menu: Restore LUT | Partial | Not captured | 103305 | LUT lives in Adjust |
| MEDIA-22 | Tile menu: New folder | Missing | Not captured | 103259 | |
| MEDIA-23 | Tile menu: New timeline | Partial | Not captured | 103259 | Scene sheet instead |
| MEDIA-24 | Tile menu: Link to media | Partial | Not captured | 103259 | Dialog exists, no menu entry |
| MEDIA-25 | Tile menu: Split scenes | Partial | Not captured | 103259 | Clip-level only |
| MEDIA-26 | Tile menu: Export clips | Done | Not captured | 103259 | |
| MEDIA-27 | Tile menu: multi-camera clip | Missing | Not captured | 103259 | |

### Yours
| ID | Item | Function | UI/UX | Evidence | Gap / note |
|---|---|---|---|---|---|
| MEDIA-28 | Yours: Favorites page | Partial | Not captured | 111010 | Per catalogue tab only |
| MEDIA-29 | Favorites empty state, filter | Missing | Not captured | 111010 | |
| MEDIA-30 | Yours: My presets | Missing | Not captured | 111030 | |
| MEDIA-31 | Yours: Brand assets | Partial | Not captured | 111016 | Store done, panel missing |
| MEDIA-32 | Brand assets refresh, settings | Missing | Not captured | 111024 | |
| MEDIA-33 | Stock library | Out of scope | — | 110949 | Online assets |
| MEDIA-34 | Space uploads | Out of scope | — | 110940 | Cloud |
| MEDIA-35 | Download state on tiles | Out of scope | — | 110358 | Online assets |

## Audio (AUDIO)

### Music and sound effects
| ID | Item | Function | UI/UX | Evidence | Gap / note |
|---|---|---|---|---|---|
| AUDIO-01 | Audio sub-nav | Done | Proposed | 221703 | |
| AUDIO-02 | Music search | Done | Matches | 221703 | |
| AUDIO-03 | Music category chips | Partial | Proposed | 221703 | Source tags, not categories |
| AUDIO-04 | Music rows (play, add) | Done | Matches | 221703 | |
| AUDIO-05 | Music preview playback | Done | Proposed | | |
| AUDIO-06 | Sound effects list | Done | Matches | 221703 | |
| AUDIO-07 | Favorites (star) | Done | Proposed | 221703 | Music uses Saved |
| AUDIO-08 | Commercial-use filter | Done | Proposed | web 04-editor-audio.md | |
| AUDIO-09 | Download state on rows | Out of scope | — | 221703 | Online assets |
| AUDIO-10 | Attribution and key sheet | Ours | Proposed | | |

### Voice-over and text to speech
| ID | Item | Function | UI/UX | Evidence | Gap / note |
|---|---|---|---|---|---|
| AUDIO-11 | Record button | Done | Matches | 102535 | |
| AUDIO-12 | Voice-over record panel | Done | Differs | 102535 | Modal dialog, not panel |
| AUDIO-13 | Input device chooser | Done | Differs | 102535 | |
| AUDIO-14 | Volume, echo, mute project | Missing | Differs | 102535 | |
| AUDIO-15 | Enhance voice option | Partial | Not captured | 102535 | In clip properties only |
| AUDIO-16 | Microphone access notice | Partial | Not captured | 102609 | OS prompt only |
| AUDIO-17 | TTS voice grid, chips | Partial | Differs | 103836 | In Text properties |
| AUDIO-18 | TTS language and filter chips | Partial | Differs | 103843, 103849 | |
| AUDIO-19 | Generate speech | Done | Differs | 103921 | Via dub.generate |
| AUDIO-20 | TTS result clip on timeline | Partial | Not captured | 103930 | Lowest-track placement pending |
| AUDIO-21 | Voice changer presets | Missing | Not captured | 104508 | After v1.0 |

## Text library (TEXTLIB)

| ID | Item | Function | UI/UX | Evidence | Gap / note |
|---|---|---|---|---|---|
| TEXTLIB-01 | Text sub-nav | Done | Proposed | 221716 | One Add text row |
| TEXTLIB-02 | Add text button | Done | Proposed | 221716 | |
| TEXTLIB-03 | Text preset grid | Partial | Not captured | 221716 | Presets live in Properties |
| TEXTLIB-04 | Text effects grid | Partial | Not captured | 103727 | |
| TEXTLIB-05 | Search and category chips | Missing | Not captured | 103727 | |
| TEXTLIB-06 | Favorites chip and list | Missing | Not captured | 103727 | |
| TEXTLIB-07 | All/Commercial filter | Missing | Not captured | web 05-editor-text-library.md | |
| TEXTLIB-08 | Text templates shelf | Missing | Not captured | 104218 | |
| TEXTLIB-09 | Download and Pro marks | Out of scope | — | 103727 | Online assets |
| TEXTLIB-10 | Font import | Ours | — | | |

## Stickers (STICK)

| ID | Item | Function | UI/UX | Evidence | Gap / note |
|---|---|---|---|---|---|
| STICK-01 | Sticker search | Done | Proposed | 221721 | |
| STICK-02 | Category chips | Partial | Differs | 221721 | Ours: All, Emoji, Icons, Flags, Shapes |
| STICK-03 | Sticker tile grid | Done | Matches | 221721; [ui/editor.md](../ui/editor.md#stickers-grid) | |
| STICK-04 | Add to timeline (plus) | Done | Proposed | 110403 | |
| STICK-05 | Tile favorite star | Done | Proposed | 110403 | |
| STICK-06 | Download arrow on tile | Out of scope | — | 110358 | Ours: collection download card |
| STICK-07 | Pro mark on tile | Out of scope | — | 110358 | |
| STICK-08 | Result count, See all | Ours | Proposed | | |
| STICK-09 | Recent list | Ours | Proposed | | |
| STICK-10 | Shapes cards | Ours | Proposed | [ui/editor.md](../ui/editor.md#generated-shapes) | |
| STICK-11 | Emoji and icon collections | Ours | Proposed | | |
| STICK-12 | Logos provider | Missing | — | | Blocked |

## Templates (TMPL)

| ID | Item | Function | UI/UX | Evidence | Gap / note |
|---|---|---|---|---|---|
| TMPL-01 | Templates tab (editor) | Missing | Not captured | 221747 | |
| TMPL-02 | Template tile grid | Missing | Not captured | 221747 | |
| TMPL-03 | Tile stats, star | Missing | Not captured | 110435 | |
| TMPL-04 | Use template | Missing | Not captured | 110442 | |
| TMPL-05 | Template download, error tile | Out of scope | — | 110435 | Template stores |
| TMPL-06 | Home Templates page | Missing | Not captured | 221440 | |
| TMPL-07 | Sample project | Missing | — | | Ours |
