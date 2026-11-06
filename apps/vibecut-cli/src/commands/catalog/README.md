# Catalog commands

`commands --json` returns one generated catalog object (`api_version`,
`registry_hash`, `limits`, `entries`) from the same engine declaration as dispatch;
`schema --json` returns the generated request, response and event envelope schemas
with the same identity. There is no CLI-side command table. The CLI test suite
generates both outputs twice, compares bytes, and checks the catalog digest and
generated command reference against the checked-in engine outputs.
