# xtask

Workspace checks, native-media provisioning, development build admission and the local ticket board. This binary is tooling, outside the product dependency graph; it never edits tickets or publishes reports.

Run from a checkout with `cargo xtask <command>`. The Cargo alias builds the launcher in `target/xtask`, independently of a running build's warm target slot. Product compilation on shared macOS and Windows developer machines uses `cargo xtask build -- <command...>`. The wrapper applies `ffmpeg::environment`, so every admitted command finds and loads the provisioned LGPL SDK at build and run time. Hosted CI runners run Cargo directly. Nested xtask commands keep the same native runpath flags and library paths, so direct and aggregate checks share Cargo fingerprints.

| Command | Behaviour |
|---|---|
| `build -- COMMAND [ARGS...]` | FIFO admission and command exit status on macOS and Windows; OS-native arguments, inherited standard streams |
| `frontier [ASSIGNEE]`, `frontier --blocking` | Ready tickets ordered by priority, open transitive dependents and ID, printing the dependent count; `--blocking` ranks every unresolved ticket by unresolved transitive dependents with its status; missing/wontfix blocker diagnostics |
| `board [PORT]` | Read-only project overview on IPv4 localhost, default port 8765; starts the configured Cloudflare tunnel when present |
| `cleanup [--environment NAME] [--dry-run] TARGET...` | Release settled worker dispatches or remove worktree targets, prune two-day-old build sessions/slots; `--dry-run` prints the plan |
| `pull-reports [--environment NAME] [--ssh HOST] [--dry-run] <worktree>...` | Merge a worktree's `.scratch` reports into the main checkout without overwriting newer local files |
| `checks`, `docs`, `layers`, `assets` | Repository policy; `docs` includes the docs index, README module maps and tracker-free Markdown against only-shrinking `xtask/docs-baseline.txt`; `checks` also checks formatting and the integration-test entry point |
| `modules` | Module structure gate: no `mod.rs`, responsibility docs, banned names (tests included), folder width and intra-crate layer direction from `xtask/modules.toml` (crate and `[[crate.folder]]` tables), with only-shrinking `xtask/modules-baseline.txt` |
| `catalog` | Regenerate `crates/engine/catalog.digest` and `docs/product/agent-control/commands.md` from the registry (the engine `control_catalog` example) |
| `clippy`, `test`, `lint`, `ci [--require-oracles]` | Individual or aggregate [verification](../docs/engineering/verification.md) |
| `ffmpeg [--target PLATFORM]`, `ffmpeg --cache-key` | Resolve/provision native media dependencies and cache identity |
| `media-bench`, `clip-eval SET [OPTIONS]` | Media and labelled clip-pipeline measurements |
| `new-crate NAME` | Scaffold a core library already admitted in the architecture table |

## Build slots

The per-user cache is `.cache/vibecut` beneath `HOME` (`USERPROFILE` on Windows). A host-derived number of OS-locked slots own warm `target/slot-N` directories. `locks/build-queue` holds numbered, locked waiter files; `locks/admission` serializes enqueue, stale-entry cleanup and slot claims. No process IDs establish liveness: crashed waiters release their locks automatically and the next waiter removes dead queue files. Admission is FIFO and prefers a free slot already owned by the checkout. Shrinking capacity stops new admissions but never releases or kills a running build; slots above a smaller count stay reusable and `cleanup` ages them out.

The slot ceiling is computed when the wrapper starts and enforced at each admission: `min(logical CPUs, floor(total_memory * 0.70 / 3 GiB) + 1, 64)`. 3 GiB is the peak reserve each running slot holds after compile (worst measured build 1.5 GiB plus one 1 GiB link and one 0.5 GiB test binary); the extra slot is the first build, which is never memory-gated. The measured 32 GiB host gets 8 slots. `VIBECUT_BUILD_SLOTS=1..=logical CPUs` (capped at 64) overrides the ceiling; zero, out-of-range, non-numeric or non-UTF-8 values are rejected with a typed error before queueing.

Default Cargo jobs share a budget of twice the logical CPUs. Under the admission lock each command reserves `min(max(2, logical CPUs / ceiling), remaining jobs)`; fewer than two remaining jobs holds admission. Reservations cover all live slot leases, including those above a reduced ceiling, stay fixed until the lease releases, and never resize or kill existing children. Each slot's `locks/slot-N.jobs` records its job count or an `exclusive` marker; only a held lease makes the record live. Missing or invalid metadata for a live lease reserves the whole budget and holds admission until the lease exits. All wrappers must implement this protocol, so drain older binaries and rebuild launchers before relying on the budget.

An explicit `CARGO_BUILD_JOBS` or command argument `--jobs`, `--jobs=N`, `-j N` or `-jN` (recognized anywhere in the wrapped arguments, including nested xtask commands) is honoured even above the host budget. Such a command reserves the entire budget and waits until all other leases release; no default-jobs command overlaps it.

Only the first active build bypasses the resource gates. Every extra slot requires all of:
- a 20-second ramp since the last admission;
- free physical memory, minus a 3 GiB peak reserve for every running slot, at or above 30%;
- load below the bound: 1-minute loadavg below twice the logical CPUs on Unix, aggregate processor load below 90% on Windows (two `GetSystemTimes` samples).

A neighbour admitted during compile keeps the whole reserve until its lease ends, because link and test fan-out happen late. A build admitted while instant free memory is already below 30% halves its own `CARGO_BUILD_JOBS` (never below one). Probe failure blocks extra admission. macOS uses `sysctl kern.memorystatus_level` and `hw.memsize`; Linux reads `MemAvailable`/`MemTotal` from `/proc/meminfo` and the first field of `/proc/loadavg` (a missing, zero-total, truncated or unparsable source holds the slot); Windows uses `GlobalMemoryStatusEx` with `GetSystemTimes`. Other Unix systems admit by available slots within the CPU-derived ceiling.

On Unix the wrapped command inherits only its slot descriptor; queue and admission descriptors keep close-on-exec. The launcher restores close-on-exec on its own slot descriptor right after spawning, then waits. The slot lock releases on the last descriptor close, so killing the launcher, or a descendant outliving its command, cannot free the target directory early. A command that closes inherited descriptors must finish its protected work first. On Windows the launcher puts itself in a kill-on-close Job Object (`win-custody`) before taking any lock, so the command and every descendant join it at creation. Killing the launcher, even with `taskkill /F` and no `/T`, makes the kernel kill the whole tree, and a descendant left by a normally exiting command dies when the launcher exits. The launcher holds the slot lock until its process ends. Standard streams stay inherited on both platforms.

A worktree switch prunes workspace artifacts and old test binaries, then touches tracked regular files so Cargo does not reuse stale workspace outputs; registry dependencies and the isolated launcher stay warm. It clears the held slot only when that slot alone exceeds the 40 GiB budget (warm slots measure 5-26 GiB) or free disk falls below 30 GiB. If clearing cannot restore 30 GiB free, the build is refused. Other running builds keep their artifacts. Disk queries use `df -Pk` on Unix and `GetDiskFreeSpaceEx` on Windows; query failure refuses the build. Cache accounting uses allocated blocks on Unix and logical file sizes on Windows. Child builds receive their reserved Cargo jobs and clear `RUSTC_WRAPPER`. No external compiler cache is enabled: sccache 0.19 was measured and rejected because its wrapper exceeds the Windows command-line limit for `ffmpeg-sys-next`.

Successful exact `cargo xtask ci [--require-oracles]` commands leave receipts in `ci-receipts`, keyed by Git tree and oracle mode. Only trees clean before waiting, after admission and after execution qualify, including untracked files. An identical clean tree reuses its receipt across rebases. Receipts are local evidence and do not replace environment-specific release checks.

Windows `ci` links and runs ai's standalone `whisper` and `gipformer` library tests before the default-feature workspace suite.

## FFmpeg

`ffmpeg` resolves and builds the latest stable FFmpeg from verified source, logs the version and admits only the target matching the running host. Every platform builds static zlib from its release archive and enables FFmpeg's PNG codec. The recipe pins the published SHA-256 per zlib version, so a newly resolved release is refused until the pin is reviewed and updated; a corrupt or substituted archive is removed before any install prefix exists.

Linux builds the GPL FFmpeg with libx264 and admits only `linux-x86_64`/`linux-aarch64`. It needs Bash, Make, Perl, pkg-config, Clang/LLVM (`clang`, `clang++`, `llvm-ar`, `llvm-nm`), CMake, Ninja, NASM on x86_64, and the app's desktop dependencies: `libasound2-dev`, `libxkbcommon-dev`, `libwayland-dev`, `libudev-dev`, `libx11-dev`, `libxcursor-dev`, `libxrandr-dev`, `libxi-dev`, `libxinerama-dev`, `libgl1-mesa-dev` and `libegl1-mesa-dev`. Managed tools carry an `$ORIGIN/../lib` rpath, and Cargo children receive `FFMPEG_DIR`, `PKG_CONFIG_PATH` and `LD_LIBRARY_PATH`.

Windows needs architecture-matched MSYS2 CLANG64 (x64) or CLANGARM64 (arm64), Bash, Make, Perl (libvpx configure), CMake, Ninja, pkg-config, NASM and Clang/LLVM; Rust uses MSVC. Native xtask caches under `USERPROFILE`, resolves Bash/Make/cygpath to absolute MSYS2 `usr/bin` paths on `PATH` and logs them. Incomplete installations are refused; Windows/WSL/Git Bash are never substituted. Static Opus/libvpx builds request static pkg-config metadata, including private pthread links.

Windows excludes FFmpeg's `gfxcapture` screen filter through configuration; case-insensitive builds resolve libc++'s `<version>` to the archive's `VERSION`. For workspace checks xtask merges general, underscore-target and hyphen-target `BINDGEN_EXTRA_CLANG_ARGS` in that order, then appends AMX include guards and an explicit native MSVC Clang target in the hyphen-target variable. The target applies to FFmpeg and Whisper because MSYS2 libclang's GNU default has unsigned enums that disagree with whisper-rs's MSVC representation. The AMX headers are excluded together until upstream bindings handle vector layouts. Workspace Cargo children receive MSVC/SDK header paths in `INCLUDE`, discovered through `cc`'s Windows registry support or inherited from a configured developer shell; only `INCLUDE` is forwarded from that environment, to preserve the FFmpeg DLL search path. Missing MSVC tools or SDK paths fail with setup guidance.

Windows workspace Cargo children also receive the managed static `/MD` Sherpa library directory through `SHERPA_ONNX_LIB_DIR`. Native AI archive selection, cache integrity and overrides belong to [ai's native build contract](../crates/ai/src/gipformer/README.md).

On Windows the FFmpeg environment owner projects only DLLs from the verified native prefix into `runtime-dlls` using atomic hard links, and prepends that directory to child `PATH`. It excludes the prefix's `bin` from inherited `PATH` so managed programs cannot shadow the external test oracles. Existing runtime links must match the native DLL hashes.

External `ffmpeg` and `ffprobe` are separately installed tools for oracle admission ([verification](../docs/engineering/verification.md#checks)). Distribution obligations live in [open-source policy](../docs/engineering/open-source.md#ffmpeg-distribution); the [workflows summary](../docs/engineering/workflows.md#ffmpeg-provisioning) routes here.

## Tickets and limits

Ticket commands and the name policy find the main checkout through Git's absolute common directory, so linked worktrees read the same `.scratch` data. The [issue tracker](../docs/agents/issue-tracker.md) owns the ticket format.

The board shows overall delivery, milestone completion (ticket counts, excluding wontfix), priority scope and the dependencies holding up delivery. Milestone, priority, assignee and status filters plus search narrow every panel, and the filtered view travels in the URL. Critical-path and unblock counts include cross-milestone edges. The unblock count is the open tickets transitively blocked by a ticket, shared with `frontier`; `frontier --blocking` counts unresolved transitive dependents for every unresolved ticket. Missing/wontfix blockers prevent readiness; cycles or dependency depth beyond 512 return an error.

The server reloads tickets on each request, loads ticket text on demand, and accepts only GET for `/`, `/data.json` and `/ticket.json`. It never serves filesystem paths or writes ticket data. Request lines are capped at 8 KiB and reads/writes time out after two seconds. `VIBECUT_BOARD_TOKEN` requires `?token=<value>` on every request; the page keeps it for its own requests. Ticket files are trusted local data and there is no multi-user editing.

`tickets/tunnel.rs` owns Cloudflare process startup and cleanup, `tickets.rs` owns the graph and HTTP endpoints, and `tickets/board.html` owns the overview. For a stable public hostname, install `cloudflared`, create a named tunnel and its DNS route, then store this in the ignored `.privates/board-tunnel.json` of the main checkout, with the credentials file in `.privates/` too:

```json
{
  "executable": "/absolute/path/to/cloudflared",
  "tunnel": "tunnel-uuid",
  "hostname": "board.example.com",
  "credentials": "/absolute/path/to/tunnel-credentials.json"
}
```

`cargo xtask board [PORT]` generates the tunnel ingress for that port and runs the connector alongside the board. Without the file it serves localhost only. Tunnel setup follows the [Cloudflare guide](https://developers.cloudflare.com/tunnel/features/locally-managed-tunnels/create-local-tunnel/).

## Maintenance

`cleanup` resolves the Orca workspaces directory and the main checkout from `orca worktree list` and Git, never from a compiled path. It resolves each `ctx_` dispatch in `orca orchestration worker-list`, releases it, and closes its terminal only after a matching `released` or `already_released` receipt. A settled dispatch with no terminal is reported cleaned without a release call. A `retained` receipt with `reason=ownership_transferred` is cleaned when the follow-up owner is also targeted or already released, and its terminal is left for that owner. Other retained, pending, unknown or malformed receipts leave the terminal open and fail that target. Discovery prefers the local dispatch record; a dispatch found only on the paired runtime is released there. `--environment` selects remote worktrees and terminal handles. Remaining arguments continue after a failure, and any failed step makes the command exit non-zero.

A local worktree drops its ignored `.scratch` before removal. Its branch is deleted locally and on `origin` when it is an ancestor of `origin/main` or `git cherry origin/main` proves no unique patches; unmerged branches are retained. Remote deletion validates the advertised remote tip and uses an exact lease, so a newer unmerged push is retained. The main checkout's `git pull --ff-only` runs before branch cleanup. Build slots held by a live build are skipped. An idle slot first drops incremental sessions and dependency artifacts older than two days; dry runs account for the same reclaimed bytes before reporting whole-slot removal.

`pull-reports` copies a local worktree's reports directly. A remote source requires `--ssh HOST` and uses `scp` into a temporary staging directory, because the paired Orca server has no file-transfer command. The merge replaces only files whose remote timestamp is newer.

## Distribution

`dist` owns build/package/export/native/universal, smoke/install-smoke, library replacement, signing lifecycle, icons and nightly/draft publication. Its native media environment is shared with normal checks through `ffmpeg::environment_at`. [Packaging](../packaging/README.md) owns commands, inputs, limits, platform gates and recovery; [verification](../docs/engineering/verification.md) owns workspace checks. All generated output directories must be new. Credentials and sensitive platform tool output are suppressed, and safe command receipts stay local to outputs. Unit tests prove no oldest-OS, hardware or performance acceptance. The image crate regenerates PNG/ICO icons; normal builds consume committed icon assets.
