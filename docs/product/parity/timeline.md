# Timeline and keyframes

Snapshot of main `443181f4` on 2026-11-14. Legend, evidence names and update rule: [parity](README.md).

## Timeline (TL)

### Toolbar
| ID | Item | Function | UI/UX | Evidence | Gap / note |
|---|---|---|---|---|---|
| TL-01 | Select / split tool | Done | Proposed | 14b/110530 | Caret strip width open |
| TL-02 | Undo | Done | Matches | 05/221643 | |
| TL-03 | Redo | Done | Matches | 05/221643 | |
| TL-04 | Split at playhead | Done | Matches | web screens/13 | |
| TL-05 | Split, delete left | Done | Proposed | 14b/110530 | |
| TL-06 | Split, delete right | Done | Proposed | 14b/110530 | |
| TL-07 | Delete | Done | Matches | web screens/13 | |
| TL-08 | Reverse | Partial | Differs | web screens/13 | In Speed tab, not toolbar |
| TL-09 | Crop | Done | Proposed | web screens/18 | |
| TL-10 | Flip / mirror | Done | Proposed | web screens/13 | Vertical flip ours |
| TL-11 | Freeze frame | Done | Proposed | web screens/13 | Glyph unmeasured |
| TL-12 | Transcript button | Partial | Differs | web flows/09 | No clip entry |
| TL-13 | Split scene | Done | Proposed | web flows/09; [ui/editor.md](../ui/editor.md#split-scene) | Menu only |
| TL-14 | Create from reference | Out of scope | — | web flows/12 | Generated media |
| TL-15 | Download clip | Done | Proposed | web flows/12; [ui/editor.md](../ui/editor.md#clip-download) | |
| TL-16 | Extract audio | Done | Proposed | 14b/110530 | |
| TL-17 | Duplicate | Done | Proposed | web screens/09 | |
| TL-18 | Voice-over mic | Partial | Differs | 14b/102535 | No echo, mute, enhance |
| TL-19 | Main track magnet | Done | Proposed | 14b/110530 | No default key |
| TL-20 | Auto snapping | Done | Matches | 14b/110530 | |
| TL-21 | Linkage switch | Missing | — | 14b/110530 | No link model |
| TL-22 | Preview axis | Done | Matches | web flows/08 | |
| TL-23 | Zoom out, slider, in | Done | Matches | 05/221643 | |
| TL-24 | Zoom to fit timeline | Missing | — | 14b/110530 | |
| TL-25 | Hide / show timeline | Missing | — | web flows/12 | Web-only evidence |
| TL-26 | Add track button | Ours | Proposed | | |
| TL-27 | Ripple toggle | Ours | Proposed | | |
| TL-28 | Scene switcher | Ours | Proposed | | |
| TL-29 | Narrow-width toolbar scroll | Ours | Proposed | | Wraps at 398 px |
| TL-95 | Playhead follow in playback | Done | Not captured | [ui/editor.md](../ui/editor.md) | Page-style, rule ours |

### Tracks and headers
| ID | Item | Function | UI/UX | Evidence | Gap / note |
|---|---|---|---|---|---|
| TL-30 | Track kinds | Done | Proposed | 14b/102704 | |
| TL-31 | Header cells by kind | Done | Proposed | 14b/102704 | |
| TL-32 | Lock track | Done | Proposed | 14b/102805 | |
| TL-33 | Hide track | Done | Proposed | 14b/102809 | |
| TL-34 | Mute track | Done | Proposed | 14b/102844 | |
| TL-35 | Track settings button | Done | Proposed | 14b/102817, 103113, 103118 | Menu row effects unmeasured |
| TL-36 | Main track Cover button | Missing | — | 14b/102704 | No cover feature |
| TL-37 | Row height by kind | Done | Matches | 14b/102704; [ui/editor.md](../ui/editor.md#track-rows-by-kind) | |
| TL-38 | Clip colour by kind | Done | Matches | 14b/102704; [ui/editor.md](../ui/editor.md#track-rows-by-kind) | Light fills proposed |
| TL-39 | New track from drop | Done | Proposed | 14b/102949 | |
| TL-40 | Track reorder by drag | Ours | Proposed | | |
| TL-41 | Track rename | Ours | Proposed | | |

### Editing operations
| ID | Item | Function | UI/UX | Evidence | Gap / note |
|---|---|---|---|---|---|
| TL-42 | Group / ungroup | Missing | — | 14b/110530 | Needs model design |
| TL-43 | Compound clip | Missing | — | 14b/110011 | |
| TL-44 | Undo compound clip | Missing | — | 14b/110530 | |
| TL-45 | Multi-camera clip | Missing | — | 14b/110011 | Disabled in reference |
| TL-46 | Deactivate clip | Partial | Proposed | 14b/110011 | Hide item, no V key |
| TL-47 | Trim clip | Missing | — | 14/095625 | |
| TL-48 | Replace clip | Done | Proposed | 14/095625 | |
| TL-49 | Copy, cut, paste | Partial | Proposed | 14b/110011 | Copy, Cut not in clip menu |
| TL-50 | Copy / paste attributes | Partial | Proposed | 14b/110011 | No menu rows |
| TL-51 | Save preset (clip) | Missing | — | 14b/103312 | |
| TL-52 | Export selected clips | Partial | Differs | 14b/110011 | Export range only |
| TL-53 | Sync video and audio | Missing | — | 14b/110011 | |
| TL-54 | Extract / restore audio | Done | Proposed | 14b/110011 | |
| TL-55 | Isolate voice | Missing | — | 14b/103348 | |
| TL-56 | Rotate | Partial | Proposed | 14b/110530 | Properties only |
| TL-57 | Select leftward / rightward | Done | Proposed | 14b/110530 | |
| TL-58 | Select by clip, Deselect | Partial | Not captured | 14b/103135 | |
| TL-59 | In / out points | Done | Proposed | 14b/110530 | |
| TL-60 | Last / next cut point | Missing | — | 14b/110530 | |
| TL-61 | Volume ±0.1 dB keys | Missing | — | 14b/110530 | |
| TL-62 | Speed panel keys | Missing | — | 14b/110530 | |
| TL-63 | Show variable speed animation | Missing | — | 14b/103312 | |
| TL-64 | Fade handles, volume line | Ours | Proposed | | |

### Drag feedback
| ID | Item | Function | UI/UX | Evidence | Gap / note |
|---|---|---|---|---|---|
| TL-65 | Grey slot in target row | Done | Matches | 14b/102933 | Light slot colours proposed |
| TL-66 | New-track insertion line | Done | Matches | 14b/102949 | |
| TL-67 | Dragged clip outline | Done | Matches | 14b/102933 | |
| TL-68 | Main track placeholder from 0 | Done | Matches | 14b/103040 | Engine keeps the gap, magnet on or off |
| TL-69 | Snap edge lines | Partial | Proposed | 14b/103040 | |
| TL-70 | Trim past source end | Differs | Differs | 14b/103022 | Ours shows a pill, deliberate |
| TL-71 | Refusal pill | Ours | Proposed | [ui/editor.md](../ui/editor.md#timeline-gestures) | |
| TL-72 | Drop on clip = Replace | Missing | — | web flows/10 | |

### Markers and snapping
| ID | Item | Function | UI/UX | Evidence | Gap / note |
|---|---|---|---|---|---|
| TL-73 | Add marker | Done | Proposed | 14b/110530 | |
| TL-74 | Marker with new colour | Missing | — | 14b/110530 | |
| TL-75 | Next / previous marker | Done | Proposed | 14b/110530 | |
| TL-76 | Marker editor popover | Ours | Proposed | | |
| TL-77 | Snap indicator | Done | Proposed | 14b/103040 | |

### Context menus
| ID | Item | Function | UI/UX | Evidence | Gap / note |
|---|---|---|---|---|---|
| TL-78 | Track menu: Track height | Done | Proposed | 14b/103113 | Row scale unmeasured |
| TL-79 | Track menu: Waveform size | Done | Proposed | 14b/103118 | Bar reach unmeasured |
| TL-80 | Track menu: ours | Ours | Proposed | | Paste, rename, mute, delete |
| TL-81 | Row menu: Close gap | Missing | — | 14b/103135 | |
| TL-82 | Row menu: Close all gaps | Missing | — | 14b/103135 | |
| TL-83 | Row menu: Paste | Missing | — | 14b/103135 | |
| TL-84 | Row menu: Render | Missing | — | 14b/103213 | No render cache |
| TL-85 | Clip menu: chat entry | Missing | — | 14b/110011 | |
| TL-86 | Clip menu: Edit submenu | Missing | Not captured | 14b/110011 | |
| TL-87 | Clip menu: AI generate items | Out of scope | — | 14b/110011 | Generated media |
| TL-88 | Clip menu: Transcript | Missing | — | 14b/110011 | |
| TL-89 | Clip menu: Split scenes | Done | Proposed | 14b/110011 | |
| TL-90 | Text clip menu extras | Missing | — | 14b/103312 | |
| TL-91 | Audio clip: link, open location | Done | Proposed | 14b/103343 | |
| TL-92 | Media tile menu | Partial | Not captured | 14b/103259 | See MEDIA |
| TL-93 | Player right-click | Differs | Not captured | 14b/103250 | Reference has none |
| TL-94 | Missing-media clip | Done | Proposed | 14/095554 | Name sits in a top band and the two lines centre beneath; below 43 px the name yields, below 26 px the lines yield |

## Keyframes (KEY)

| ID | Item | Function | UI/UX | Evidence | Gap / note |
|---|---|---|---|---|---|
| KEY-01 | Diamond per property | Done | Matches | 14b/103405 | |
| KEY-02 | Prev / next key arrows | Done | Matches | 14b/103419 | |
| KEY-03 | Typing a value adds a key | Done | Not captured | 14b/103419 | |
| KEY-04 | First-use key hint | Done | Differs | 14b/103405 | One-line hint on the first diamond's row; the card passes clicks through, only OK takes them |
| KEY-05 | Interpolated value | Done | — | 14b/103429 | |
| KEY-06 | Diamonds on timeline clip | Partial | Proposed | 14b/103419 | |
| KEY-07 | Key menu: Ease in / out / Easing | Done | Proposed | 14b/103437 | |
| KEY-08 | Key menu: Linear, all presets | Done | Proposed | 14b/103437 | |
| KEY-09 | Key menu: Copy, Delete | Done | Proposed | 14b/103437 | |
| KEY-10 | Lane toggle (Alt K) | Done | Differs | 14b/103444 | Alt+K only, no menu row |
| KEY-11 | Lane rows per channel | Done | Proposed | 14b/103444 | |
| KEY-12 | Lane toolbar | Partial | Proposed | 14b/103452 | |
| KEY-13 | Add key shortcuts | Partial | — | 14b/110530 | No Shift+click on a lane |
| KEY-14 | Key paste, multi-select, retime | Ours | Proposed | | |
