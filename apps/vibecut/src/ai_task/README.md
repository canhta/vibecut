# AI task progress

Bounded progress and exact per-upload consent shared by every desktop AI workflow. One coalescing slot holds the current stage or job fraction, so callers never drain an unbounded event queue; the last writer wins.

`TaskEvents` implements engine's `Progress` and `ConsentPrompt` and carries one cancellation token. An upload request holds exactly the provider, model, languages, payload size, duration, cost and policy links the caller must disclose; the workflow blocks until the user approves or denies, the request is answered once, and a cancelled task returns `Cancelled` without sending approval. The tower keeps the request until answered, so a modal survives tab switches.
