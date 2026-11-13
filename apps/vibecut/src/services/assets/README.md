# Asset intake

Interface: `music`, `voiceover`, `freeze`.

Services that bring a new source asset into the project from outside the timeline. Each
publishes its file into the project media folder and hands the asset to the engine; the
dialogs and panels that drive them live in the editor.

| Module | Holds |
|---|---|
| `music` | One process-shared music library and its in-memory session credential; `run_music` sends the registry's `music.*` commands. |
| `voiceover` | [Voice-over capture](voiceover/README.md): the `InputHost` microphone seam, its cpal host, the bounded queue and encoder worker, and atomic take promotion. |
| `freeze` | The freeze-frame request adapter: `timeline.freeze_frame` through control admission, with the trusted caller, runner and protected paths. |
