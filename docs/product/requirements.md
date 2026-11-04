# Requirements

## Principles

1. **Quality and stability first.** Correct output (picture, sound, timing,
   subtitles) and an editor that never crashes or loses work decide whether people
   use VibeCut. Never trade them for features or speed.
2. **Light and fast.** The reason for a Rust-native desktop app: small install,
   low memory, quick startup, fast preview and export.

## Scope

VibeCut is a local-first desktop video editor for macOS and Windows.

| Priority | User outcome |
|---|---|
| P0 | Transcribe Vietnamese, English or Chinese speech; translate between any pair of these languages or show bilingual subtitles; edit; burn into MP4 or export SRT/VTT. |
| P0 | Get suggested vertical shorts from long Vietnamese, English or Chinese talking videos; review, trim and export selected clips. |
| P1 | Improve reframing, batch operations and output quality. |
| P2 | Optionally dub into Vietnamese, English or Chinese. |

Every AI feature lets the user pick its provider; more providers come later.
Transcription: local whisper.cpp and Gipformer, OpenAI, ElevenLabs. Translation and
clip suggestion: OpenAI and Anthropic. Dubbing voices: OpenAI and ElevenLabs. Each
run uses exactly one provider. A fallback runs only when the user configured it in
settings; nothing switches provider on its own.

Exclude montage generation, automatic B-roll, voice cloning and Linux from the
current scope. Linux support is deferred. Suggestions remain subject to user review.

## Languages

Speech recognition, translation, subtitle segmentation and text rendering are
optimized for Vietnamese (`vi`), English (`en`) and Chinese (`zh`, Simplified and
Traditional) first; more languages are added later without code changes to the
core. Every ordered pair translates; bilingual subtitles pair any two. The UI
ships in English and Vietnamese and accepts more catalogs.
