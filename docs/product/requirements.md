# Requirements

## Principles

1. Quality and stability first: correct picture, sound, timing and subtitles, and an editor that never crashes or loses work. Features and speed never buy these down.
2. Light and fast: small install, low memory, quick startup, fast preview and export.

## Scope

Local-first desktop video editor for macOS, Windows and Linux.

| Priority | User outcome |
|---|---|
| P0 | Transcribe Vietnamese, English or Chinese; translate between any pair or show bilingual subtitles; edit; burn into MP4 or export SRT/VTT. |
| P0 | Get suggested vertical shorts from long talking videos; review, trim, export. |
| P1 | Coding agents (Claude Code, Codex, opencode, any MCP client) drive the editor through MCP, CLI and an agent skill. |
| P1 | An agent chat inside the editor proposes edits from a prompt; the user previews them and applies or discards them as one step. |
| P1 | Better reframing, batch operations, output quality; full editor parity, including footage clean-up (stabilize, noise, flicker) and a brand kit of reusable logos, colours, fonts and presets. |
| P2 | Optional dubbing into Vietnamese, English or Chinese. |
| P2 | Features that need a local model (subject cutout, body effects) install their weights through the model manager; voice presets are local audio processing. |

Out of scope: montage generation, automatic B-roll, voice cloning, generated media (AI avatars, image, video and style generation), beauty retouch, online asset and template stores, accounts, sharing and cloud sync. AI suggestions always go through user review.

Built-in content (effects, transitions, filters, stickers, text and caption presets, templates) ships as a curated set with a recorded licence, or comes from files the user imports. Sounds and fonts may also be fetched, one item at a time on the user's action, from an open source whose terms are recorded. The editor works fully without a network and never requires an online catalogue.

Build order: foundation (structure, build, test kit), the reference product's core editor features, AI editing in the editor and agent control, Home, then long video to shorts. Ticket priorities encode it ([issue tracker](../agents/issue-tracker.md)).

## Providers

The user picks the provider and the model for every AI feature:

| Feature | Providers |
|---|---|
| Transcription | whisper.cpp and Gipformer (local), OpenAI, ElevenLabs |
| Translation, clip suggestion | OpenAI, Anthropic (clips also: local heuristics) |
| Chapters, title and description | OpenAI, Anthropic |
| Agent chat | OpenAI, Anthropic |
| Dubbing voices | OpenAI, ElevenLabs |

Each run uses one provider. A fallback runs only when the user configured it.

## Languages

Optimized for Vietnamese, English and Chinese (Simplified and Traditional); more languages arrive as data. Every ordered pair translates; bilingual subtitles pair any two. The UI ships in English and Vietnamese.
