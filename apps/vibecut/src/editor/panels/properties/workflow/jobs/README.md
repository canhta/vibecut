# Properties jobs

Interface: `footage::FootageWorkflow`, `footage::FootageStatus`, `loudness::LoudnessWorkflow`, `loudness::Failed`, `loudness::scope_target`, `lut::LutWorkflow`, `narration::NarrationWorkflow`, `narration::Action`, `narration::Failed`.

The narration workflow (`narration.rs`) submits the registered `dub.generate` command to its own runner with a human caller and a `ai_access::narration_access` (the run's `TaskEvents` consent prompt and the narrator, nothing else), the same path an agent or the CLI drives. It keeps the form, one run, one pending take and its fit decision, advances the job once per frame from the frame, reads the take from the retained job output, and writes the project only through `dub.place` and `dub.set_ducking`. A refusal, a decline, a provider failure or a cancel (`dub.cancel`) leaves the project untouched; selecting another item cancels the job. Placement passes `dub.place` the playhead as its start and the lowest audio track as its placement (`Track` when free for the take, `Below` otherwise), then selects the new clip; a plan-restricted voice is the job's `plan_required` code and reads as its own line naming the voice. Tests inject the speech opener and the take folder with `Editor::set_narration`, and seek the playhead through the playback service.

The three row-started engine jobs of the properties area, each with its own runner over the shared
process media access: the LUT import (`lut.rs`), the footage analyses, stabilize and person
matte (`footage.rs`), and the Audio tab's loudness measurement (`loudness.rs`). Each
submit through the registered control command with a human caller, poll once per frame from the
frame, publish a finished job through `job_status`, and write the project only through engine
commands. The behaviour of each is in the [workflow notes](../README.md).
