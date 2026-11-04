<h1 align="center">
  <img src="docs/brand/logo.svg" alt="VibeCut" width="64" valign="middle" /> VibeCut
</h1>

<p align="center">
  <img src="https://img.shields.io/badge/license-MIT-08C?style=flat" alt="License: MIT" />
  <img src="https://img.shields.io/badge/macOS%20%7C%20Windows-4493F8?style=flat-square" alt="Platforms: macOS and Windows" />
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
<tr>
<td width="50%" valign="middle">

### AI Captions

Captions in Vietnamese, English and Chinese, translated to any pair, shown bilingual.

</td>
<td width="50%"><!-- docs/assets/readme/captions.gif --></td>
</tr>
<tr>
<td width="50%" valign="middle">

### AI Shorts

Long video in, vertical clips out.

</td>
<td width="50%"><!-- docs/assets/readme/shorts.gif --></td>
</tr>
</table>

**Also in the box:** crash recovery · proxy editing · custom shortcuts · CLI · MCP for coding agents

## Install

Download the latest macOS or Windows build from
[Releases](https://github.com/canhta/vibecut/releases/latest).

Build from source:

```sh
cargo xtask ffmpeg
cargo run --release -p vibecut
```

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md).

## License

[MIT](LICENSE)
