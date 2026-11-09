# Sound library

## Sound library

`sounds.search` calls `GET https://freesound.org/apiv2/search/` with `sort=downloads_desc`, bounded `page`/`page_size`, explicit `fields`, and `filter=license:("Creative Commons 0" OR "Attribution")` while the commercial-only switch is on. NC/Sampling+ and undocumented licence values are dropped even if the server ignores the filter. Author, licence, attribution, source page and preview URL travel on every result.

Credentials are bring-your-own: `Token::from_env` reads `FREESOUND_API_KEY` for the headless adapter, and the composition root may inject an OS-keychain token. The token is held in memory, never serialized, never placed in a URL, and Debug is redacted. Without a token search degrades to the saved local library; adding a saved sound never requires the network. API errors distinguish `NoCredentials`, `Auth`, `Quota`, `RateLimit { retry_after_seconds }`, `Network`, `Server`, `InvalidResponse`, `Limit`, `NoAudioDevice`, `NotCached`, `LibraryCorrupt` and `Cancelled`; the CLI maps them to stable codes.

`sounds.search` and `sounds.save` are library operations, not Session edits: they never enter undo and callers run remote work off the UI thread. `sounds.add_to_timeline` is the registered edit (one undo step).

`SoundLibrary` owns a versioned `library.json` plus content-addressed preview bytes under the app-supplied root. Saving is an idempotent upsert; unsave and clear-all remove unreferenced previews. A project-owned writer lease excludes simultaneous handles and processes for the same root; sharing uses one `Arc<SoundLibrary>`. A failed library-file publication preserves the in-memory saved list. The cache key includes the exact preview URL, so a changed URL is a miss and stale bytes are removed when no saved sound still cites them. Search responses are capped at 1 MiB, library files at 1 MiB and 1,024 entries, previews at 32 MiB.

`SoundDownloads` streams through the runner (`JobKind::SoundDownload`, network retries) with HTTPS-only bounded redirects, content-length checks, cancellation before every read and write, and atomic publication; no credential is sent to preview CDN URLs.

`sounds.add_to_timeline` takes an asset already in project-owned media and commits it plus the audio item in one undo step; the item stores `sourceType=library` metadata. The `sounds::add_to_timeline`/`add_saved` helpers first copy the cached preview into the caller's project media directory under its content hash (atomic, hash-verified, reused when present), so unsave, clear-all and URL invalidation only remove cache entries, never a file a project references. The CLI uses `<project file>.media/`; managed projects pass their own `media` directory.

`SoundPreview` composes the cache/download owner with an app-owned `PreviewSink`; play and stop are cancellable and a missing device is a typed error.

Container export writes deduplicated attribution and licence records from every library audio item into the MP4/WebM `comment` tag through media's bounded muxer metadata seam. Export refuses more than 64 distinct records or 8 KiB of attribution text with a typed size error, preserving any existing output.
