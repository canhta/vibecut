# GPU effects

Interface: `GpuEffects`.

`GpuEffects` implements render's device-free effect interface on the eframe-owned wgpu
device and queue. Native startup attaches it to each preview player; headless players
use CPU. Shader entry names and ordered separable passes are data. Bindings expose the
current run and immutable instance input (binding 6). The closed shader set reads
track-below only for blend and instance input only for mirror; it never reads coverage.
Gamma and Invert read only the current run. Their ordered uniforms are `[gamma]`
and `[channel, blend]`. They share the CPU reference's straight linear to encoded
sRGB conversion, operation and decoded premultiplication. Invert's Alpha choice
changes coverage and re-premultiplies the original straight colour. Other new
pixel entries remain unsupported on this adapter and use the render CPU path.
All supplied input geometry is validated before allocation, including unused inputs.
Unused read-only bindings follow the current ping-pong buffer; the writable output
and any required auxiliary snapshot remain distinct. Blur does no colour or alpha conversion.

`frame` acquires one nonblocking adapter permit before any scene source/output work.
Contention chooses CPU for that entire frame. Standalone calls use the same scoped
implementation. Each scope fixes its first geometry and exact borrowed hooks object.
No device or GPU dependency crosses the render interface.

Mapped uploads make all staging explicit: work A/B, readback and upload-current cost
4I; Sample/blend add a distinct auxiliary/upload pair for 6I. Non-Sample calls drop
the auxiliary pair. Metadata has distinct slots per pass, charged at 1024 bytes/pass
(up to 512), with at most 49 values each; actual fixed scope records are charged
separately within 4096 bytes. Caller and result Images reserve independently. These
bounds exclude native driver/pipeline memory and do not guarantee export admission.

Completed scratch is reused only within its scope. CPU runs, masks, transforms,
transitions and the next decode release idle capacity. Submitted completion owns
buffers, leases and the permit through timeout/unwind; a pending submission cannot
be recycled. Completion is serviced by native polling, with no detached poller. A
30-second wait and total one-second mapping receipt ceiling return Unsupported;
Budget, Cancelled and Invalid propagate without publishing a backdrop. A driver
that never confirms completion retains bounded charged quarantine. No unsafe casts.
