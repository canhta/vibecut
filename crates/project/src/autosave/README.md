# Autosave snapshots

`autosave::save(path, project, keep)` writes into `<path>.autosave`, with increasing
20-digit snapshot numbers and no clock dependency. Retention admits 1..=1000.
Only canonical regular snapshot files participate; unrelated files, symlinks
and directories are preserved. Enumeration caps at 10,000 entries. Publication
never overwrites a snapshot and happens before pruning; pruning errors are
returned while the complete new snapshot remains. Autosave directories cannot
be symlinks. A crash can leave a temporary file, an extra complete snapshot, or
an incomplete pruning pass; no partial snapshot is promoted.
