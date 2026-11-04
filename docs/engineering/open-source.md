# Open-source policy

## Licensing

Project contributions use [MIT](../../LICENSE). Admit dependencies licensed under
MIT, Apache-2.0, BSD, ISC, Zlib or Unicode, or unmodified MPL-2.0; any other license
requires an ADR. Review license metadata when changing dependencies.

## Assets

Keep media, model weights and large binaries out of Git. Any admitted small asset
must be openly licensed and have its source and license recorded beside it.
[Verification](verification.md#checks) owns generated fixture placement.

## Privacy

Local-first is the default. Remote AI is opt-in: obtain explicit consent before
sending media or transcripts. Never log API keys or transcript text, and never
commit credentials or personal recordings. Review provider boundaries and error
paths for leaks. Use [private reporting](../../SECURITY.md) for vulnerabilities.
