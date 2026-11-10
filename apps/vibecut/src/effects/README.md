# GPU effects

Interface: `GpuEffects`.

`GpuEffects` implements render's device-free effect interface on the eframe-owned wgpu
device and queue. Native startup attaches it to each preview player; headless players
use CPU. Shader entry names and ordered separable passes are data. Bindings expose the
current run, immutable instance input (binding 6), optional coverage and track-below working images. Every run uploads the same required instance input when it has Sample passes; geometry is validated before allocation. Blur does no colour or alpha
conversion.

Compute buffers, upload, pass metadata and readback are charged to the engine hooks;
native driver and pipeline allocations are not. Device limit, validation or readback
failure returns `Unsupported` for CPU fallback. Submitted work keeps its byte lease until
GPU completion, even if the bounded readback wait times out. No unsafe mapping or buffer
casts.
