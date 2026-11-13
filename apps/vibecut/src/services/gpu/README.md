# GPU adapters

Interface: `capture`, `effects`.

What touches the eframe-owned wgpu device and queue. `effects` implements render's device-free effect interface and is attached to preview players and export; `capture` reads back one fresh frame for evidence. Neither knows a window.

| Module | Contract |
|---|---|
| `capture` | [Capture](capture/README.md) |
| `effects` | [GPU effects](effects/README.md) |
