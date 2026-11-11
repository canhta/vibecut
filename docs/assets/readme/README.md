# README media

`hero.jpg`, `timeline.gif` and `timeline.jpg` are original offscreen captures of
the real VibeCut editor over a credited excerpt of **Big Buck Bunny** (2008,
Blender Foundation). They are documentation postprocessing outputs, not app export
formats and not a product performance benchmark.

## Film source and licence

- Film: [Big Buck Bunny](https://peach.blender.org/) (2008), Blender Foundation.
- Excerpt credit: `(c) copyright 2008, Blender Foundation / www.bigbuckbunny.org`.
- Film licence: [CC-BY-3.0](https://creativecommons.org/licenses/by/3.0/), full
  unmodified text at [CC-BY-3.0.txt](../../licenses/CC-BY-3.0.txt).
- Original source used for this fixture: public 720p H.264 master at
  `https://www.peach.themazzone.com/big_buck_bunny_720p_h264.mov`, 416,751,190
  bytes, SHA-256
  `45c8bafeb9a53df7f491198d2e71529701bcf1cd51805782089fac1d32869f9b`.
- Shared excerpt: 12.000 seconds starting at 50.0 s, 1280x720, 30 fps, SAR 1:1,
  H.264/AAC 48 kHz stereo with the film's original audio. Transformation is
  `fps=30,scale=1280:720:flags=lanczos,setsar=1`, libx264 medium CRF 20,
  yuv420p, GOP 30, BT.709 TV, `movflags faststart`, `map_metadata -1`, no
  data/subtitle streams, AAC 160k. Excerpt SHA-256
  `66518061bbabbde0c7d48b639675ce8c46a549182d32c3cc1203f2cded7c21a1`.
- The full master, excerpt file, projects, raw frames and manifests stay outside
  Git. The compact excerpt is also published as the dedicated versioned
  [fixtures-big-buck-bunny-2026-11-11](https://github.com/canhta/vibecut/releases/tag/fixtures-big-buck-bunny-2026-11-11)
  fixture prerelease; it is not the app, nightly, built-in content or a runtime
  download. The film is CC-BY-3.0; VibeCut code and this capture remain
  GPL-3.0-only. Blender Foundation does not endorse VibeCut.
  [ADR 0007](../../adr/0007-big-buck-bunny-documentation.md) records the narrow
  admission.

## Files and provenance

Each final file has a populated `.attribution` sidecar with the eight fields the
[asset checker](../../../xtask/README.md) requires. Embedded UI fonts/icons keep
their existing grants:

| Component | Grant and existing owner |
|---|---|
| VibeCut capture/code | GPL-3.0-only, [LICENSE](../../../LICENSE) |
| Big Buck Bunny excerpt | CC-BY-3.0, [licence text](../../licenses/CC-BY-3.0.txt) |
| Inter and Noto UI fonts | OFL-1.1, [Inter sidecar](../../../assets/fonts/Inter-Bold.ttf.attribution), [OFL text](../../../assets/fonts/OFL-Inter.txt) |
| Phosphor icons | MIT, [icon licence](../../../assets/icons/phosphor/LICENSE) |

The authored Vietnamese/English caption and title are demo edits, not recognized
or translated speech. No provider/model received the film.

| File | Bytes | SHA-256 | Content |
|---|---:|---|---|
| `hero.jpg` | 97039 | `94e18339c2939944c35a998bd810aa7f616e73208cdb623183b8aebb379128fe` | 960x600 editor still with the film, audio, title and bilingual caption |
| `timeline.gif` | 108685 | `971c15b1b69dbfb87514cff441a6d15fd755590ad27f51fb4812dd12578d4f5c` | 960x600, 80 frames at 10 fps, 100 ms each, 8.00 s, forever loop |
| `timeline.jpg` | 97240 | `24ea8f891526f08d306b49e1ce9c65b43e7117eaf4d590616080afc69edccaa7` | 960x600 poster at the trimmed state (frame 40) |

Generated at source commit `f8f2d6474d293bfb98ee046c8984362e487bcc2e`.
Measured source-vs-decoded RGB mean/max absolute error was 0.861/24 for
`hero.jpg`, 0.863/24 for `timeline.jpg`, and 0.791/101 for the palette-optimized
GIF frame 40. The per-run manifest is the authoritative receipt for a particular
capture.

## Regeneration

Prerequisites are the existing external FFmpeg/FFprobe oracle, the normal warm
MSYS2/Clang runtime DLLs, and the shared local excerpt. Capture refuses less than
8 GiB of scratch headroom, a missing/unreadable source or missing tools. There is
no synthetic fallback.

```sh
cargo xtask build -- cargo xtask ui-audit --readme hero --source /absolute/path/to/big-buck-bunny-12s-720p30.mp4
cargo xtask build -- cargo xtask ui-audit --readme timeline --source /absolute/path/to/big-buck-bunny-12s-720p30.mp4
```

`--readme` is a consumed mode of the existing [ui-audit example](../../../apps/vibecut/examples/ui_audit.rs);
ordinary scene selection is unchanged and `xtask ui_audit` only forwards the
arguments. The mode imports the explicit local excerpt through the real
`Editor`/`Session`/`Files` path, builds the multitrack project through real
commands and renders with the real offscreen editor UI at 1440x900 logical
pixels, dark English, scale 1. Output is 960x600; no native window or picker is
opened.

`hero` seeks the real preview to 3 seconds, selects the first video and captures
the settled four-panel editor. `timeline` records four settled states and holds
each for 20 output frames: initial multitrack project, split the first item at
4 seconds, trim the returned right item's end by 0.5 seconds, then undo. The
literal source ranges are independently asserted as `0..4 s`, `4..6 s`,
`4..5.5 s` after trim, and `4..6 s` again after undo; other tracks must keep
their ranges. The poster is the trimmed state.

After every edit the runner seeks the real snapshot at 3 seconds, polls until the
frame generation equals the current generation and the frame time equals the
requested tick, requires the presented tick to match, settles effect/thumbnail
work and captures. Raw PNG frames, source probe, tool identities, source/excerpt
hashes, revisions, project hashes, generation/tick per boundary and all 80 frame
mappings stay outside Git under the run's target `readme-media/<slot>/` subtree
(`manifest.json` beside the frames). Only the three final media files and their
sidecars are tracked.

## Independent output checks

The runner encodes with the existing external FFmpeg (no new image/GIF
dependency), then independently decodes/probes the result:

- GIF: 960x600, exactly 80 decoded frames, 0.100 s delay per frame, 8.00 s total,
  forever loop, at most 5,000,000 bytes. A palette pass is used only when the
  direct encode exceeds the size bound.
- JPG/GIF poster: 960x600, at most 512 KiB, non-blank decoded RGB, and measured
  source-vs-decoded mean/max channel error recorded in the manifest rather than a
  renderer-generated checksum.
- The run inspects opening, each action boundary and closing output frames before
  acceptance. Decoded source frames at 1 s and 3 s must differ, proving the preview
  is file-backed rather than a static placeholder.

## Remaining limits

This delivery fills only the hero and timeline slots. The other eight README
media slots remain comments until their own real, reproducible captures are
admitted. No MCP/agent chat, provider/transcription/translation run, Shorts model
run, native GPU/device result, export-performance claim or rendered-GitHub visual
acceptance is created here. The parent README-media work stays open for those
slots and any rendered-page evidence.
