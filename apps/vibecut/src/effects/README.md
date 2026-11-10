# GPU effects

Interface: `GpuEffects`.

`GpuEffects` implements render's device-free effect interface on the eframe-owned wgpu
device and queue. Native startup attaches it to each preview player; headless players
use CPU. Shader entry names and ordered separable passes are data. Bindings expose the
current run and immutable instance input (binding 6). The closed shader set reads
track-below only for blend and instance input only for mirror; it never reads coverage.
All supplied input geometry is validated before allocation, including unused inputs.
Unused read-only bindings follow the current ping-pong buffer; the writable output
and any required auxiliary snapshot remain distinct. Blur does no colour or alpha conversion.

Non-Sample calls allocate three image buffers and upload one image; Sample or blend
calls allocate four and upload two. The transient reservation is four or five times
`Image::bytes`, respectively, including one sequential packing vector, plus 1024 bytes
per pass. Caller images and the validated result Image hold separate reservations.

Compute buffers, upload, pass metadata and readback are charged to the engine hooks;
native driver and pipeline allocations are not. Device limit, validation or readback
failure returns `Unsupported` for CPU fallback. Submitted work keeps its byte lease until
GPU completion, even if the bounded readback wait times out. No unsafe mapping or buffer
casts.
