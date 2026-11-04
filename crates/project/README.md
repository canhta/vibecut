# project

Owns project persistence and timeline validation. `atomic_write` replaces a file
using a synced temporary file in its directory; `autosave` lists and rotates
project snapshots. Excludes editing commands and scheduling.

Atomic replacement retains existing permissions; parent sync is best effort.
Workspace CI verifies compilation and structure; power-loss durability is unverified.
