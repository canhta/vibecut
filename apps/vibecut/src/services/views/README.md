# Services views

Interface: `about`, `progress`.

Views that more than one window draws and none owns.

| Module | Holds |
|---|---|
| `about` | The About dialog: its facts (`VERSION`, the terms and privacy URLs, `entries()`: FFmpeg, then one row per bundled-asset author and licence that `build.rs` reads from the `.attribution` sidecars, then every shipped Rust crate read from the committed `apps/vibecut/third-party-crates.txt`, which `cargo xtask licences` writes and `cargo xtask checks` verifies; no network) and the one `show` view the editor frame and Home both host. |
| `progress` | `job_progress` and `line`: the status line with its percentage over the kit progress bar, drawn from an engine snapshot or a surface's own status words. The percentage is now a "Status N%" line above the bar, not text inside it. |
