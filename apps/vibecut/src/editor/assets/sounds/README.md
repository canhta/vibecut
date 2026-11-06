# Sounds tab


The assets Sounds tab consumes engine's sound library: Discover searches the
provider with a credential and otherwise searches the saved local library;
Saved always reads the local library and works offline. Search requests use the
commercial-only filter by default, and the switch reruns the search with the
exact filter omitted. Every result and saved row shows the author, duration,
licence and precomputed attribution text.

Save and unsave are local-library operations; they never enter project history.
Preview uses the engine's play/stop seam and an app-owned cpal sink that keeps
one stream on a dedicated thread. Add copies the cached preview into
project-owned media and applies the registered `sounds.add_to_timeline` command
as one undo step. An uncached add first submits the shared cache job, shows its
progress with Cancel, and adds only after the complete preview publishes.
Auth, quota, rate-limit, network, server, malformed-response, size, publication
and no-audio-device failures are typed states with creator copy; no token is a
local-only state with the Freesound terms decision stated once. The panel never
mutates Project JSON except through the add command.

`tests/it/sounds_ui.rs` drives the real widgets through fixture JSON/download
transports and a fake preview sink; it covers search, the commercial filter,
save, the Saved tab, preview play/stop, no device, empty results, provider
errors, cache progress/cancel, publication failure and add. The offscreen
screenshot is written under `target/`.
