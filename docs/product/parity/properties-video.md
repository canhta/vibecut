# Properties: video clip

Snapshot of main `66512fec` on 2026-11-14. Legend, evidence names and update rule: [parity](README.md).

## Video clip (PVID)

### Tab bar
| ID | Item | Function | UI/UX | Evidence | Gap / note |
|---|---|---|---|---|---|
| PVID-01 | Tabs Video, Audio, Speed, Animation, Adjust | Partial | Differs | 221743 | Flat tabs plus Effects |
| PVID-02 | AI stylize tab | Out of scope | — | 221743 | Style generation |
| PVID-03 | Tracking tab (overlay clip) | Missing | Not captured | 102704-102740 | |
| PVID-04 | Basic, Remove BG, Mask, Retouch | Partial | Proposed | 221743 | No Retouch |
| PVID-05 | Retouch sub-tab | Out of scope | — | 221743 | Beauty retouch |

### Basic
| ID | Item | Function | UI/UX | Evidence | Gap / note |
|---|---|---|---|---|---|
| PVID-06 | Scale slider and field | Done | Matches | 221703 | |
| PVID-07 | Uniform scale toggle | Partial | Differs | 221743 | Independent X/Y rows |
| PVID-08 | Position X/Y | Done | Matches | 221743 | |
| PVID-09 | Rotate field and dial | Partial | Proposed | 221743 | Dial unverified |
| PVID-10 | Align icons row | Done | Proposed | 221743 | |
| PVID-11 | Fit mode row | Ours | Proposed | | |
| PVID-12 | Keyframe diamonds | Done | Matches | 103405 | Header cluster unbuilt |
| PVID-13 | Blend, mode, opacity | Done | Proposed | 221743 | |
| PVID-14 | Stabilize | Done | Proposed | 221743 | |
| PVID-15 | Reduce image noise | Done | Proposed | web 14 | |
| PVID-16 | Remove flicker | Done | Proposed | web 14 | |
| PVID-17 | Enhance quality row | Missing | Not captured | 221743 | |
| PVID-18 | Background sub-tab | Done | Proposed | web 14 | |

### Remove BG
| ID | Item | Function | UI/UX | Evidence | Gap / note |
|---|---|---|---|---|---|
| PVID-19 | Auto removal | Partial | Differs | 104613-104648 | Toggle, not checkbox |
| PVID-20 | Auto removal Stroke row | Missing | Not captured | 151551-151628 | |
| PVID-21 | Custom removal Brush, Eraser | Done | Proposed | 104709 | |
| PVID-22 | Smart brush, Smart eraser | Missing | Not captured | 104720-104726 | Model-backed |
| PVID-23 | Custom removal Size 1-100 | Done | Matches | 100332-100347 | |
| PVID-24 | Custom removal Apply | Partial | Differs | 100402-100407 | Ours paints live |
| PVID-25 | Chroma key eyedropper | Done | Proposed | 104920-104926 | |
| PVID-26 | Chroma Intensity, Shadow | Partial | Differs | 104911, 151402 | Default 40 vs 20 |
| PVID-27 | Chroma Feather, Clean up edge | Missing | Not captured | 151342-151410 | |
| PVID-28 | Chroma controls grey until pick | Partial | Differs | 104911 | Ours hides them |
| PVID-29 | AI background option | Out of scope | — | 151402 | Generated media |

### Mask
| ID | Item | Function | UI/UX | Evidence | Gap / note |
|---|---|---|---|---|---|
| PVID-30 | Mask shape list | Done | Proposed | web 18 | Adds Path shape |
| PVID-31 | Feather, size, position, invert | Done | Proposed | | Corner radius unbuilt |
| PVID-32 | Freeform path keyframes | Missing | — | | |

### Adjust
| ID | Item | Function | UI/UX | Evidence | Gap / note |
|---|---|---|---|---|---|
| PVID-33 | Adjust Basic sliders | Done | Matches | web 18 | |
| PVID-34 | HSL, Curves, Color wheel | Done | Proposed | | HSL bands ours |
| PVID-35 | Adjust Mask sub-tab | Missing | Not captured | 221802 | |
| PVID-36 | Apply to all, Save as preset | Done | Proposed | web 18 | |
| PVID-37 | LUT import, Strength | Ours | Proposed | | macOS/Linux acceptance open |
| PVID-38 | AI colour correction | Missing | Not captured | web 18 | Model feature |

### Speed
| ID | Item | Function | UI/UX | Evidence | Gap / note |
|---|---|---|---|---|---|
| PVID-39 | Standard slider 0.1-100x | Done | Differs | 100018-100039; [ui/editor.md](../ui/editor.md#speed-tab-standard-view) | Ticks pending |
| PVID-40 | Duration field | Done | Proposed | 100018 | |
| PVID-41 | Change audio pitch | Partial | Differs | 100036 | Above-5x note pending |
| PVID-42 | Reset button label | Partial | Differs | 221757 | |
| PVID-43 | Curve presets (8) | Done | Differs | 100057 | 4 per row, not 6 |
| PVID-44 | Curve plot, points, Reset, + | Done | Differs | 100104-100108 | Plot below the fold |
| PVID-45 | Smooth slow-mo | Missing | Not captured | 100036 | Optical flow |
| PVID-46 | Reverse switch | Ours | Proposed | | |

### Animation and audio of a video clip
| ID | Item | Function | UI/UX | Evidence | Gap / note |
|---|---|---|---|---|---|
| PVID-47 | Animation In/Out/Combo | Done | Proposed | web 14 | |
| PVID-48 | Animation tiles with preview | Done | Proposed | web 14 | Fewer looks |
| PVID-49 | Animation duration | Done | Proposed | web 14 | |
| PVID-50 | Audio volume, fades | Done | Proposed | 104337-104416 | |
| PVID-51 | Reduce noise, enhance voice | Done | Proposed | 104426-104445 | |
| PVID-52 | Normalize loudness | Done | Proposed | 104430 | |
