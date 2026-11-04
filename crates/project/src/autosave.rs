use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// The folder name used next to a project.
pub const AUTO_SAVE_DIR: &str = "Auto-Save";

/// `Auto-Save` folder for a project file.
pub fn auto_save_dir(project_path: &Path) -> PathBuf {
    project_path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."))
        .join(AUTO_SAVE_DIR)
}

/// A file-name-safe version of a project name (path separators and control characters replaced).
pub fn file_stem(name: &str) -> String {
    let s: String = name
        .chars()
        .map(|c| {
            if matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|') || c.is_control() {
                '_'
            } else {
                c
            }
        })
        .collect();
    let s = s.trim().trim_matches('.').to_string();
    if s.is_empty() { "Untitled".into() } else { s }
}

fn is_stamp(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() == 19
        && b.iter().enumerate().all(|(i, c)| match i {
            4 | 7 | 13 | 16 => *c == b'-',
            10 => *c == b'_',
            _ => c.is_ascii_digit(),
        })
}

/// Parse `YYYY-MM-DD_HH-MM-SS` or `YYYY-MM-DD_HH-MM-SS-<unix>` into the stamp and the unix time
/// (`None` for files written before the unix time was part of the name).
fn parse_stamp(s: &str) -> Option<(&str, Option<i64>)> {
    if is_stamp(s) {
        return Some((s, None));
    }
    let stamp = s.get(..19).filter(|st| is_stamp(st))?;
    let digits = s.get(19..)?.strip_prefix('-')?;
    let unix: i64 = digits.parse().ok()?;
    (unix.to_string() == digits).then_some((stamp, Some(unix)))
}

/// Auto-saves of project `name` in `dir`, oldest first (by real time, not by the local clock; files
/// without a unix time in their name are older than those with one).
pub fn list_auto_saves(dir: &Path, name: &str) -> Vec<PathBuf> {
    let prefix = format!("{}-", file_stem(name));
    let Ok(rd) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut v: Vec<(Option<i64>, String, PathBuf)> = rd
        .filter_map(|e| e.ok())
        .filter_map(|e| {
            let f = e.file_name().to_string_lossy().into_owned();
            let (stamp, unix) = parse_stamp(f.strip_prefix(&prefix)?.strip_suffix(".vibecut")?)?;
            Some((unix, stamp.to_string(), e.path()))
        })
        .collect();
    v.sort();
    v.into_iter().map(|(_, _, p)| p).collect()
}

/// Delete the oldest auto-saves of `name` so at most `max` remain. Returns the deleted paths.
pub fn rotate(dir: &Path, name: &str, max: usize) -> io::Result<Vec<PathBuf>> {
    let all = list_auto_saves(dir, name);
    let n = all.len().saturating_sub(max.max(1));
    let mut gone = Vec::new();
    for p in all.into_iter().take(n) {
        fs::remove_file(&p)?;
        gone.push(p);
    }
    Ok(gone)
}
