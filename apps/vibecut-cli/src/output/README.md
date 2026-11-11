# Output

Process results, progress presentation, upload disclosures.

CliConsent implements the provider-owned ConsentPrompt result without changing disclosure or
approval scope. Stderr write failures remain intentionally ignored; no IO cause is introduced.
Mixed CLI workflows compose provider failures through the engine aggregate, and diagnostics
borrow raw or aggregate provider setup refusals for the existing model-download hint.

Acquisition diagnostics retain raw or engine-composed Transcript causes. The existing setup
hint borrows an explicit Transcript Providers ModelMissing cause without source-chain search.
