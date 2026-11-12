# transport

Bounded blocking HTTPS for buffered requests and streaming downloads: `HttpRequest`, `HttpResponse`, `HttpStreamResponse`, the `HttpTransport` and `DownloadTransport` seams, the `UreqTransport` adapter, `download_redirect`, `retry_after`, `MAX_RESPONSE_BYTES` and `Error`.

`UreqTransport` bounds total/read/write time (120 s; `with_timeout` overrides), connect time at 10 s and follows no redirects, so an authorization header cannot move to another host. It reads at most the caller's ceiling and returns `Payload` past it. Request and response Debug output redacts headers and bodies; `Error` carries only `Invalid`, `Payload` or `Network`. `ureq` stays at 2.12.1 because `sherpa-onnx-sys` pins `ureq ^2.12`; this crate is the only workspace member that declares the production HTTP client.

| Module | Responsibility |
|---|---|

Excludes provider identity, credentials, consent, request serialization, parser limits, cache storage, retry budgets, job scheduling and redirect/Range/ETag policy, which stay with the caller. No transport implementation retries.
