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
| `cleanup [--environment NAME] [--dry-run] TARGET...` | Release settled worker dispatches or remove worktree targets, prune two-day-old build sessions/slots; `--dry-run` prints the plan |
| `pull-reports [--environment NAME] [--ssh HOST] [--dry-run] <worktree>...` | Merge a worktree's `.scratch` reports into the main checkout without overwriting newer local files |
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

## Maintenance

`cleanup` resolves the Orca workspaces directory and the main checkout from
`orca worktree list` and Git, never from a compiled path. It resolves each
`ctx_` dispatch in `orca orchestration worker-list`, releases it, and closes its
terminal only after a matching `released` or `already_released` receipt. Retained,
pending, unknown or malformed release receipts leave the terminal open and fail
that target. The remaining arguments continue; any failed step makes the command
exit non-zero. A local worktree drops its ignored `.scratch` before
removal; a paired environment passes `--environment` to every Orca call. Build
cache slots held by a live build are skipped, and the final
`git pull --ff-only` runs in the main checkout.

`pull-reports` copies a local worktree's reports directly. A paired Orca server
has no file-transfer command, so a remote source requires `--ssh HOST` and uses
`scp` into a temporary staging directory. The merge replaces only files whose
remote timestamp is newer; equal or newer local reports are kept.

`cargo test -p xtask` covers policy gates, graph boundaries, build admission with
independent processes (waiter/holder crashes, a surviving wrapped command after
launcher death, a surviving descendant after normal launcher exit, FIFO and the
last-slot race), memory probe failure, receipt cleanliness and worktree/disk
cleanup. macOS is exercised locally; Windows runs the other xtask commands
directly, without the shared build wrapper.

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
