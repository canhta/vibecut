# Library commands

`sounds` is an adapter over the [engine sound library](../../../../../crates/engine/src/libraries/sound_library/README.md#sound-library).

- `sounds search --library <dir> [--page N] [--page-size N] [--all-licences] [query]` uses `FREESOUND_API_KEY` when present, otherwise searches only saved local sounds. Results carry author, licence, attribution and URLs.
- `sounds save <library> <sound-json>` and `sounds unsave <library> <id>` manage the durable local library.
- `sounds add <project> <library> <id> [--start <ticks>]` requires the exact preview URL to be cached locally and never downloads implicitly. It commits the asset and audio item as one undo step through `sounds.add_to_timeline`, copying the sound into `<project>.media/`.

Missing cache, missing credential, auth, quota, rate-limit, network and no-audio-device failures keep distinct stable codes. No command prints or persists the token.
