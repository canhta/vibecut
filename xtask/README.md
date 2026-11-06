# xtask

Workspace checks, native-media provisioning, development build admission and the
local ticket board. This binary is tooling, outside the product dependency graph;
it never edits tickets or publishes reports.

Run from a checkout with `cargo xtask <command>`. The Cargo alias builds the
launcher in `target/xtask`, independently of a running build's warm target slot.
Product compilation commands on shared macOS and Windows developer machines use
`cargo xtask build -- <command...>`; launcher compilation itself bootstraps outside
the slots. The wrapper applies `ffmpeg::environment`, so every admitted command
discovers and loads the provisioned LGPL SDK at build and run time without
per-command paths. Dedicated hosted CI runners run Cargo commands directly.
Nested xtask commands retain the same native runpath flags and library paths,
so direct and aggregate checks share Cargo fingerprints.

| Command | Behaviour |
|---|---|
| `build -- COMMAND [ARGS...]` | FIFO admission and command exit status on macOS and Windows; OS-native arguments, inherited standard streams |
| `frontier [ASSIGNEE]`, `frontier --blocking` | Ready tickets ordered by priority, open transitive dependents and ID, printing the dependent count; `--blocking` ranks every unresolved ticket by unresolved transitive dependents with its status; missing/wontfix blocker diagnostics |
| `board [PORT]` | Read-only live ticket page and `data.json` on IPv4 localhost, default port 8765 |
| `cleanup [--environment NAME] [--dry-run] TARGET...` | Release settled worker dispatches or remove worktree targets, prune two-day-old build sessions/slots; `--dry-run` prints the plan |
| `pull-reports [--environment NAME] [--ssh HOST] [--dry-run] <worktree>...` | Merge a worktree's `.scratch` reports into the main checkout without overwriting newer local files |
| `checks`, `docs`, `layers`, `assets` | Repository policy; `checks` also checks formatting and the integration-test entry point |
| `modules` | Module structure gate: responsibility docs, banned names, folder width and intra-crate layer direction from `xtask/modules.toml`, with only-shrinking `xtask/modules-baseline.txt` |
| `clippy`, `test`, `lint`, `ci [--require-oracles]` | Individual or aggregate [verification](../docs/engineering/verification.md) |
| `ffmpeg [--target PLATFORM]`, `ffmpeg --cache-key` | Resolve/provision native media dependencies and cache identity |
| `media-bench`, `clip-eval SET [OPTIONS]` | Media and labelled clip-pipeline measurements |
| `new-crate NAME` | Scaffold a core library already admitted in the architecture table |

## Build slots

The per-user cache is `.cache/vibecut` beneath `HOME` (`USERPROFILE` on Windows). A
host-derived number of OS-locked slots own warm `target/slot-N` directories. The
`locks/build-queue` directory holds numbered, locked waiter files; `locks/admission`
serializes enqueue, stale-entry cleanup and slot claims. No process IDs establish
liveness. Crashed waiters release their locks automatically; dead queue files are
removed by the next waiter. Admission retains FIFO order, preferring a free slot
already owned by the checkout. Shrinking capacity stops new admissions but never
releases or kills a running build; slots above a smaller count stay reusable and
`cleanup` ages them out.

The ceiling is derived from host facts when the wrapper starts and enforced at each
admission: `min(logical CPUs, floor(total_memory * 0.70 / 1.5 GiB), 64)`. The
numerator keeps the existing 30% free-memory reservation; 1.5 GiB is the worst
measured workspace-build footprint (builds measure 0.3-1.5 GiB). The CPU term is the
runaway bound for slot count. Aggregate default Cargo jobs have a separate shared
budget of twice the logical CPUs. Under the admission lock, each command reserves
`min(max(2, logical CPUs / ceiling), remaining jobs)`; fewer than two remaining jobs
holds admission. Reservations cover all live slot leases, including those above
a reduced ceiling, so overlapping wrappers with different ceilings cannot exceed
the budget. A reservation stays fixed until its lease releases; existing children
are never resized or killed. `VIBECUT_BUILD_SLOTS=1..=logical CPUs` (capped at 64) overrides the
ceiling for tests and emergencies; zero, out-of-range, non-numeric or non-UTF-8
values are rejected with a typed error before queueing.

On macOS, Linux and Windows, only the first active build bypasses the resource gates.
Every extra slot requires all of: a 20-second ramp since the last admission, free
physical memory at or above 30%, and load below the documented bound (1-minute
loadavg below twice the logical CPUs on Unix; aggregate processor load below 90% on
Windows, sampled from two `GetSystemTimes` readings). Probe failure blocks extra
admission (fail closed): macOS uses `sysctl kern.memorystatus_level` and `hw.memsize`,
Linux reads `MemAvailable`/`MemTotal` from `/proc/meminfo` and the first field of
`/proc/loadavg` (a missing, zero-total, truncated or unparsable source holds the
slot), and Windows uses `GlobalMemoryStatusEx` with `GetSystemTimes`. One source
function per platform feeds the single admission path. Other Unix systems admit by
available slots alone within their CPU-derived ceiling.

On Unix, the wrapped command inherits only its slot descriptor, using safe `rustix`
descriptor APIs. Queue and admission descriptors retain close-on-exec. The
launcher restores close-on-exec on its own slot descriptor immediately after
spawning, then waits for the command. The shared slot lock releases on the last
descriptor close, so killing the launcher or a command leaving a surviving
descendant cannot make its target directory reusable early. Spawn failures close
the slot normally; standard streams remain inherited. A command that deliberately
closes inherited descriptors must finish its protected work before closing the
lease; ordinary Cargo/compiler subprocesses inherit it. On Windows the launcher
holds the slot lock until its child exits, including forced child termination.
Killing the Windows launcher can release the slot while its child is still alive;
detached descendants can also outlive the lease. Keep the launcher running until
the build finishes. Windows Job Object protection needs a safe API not currently
provided by the admitted dependency.

A worktree switch prunes workspace artifacts and old test binaries, then touches
tracked regular files to prevent Cargo reusing stale workspace outputs; registry
dependencies and the isolated launcher stay warm. The holder clears its own slot if all slot caches exceed
50 GiB (measured on switches) or disk free space falls below 30 GiB. If clearing
cannot restore 30 GiB free, the build is refused. Only the held slot is deleted;
other running builds retain their artifacts. Disk queries use `df -Pk` on Unix and
`GetDiskFreeSpaceEx` on Windows; query failure refuses the build. Cache accounting
uses allocated blocks on Unix and logical file sizes on Windows. Child builds
receive their reserved Cargo jobs and clear `RUSTC_WRAPPER`. Explicit caller
`CARGO_BUILD_JOBS` or command arguments `--jobs`, `--jobs=N`, `-j N`, `-jN` are
preserved as exceptions to the assigned-jobs bound, even above the host budget.
Such commands reserve the entire budget and wait until all other leases release;
no default-jobs command overlaps them. Job flags are conservatively recognized
anywhere in the wrapped command arguments, including nested xtask commands.
Each slot's `locks/slot-N.jobs` records its job count or an `exclusive` marker under
the admission lock; the marker prevents overlap even when wrappers observe different
CPU availability (for example, because of process affinity).
Only a held lease makes that record live; a release or crash frees its jobs without
metadata cleanup. Missing or invalid metadata for a live lease reserves the whole
budget and holds admission until that lease exits. All wrappers must implement the
reservation protocol: drain older binaries and rebuild their launchers before
relying on the shared budget or override exclusivity. Older binaries can reuse
slots with stale records and do not enforce the new reservations themselves.

Successful exact `cargo xtask ci [--require-oracles]` commands leave receipts in
`ci-receipts`, keyed by Git tree and oracle mode. Only trees clean before waiting,
after admission and after execution qualify, including untracked files. An
identical clean tree reuses its receipt across rebases. Receipts are local
verification evidence; they do not replace environment-specific release checks.

Windows `ci` links and runs ai's standalone `whisper` and `gipformer` library
tests before the default-feature workspace suite, exercising native linking and
runtime loading without relying on another backend's native dependencies.

## Tickets and limits

The ticket commands and name policy find the main checkout through Git's absolute
common directory, so linked worktrees read the same `.scratch` data. The
[issue tracker](../docs/agents/issue-tracker.md) owns the ticket format. The board
retains progress, kanban, next-up and dependency-wave views; wave, critical-path
and unblock counts include cross-milestone edges. Unblock counts are open
tickets transitively blocked by a ticket, shared by `frontier`; `frontier
--blocking` instead counts unresolved transitive dependents for every
unresolved ticket. Missing/wontfix blockers prevent readiness; cycles or
dependency depth beyond 512 return an error. Each request reloads tickets. The
server only accepts GET for `/` and `/data.json`; it never
serves filesystem paths or writes ticket data. Request lines are capped at 8 KiB
and reads/writes time out after two seconds. Ticket files are trusted local data;
this is not a public or multi-user server.

## Maintenance

`cleanup` resolves the Orca workspaces directory and the main checkout from
`orca worktree list` and Git, never from a compiled path. It resolves each
`ctx_` dispatch in `orca orchestration worker-list`, releases it, and closes its
terminal only after a matching `released` or `already_released` receipt. A
settled dispatch with no terminal is reported cleaned without a release call; a
`retained` receipt with `reason=ownership_transferred` is cleaned when the
follow-up owner is also targeted or already released, and its terminal is left
for that owner. Other retained, pending, unknown or malformed receipts leave the
terminal open and fail that target. Discovery prefers the local dispatch record;
Orca federates releases for its remote terminals. A dispatch found only on the
paired runtime is released there. `--environment` selects remote worktrees and
terminal handles. The remaining arguments continue; any failed step makes the
command exit non-zero. A
local worktree drops its ignored `.scratch` before removal. Its branch is
deleted locally and on `origin` when it is an ancestor of `origin/main` or
`git cherry origin/main` proves it has no unique patches; unmerged branches are
retained. Remote deletion validates the advertised remote tip and uses an exact
lease so a newer unmerged push is retained. The main checkout's
`git pull --ff-only` runs before branch cleanup so
those checks see a freshly merged `origin/main`. Build cache slots held by a live
build are skipped.
An idle slot over the size budget first drops dependency artifacts older than
two days, retaining fresh artifacts when that restores the budget; dry runs
account for the same reclaimed bytes before reporting whole-slot removal.

`pull-reports` copies a local worktree's reports directly. A paired Orca server
has no file-transfer command, so a remote source requires `--ssh HOST` and uses
`scp` into a temporary staging directory. The merge replaces only files whose
remote timestamp is newer; equal or newer local reports are kept.

`cargo test -p xtask` covers policy gates, graph boundaries, build admission with
independent processes (waiter/holder crashes, a surviving wrapped command after
launcher death, a surviving descendant after normal launcher exit, FIFO and the
last-slot race), mixed-capacity job reservations, concurrent claims and releases
at the shared job bound, exclusive caller jobs and fail-closed reservation metadata,
child exit/kill lease release, capacity derivation and override
validation, shrinking capacity without killing running builds, reuse of slots above
a shrunk count, memory admission/probe failure, the Linux `/proc/meminfo` and
`/proc/loadavg` parsers (normal, missing keys, truncated, zero total, hostile values
and exact thresholds), Windows processor-load arithmetic, receipt cleanliness and
worktree/disk cleanup. Launcher and descendant survival
tests apply to Unix; the shared queue and child lifetime tests run on Windows too.

## Distribution

`dist` owns build/package/export/native/universal, smoke/install-smoke, library
replacement, signing lifecycle, icons and nightly/draft publication. Its native
media environment is shared with normal checks through `ffmpeg::environment_at`.
[Packaging](../packaging/README.md) owns commands, inputs, limits, platform gates and
recovery; [verification](../docs/engineering/verification.md) owns workspace checks.
All generated output directories must be new. Credentials and sensitive platform
tool output are suppressed, and safe command receipts remain local to outputs.

Distribution unit tests cover source/patch hashes, architecture/deployment admission,
runpaths, PE headers, signing cleanup/redaction and publication refusal/order.
Targeted tests run with `cargo test -p xtask`; native packaging and clean-host hosted
runs provide the integration evidence. No oldest-OS, hardware or performance
acceptance follows from unit tests alone. The generic image crate is reused for
PNG/ICO icon regeneration; normal builds consume committed icon assets.
