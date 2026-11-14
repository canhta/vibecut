# Editor shell, layout, settings and shortcuts

Snapshot of main `66512fec` on 2026-11-14. Legend, evidence names and update rule: [parity](README.md).

## Header and sub-navigation (SHELL)

### Header and window menu
| ID | Item | Function | UI/UX | Evidence | Gap / note |
|---|---|---|---|---|---|
| SHELL-01 | Project title | Done | Proposed | 221513 | Inline rename |
| SHELL-02 | Autosave status line | Done | Matches | 2026-11-14b row 17 | |
| SHELL-03 | Export button | Done | Matches | 221513 | |
| SHELL-04 | Shortcuts button | Done | Proposed | 110515 | |
| SHELL-05 | Menu: Back to home | Done | Proposed | 2026-11-14b row 19 | Closes the saved project |
| SHELL-06 | Menu: Settings | Done | Proposed | 110747 | |
| SHELL-07 | Menu: Layout presets | Partial | Differs | 111918 | Only Reset layout |
| SHELL-08 | Menu: About, terms | Missing | Not captured | 100436 | No About dialog |
| SHELL-09 | Menu: Help | Done | Not captured | | |
| SHELL-10 | Feedback, theme, language, models | Ours | Proposed | | Reference has no light theme |
| SHELL-11 | Project save, save as, delete | Ours | Proposed | web 02-editor-shell.md | Ctrl+S with a Saved toast |
| SHELL-12 | Scene switcher | Ours | Proposed | | |
| SHELL-37 | macOS menu bar | Done | Not captured | | From the action registry |

### Feature tab strip
| ID | Item | Function | UI/UX | Evidence | Gap / note |
|---|---|---|---|---|---|
| SHELL-13 | Strip layout and overflow | Done | Matches | 221513 | |
| SHELL-14 | Tab: Media | Done | Matches | 221635 | |
| SHELL-15 | Tab: Audio | Done | Matches | 221703 | |
| SHELL-16 | Tab: Text | Partial | Matches | 221716 | Library grid missing, see TEXTLIB |
| SHELL-17 | Tab: Stickers | Done | Matches | 221721 | |
| SHELL-18 | Tab: Effects | Done | Matches | 221727 | |
| SHELL-19 | Tab: Transitions | Done | Matches | 221731 | |
| SHELL-20 | Tab: Captions | Done | Matches | 221736 | |
| SHELL-21 | Tab: Filters | Done | Proposed | 221739 | Icon proposed |
| SHELL-22 | Tab: Adjustment | Done | Matches | 221743 | |
| SHELL-23 | Tab: Templates | Missing | — | 221747 | See TMPL |
| SHELL-24 | Tab: AI avatars | Out of scope | — | 221751 | Generated media |
| SHELL-25 | Panel width limits | Done | Matches | 2026-11-14c | Min 398 / 398 / 368 px |

### Sub-navigation
| ID | Item | Function | UI/UX | Evidence | Gap / note |
|---|---|---|---|---|---|
| SHELL-26 | Sub-nav column | Done | Matches | 221513 | Stays down to 398 px |
| SHELL-27 | Media > Import group | Partial | Proposed | 221513, 110322 | Header row only, no menu |
| SHELL-28 | Media > Media page | Done | Proposed | 221635 | |
| SHELL-29 | Media > Timeline page | Done | Proposed | 221513 | Lists scenes |
| SHELL-30 | Media > Subprojects page | Missing | Not captured | 221513 | |
| SHELL-31 | Yours group | Partial | Not captured | 221643 | See MEDIA |
| SHELL-32 | Generate group | Out of scope | — | 221646 | Generated media |
| SHELL-33 | Spaces group | Out of scope | — | 221649 | Accounts |
| SHELL-34 | Library group | Out of scope | — | 221654 | Online assets |
| SHELL-35 | Partner app group | Out of scope | — | 221513 | |
| SHELL-36 | Collapsible groups | Partial | Not captured | 221643 | Ours has no accordion |

## Window and panel layout (LAYOUT)

| ID | Item | Function | UI/UX | Evidence | Gap / note |
|---|---|---|---|---|---|
| LAYOUT-01 | Minimum window | Done | Matches | 14b/111859 | 1268x650 |
| LAYOUT-02 | Media / player divider limits | Done | Matches | 14c | |
| LAYOUT-03 | Player / properties limits | Done | Matches | 14c | |
| LAYOUT-04 | Timeline divider limits | Done | Matches | 14b/111942 | |
| LAYOUT-05 | Default columns | Partial | Matches | 14c | |
| LAYOUT-06 | Reset current layout | Done | Proposed | 14b/112001 | |
| LAYOUT-07 | Layout presets | Missing | — | 2026-11-14 notes | Five named presets |
| LAYOUT-08 | Layout icons in title bar | Missing | Not captured | 05/221643 | |
| LAYOUT-09 | Pro, Share buttons | Out of scope | — | 05/221643 | Accounts, sharing |
| LAYOUT-10 | Autosave state in title bar | Done | Matches | 05/221643 | |
| LAYOUT-11 | Detachable panels | Ours | — | | |

## Settings (SET)

| ID | Item | Function | UI/UX | Evidence | Gap / note |
|---|---|---|---|---|---|
| SET-01 | Settings window, four tabs | Done | Differs | 14b/110747 | Modal, no Save button |
| SET-02 | Draft: save and download paths | Missing | — | 14b/110747 | Managed library |
| SET-03 | Draft: cache auto delete | Partial | Proposed | 14b/110758 | |
| SET-04 | Draft: cloud sync | Out of scope | — | 14b/110758 | Cloud sync |
| SET-05 | Edit: frame step, adjust value | Missing | — | 14b/110800 | |
| SET-06 | Edit: image duration | Missing | — | 14b/110800 | |
| SET-07 | Edit: target loudness | Missing | — | 14b/110800 | Per project |
| SET-08 | Edit: audio while scrubbing | Missing | — | 14b/110804 | |
| SET-09 | Edit: linkage settings | Missing | — | 14b/110804 | Needs TL-21 |
| SET-10 | Edit: arrange layers | Missing | — | 14b/110804 | |
| SET-11 | Edit: default frame rate | Done | Proposed | 14b/110804 | Under Draft tab |
| SET-12 | Edit: time code format | Missing | — | 14b/110804 | |
| SET-13 | Edit: export sound | Missing | — | 14b/110804 | |
| SET-14 | Edit: partner sync | Out of scope | — | 14b/110804 | Accounts |
| SET-15 | Perf: hardware encode, decode | Partial | Proposed | 14b/110829 | One choice |
| SET-16 | Perf: auto optimization, remux | Missing | — | 14b/110829 | |
| SET-17 | Perf: proxy | Partial | Proposed | 14b/110833 | Per project |
| SET-18 | Perf: render cache size, clear | Partial | Proposed | 14b/110833 | |
| SET-19 | Perf: auto render | Missing | — | 14b/110833 | |
| SET-20 | General: language | Done | Differs | 14b/110843 | English, Vietnamese |
| SET-21 | General: notifications | Missing | — | 14b/110847 | |
| SET-22 | General: software update | Missing | — | 14b/110847 | |
| SET-23 | Edit: snapping, ripple, axis | Ours | Proposed | | |
| SET-24 | Theme toggle | Ours | Proposed | | Reference has none |
| SET-25 | Shortcut import, export | Ours | Proposed | | |
| SET-26 | AI preferences | Ours | Proposed | [ui/editor.md](../ui/editor.md#ai-preferences) | |

## Keyboard shortcuts (SHORTCUT)

| ID | Item | Function | UI/UX | Evidence | Gap / note |
|---|---|---|---|---|---|
| SHORTCUT-01 | Open button in title bar | Done | Proposed | 14b/110515 | |
| SHORTCUT-02 | Window, four tabs | Done | Matches | 14b/110515 | |
| SHORTCUT-03 | Preset dropdown, five presets | Done | Proposed | 14b/110522 | Popup list not captured |
| SHORTCUT-04 | Click to rebind | Done | Proposed | 14b/110640 | |
| SHORTCUT-05 | Conflict prompt, Override | Done | Proposed | 14b/110641 | Prompt layout not captured |
| SHORTCUT-06 | Esc recorded as a key | Done | — | 14b/110656 | |
| SHORTCUT-07 | Reset, Save, Cancel | Done | Matches | 14b/110708 | |
| SHORTCUT-08 | No search field | Done | — | 14b/110515 | |
| SHORTCUT-09 | Matching timeline defaults | Done | Proposed | 14b/110530; [keyboard.md](../agent-control/keyboard.md) | |
| SHORTCUT-10 | Main track magnet P | Missing | — | 14b/110530 | |
| SHORTCUT-11 | Timeline zoom keys | Done | — | 14b/110530 | |
| SHORTCUT-12 | Activate / deactivate V | Missing | — | 14b/110530 | |
| SHORTCUT-13 | Extract audio key | Done | — | 14b/110530 | Restore has no chord |
| SHORTCUT-14 | Marker keys | Partial | — | 14b/110530 | No new-colour marker key |
| SHORTCUT-15 | Group, compound, speed keys | Missing | — | 14b/110530 | |
| SHORTCUT-16 | Shuttle J, K, L | Partial | Proposed | 14b/110530 | Ours seeks 1 s |
| SHORTCUT-17 | Step back many frames | Differs | Proposed | 14b/110530 | Shift+Left jumps 1 s |
| SHORTCUT-18 | Select by clip Shift X | Differs | — | 14b/110530 | Ours: export in/out |
| SHORTCUT-19 | Duplicate Ctrl D | Ours | Proposed | | |
| SHORTCUT-20 | Player: play, rulers, guides | Done | Proposed | 14b/110531 | |
| SHORTCUT-21 | Player: full screen, zoom keys | Done | — | 14b/110531 | Shifted keys unconfirmed on hardware |
| SHORTCUT-22 | Copy, cut, paste, undo, redo | Done | Proposed | 14b/110536 | |
| SHORTCUT-23 | Copy and paste attributes | Done | Proposed | 14b/110536 | |
| SHORTCUT-24 | Import, Export, New project keys | Partial | — | 14b/110536 | New project has no action |
| SHORTCUT-25 | Tab, Dictate, Back to edit | Missing | — | 14b/110536 | |
| SHORTCUT-26 | Resize text from centre | Missing | — | 14b/110542 | |
