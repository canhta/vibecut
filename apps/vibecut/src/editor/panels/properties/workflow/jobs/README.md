# Properties jobs

Interface: `footage::FootageWorkflow`, `footage::FootageStatus`, `lut::LutWorkflow`.

The two row-started engine jobs of the properties area, each with its own runner over the shared
process media access: the LUT import (`lut.rs`) and the stabilize analysis (`footage.rs`). Both
submit through the registered control command with a human caller, poll once per frame from the
frame, publish a finished job through `job_status`, and write the project only through engine
commands. The behaviour of each is in the [workflow notes](../README.md).
