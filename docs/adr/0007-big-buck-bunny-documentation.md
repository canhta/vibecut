# Big Buck Bunny documentation and fixture use

Status: Accepted.

The README uses a credited 12-second excerpt of Big Buck Bunny (Blender
Foundation, 2008, CC-BY-3.0) for the hero image and the split/trim/undo timeline
sequence. The maintainer chose recognisable real footage over a test-pattern demo;
the remaining README slots stay open.

Only documentation media under `docs/assets/readme/` and explicit local media
smoke/integration fixtures may use this excerpt. The film, source master, excerpt
files, projects and raw frames stay out of Git and out of the product bundle. The
shared excerpt is published separately as the versioned
[fixtures-big-buck-bunny-2026-11-11](https://github.com/canhta/vibecut/releases/tag/fixtures-big-buck-bunny-2026-11-11)
fixture prerelease. It is not built-in content, a library item, a model/provider
input, or an offer to download the full film at runtime.

The required excerpt credit is `(c) copyright 2008, Blender Foundation /
www.bigbuckbunny.org`. The unmodified licence text is
[CC-BY-3.0](../licenses/CC-BY-3.0.txt), linked from the media sidecars and
[asset README](../assets/readme/README.md). VibeCut code and the capture itself
remain GPL-3.0-only; the film is not relabelled as project-owned or GPL-only, and
Blender Foundation does not endorse VibeCut.

Rejected: synthetic-only README footage (less legible product demo), bundling the
film or excerpt as a Git blob or app asset, treating CC-BY-3.0 as a blanket asset
licence, and downloading the film automatically at runtime.
