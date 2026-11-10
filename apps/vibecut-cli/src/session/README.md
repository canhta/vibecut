# Session

Leased control lifetime, shared export admission, runtime settings.

`run_leased` owns the one-shot project rule: acquire the writer lease, load, restore sidecar metadata, run one closure over the session, then publish once. Every command except `exec project.save` and `sounds add` uses `WritePolicy::Changed`: save when the project bytes changed, call `Session::saved` after that save, and touch metadata when the command reported a change. `exec project.save` and `sounds add` keep their observed `WritePolicy::Always` rule: republish project and metadata without touching metadata or advancing the save boundary. `cut` keeps its 100 undo snapshots; the other commands keep 1.
