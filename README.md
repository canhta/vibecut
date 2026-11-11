<h1 align="center">
  <img src="docs/brand/logo.svg" alt="VibeCut" width="64" valign="middle" /> VibeCut
</h1>

<p align="center">
  <img src="https://img.shields.io/badge/license-GPL--3.0--only-08C?style=flat" alt="License: GPL-3.0-only" />
  <img src="https://img.shields.io/badge/macOS%20%7C%20Windows%20%7C%20Linux-4493F8?style=flat-square" alt="Platforms: macOS, Windows and Linux" />
</p>

<p align="center">
  <strong>The open-source CapCut alternative your AI agents can drive.</strong><br/>
  A free, fast, native video editor for macOS, Windows and Linux, with MCP and a CLI built in.
</p>

<!-- Hero: docs/assets/readme/hero.jpg (editor: media panel, preview, timeline, properties) -->

## Features

<table>
<tr>
<td width="50%" valign="middle">

### Agents Edit Video

Claude Code, Codex or any MCP client opens projects, runs editor commands,
inspects preview frames and exports. The same commands run from the CLI.

</td>
<td width="50%"><!-- docs/assets/readme/agents.gif --></td>
</tr>
<tr>
<td width="50%" valign="middle">

### AI Captions

Transcribe Vietnamese, English or Chinese, translate between any pair, and show
bilingual subtitles.

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
<tr>
<td width="50%" valign="middle">

### Multi-track Timeline

Cut, split, trim, ripple, snap and group clips across video, audio and text
tracks, with undo.

</td>
<td width="50%"><!-- docs/assets/readme/timeline.gif --></td>
</tr>
<tr>
<td width="50%" valign="middle">

### Text

Styled titles with font, size, color and background, edited directly on the
preview.

</td>
<td width="50%"><!-- docs/assets/readme/text.gif --></td>
</tr>
<tr>
<td width="50%" valign="middle">

### Effects, Transitions and Keyframes

Keyframe transform, opacity and effect parameters, with transitions, blend
modes, masks and chroma key.

</td>
<td width="50%"><!-- docs/assets/readme/effects.gif --></td>
</tr>
<tr>
<td width="50%" valign="middle">

### Speed and Color

Constant speed and color adjustments such as brightness, contrast and
saturation.

</td>
<td width="50%"><!-- docs/assets/readme/color.gif --></td>
</tr>
<tr>
<td width="50%" valign="middle">

### Audio

Volume, fades, multi-track mix and loudness normalization.

</td>
<td width="50%"><!-- docs/assets/readme/audio.gif --></td>
</tr>
<tr>
<td width="50%" valign="middle">

### Export Anywhere

MP4, MOV or WebM, in landscape, portrait, square or custom canvases, with H.264
hardware encoding where the platform supports it.

</td>
<td width="50%"><!-- docs/assets/readme/export.gif --></td>
</tr>
</table>

**Also in the box:** local or cloud AI of your choice · crash recovery · custom shortcuts

## Install

Download the latest build from
[Releases](https://github.com/canhta/vibecut/releases/latest): macOS and Windows
packages, plus the Linux tarball in the nightly pre-release.

Build from source on any of the three platforms:

```sh
cargo xtask ffmpeg
cargo run --release -p vibecut
```

Linux build packages are listed in
[workflows](docs/engineering/workflows.md#ffmpeg-provisioning).

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md).

## License

[GPL-3.0-only](LICENSE)
