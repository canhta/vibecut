# VibeCut

VibeCut is an open-source CapCut alternative: a Rust-first, local-first desktop
video editor for macOS and Windows.
It aims to turn long-form English talking videos into AI-suggested vertical short
clips that you review, trim and export, and to transcribe, translate and edit
English-to-Vietnamese or bilingual subtitles for burn-in MP4 or SRT/VTT export.
Vietnamese dubbing may follow later. Status: **pre-alpha, design phase**; this
repository contains only the project skeleton.

AI is opt-in: media is uploaded only with explicit consent. OpenAI is the first
planned provider; local models and Linux support come later.

## Development

Install [rustup](https://rustup.rs/), then run `cargo xtask ci`.
The toolchain file pins Rust and installs rustfmt and Clippy.

See [CONTRIBUTING.md](CONTRIBUTING.md), [GLOSSARY.md](GLOSSARY.md) and the
[initial decisions](docs/adr/0001-project-foundations.md). Licensed under [MIT](LICENSE).
