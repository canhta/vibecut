# Permissive dependency licences

Status: Accepted.

`cargo deny check` found five licences outside the admitted list in transitive
dependencies: MIT-0 (`borrow-or-share`, `ppmd-rust`), BSL-1.0 (`clipboard-win`,
`error-code`), 0BSD (`doctest-file`, `recvmsg`), bzip2-1.0.6 (`libbz2-rs-sys`)
and CC0-1.0 (`ppmd-rust`). Each is a permissive licence compatible with
GPL-3.0-only distribution: MIT-0 and 0BSD are MIT and BSD without the notice
requirement, BSL-1.0 asks for its notice only in source distributions, the
bzip2-1.0.6 licence is a BSD-style notice-and-disclaimer licence, and CC0-1.0
waives rights to the extent the law allows.

All five are admitted as dependency licences. They stay per-crate exceptions in
[deny.toml](../../deny.toml), so a new crate under one of them is still read
before it enters the graph. Notices travel with the bundle like every other
admitted licence ([open-source policy](../engineering/open-source.md#licensing)).

Rejected: replacing the parents (arboard, interprocess, zip, fluent-uri) to
avoid the licences, which would drop maintained crates for no licensing gain,
and adding the five to the global allow list, which would admit any future
crate under them unreviewed.
