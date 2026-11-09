//! Tracked Markdown outside `.scratch/` carries no tracker ids or private names.
use super::{TEXT, Violation};
use std::{collections::BTreeSet, fs, path::Path};

pub(super) fn check(root: &Path, tracked: &[String]) -> Result<BTreeSet<Violation>, String> {
    let mut violations = BTreeSet::new();
    for path in tracked {
        let text = fs::read_to_string(root.join(path)).map_err(|e| format!("{path}: {e}"))?;
        let mut ids = BTreeSet::new();
        for line in text.lines() {
            if attribution_added(line) {
                continue;
            }
            ids.extend(tracker_ids(line));
        }
        for id in ids {
            violations.insert(Violation {
                path: path.clone(),
                rule: TEXT,
                detail: format!("tracker id `{id}`"),
            });
        }
    }
    for path in forbidden(root)? {
        violations.insert(Violation {
            path,
            rule: TEXT,
            detail: "contains a forbidden host or agent name".into(),
        });
    }
    Ok(violations)
}

pub(super) fn attribution_added(line: &str) -> bool {
    line.trim_start().starts_with("added:")
}

pub(super) fn tracker_ids(line: &str) -> BTreeSet<String> {
    let bytes = line.as_bytes();
    let mut ids = BTreeSet::new();
    let mut index = 0;
    while index < bytes.len() {
        if let Some(end) = ticket(bytes, index) {
            ids.insert(
                line.get(index..end)
                    .unwrap_or_default()
                    .to_ascii_lowercase(),
            );
            index = end;
            continue;
        }
        if let Some(end) = ticket_path(bytes, index) {
            ids.insert(
                line.get(index..end)
                    .unwrap_or_default()
                    .to_ascii_lowercase(),
            );
            index = end;
            continue;
        }
        index += 1;
    }
    ids
}

/// `ticket \d+`, with word boundaries on both sides.
fn ticket(bytes: &[u8], start: usize) -> Option<usize> {
    const WORD: &[u8] = b"ticket";
    let head = bytes.get(start..start + WORD.len())?;
    if !head.eq_ignore_ascii_case(WORD) || !boundary(bytes, start) {
        return None;
    }
    let mut index = start + WORD.len();
    if !bytes.get(index)?.is_ascii_whitespace() {
        return None;
    }
    while bytes.get(index).is_some_and(u8::is_ascii_whitespace) {
        index += 1;
    }
    let digits = index;
    while bytes.get(index).is_some_and(u8::is_ascii_digit) {
        index += 1;
    }
    (index > digits && bytes.get(index).is_none_or(|byte| !word(*byte))).then_some(index)
}

/// `p\d-[a-z-]+/\d+`, with word boundaries on both sides.
fn ticket_path(bytes: &[u8], start: usize) -> Option<usize> {
    if !bytes
        .get(start)
        .is_some_and(|byte| *byte == b'p' || *byte == b'P')
        || !boundary(bytes, start)
    {
        return None;
    }
    let mut index = start + 1;
    while bytes.get(index).is_some_and(u8::is_ascii_digit) {
        index += 1;
    }
    if index == start + 1 || bytes.get(index) != Some(&b'-') {
        return None;
    }
    index += 1;
    let words = index;
    while bytes
        .get(index)
        .is_some_and(|byte| byte.is_ascii_alphabetic() || *byte == b'-')
    {
        index += 1;
    }
    if index == words || bytes.get(index) != Some(&b'/') {
        return None;
    }
    index += 1;
    let digits = index;
    while bytes.get(index).is_some_and(u8::is_ascii_digit) {
        index += 1;
    }
    (index > digits && bytes.get(index).is_none_or(|byte| !word(*byte))).then_some(index)
}

fn boundary(bytes: &[u8], start: usize) -> bool {
    start == 0 || !word(bytes.get(start - 1).copied().unwrap_or_default())
}

fn word(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_'
}

fn forbidden(root: &Path) -> Result<BTreeSet<String>, String> {
    let path = crate::checkout::main(root)?.join(".scratch/forbidden-names.txt");
    if !path.exists() {
        return Ok(BTreeSet::new());
    }
    let text = fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    let patterns: Vec<&str> = text
        .lines()
        .filter(|line| !line.trim().is_empty())
        .collect();
    if patterns.is_empty() {
        return Ok(BTreeSet::new());
    }
    let mut args = vec!["grep", "-n", "-i", "-E"];
    for pattern in &patterns {
        args.push("-e");
        args.push(pattern);
    }
    args.push("--");
    args.push("*.md");
    let out = crate::output(root, "git", &args)?;
    match out.status.code() {
        Some(1) => Ok(BTreeSet::new()),
        Some(0) => {
            let mut files = BTreeSet::new();
            for line in String::from_utf8_lossy(&out.stdout).lines() {
                let mut fields = line.splitn(3, ':');
                let Some(file) = fields.next() else {
                    continue;
                };
                let Some(content) = fields.nth(1) else {
                    continue;
                };
                if file.starts_with(".scratch/") || attribution_added(content) {
                    continue;
                }
                files.insert(file.to_owned());
            }
            Ok(files)
        }
        _ => Err(format!(
            "name scan: {}",
            String::from_utf8_lossy(&out.stderr)
        )),
    }
}
