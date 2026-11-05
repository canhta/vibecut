# xtask

Workspace checks, native-media provisioning, development build admission and the
local ticket board. This binary is tooling, outside the product dependency graph;
it never edits tickets or publishes reports.

Run from a checkout with `cargo xtask <command>`. The Cargo alias builds the
launcher in `target/xtask`, independently of a running build's warm target slot.
Product compilation commands on shared Unix developer machines use
`cargo xtask build -- <command...>`; launcher compilation itself bootstraps outside
the slots. This maintainer-machine wrapper is Unix-only; on Windows and dedicated
hosted CI runners, run Cargo commands directly.

| Command | Behaviour |
|---|---|
| `build -- COMMAND [ARGS...]` | Unix-only FIFO admission and command exit status; OS-native arguments, inherited standard streams; on Windows run Cargo directly |
| `frontier [ASSIGNEE]` | Open, unblocked tickets ordered by priority and ID; missing/wontfix blocker diagnostics |
| `board [PORT]` | Read-only live ticket page and `data.json` on IPv4 localhost, default port 8765 |
| `checks`, `docs`, `layers`, `assets` | Repository policy; `checks` also checks formatting |
| `clippy`, `test`, `lint`, `ci [--require-oracles]` | Individual or aggregate [verification](../docs/engineering/verification.md) |
| `ffmpeg [--target PLATFORM]`, `ffmpeg --cache-key` | Resolve/provision native media dependencies and cache identity |
| `media-bench`, `clip-eval SET [OPTIONS]` | Media and labelled clip-pipeline measurements |
| `new-crate NAME` | Scaffold a core library already admitted in the architecture table |

## Build slots

The per-user cache is `.cache/vibecut` beneath `HOME`. Three OS-locked slots own warm `target/slot-N`
directories. The `locks/build-queue` directory holds numbered, locked waiter
files; `locks/admission` serializes enqueue, stale-entry cleanup and slot claims.
No process IDs establish liveness. Crashed waiters release their locks automatically;
dead queue files are removed by the next waiter. Admission retains FIFO order, preferring a
free slot already owned by the checkout. On macOS, only the first active build
bypasses the 30% memory and 20-second ramp checks; an unavailable `sysctl` blocks
extra admission. Other Unix systems admit by available slots alone.

The wrapped command inherits only its slot descriptor, using safe `rustix`
descriptor APIs. Queue and admission descriptors retain close-on-exec. The
launcher restores close-on-exec on its own slot descriptor immediately after
spawning, then waits for the command. The shared slot lock releases on the last
descriptor close, so killing the launcher or a command leaving a surviving
descendant cannot make its target directory reusable early. Spawn failures close
the slot normally; standard streams remain inherited. A command that deliberately
closes inherited descriptors must finish its protected work before closing the
lease; ordinary Cargo/compiler subprocesses inherit it.

A worktree switch prunes workspace artifacts and old test binaries, then touches
tracked regular files to prevent Cargo reusing stale workspace outputs; registry
dependencies and the isolated launcher stay warm. The holder clears its own slot if all slot caches exceed
50 GiB (measured on switches) or disk free space falls below 30 GiB. If clearing
cannot restore 30 GiB free, the build is refused. Only the held slot is deleted;
other running builds retain their artifacts. Disk queries use `df -Pk`; query failure
refuses the build. Child builds default to four Cargo jobs and clear
`RUSTC_WRAPPER`.

Successful exact `cargo xtask ci [--require-oracles]` commands leave receipts in
`ci-receipts`, keyed by Git tree and oracle mode. Only trees clean before waiting,
after admission and after execution qualify, including untracked files. An
identical clean tree reuses its receipt across rebases. Receipts are local
verification evidence; they do not replace environment-specific release checks.

## Tickets and limits

The ticket commands and name policy find the main checkout through Git's absolute
common directory, so linked worktrees read the same `.scratch` data. The
[issue tracker](../docs/agents/issue-tracker.md) owns the ticket format. The board
retains progress, kanban, next-up and dependency-wave views; wave, critical-path
and unblock counts include cross-milestone edges. Missing/wontfix blockers prevent
readiness; cycles or dependency depth beyond 512 return an error. Each request
reloads tickets. The server only accepts GET for `/` and `/data.json`; it never
serves filesystem paths or writes ticket data. Request lines are capped at 8 KiB
and reads/writes time out after two seconds. Ticket files are trusted local data;
this is not a public or multi-user server.

`cargo test -p xtask` covers policy gates, graph boundaries, build admission with
independent processes (waiter/holder crashes, a surviving wrapped command after
launcher death, a surviving descendant after normal launcher exit, FIFO and the
last-slot race), memory probe failure, receipt cleanliness and worktree/disk
cleanup. macOS is exercised locally; Windows runs the other xtask commands
directly, without the shared build wrapper.
