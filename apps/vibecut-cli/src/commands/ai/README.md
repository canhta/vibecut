# AI commands

`subtitle`, `providers` and
`model` are adapters over the [engine subtitle workflow](../../../../../crates/engine/src/subtitles/README.md#subtitle-workflow):
the CLI parses flags, prints local/remote capabilities and upload disclosures, and
never implements chunking, consent/cost, transcription, translation, checkpoint or
serialization logic. `subtitle` has no graceful signal-to-job cancellation handler;
process interruption follows crash semantics, and completed ASR windows remain
reusable through the content-addressed cache. Local models are never downloaded
implicitly; `model download` requires `--yes-download` after printing the exact
licence, source, size and hashes. A completed export/subtitle run reports its
output paths, not render's detailed receipt.
`clips` is an adapter over the [shorts suggestion workflow](../../../../../crates/shorts/src/suggestion/README.md):
it parses suggestion, ASR, translation and export flags, prints the provider's
upload scope before any remote byte leaves the machine, and maps candidates or
typed failures to stdout/stderr. It owns no segmentation, prompt, selection,
snapping or export logic; `--provider` never falls back. Remote suggestion
credentials are refused before media work, and `providers --operation clips`
lists the same offline catalog without opening an adapter. Boundary adjustment
kinds, including sub-frame `word_fade`, follow the [CLI contract](../../../../../docs/product/agent-control/cli.md).
