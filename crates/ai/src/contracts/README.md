# Provider contracts

`ProviderConfig::new` and serde validate schema version 1, nonempty provider,
model/settings identity and tagged `location` (`kind: local` or `remote`). Remote
URLs require HTTPS and reject embedded credentials, query and fragments. Backend
names are extensible data. `auth_ref` is an external secret-store reference, never
a key; callers must not put secrets in configuration. The settings fingerprint
must include every option affecting recognition. Configuration serializes for
persistence but Debug redacts endpoint, model reference and auth reference through
`NfcText`. Tokens cannot be serialized or deserialized.

`Capabilities` describes an exact model, mode, operation and locale/script. No
language list or implicit locale fallback is built into this crate. Timing units
are a resolved set of supported segment/word/character/grapheme/token capabilities;
unresolved conditional timing is omitted by the adapter. Other feature flags
explicitly distinguish supported/unsupported/unknown/conditional. Timing evidence
is advertised, fixture-verified or live-verified, with a revision. Presence does
not prove accuracy. Upload limits are direct invocation bytes and duration in
integer microseconds; `None` means unknown, never unlimited or zero. Current
execution passes complete mono audio windows; streaming flags describe adapters
without introducing a streaming job protocol. Unsupported capabilities and locale
pairs fail before execution.
