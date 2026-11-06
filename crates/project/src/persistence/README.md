# Persistence and limits

`encode`/`decode` use one JSON envelope: `format`, `schema_version`, `generator`,
`project`. Current schema 20 adds `canvas.preset` and `canvas.last_custom_size`; schema 19 splits the text background `padding` into `padding_x`/`padding_y` and adds its `offset_x`/`offset_y` plus text `align` and `decoration`; schema 18 added item `retime` (speed, pitch policy and the retained processing window of split pieces) and ordered transition split pieces; schema 17 added `settings.loudness` (clip/master targets and the last measured gains) to schema 16 timeline `transitions`, which followed schema 15 audio/video clip `fades`, schema 14 optional imported cue style, schema 13 text weight, italic, letter spacing and line height (also on text masks), and schema 12 audio `source` (upload or library provenance), volume/mute and master controls.
ProjectId is stored as a canonical decimal string, preserving
all u64 identity bits for control clients; other IDs remain numeric. Schemas 1-19
are refused rather than migrated or silently defaulted. Older schemas return `TooOld`, newer schemas `TooNew`; there are no
migration or compatibility paths. Unknown current-schema project/envelope fields
are refused. `WriterLease::save` serializes completely before touching the destination;
read-only `load` reads at most 16 MiB + 1 byte, including when a file grows while reading.

Ingestion and serialization cap JSON at 16 MiB, nesting at 64, individual raw
JSON strings at 1 MiB and combined comma/colon separators at 200,000, including
ignored fields. Those aggregate budgets can be reached before individual model
caps: 10,000 assets, 256 tracks, 100,000 total items/bookmarks, 100,000 cues per
caption item, and 1 MiB per stored text. Exceeding a budget returns an error;
no truncation or silent repair occurs. JSON diagnostics expose line/column only,
without user content. Raw serde use does not apply the ingestion budgets.
