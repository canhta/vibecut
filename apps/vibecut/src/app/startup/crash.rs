//! The local crash log: one report per panic in the app data folder, read back on the next start.
use crate::services::views::about::{COMMIT, VERSION};
use std::{
    fs, io,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

/// Reports kept; a start removes the oldest beyond this.
const KEPT: usize = 5;
/// Present while a report is unread: the next start offers the folder once and removes it.
const UNREAD: &str = "unread";
const PREFIX: &str = "crash-";
const SUFFIX: &str = ".txt";

/// Install the panic hook that writes a report into `folder` before the default hook runs.
/// The report holds the time, version, OS, the panic's file and line, the last action ids and
/// the backtrace with file names only; never project content, a message or a path. Nothing
/// is uploaded.
pub fn install(folder: PathBuf) {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let backtrace = std::backtrace::Backtrace::force_capture().to_string();
        let time = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string();
        let location = info
            .location()
            .map(|at| format!("{}:{}", file_name(at.file()), at.line()));
        let text = report(
            &time,
            location.as_deref(),
            &crate::actions::recent(),
            &backtrace,
        );
        if let Err(error) = save(&folder, &text) {
            tracing::warn!(category = "crash_log_failed", kind = ?error.kind(), "Crash report could not be written");
        }
        previous(info);
    }));
}

/// Write one report as a new `crash-<unix ms>.txt` in `folder` and mark it unread.
pub fn save(folder: &Path, report: &str) -> io::Result<PathBuf> {
    fs::create_dir_all(folder)?;
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| since.as_millis());
    let mut last = io::Error::from(io::ErrorKind::AlreadyExists);
    // Two panics in one millisecond take the next free name.
    for step in 0..16u128 {
        let path = folder.join(format!(
            "{PREFIX}{:020}{SUFFIX}",
            millis.saturating_add(step)
        ));
        match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
        {
            Ok(mut file) => {
                io::Write::write_all(&mut file, report.as_bytes())?;
                fs::write(folder.join(UNREAD), b"")?;
                return Ok(path);
            }
            Err(error) => last = error,
        }
    }
    Err(last)
}

/// Whether the last run left an unread report; clears the mark and keeps only the newest
/// `KEPT` reports. A missing folder is no crash.
pub(super) fn take_unread(folder: &Path) -> bool {
    let unread = fs::remove_file(folder.join(UNREAD)).is_ok();
    let Ok(entries) = fs::read_dir(folder) else {
        return unread;
    };
    let mut reports: Vec<PathBuf> = entries
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with(PREFIX) && name.ends_with(SUFFIX))
        })
        .collect();
    reports.sort();
    let excess = reports.len().saturating_sub(KEPT);
    for path in reports.iter().take(excess) {
        if let Err(error) = fs::remove_file(path) {
            tracing::warn!(category = "crash_log_prune_failed", kind = ?error.kind(), "Old crash report kept");
        }
    }
    unread
}

/// The report text. Backtrace frame locations keep only their file name and line, so no
/// source, registry or user path leaves the build.
fn report(time: &str, location: Option<&str>, actions: &[&str], backtrace: &str) -> String {
    let mut text = format!(
        "VibeCut crash report\ntime: {time}\nversion: {VERSION}\ncommit: {}\nos: {} {}\npanic at: {}\nlast actions: {}\nbacktrace:\n",
        COMMIT.unwrap_or("unknown"),
        std::env::consts::OS,
        std::env::consts::ARCH,
        location.unwrap_or("unknown"),
        actions.join(", "),
    );
    for line in backtrace.lines() {
        let trimmed = line.trim_start();
        match trimmed.strip_prefix("at ") {
            Some(at) => {
                text.push_str(line.get(..line.len() - trimmed.len()).unwrap_or_default());
                text.push_str("at ");
                text.push_str(file_name(at));
            }
            None => text.push_str(line),
        }
        text.push('\n');
    }
    text
}

/// The last component of a source path, whichever separator it uses.
fn file_name(path: &str) -> &str {
    path.rsplit(['/', '\\']).next().unwrap_or(path)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::indexing_slicing)]
    use super::*;

    /// Fails if a report carries a source or user path, or drops the version, OS, panic line,
    /// action ids or frame names.
    #[test]
    fn report_keeps_file_names_and_actions_and_drops_every_path() {
        let backtrace = "   0: vibecut::app::registry::Workspace::tick\n             at C:\\Users\\someone\\vibecut\\apps\\vibecut\\src\\app\\registry.rs:301:9\n   1: std::rt::lang_start\n             at /rustc/abc/library/std/src/rt.rs:206:5\n   2: <unknown>\n";
        let text = report(
            "2026-11-15T10:00:00Z",
            Some("activation.rs:42"),
            &["timeline.split", "project.save"],
            backtrace,
        );
        for kept in [
            "time: 2026-11-15T10:00:00Z",
            &format!("version: {VERSION}"),
            &format!("os: {} {}", std::env::consts::OS, std::env::consts::ARCH),
            "panic at: activation.rs:42",
            "last actions: timeline.split, project.save",
            "0: vibecut::app::registry::Workspace::tick",
            "at registry.rs:301:9",
            "at rt.rs:206:5",
            "2: <unknown>",
        ] {
            assert!(text.contains(kept), "{kept} missing from\n{text}");
        }
        for dropped in ["Users", "someone", "/rustc/", "\\", "library/"] {
            assert!(!text.contains(dropped), "{dropped} leaked into\n{text}");
        }
    }

    /// Fails if a start offers a report twice, misses an unread one, or keeps more than the
    /// newest few on disk.
    #[test]
    fn a_start_offers_an_unread_report_once_and_keeps_the_newest() {
        let directory = tempfile::tempdir().unwrap();
        let folder = directory.path().join("crashes");
        assert!(!take_unread(&folder), "no folder is no crash");
        let saved: Vec<PathBuf> = (0..KEPT + 2)
            .map(|index| save(&folder, &format!("report {index}")).unwrap())
            .collect();
        assert!(take_unread(&folder));
        assert!(!take_unread(&folder), "offered once");
        let mut left: Vec<PathBuf> = fs::read_dir(&folder)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .collect();
        left.sort();
        assert_eq!(left, saved[saved.len() - KEPT..]);
        assert_eq!(
            fs::read_to_string(&left[KEPT - 1]).unwrap(),
            format!("report {}", KEPT + 1)
        );
    }
}
