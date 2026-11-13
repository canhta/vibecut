# Capture

Interface: `Capture::request`, `Capture::poll`.

`request` queues a fresh GPU readback during a UI frame after the command or
inspection of interest, and refuses a zero or over-five-second timeout or a
non-positive region; `poll` waits without blocking until that deadline. Window and panel
rectangles are supported. A fully occluded or minimized window may time out; unavailable
and timed-out integrations return `CaptureUnavailable` and `CaptureTimeout`. Captures
show presentation, never command success.
