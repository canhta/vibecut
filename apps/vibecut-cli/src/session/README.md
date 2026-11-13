# Session

Leased control lifetime, shared export admission, runtime settings.

`run_leased` owns the one-shot project rule: acquire the writer lease, load, restore sidecar metadata, run one closure over the session, then publish once. Every `run_leased` command except `exec project.save` and `sounds add` uses `WritePolicy::Changed`: save when the project bytes changed, call `Session::saved` after that save, and touch metadata when the command reported a change. `exec project.save` and `sounds add` keep their observed `WritePolicy::Always` rule: republish project and metadata without touching metadata or advancing the save boundary. `cut` keeps its 100 undo snapshots; the other commands keep 1.

`registry::run` sends one command through `Session::control_typed` for the adapters that have no project file (fonts and the sound library), with the access objects in `Lent` and the typed refusal kept for the CLI's exit codes. `libraries::with_fonts` and `libraries::with_sounds` build those access objects from the per-user font store, the opened library root and `FREESOUND_API_KEY`; command parameters never carry a library root or token; human-granted font import carries only its chosen source path until the job consumes it.

`libraries::prepare_project_libraries` loads only referenced custom text/caption families
from the same verified font store before control rendering and export, and lends the sticker
cache to the renderer so an export draws downloaded emoji and icons and refuses ones that are
not downloaded. Opening a project never downloads or changes its authored family key.

`runtime::vision_access` (over `shorts::installed_faces`) lends the local YuNet face detector behind `reframe.generate`; `runtime::vision_and_segmentation` also returns the person segmenter (`shorts::installed_person_matte`) over the same lookup, which the control session lends to its `MediaAccess` for `footage.analyze {kind: person_matte}`.
It reads the pinned `yunet-2026may` bundle from the shared verified model cache and never
downloads; a missing bundle becomes the typed setup refusal naming `model download`.
The one-shot and control adapters and the headless MCP inbox lend the same access object,
so the command family exists only once in the registry.

The control and headless MCP roots lend optional local `CaptionAccess` with `DenyUploads`.
Their setup closure returns the Subtitles result and retains its typed setup refusal.
Shared runtime construction retains setup refusals for caption commands without stopping
ordinary control work; provider setup and checkpoint ownership follow the
[caption contract](../../../../crates/engine/src/subtitles/README.md#caption-workflow-commands).

Control JSON parsing and request-envelope construction return the wire owner's
`ControlError`. The CLI retains their typed category for framed diagnostics and
composes them into the engine error only at mixed workflow boundaries; command and
batch byte limits retain their existing labels and exit policies.

Export JSON includes bounded `word_highlight_warnings` from the shared scene/geometry preflight. Registry job admissions carry the same typed warning data; the one-shot adapter prints only that safe data on stderr. SRT/VTT serializers remain unchanged.
