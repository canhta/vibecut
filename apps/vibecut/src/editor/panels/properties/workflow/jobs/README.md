# Properties jobs

Interface: `footage::FootageWorkflow`, `footage::FootageStatus`, `loudness::LoudnessWorkflow`, `loudness::Failed`, `loudness::scope_target`, `lut::LutWorkflow`, `narration::NarrationWorkflow`, `narration::Action`, `narration::Failed`.

The narration workflow (`narration.rs`) is not an engine job: the Text to speech tab runs `Narrator::prepare`, `approve` and `synthesize` on its own worker thread, with the shared `TaskEvents` consent sheet and a `CancelFlag`, because the take is ordinary audio the panel places itself. It keeps the form, one run, one pending take and its fit decision, polls the worker once per frame from the frame, and writes the project only through `dub.place` and `dub.set_ducking`. A refusal, a decline, a provider failure or a cancel leaves the project untouched; dropping a run cancels its consent wait and its provider call. Tests inject the speech opener and the take folder with `Editor::set_narration`.

The three row-started engine jobs of the properties area, each with its own runner over the shared
process media access: the LUT import (`lut.rs`), the footage analyses, stabilize and person
matte (`footage.rs`), and the Audio tab's loudness measurement (`loudness.rs`). Each
submit through the registered control command with a human caller, poll once per frame from the
frame, publish a finished job through `job_status`, and write the project only through engine
commands. The behaviour of each is in the [workflow notes](../README.md).
