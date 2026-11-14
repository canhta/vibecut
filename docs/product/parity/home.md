# Home

Snapshot of main `66512fec` on 2026-11-14. Legend, evidence names and update rule: [parity](README.md).

## Home (HOME)

### Left rail
| ID | Item | Function | UI/UX | Evidence | Gap / note |
|---|---|---|---|---|---|
| HOME-01 | Rail: Home entry | Done | Matches | 221432; [ui/home.md](../ui/home.md) | |
| HOME-02 | Rail: Templates entry | Missing | — | 221440 | No templates page; see TMPL |
| HOME-03 | Rail: AI agent page | Partial | Not captured | 221453 | Agent chat lives in the editor |
| HOME-04 | Rail: Library entry | Missing | — | 221502 | Rail holds Home and Shorts |
| HOME-05 | Rail: Spaces entry | Out of scope | — | 110940 | Accounts and cloud sync |
| HOME-06 | Rail: Shorts entry | Ours | Proposed | `apps/vibecut/src/home/view/chrome.rs` | Reference has a card |
| HOME-07 | Top utility icons | Partial | Differs | 221432 | Only open-project and job status |
| HOME-08 | Gear menu | Partial | Not captured | 110737 | No Settings on Home |

### Create and tools
| ID | Item | Function | UI/UX | Evidence | Gap / note |
|---|---|---|---|---|---|
| HOME-09 | Create project banner | Done | Matches | 221432; [ui/home.md](../ui/home.md) | |
| HOME-10 | Create menu: ratio presets | Missing | — | web 01-home.md | Banner creates directly |
| HOME-11 | Create menu: image and magic tools | Out of scope | — | requirements: generated media | |
| HOME-12 | AI tool cards (generative) | Out of scope | — | 221432 | Generated media |
| HOME-13 | Long video to shorts card | Done | Proposed | 221432 | Card geometry unmeasured |
| HOME-14 | Agent prompt card | Done | Matches | 221432, 221910; [ui/home.md](../ui/home.md) | |
| HOME-15 | Popular features row | Out of scope | — | web 01-home.md | Online promos |
| HOME-16 | Smart Insights page | Out of scope | — | web 01-home.md | Cloud assistant |

### Projects
| ID | Item | Function | UI/UX | Evidence | Gap / note |
|---|---|---|---|---|---|
| HOME-17 | Projects heading and grid | Done | Matches | 221432; [ui/home.md](../ui/home.md) | |
| HOME-18 | Project search | Done | Matches | 221432 | |
| HOME-19 | Grid/list view toggle | Done | Matches | 221432 | |
| HOME-20 | Trash view | Done | Matches | 221432 | |
| HOME-21 | Sync toggle | Out of scope | — | 221432 | Cloud sync |
| HOME-22 | Sort control | Ours | Proposed | `apps/vibecut/src/home/view/projects.rs` | |
| HOME-23 | Project thumbnail card | Done | Matches | 221432 | Relists on Home focus |
| HOME-24 | Card name and detail line | Done | Matches | 221432 | |
| HOME-25 | Card menu (open, rename, duplicate) | Done | Not captured | web 01-home.md | |
| HOME-26 | Rename, info, delete dialogs | Done | Not captured | [ui/home.md](../ui/home.md) | |
| HOME-27 | Multi-select | Done | Not captured | web flows/11 | |
| HOME-28 | Empty, loading, error states | Done | Proposed | `apps/vibecut/src/home/view/projects.rs` | |
| HOME-29 | Open project from file | Ours | Proposed | `apps/vibecut/src/home/view/chrome.rs` | |
| HOME-30 | Unreadable project handling | Done | Proposed | smoke 2026-11-14 | Listed apart with reason |

### Brand kit and onboarding
| ID | Item | Function | UI/UX | Evidence | Gap / note |
|---|---|---|---|---|---|
| HOME-31 | Brand kit page | Partial | Not captured | 111111 | Store done, page open |
| HOME-32 | Brand kit sections | Missing | Not captured | 111111 | Videos, images, fonts, music |
| HOME-33 | Brand kit intro banner | Missing | Not captured | 111111 | |
| HOME-34 | Onboarding and tour | Ours | — | | Welcome not built |
