# Startup notices

Interface: `install_crash_log`, `save_crash_report`, and `Notices` for the registry.

What one start offers once, in one quiet bottom-right kit toast over Home: a newer public
release and the last run's crash report. What the release request sends and what a crash
report holds are stated in the [open-source policy](../../../../../docs/engineering/open-source.md#privacy).

| Module | Responsibility |
|---|---|
| `update` | The release request, its due rule and the version comparison. |
| `crash` | The panic hook's report files: writing, the unread mark and the bound. |
| `notice` | `Notices`: the check worker, the two notices and their toast. |

## Release check

`Workspace::check_for_update(transport, now)` is due when `update_check` is on and
`update_checked_at` (Unix seconds, recorded when a check starts) is unset, a day old or in
the future. It runs on a `wake` worker through the injected `transport::HttpTransport` (the
native binary passes `UreqTransport` with a 15 s limit; tests pass a fake). Only a `200`
reply whose `tag_name` is a plain `MAJOR.MINOR.PATCH`, optionally `v`-prefixed, strictly
above this build's version is offered; drafts, prereleases, any other status, a malformed
reply, one over 1 MiB and offline are silent. The link opens
`https://github.com/canhta/vibecut/releases/tag/<tag>` through the egui open-URL path; the
tag is validated before it enters the URL.

## Crash log

`install_crash_log(folder)` runs in the native binary before the workspace opens; the
folder is `crashes/` beside `recovery/` and `projects/`. Each panic writes
`crash-<unix ms, 20 digits>.txt` and the `unread` mark, then runs the default hook. The
action ids come from `actions::recent` (the last 16 registry ids `Editor::activate`
resolved). `Notices::new` removes the mark, keeps the newest five reports and, when the
mark was there, offers the folder. Open folder hands it to the native shell
(`Workspace::take_folder_request`), which opens it in the platform file manager
(`explorer`, `open` or `xdg-open`).
