# Canvas bounds

Names, notes and text use captions' NFC text owner. Canvas frames must have exact,
positive integer tick durations. Canvas area is at most 2^26 pixels; output audio
admits 8–192 kHz. Audio/video source volume and master volume admit -60..20 dB on a 0.01 dB grid; loudness targets admit -50..-5 LUFS and -20..0 dBTP on the same grid, and measured gains -60..60 dB for at most 4,096 sorted clips; font sizes admit 1..4096 pixels.

`Canvas.preset` records the chosen ratio preset (`original`, `16:9`, `4:3`,
`2:1`, `9:16`, `1:1`, `3:4`); `None` means a custom size. Fixed presets use a
1920 long edge: 16:9 = 1920x1080, 4:3 = 1440x1080, 2:1 = 1920x960, 9:16 =
1080x1920, 1:1 = 1080x1080 and 3:4 = 1080x1440. `original` resolves the retained
`original_canvas_size`. `CanvasSize::rounded_even` gives the nearest even size
on each odd axis (ties up) so encoders can open the frame: one pixel becomes
two, and an odd maximum that would overflow or exceed the area bound refuses.
The first-media capture of `original_canvas_size` and project load apply the same
rounding (load repairs older odd canvases instead of refusing the file).
`Canvas.last_custom_size` remembers the last committed custom width/height.
Both fields are schema 20. Preset labels are UI text; the
stored width/height remain authoritative.
