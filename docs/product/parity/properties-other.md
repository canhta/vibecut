# Properties: audio, text, caption and other items

Snapshot of main `443181f4` on 2026-11-14. Legend, evidence names and update rule: [parity](README.md).

## Audio clip (PAUD)

| ID | Item | Function | UI/UX | Evidence | Gap / note |
|---|---|---|---|---|---|
| PAUD-01 | Audio clip tabs | Partial | Differs | 104337 | No Voice changer tab |
| PAUD-02 | Volume -∞ to 20 dB | Done | Matches | 104346-104405; [ui/editor.md](../ui/editor.md#audio-tab) | |
| PAUD-03 | Volume keyframe | Done | Matches | 104337 | |
| PAUD-04 | Fade in, out 0-10 s | Done | Matches | 104411-104416 | |
| PAUD-05 | Normalize loudness | Done | Proposed | 104430 | Ours adds scopes, target |
| PAUD-06 | Enhance voice, Intensity | Done | Proposed | 104441-104445 | |
| PAUD-07 | Reduce noise | Done | Proposed | 104426 | |
| PAUD-08 | Audio translator | Missing | Not captured | 104337 | |
| PAUD-09 | Isolate voice | Missing | Not captured | 103348 | |
| PAUD-10 | Fill channel | Missing | Not captured | 104451 | |
| PAUD-11 | Ducking | Ours | Proposed | | Narration takes only |
| PAUD-12 | Voice filters grid | Missing | Not captured | 104508 | After v1.0 |
| PAUD-13 | Voice characters grid | Missing | Not captured | 104516 | After v1.0 |
| PAUD-14 | Speech to song | Missing | Not captured | 104520 | |
| PAUD-15 | Voice changer Apply to all | Missing | Not captured | 104508 | |
| PAUD-16 | Speed slider, Duration | Done | Proposed | 104527-104547 | Tick mark ink proposed |
| PAUD-17 | Change audio pitch | Done | Proposed | 104527 | |

## Text item (PTEXT)

### Tabs
| ID | Item | Function | UI/UX | Evidence | Gap / note |
|---|---|---|---|---|---|
| PTEXT-01 | Top tabs | Partial | Differs | 103613 | No Animation, Tracking tabs |
| PTEXT-02 | Sub-tabs Basic, Bubble, Effects | Missing | Not captured | 103723-103727 | |
| PTEXT-03 | AI avatars tab | Out of scope | — | 103755 | AI avatars |

### Basic
| ID | Item | Function | UI/UX | Evidence | Gap / note |
|---|---|---|---|---|---|
| PTEXT-04 | Text content box | Done | Proposed | 103613 | |
| PTEXT-05 | Font dropdown | Done | Proposed | 103657-103714; [ui/editor.md](../ui/editor.md#font-picker) | |
| PTEXT-06 | Font size | Done | Proposed | 103613 | |
| PTEXT-07 | Bold, underline, italic | Done | Proposed | 103613 | |
| PTEXT-08 | Case TT, tt, Tt | Done | Differs | 103613 | Off the 120 px column |
| PTEXT-09 | Fill colour | Done | Proposed | 103613 | |
| PTEXT-10 | Character, line spacing | Done | Proposed | 103613 | |
| PTEXT-11 | Alignment, six icons | Partial | Proposed | 103613 | No vertical text |
| PTEXT-12 | Preset style grid | Partial | Differs | 103613 | Look tiles by shelf |
| PTEXT-13 | Transform group | Done | Matches | 103613 | |
| PTEXT-14 | Blend, opacity | Done | Proposed | 103613 | |
| PTEXT-15 | Stroke colour, Thickness | Done | Matches | 095729-095751 | |
| PTEXT-16 | Background | Done | Proposed | 103613 | Ours adds padding, radius |
| PTEXT-17 | Glow | Done | Proposed | 103613 | Switch only |
| PTEXT-18 | Shadow controls | Done | Matches | 095914-095917 | |
| PTEXT-19 | Curve (bend text) | Missing | Not captured | 103632 | |
| PTEXT-20 | Colour picker popover | Done | Proposed | 095732-095751 | |
| PTEXT-21 | Save as preset | Done | Proposed | 103808 | |

### Bubble, Effects, Animation, Tracking, TTS
| ID | Item | Function | UI/UX | Evidence | Gap / note |
|---|---|---|---|---|---|
| PTEXT-22 | Bubble grid | Missing | Not captured | 103723 | |
| PTEXT-23 | Text effects grid | Partial | Differs | 103727 | In Basic style row |
| PTEXT-24 | Text effects Reset | Missing | Not captured | 103727 | |
| PTEXT-25 | Animation In, Out | Done | Proposed | 103738 | |
| PTEXT-26 | Animation Loop | Partial | Differs | 103738 | Combo slot instead |
| PTEXT-27 | Animation tile grid | Partial | Proposed | 103738 | 15 / 15 / 10 looks |
| PTEXT-28 | Tracking, Motion tracking | Missing | Not captured | 103742 | |
| PTEXT-29 | Text to speech tab | Partial | Differs | 103836-103930; [ui/editor.md](../ui/editor.md#text-to-speech) | Model, language stay dropdowns |
| PTEXT-30 | Generate speech to clip | Partial | Differs | 103921-103930 | Preview then Place at playhead |
| PTEXT-31 | TTS voice chips, filter | Done | Differs | 103843-103849 | Chips are provider categories |

## Caption item (PCAP)

| ID | Item | Function | UI/UX | Evidence | Gap / note |
|---|---|---|---|---|---|
| PCAP-01 | Caption top tabs | Partial | Differs | 104149 | No Captions, Animation tab |
| PCAP-02 | Apply to all main captions | Missing | Not captured | 104149 | |
| PCAP-03 | Caption Basic, as text | Done | Proposed | 104149-104210 | |
| PCAP-04 | Caption Templates sub-tab | Partial | Differs | 104218 | Styles in Captions panel |
| PCAP-05 | Caption Bubble, Effects | Missing | Not captured | 104149 | |
| PCAP-06 | Animation Captions sub-tab | Partial | Differs | 104230 | Word-highlight styles |
| PCAP-07 | Keywords section | Ours | Proposed | | |
| PCAP-08 | Caption Save as preset | Partial | Proposed | 104210 | |

## Sticker, effect, filter and adjustment items (POTHER)

| ID | Item | Function | UI/UX | Evidence | Gap / note |
|---|---|---|---|---|---|
| POTHER-01 | Sticker tabs | Partial | Differs | 102720 | No Tracking tab |
| POTHER-02 | Sticker Basic transform | Done | Proposed | 102720 | |
| POTHER-03 | Sticker Animation | Done | Proposed | 102720 | |
| POTHER-04 | Sticker Tracking | Missing | Not captured | 102720 | |
| POTHER-05 | Effect as own lane item | Partial | Differs | 105144 | Ours: adjustment layer only |
| POTHER-06 | Effect Details, Name | Partial | Differs | 105144 | Per-clip effect stack |
| POTHER-07 | Effect param sliders 0-100 | Done | Proposed | 105209-105242 | |
| POTHER-08 | Effect reset, keyframes | Done | Proposed | 105251 | |
| POTHER-09 | Effect Mask tab | Missing | Not captured | 105253 | |
| POTHER-10 | Effect stack reorder, toggle | Ours | Proposed | | |
| POTHER-11 | Filter panel, Name | Done | Proposed | 105256 | |
| POTHER-12 | Filter Intensity 0-100 | Done | Proposed | 105306-105317 | |
| POTHER-13 | Filter reset, keyframe | Partial | Proposed | 105256 | Reset unverified |
| POTHER-14 | Adjustment layer Adjust tab | Partial | Differs | 102732 | Grade rows in Effects stack |
| POTHER-15 | Adjustment Basic, HSL, Curves, wheel | Partial | Differs | 102732 | |
| POTHER-16 | Adjustment Mask tab | Missing | Not captured | 102732 | |
| POTHER-17 | Missing media panel | Done | Proposed | 095550; [ui/editor.md](../ui/editor.md#missing-media) | |
