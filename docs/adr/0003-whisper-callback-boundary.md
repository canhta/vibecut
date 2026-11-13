# Whisper callback boundary

Admit unsafe code only in `ai::recognition::whisper::callbacks` to install whisper-rs's raw
abort/progress callbacks. Released safe setters have an invalid erased-closure
cast and lose callback ownership; using them cannot provide reliable cancellation.
Do not fork or vendor the binding. Replace this module with the safe binding API
when a corrected stable release supplies equivalent ownership guarantees.

The call frame owns an Arc of atomic cancellation/progress state until synchronous
inference returns. ABI-matched trampolines access only those atomics and catch
unwinding panics. No native context/state pointer is dereferenced. Parameters do
not escape or clone. A scoped observer bridges borrowed execution hooks to the
atomic state, observes cancellation during native work, and joins before return;
engine continues to own job scheduling and visible Cancelling state.
The observer acknowledges cancellation immediately via execution hooks, suppresses
further progress, then waits for native work to drain to its next abort poll.
The bundled runtime cannot interrupt an encoder graph; the accepted residual is
one encoder window, with backend-specific observed limits and regression margins
recorded in the ai README. Native cancellation returns the existing
typed Cancelled error and never releases partial recognition.

All other ai modules deny unsafe code. Current safety rules live in
[source standards](../engineering/standards.md); the adapter contract and
verification requirements live in the [ai README](../../crates/ai/README.md).
