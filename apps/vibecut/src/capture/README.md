# Capture

`Capture::request` queues a fresh GPU readback and `poll` waits without blocking
for at most five seconds; window and panel rectangles are supported. Unavailable
or timed-out integrations return typed failures. Captures demonstrate presentation,
never command success; inspect engine state separately. Fully occluded/minimized
windows may time out rather than return an old framebuffer.
