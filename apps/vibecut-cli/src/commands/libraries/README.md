# Library commands

`sounds` and `fonts` are adapters over the registry commands the [libraries owners](../../../../../crates/libraries/src/lib.rs) implement; the CLI opens the stores and lends them through the control caller.

- `fonts list [--query <text>]`, `fonts resolve <family>` `fonts download <family> --accept-license` and `fonts import <file> --accept-license` run `fonts.list`, `fonts.load`, `fonts.download` and `fonts.import` against the per-user font store. The font catalog owns validation, atomic custody and load states.
- `sounds search --library <dir> [--page N] [--page-size N] [--all-licences] [query]` runs `sounds.search`; the composition root opens the library and reads `FREESOUND_API_KEY`, so neither path nor token is a command parameter. Without a credential search returns saved local sounds. Results carry author, licence, attribution and URLs.
- `sounds save <library> <sound-json>` and `sounds unsave <library> <id>` run `sounds.save` and `sounds.unsave` against the same rented library.
- `sounds add <project> <library> <id> [--start <ticks>]` requires the exact preview URL to be cached locally and never downloads implicitly. It commits the asset and audio item as one undo step through `sounds.add_to_timeline`, copying the sound into `<project>.media/`.

Missing cache, missing credential, auth, quota, rate-limit, network and no-audio-device failures keep distinct stable codes. No command prints or persists the token. The three `sounds.*` library commands are `grant = Human`, so an agent transport refuses them.
