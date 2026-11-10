# Session

Leased control lifetime, shared export admission, runtime settings.

`run_leased` owns the one-shot project rule: acquire the writer lease, load, restore sidecar metadata, run one closure over the session, then publish once. Every `run_leased` command except `exec project.save` and `sounds add` uses `WritePolicy::Changed`: save when the project bytes changed, call `Session::saved` after that save, and touch metadata when the command reported a change. `exec project.save` and `sounds add` keep their observed `WritePolicy::Always` rule: republish project and metadata without touching metadata or advancing the save boundary. `cut` keeps its 100 undo snapshots; the other commands keep 1.

`registry::run` sends one command through `Session::control_typed` for the adapters that have no project file (fonts and the sound library), with the access objects in `Lent` and the typed refusal kept for the CLI's exit codes. `libraries::with_fonts` and `libraries::with_sounds` build those access objects from the per-user font store, the opened library root and `FREESOUND_API_KEY`; a command parameter never carries a library path, a token or a source path.
