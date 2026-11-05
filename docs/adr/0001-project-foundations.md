# Project foundations

A local-first desktop editor targets macOS, Windows and Linux, and a
Rust core runnable headless so UI choices do not constrain editing or automation.
AI providers use adapters, starting with opt-in OpenAI and explicit consent before
media upload, with local models later to extend offline use. Choose GPL-3.0-only and English
docs for contribution clarity, replace designs in place without parallel versions
or compatibility paths to reduce pre-release maintenance, and keep `AGENTS.md` as
the sole agent instruction file (`CLAUDE.md` contains only `@AGENTS.md`) to avoid
conflicting instructions; current contracts live in [owner docs](../README.md).
