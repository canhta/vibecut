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
changes coverage and re-premultiplies the original straight colour.
Black & White, Channel Mixer and Two-Tone also read only the current run and
preserve alpha. Black & White has no uniforms and computes linear Rec.709 luma.
Channel Mixer takes three encoded-sRGB rows of `[r,g,b,constant]`, clamped before
decoding. Two-Tone takes encoded black RGB, white RGB and amount; it maps encoded
linear luma, mixes in encoded space and decodes once.
Levels takes `[input black, input white, output black, output white, exponent]`;
its encoded remap floors only the input span, retaining equal/reversed bounds.
Strobe takes encoded RGB and original-picture fraction, decodes the colour once
and mixes in linear light. Both read only current input and preserve alpha.
Strobe time gating stays entirely in the existing effect evaluator, including
idle no-pass phases. Remaining pixel entries stay on the render CPU path.
All supplied input geometry is validated before allocation, including unused inputs.
Unused read-only bindings follow the current ping-pong buffer; the writable output
and any required auxiliary snapshot remain distinct. Blur does no colour or alpha conversion.

`frame` acquires one nonblocking adapter permit before any scene source/output work.
Contention chooses CPU for that entire frame. Standalone calls use the same scoped
implementation. Each scope fixes its first geometry and exact borrowed hooks object.
No device or GPU dependency crosses the render interface.

Export attachment follows the [export dialog policy](../editor/dialogs/export/README.md).

Mapped uploads make all staging explicit: work A/B, readback and upload-current cost
4I; Sample/blend add a distinct auxiliary/upload pair for 6I. Non-Sample calls drop
the auxiliary pair. Metadata has distinct slots per pass, charged at 1024 bytes/pass
(up to 512), with at most 49 values each; actual fixed scope records are charged
separately within 4096 bytes. These bounds exclude native driver/pipeline memory and
do not guarantee export admission.

Device geometry support and [aggregate byte admission](../../../../crates/engine/src/runtime/README.md#background-jobs-and-byte-policy)
are separate: supported dimensions can still refuse once caller/result images,
decoding and encoding consume the shared budget.

Completed scratch reuses buffer allocations only within its frame scope, not image
contents. Every apply/composite still packs and uploads its inputs, submits and waits
for native work, reads the output back and constructs a separately charged CPU Image.
CPU runs, masks, transforms, transitions and the next decode release idle capacity.
Submitted completion owns
buffers, leases and the permit through timeout/unwind; a pending submission cannot
be recycled. Completion is serviced by native polling, with no detached poller. A
30-second wait and total one-second mapping receipt ceiling return Unsupported;
Budget, Cancelled and Invalid propagate without publishing a backdrop. A driver
that never confirms completion retains bounded charged quarantine. No unsafe casts.
