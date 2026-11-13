# Properties jobs

Interface: `footage::FootageWorkflow`, `footage::FootageStatus`, `loudness::LoudnessWorkflow`, `loudness::Failed`, `loudness::scope_target`, `lut::LutWorkflow`.

The three row-started engine jobs of the properties area, each with its own runner over the shared
process media access: the LUT import (`lut.rs`), the footage analyses, stabilize and person
matte (`footage.rs`), and the Audio tab's loudness measurement (`loudness.rs`). Each
submit through the registered control command with a human caller, poll once per frame from the
frame, publish a finished job through `job_status`, and write the project only through engine
commands. The behaviour of each is in the [workflow notes](../README.md).
