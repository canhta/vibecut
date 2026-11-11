# README media

`hero.jpg`, `timeline.gif` and `timeline.jpg` are original offscreen captures of
the real VibeCut editor. They are documentation postprocessing outputs, not app
export formats and not a product performance benchmark.

## Files and provenance

Each final file has a populated `.attribution` sidecar with the eight fields the
[asset checker](../../../xtask/README.md) requires. The author is the
VibeCut contributor capture, the media licence is the project's
[GPL-3.0-only](../../../LICENSE), and the embedded UI fonts/icons retain their own
existing grants:

| Component | Grant and existing owner |
|---|---|
| VibeCut application/project capture | GPL-3.0-only, [LICENSE](../../../LICENSE) |
| Inter and Noto UI fonts | OFL-1.1, [Inter sidecar](../../../assets/fonts/Inter-Bold.ttf.attribution), [OFL text](../../../assets/fonts/OFL-Inter.txt) |
| Phosphor icons | MIT, [icon licence](../../../assets/icons/phosphor/LICENSE) |

The synthetic source is generated fresh for each run; no personal recording,
third-party mark or reference-product picture is embedded. The capture shows
authored Vietnamese/English cue text, not recognized or translated speech.

| File | Bytes | SHA-256 | Content |
|---|---:|---|---|
| `hero.jpg` | 83466 | `d0ad60da7b8580eb5294d7061e3e6f139cb7ba22b8115577dfc91ac0fbb4ed66` | 960x600 editor still with file-backed video, sine audio, title and bilingual caption |
| `timeline.gif` | 72004 | `570d8367999e6498759894894a6491e388befbc6b69c2a3cafb7d08cc5a1e78a` | 960x600, 80 frames at 10 fps, 100 ms each, 8.00 s, forever loop |
| `timeline.jpg` | 85749 | `db12b25d87bb8db7b39032197dfe89b91645e70ffd787ccbd3903eeabc10507a` | 960x600 poster at the trimmed state (frame 40) |

These values are refreshed by the regeneration command below; the per-run
manifest is the authoritative receipt for a particular capture.

## Regeneration

Prerequisites are the existing external FFmpeg/FFprobe oracle and the normal
warm MSYS2/Clang runtime DLLs used by the other oracle-backed checks. Capture
refuses less than 8 GiB of scratch headroom and refuses missing tools.

```sh
cargo xtask build -- cargo xtask ui-audit --readme hero
cargo xtask build -- cargo xtask ui-audit --readme timeline
```

`--readme` is a consumed mode of the existing [ui-audit example](../../../apps/vibecut/examples/ui_audit.rs);
ordinary scene selection is unchanged and `xtask ui_audit` only forwards the
arguments. The mode generates a 12-second 1280x720/30 fps `testsrc2` source with a
997 Hz sine (`libx264`/`yuv420p`/gop 30/BT.709 plus mono AAC), imports it through
the real `Editor`/`Session`/`Files` path, builds the multitrack project through
real commands and renders with the real offscreen editor UI at 1440x900 logical
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
work and captures. Raw PNG frames, the source probe, tool identities, revisions,
project hashes, generation/tick per boundary and all 80 frame mappings stay
outside Git under the run's target `readme-media/<slot>/` subtree (`manifest.json`
beside the frames). Only the three final media files and their sidecars are
tracked.

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
