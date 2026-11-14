# Captions and transcript editing

Snapshot of main `d3be7568` on 2026-11-14. Legend, evidence names and update rule: [parity](README.md).

## Captions panel (CAPS)

### Captions panel
| ID | Item | Function | UI/UX | Evidence | Gap / note |
|---|---|---|---|---|---|
| CAPS-01 | Captions sub-nav | Done | Matches | 221736 | |
| CAPS-02 | Auto captions page | Done | Matches | 103953 | |
| CAPS-03 | Spoken language dropdown | Done | Proposed | 103959 | Adds Auto detect |
| CAPS-04 | Bilingual captions dropdown | Done | Proposed | 103953 | |
| CAPS-05 | Identify filler words switch | Done | Proposed | 103953 | |
| CAPS-06 | Delete current captions | Done | Matches | 103953 | |
| CAPS-07 | Generate button | Done | Matches | 103953 | |
| CAPS-08 | Generating modal with Cancel | Partial | Differs | 104019 | Panel progress, not modal |
| CAPS-09 | Speech recognized toast | Done | Proposed | 104019 | |
| CAPS-10 | No-speech error | Done | Proposed | web 16-editor-captions-run.md | |
| CAPS-11 | Caption templates entry | Missing | Not captured | 103953 | |
| CAPS-12 | Auto lyrics | Missing | Not captured | 103953 | |
| CAPS-13 | Add captions page | Done | Proposed | 103953 | |
| CAPS-14 | Manual entry | Done | Proposed | web 16-editor-captions-run.md | |
| CAPS-15 | Caption file import | Done | Proposed | web 07-editor-captions.md | SRT, VTT, ASS |

### Cue list and editing
| ID | Item | Function | UI/UX | Evidence | Gap / note |
|---|---|---|---|---|---|
| CAPS-16 | Cue list rows | Done | Proposed | 104039 | |
| CAPS-17 | Cue search field | Partial | Not captured | 104039 | Find and replace covers it |
| CAPS-18 | Cue select, edit mode | Done | Proposed | 104047, 104052 | |
| CAPS-19 | Cue add and delete buttons | Partial | Not captured | 104047 | Unverified |
| CAPS-20 | Split, merge, line break keys | Done | Not captured | 104122 | |
| CAPS-21 | Cue help tooltip | Missing | Not captured | 104122 | |
| CAPS-22 | Double-click number plays | Partial | Not captured | 104122 | Click seeks |
| CAPS-23 | Remove filler words | Done | Proposed | 104104 | Section, not toolbar icon |
| CAPS-24 | Bilingual captions icon | Partial | Differs | 104110 | Dropdown on Auto page |
| CAPS-25 | Find and replace | Done | Proposed | 104116 | |
| CAPS-26 | Caption item properties | Done | Not captured | 104149 | See PCAP |
| CAPS-27 | Style chooser, presets | Ours | Proposed | [ui/editor.md](../ui/editor.md#captions-workflow) | |
| CAPS-28 | Word highlight, keywords | Ours | Proposed | | |
| CAPS-29 | Caption emoji stickers | Missing | — | | Ours |

### Translate and transcript
| ID | Item | Function | UI/UX | Evidence | Gap / note |
|---|---|---|---|---|---|
| CAPS-30 | Translate page | Done | Proposed | web 16-editor-captions-run.md | |
| CAPS-31 | Translate dialog (from, to) | Partial | Differs | web captions translate | Page, not dialog |
| CAPS-32 | Glossary, replace-edited | Ours | Proposed | | |
| CAPS-33 | Export SRT, TXT, VTT | Done | Proposed | web captions export | In export dialog |
| CAPS-34 | Transcript entry (clip menu) | Done | Differs | 110011 | Pane in Captions tab |
| CAPS-35 | Analyzing modal | Done | Differs | 110018; [ui/editor.md](../ui/editor.md#transcript-pane) | Spinner for the logo |
| CAPS-36 | Transcript view, sentence rows | Partial | Differs | 110059; [ui/editor.md](../ui/editor.md#transcript-pane) | Word level |
| CAPS-37 | Tabs: All, Pauses, Repeats, Fillers | Partial | Proposed | 110106, 110110 | No Filler words tab |
| CAPS-38 | Pause chips, remove pauses | Done | Proposed | 110059; [ui/editor.md](../ui/editor.md#transcript-pane) | Gap card removes all |
| CAPS-39 | Select all, total duration | Done | Proposed | 110122; [ui/editor.md](../ui/editor.md#transcript-pane) | |
| CAPS-40 | Create compound clip option | Partial | Proposed | 110059; [ui/editor.md](../ui/editor.md#transcript-pane) | Disabled; the review makes no compound clip |
| CAPS-41 | Delete, Cancel footer | Partial | Not captured | 110059 | |
| CAPS-42 | Chapters page | Ours | Proposed | [ui/editor.md](../ui/editor.md#chapters-section) | |

## Transcript editing (TRANSCRIPT)

| ID | Item | Function | UI/UX | Evidence | Gap / note |
|---|---|---|---|---|---|
| TRANSCRIPT-01 | Entry from clip menu | Done | Differs | 14b/110011 | Clip menu opens the Captions pane (TRANSCRIPT-03) |
| TRANSCRIPT-02 | Analyzing modal, Cancel | Done | Differs | 14b/110018 | Spinner for the logo |
| TRANSCRIPT-03 | Separate result window | Differs | Differs | 14b/110059 | Ours: Captions-tab pane |
| TRANSCRIPT-04 | Tabs All, Pauses, Repeats, Fillers | Partial | Proposed | 14b/110059 | No Filler words tab |
| TRANSCRIPT-05 | Sentence rows, checkboxes | Partial | Proposed | 14b/110059 | Word level |
| TRANSCRIPT-06 | Pause pills | Done | Proposed | 14b/110059 | Word gaps, not audio |
| TRANSCRIPT-07 | Repeat detection | Done | Proposed | 14b/110108 | One to three words |
| TRANSCRIPT-08 | Select all, total duration | Done | Proposed | 14b/110122 | |
| TRANSCRIPT-09 | Preview player, row seek | Partial | Proposed | 14b/110130 | |
| TRANSCRIPT-10 | Mini timeline in window | Partial | Differs | 14b/110059 | No ruler or zoom, click seeks |
| TRANSCRIPT-11 | Create compound option | Partial | Proposed | 14b/110059 | Disabled; the review makes no compound clip |
| TRANSCRIPT-12 | Cancel, Delete | Partial | Proposed | 14b/110136 | |
| TRANSCRIPT-13 | Find box | Done | Proposed | web screens/17 | |
| TRANSCRIPT-14 | Gap card, remove gaps | Done | Proposed | web screens/17 | Function only |
| TRANSCRIPT-15 | Show deleted words | Partial | Proposed | web screens/17 | List only, restore is Undo |
| TRANSCRIPT-16 | Language and track select | Partial | Proposed | web screens/17 | Language only |
