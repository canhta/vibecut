# Sound library

## Sound library

`sounds.search` calls `GET https://freesound.org/apiv2/search/` with `sort=downloads_desc`, bounded `page`/`page_size`, explicit `fields`, and `filter=license:("Creative Commons 0" OR "Attribution")` while the commercial-only switch is on. NC/Sampling+ and undocumented licence values are dropped even if the server ignores the filter. Author, licence, attribution, source page and preview URL travel on every result.

Credentials are bring-your-own: `Token::from_env` reads `FREESOUND_API_KEY` in the composition root, and the app may inject an OS-keychain token. The token is held in memory, never serialized, never placed in a URL, and Debug is redacted. Without a token search degrades to the saved local library; adding a saved sound never requires the network. API errors distinguish `NoCredentials`, `Auth`, `Quota`, `RateLimit { retry_after_seconds }`, `Network`, `Server`, `InvalidResponse`, `Limit`, `NoAudioDevice`, `NotCached`, `LibraryCorrupt` and `Cancelled`; the CLI maps them to stable codes.

`sounds.search`, `sounds.save` and `sounds.unsave` are registry commands over the library the composition root lends as `Caller::sounds`. The access object owns the root path and the token, so neither is a command parameter; search runs remote when the token is present and local otherwise. They are read-effect library operations, never Session edits: they never enter undo and a caller without the access refuses `Unsupported`. All three declare `grant = Human`, so an agent transport cannot operate the human's library. `sounds.add_to_timeline` is the registered edit (one undo step).

`SoundLibrary` owns a versioned `library.json` plus URL-keyed preview bytes under the app-supplied root. Saving is an idempotent upsert; unsave and clear-all remove unreferenced previews. A project-owned writer lease excludes simultaneous handles and processes for the same root; sharing uses one `Arc<SoundLibrary>`. A failed library-file publication preserves the in-memory saved list. Preview bytes are staged in the cache directory and probed before publication: direct stores publish through project's destination-admitting atomic writer, while job downloads rename through the job's publication fence. The cache key includes the exact preview URL, so a changed URL is a miss and stale bytes are removed when no saved sound still cites them. Search responses are capped at 1 MiB, library files at 1 MiB and 1,024 entries, previews at 32 MiB.

`SoundDownloads` streams through the runner (`JobKind::SoundDownload`, network retries) with HTTPS-only bounded redirects, content-length checks, cancellation before every read and write, and atomic publication; no credential is sent to preview CDN URLs. One download owns the library
through queue and backoff; another submission composes engine's
[`RuntimeError::CacheBusy`](../../../engine/src/runtime/README.md#background-jobs-and-byte-policy)
through `engine::control::Error::Runtime`. Cancelling or finishing the job releases that ownership.

`sounds.add_to_timeline` takes an asset already in project-owned media and commits it plus the audio item in one undo step; the item stores `sourceType=library` metadata. The `sounds::add_to_timeline`/`add_saved` helpers first copy the cached preview into the caller's project media directory under its content hash (staged, hash-verified, then published through project's destination-admitting atomic writer; reused when present), so unsave, clear-all and URL invalidation only remove cache entries, never a file a project references. The CLI uses `<project file>.media/`; managed projects pass their own `media` directory.

`SoundPreview` composes the cache/download owner with an app-owned `PreviewSink`;
play returns only at terminal drain/stop/failure, keeping active cancellation
through playback. Its caller cancellation predicate also refuses a worker stopped
before entry, before cache/network IO. Stop is idempotent and must interrupt
decoding as well as playback; the caller retains/joins its worker. Missing devices
remain typed `NoAudioDevice`. The app sink owns device timing and buffer accounting.

Container export writes deduplicated attribution and licence records from every library audio item into the MP4/WebM `comment` tag through media's bounded muxer metadata seam. Export refuses more than 64 distinct records or 8 KiB of attribution text with a typed size error, preserving any existing output.
