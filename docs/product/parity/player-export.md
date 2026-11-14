# Player and export

Snapshot of main `d3be7568` on 2026-11-14. Legend, evidence names and update rule: [parity](README.md).

## Player (PLAY)

| ID | Item | Function | UI/UX | Evidence | Gap / note |
|---|---|---|---|---|---|
| PLAY-01 | Player title | Done | Matches | 05/221643 | |
| PLAY-02 | Player menu entry | Partial | Differs | 14/100123 | Header button |
| PLAY-03 | Color oscilloscope | Missing | — | 14/100123 | |
| PLAY-04 | Export still frames | Done | Proposed | 14/100123 | |
| PLAY-05 | Add horizontal / vertical guide | Done | Proposed | 14/100216 | Teal guides |
| PLAY-06 | Drag guide from ruler | Done | Proposed | 14b/111304 | |
| PLAY-07 | Lock / unlock guides | Done | Proposed | 14b/111325 | |
| PLAY-08 | Show / hide guides | Done | Proposed | 14/100219 | |
| PLAY-09 | Clear guides | Done | Proposed | 14/100219 | |
| PLAY-10 | Show / hide rulers | Done | Matches | 14/100133 | |
| PLAY-11 | Guide and ruler geometry | Partial | Matches | 14/100216 | Rulers on viewer edges, guides span the viewer; label steps beyond 700 and 1000 open |
| PLAY-12 | Play / pause | Done | Matches | 14b/110531 | |
| PLAY-13 | Timecode | Done | Matches | 05/221643 | |
| PLAY-14 | Fit button | Done | Matches | 05/221643 | |
| PLAY-15 | Zoom control | Done | Differs | 14d/151402 | Preset list |
| PLAY-16 | Full screen | Done | Proposed | 14b/110536 | |
| PLAY-17 | Ratio button | Partial | Differs | 14b/110316 | Popup shows size only; the 1 px outlined label matches 05/221643 |
| PLAY-18 | Quality button | Partial | Proposed | 05/221643 | Stub popup; the 1 px outlined label matches |
| PLAY-19 | Player zoom keys | Done | — | 14b/110531 | Shifted keys unconfirmed on hardware |
| PLAY-20 | Transform handles, snap | Done | Proposed | 14b/110531 | |
| PLAY-21 | Missing-media canvas | Done | Matches | 14/095554 | |
| PLAY-22 | Loop, master volume | Ours | Proposed | | |
| PLAY-23 | Grid, platform safe areas | Ours | Proposed | | |
| PLAY-24 | Bookmark notes overlay | Ours | Proposed | | |

## Export (EXP)

| ID | Item | Function | UI/UX | Evidence | Gap / note |
|---|---|---|---|---|---|
| EXP-01 | Dialog frame | Done | Differs | 14b/105340 | 720x663; cover beside the form |
| EXP-02 | Cover preview, Edit cover | Missing | — | 14b/105340 | |
| EXP-03 | Timeline select | Done | Proposed | 14b/105340 | |
| EXP-04 | Name field | Done | Proposed | 14b/105340 | |
| EXP-05 | Export-to path | Partial | Differs | 14b/105745 | Picker after render |
| EXP-06 | Video section tick | Done | Proposed | 14b/105736 | |
| EXP-07 | Remove watermark | Out of scope | — | 14b/105340 | Accounts |
| EXP-08 | Resolution list | Partial | Differs | 14b/105418 | Ours adds 360P |
| EXP-09 | Bit rate presets | Done | Proposed | 14b/105425 | 6, 12, 24 Mb/s are ours |
| EXP-10 | Custom bit rate (Kbps) | Done | Proposed | 14b/105630 | |
| EXP-11 | Codec list | Partial | Differs | 14b/105429 | H.264, VP9 only |
| EXP-12 | Format list | Done | Proposed | 14b/105433 | Plus WebM |
| EXP-13 | Frame rate list | Partial | Differs | 14b/105437 | No 29.97, 59.94, 120 |
| EXP-14 | Optical flow | Missing | — | 14b/105340 | |
| EXP-15 | Color space row | Missing | Not captured | 14b/105340 | |
| EXP-16 | Sync to space | Out of scope | — | 14b/105348 | Cloud sync |
| EXP-17 | Audio formats | Partial | Differs | 14b/105528 | No MP3, WAV, FLAC |
| EXP-18 | Audio-only export | Done | Proposed | 14b/105736 | |
| EXP-19 | GIF section | Done | Proposed | 14b/105532 | Ticks beside Video and Audio |
| EXP-20 | GIF resolution list | Done | Proposed | 14b/105549 | |
| EXP-21 | Captions section | Done | Differs | 14b/105557 | Tick joins the export's group; save buttons stay |
| EXP-22 | Caption formats | Done | Proposed | 14b/105559 | Plus VTT, burned MP4 |
| EXP-23 | Check copyright toggle | Missing | — | 14b/105555 | |
| EXP-24 | Duration and size estimate | Done | Matches | 14b/105340 | Sum of the encoder-model estimate per ticked output; Windows writes 0.64x the estimate (holds the rate target) |
| EXP-25 | Cancel, Export buttons | Done | Matches | 14b/105340 | Cancel left of Export |
| EXP-26 | Progress state | Done | Not captured | 14b/105909 | |
| EXP-27 | Finished share view | Out of scope | — | 14b/105909 | Sharing |
| EXP-28 | Open folder after export | Partial | Not captured | 14b/105934 | Toast with path |
| EXP-29 | Export failure panel | Done | Proposed | | |
| EXP-30 | Image sequence, range, scenes | Ours | Proposed | | |
| EXP-31 | Download clip settings | Missing | — | web flows/12 | |
| EXP-32 | Several outputs in one export | Done | Differs | 14b/105340, 14b/105532, 14b/105736 | A job per output, one progress and toast; one picker, the rest saved beside it |
| EXP-33 | Cancel a running export | Done | Not captured | | Stops the running job, starts no later output |
