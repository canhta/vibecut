# GPU effects


`effects::GpuEffects` implements render's device-free effect interface using the
eframe-owned wgpu device/queue. Native startup attaches it to each preview player;
headless players default to CPU. Shader entry names and ordered separable passes
are data; adding shader entries does not change scene composition. Bindings expose
the item, optional coverage and track-below working images. Blur performs no
colour/alpha conversion. Compute buffers, upload, pass metadata and readback are
charged to the engine hooks; native driver/pipeline allocations remain outside
that accounting. Device limit/validation/readback failure returns Unsupported for
CPU fallback. Submitted work retains its byte lease until GPU completion, even
if the bounded readback wait times out. No unsafe mapping or buffer casts are used.
`tests/it/effects.rs` compares f32 CPU/GPU output with max absolute tolerance 2e-5.
