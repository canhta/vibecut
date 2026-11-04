# Open-source policy

## Licensing

Project contributions use [MIT](../../LICENSE). Admit dependencies licensed under
MIT, Apache-2.0, BSD, ISC, Zlib or Unicode, or unmodified MPL-2.0; any other license
requires an ADR. Review license metadata when changing dependencies.

## Assets

Keep media, model weights and large binaries out of Git. Admitted small assets
carry `<file>.attribution` sidecars with nonempty `asset`, `title`, `author`, `source`,
`license`, `license-file`, `added` and `notes` fields; READMEs and sidecars are exempt.
`added` identifies the admitting task, not a date. License paths are repository-relative.
Admitted font assets may use OFL-1.1; ship the OFL text, attribution sidecar and, for a modified
subset, the pinned regeneration procedure and hashes. Project-owned brand/icon sources and
committed exports are admitted small assets;
MIT applies. App/window icons use
[assets/app-icon](../../assets/app-icon/README.md); [brand vectors](../brand/logo.svg)
serve documentation. Regeneration and planned platform wiring live in the icon README.
[Verification](verification.md#checks) owns generated fixture placement.

## Model weights

Never bundle model weights in the app or installer; keep downloads small. The user
starts every download from the model manager. Suggest as defaults only weights under
MIT, Apache-2.0 or BSD licences. Other weights (CC-BY, custom or non-commercial) are
opt-in: show the exact licence, source and restrictions such as "non-commercial"
before download. Verify the pinned hash and never redistribute weights with the app.

## Privacy

Local-first is the default. Remote AI is opt-in: obtain explicit consent before
sending media or transcripts. Never log API keys or transcript text, and never
commit credentials or personal recordings. Review provider boundaries and error
paths for leaks. Use [private reporting](../../SECURITY.md) for vulnerabilities.
