# Scene split dialog

Interface: `State`, `Action`, `DetectSource`, `show`, `apply`.

`timeline.split_scene` (the video clip menu's Split scene) opens the scene split review for the single
selected video clip. Find runs the engine's `media.detect_scenes` job through control admission and
polls `job_status` each frame from the dialog's own runner; Cancel sends `job_cancel`. The review lists
each candidate with a checkbox (all checked by default); Split at N applies the checked indexes through
`scene_detection::apply_reviewed`, one undo step, and the dialog never edits the project. States: idle,
detecting, review, no cuts, cancelled, failed, applying. The media cache is injected by `Editor::set_media`.
Tests script the job through `DetectSource` (`set_detect_source`): a body held on a gate that runs
until the dialog cancels it, so Cancel is clicked while the job is provably running, and a body that
reports failure. The applying state is not asserted: `apply_reviewed` is synchronous, so the state paints
for one frame and commits in that same frame, and no test can observe it between frames.
