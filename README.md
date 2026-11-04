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
  A free, fast, native video editor for macOS and Windows. No paywall, no watermark, your footage stays on your machine.
</p>

<!-- Hero: docs/assets/readme/hero.jpg (editor: media panel, preview, timeline, properties) -->

## Features

<table>
<tr>
<td width="50%" valign="middle">

### Multi-track Timeline

Cut, split, trim, ripple, snap and group clips across video, audio, text and
sticker tracks, with full undo.

</td>
<td width="50%"><!-- docs/assets/readme/timeline.gif --></td>
</tr>
<tr>
<td width="50%" valign="middle">

### Text, Stickers and Graphics

Styled titles with presets and animations, stickers, shapes and backgrounds, edited
directly on the preview.

</td>
<td width="50%"><!-- docs/assets/readme/text.gif --></td>
</tr>
<tr>
<td width="50%" valign="middle">

### Effects, Transitions and Keyframes

Animate any property, add transitions, blend modes, masks, chroma key and effect
layers.

</td>
<td width="50%"><!-- docs/assets/readme/effects.gif --></td>
</tr>
<tr>
<td width="50%" valign="middle">

### Speed and Color

Constant speed, speed ramps, reverse and freeze frame; color adjustments, curves
and LUTs.

</td>
<td width="50%"><!-- docs/assets/readme/color.gif --></td>
</tr>
<tr>
<td width="50%" valign="middle">

### Audio

Volume, fades, multi-track mix, voiceover recording, noise reduction and loudness
normalization.

</td>
<td width="50%"><!-- docs/assets/readme/audio.gif --></td>
</tr>
<tr>
<td width="50%" valign="middle">

### Export Anywhere

MP4, WebM, GIF, image sequence or audio only, in 16:9, 9:16 or any canvas, with
hardware-accelerated encoding.

</td>
<td width="50%"><!-- docs/assets/readme/export.gif --></td>
</tr>
</table>

**AI built in, local first:**

- **Auto captions** in Vietnamese, English and Chinese, translation between any pair, bilingual subtitles, word highlight.
- **AI shorts:** ranked vertical clips from long videos, reframed to the speaker.
- **Text-based editing:** remove pauses and filler words, cut by editing the transcript.
- **Your choice of AI:** local models, or OpenAI, Anthropic and ElevenLabs; nothing is uploaded until you approve.

**Also in the box:** autosave and crash recovery, proxy editing, keyboard shortcuts,
English and Vietnamese UI, headless CLI. Later: dubbing, plugins, scripting, and MCP
so coding agents can drive the editor.

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
