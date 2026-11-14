# Parity with CapCut

One row per CapCut desktop item a user can see or use, mapped to VibeCut's state. This folder tracks progress and quality; it does not hold values or behaviour. Measured values live in [UI design](../ui/index.md) and its window files, scope in [requirements](../requirements.md), commands in [agent control](../agent-control/commands.md).

Snapshot: main `66512fec`, 2026-11-14.

## How to read a row

`ID | Item | Function | UI/UX | Evidence | Gap / note`

| Column | Values |
|---|---|
| Function | **Done**, **Partial**, **Missing**, **Differs** (works another way on purpose or pending a change), **Ours** (our extra), **Out of scope** (per [requirements](../requirements.md)) |
| UI/UX | **Matches** (measured values applied), **Differs** (the note says how), **Proposed** (built, values unmeasured), **Not captured** (no reference evidence), **—** (no UI) |
| Evidence | CapCut frame or web screen, then our doc link |

Evidence names (all local captures under `.spikes/`, not in the repo):

| Name | Capture |
|---|---|
| `05/221643`, bare `2216xx`-`2222xx` | `ui-reference/capcut-desktop-2026-11-05/` |
| `14/095518`, bare `0954xx`-`1005xx` | `ui-reference/capcut-desktop-2026-11-14/` |
| `14b/104709`, bare `1025xx`-`1121xx` | `ui-reference/capcut-desktop-2026-11-14b/` |
| `14c` | `ui-reference/capcut-desktop-2026-11-14c/` |
| `14d/151656`, bare `151xxx` | `ui-reference/capcut-desktop-2026-11-14d/` |
| `web screens/NN`, `web flows/NN` | `web-editors/capcut-web/` (function only, never layout) |

## Update rule

- A change that moves a row updates that row in the same commit: status, evidence, note. Keep notes to one clause; details go in the owning doc.
- New CapCut items found in a capture get new rows at the end of their group (IDs are never reused or renumbered).
- After editing, refresh the totals below by counting the area files.

## Summary

| Area | Rows | Done | Partial | Missing | Differs | Ours | Out of scope | UI Matches | UI Differs | UI Proposed | UI Not captured |
|---|---|---|---|---|---|---|---|---|---|---|---|
| [Home](home.md#home-home) | 34 | 15 | 4 | 5 | 0 | 4 | 6 | 9 | 1 | 6 | 8 |
| [Header and sub-navigation](shell.md#header-and-sub-navigation-shell) | 37 | 21 | 5 | 3 | 0 | 3 | 5 | 13 | 1 | 11 | 6 |
| [Window and panel layout](shell.md#window-and-panel-layout-layout) | 11 | 6 | 1 | 2 | 0 | 1 | 1 | 6 | 0 | 1 | 1 |
| [Settings](shell.md#settings-set) | 26 | 3 | 4 | 13 | 0 | 4 | 2 | 0 | 2 | 9 | 0 |
| [Keyboard shortcuts](shell.md#keyboard-shortcuts-shortcut) | 26 | 15 | 3 | 5 | 2 | 1 | 0 | 2 | 0 | 11 | 0 |
| [Media](assets.md#media-media) | 35 | 15 | 6 | 7 | 0 | 2 | 5 | 2 | 1 | 11 | 16 |
| [Audio](assets.md#audio-audio) | 21 | 13 | 4 | 2 | 0 | 1 | 1 | 5 | 6 | 6 | 3 |
| [Text library](assets.md#text-library-textlib) | 10 | 2 | 2 | 4 | 0 | 1 | 1 | 0 | 0 | 2 | 6 |
| [Stickers](assets.md#stickers-stick) | 12 | 4 | 1 | 1 | 0 | 4 | 2 | 1 | 1 | 7 | 0 |
| [Templates](assets.md#templates-tmpl) | 7 | 0 | 0 | 6 | 0 | 0 | 1 | 0 | 0 | 0 | 5 |
| [Captions panel](captions.md#captions-panel-caps) | 42 | 20 | 11 | 7 | 0 | 4 | 0 | 4 | 5 | 18 | 14 |
| [Transcript editing](captions.md#transcript-editing-transcript) | 16 | 1 | 5 | 9 | 1 | 0 | 0 | 0 | 1 | 5 | 1 |
| [Effects](catalogues.md#effects-fx) | 17 | 9 | 4 | 2 | 0 | 0 | 2 | 3 | 4 | 5 | 2 |
| [Transitions](catalogues.md#transitions-trans) | 16 | 9 | 2 | 1 | 0 | 3 | 1 | 2 | 1 | 9 | 1 |
| [Filters](catalogues.md#filters-filt) | 9 | 7 | 1 | 0 | 0 | 0 | 1 | 1 | 1 | 4 | 0 |
| [Adjustment](catalogues.md#adjustment-adj) | 6 | 2 | 3 | 0 | 0 | 1 | 0 | 1 | 3 | 2 | 0 |
| [Video clip](properties-video.md#video-clip-pvid) | 52 | 29 | 8 | 9 | 0 | 3 | 3 | 6 | 8 | 26 | 8 |
| [Audio clip](properties-other.md#audio-clip-paud) | 17 | 8 | 1 | 7 | 0 | 1 | 0 | 3 | 1 | 6 | 7 |
| [Text item](properties-other.md#text-item-ptext) | 31 | 17 | 8 | 5 | 0 | 0 | 1 | 3 | 8 | 14 | 5 |
| [Caption item](properties-other.md#caption-item-pcap) | 8 | 1 | 4 | 2 | 0 | 1 | 0 | 0 | 3 | 3 | 2 |
| [Sticker, effect, filter and adjustment items](properties-other.md#sticker-effect-filter-and-adjustment-items-pother) | 17 | 7 | 6 | 3 | 0 | 1 | 0 | 0 | 5 | 9 | 3 |
| [Timeline](timeline.md#timeline-tl) | 95 | 44 | 11 | 26 | 2 | 10 | 2 | 13 | 5 | 45 | 5 |
| [Keyframes](timeline.md#keyframes-key) | 14 | 10 | 3 | 0 | 0 | 1 | 0 | 2 | 2 | 7 | 1 |
| [Player](player-export.md#player-play) | 24 | 15 | 5 | 1 | 0 | 3 | 0 | 5 | 6 | 11 | 0 |
| [Export](player-export.md#export-exp) | 31 | 10 | 10 | 7 | 0 | 1 | 3 | 2 | 11 | 6 | 3 |
| [AI chat](ai-shorts.md#ai-chat-agent) | 17 | 8 | 3 | 3 | 1 | 1 | 1 | 5 | 2 | 4 | 2 |
| [Long video to shorts](ai-shorts.md#long-video-to-shorts-shorts) | 24 | 9 | 7 | 4 | 0 | 2 | 2 | 8 | 2 | 5 | 3 |
| [States](states.md#states-state) | 12 | 7 | 3 | 0 | 1 | 0 | 1 | 2 | 2 | 4 | 3 |
| **Total** | **667** | **307** | **125** | **134** | **7** | **53** | **41** | **98** | **82** | **247** | **105** |
