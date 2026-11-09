# Capture

`Capture::request` queues a fresh GPU readback; `poll` waits without blocking for at most five seconds. Window and panel rectangles are supported. Unavailable or timed-out integrations return typed failures; a fully occluded or minimized window may time out. Captures show presentation, never command success.
