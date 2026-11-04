<h1 align="center">
  <img src="docs/brand/logo.svg" alt="VibeCut" width="64" valign="middle" /> VibeCut
</h1>

<p align="center">
  <img src="https://img.shields.io/badge/license-MIT-08C?style=flat" alt="License: MIT" />
  <img src="https://img.shields.io/badge/macOS%20%7C%20Windows-4493F8?style=flat-square" alt="Platforms: macOS and Windows" />
  <img src="https://img.shields.io/badge/status-pre--alpha-E5A00D?style=flat-square" alt="Status: pre-alpha" />
</p>

<p align="center">
  <strong>The open-source CapCut alternative.</strong><br/>
  A fast native video editor with AI subtitles and AI shorts. Your footage stays on your machine.
</p>

<!-- Hero: docs/assets/readme/hero.jpg (editor with timeline, preview and bilingual subtitles) -->

## Features

<table>
<tr>
<td width="50%" valign="middle">

### AI Subtitles

Transcribe Vietnamese, English or Chinese, translate between any pair, show two
languages at once, then burn in or export SRT/VTT.

</td>
<td width="50%"><!-- docs/assets/readme/subtitles.gif --></td>
</tr>
<tr>
<td width="50%" valign="middle">

### AI Shorts

Drop in a long talk or podcast; get ranked vertical clips to review, trim and export.

</td>
<td width="50%"><!-- docs/assets/readme/shorts.gif --></td>
</tr>
<tr>
<td width="50%" valign="middle">

### Timeline Editing

Multi-track cut, split, trim, text, audio, effects, masks and keyframes, exported
to 16:9 or 9:16.

</td>
<td width="50%"><!-- docs/assets/readme/timeline.gif --></td>
</tr>
<tr>
<td width="50%" valign="middle">

### Your AI, Your Choice

Run speech recognition locally, or pick OpenAI, Anthropic or ElevenLabs. Nothing
leaves your computer until you approve it.

</td>
<td width="50%"><!-- docs/assets/readme/providers.png --></td>
</tr>
</table>

**Also planned:** headless CLI, AI dubbing, plugins, scripting and MCP.

## Install

Pre-alpha: no release yet. Build from source:

```sh
cargo xtask ffmpeg   # once: builds the latest LGPL FFmpeg
cargo xtask ci
```

## Contributing

Sign off every commit (`git commit -s`). See [CONTRIBUTING.md](CONTRIBUTING.md);
agents start at [AGENTS.md](AGENTS.md).

## License

[MIT](LICENSE)
