//! Relative Markdown links, their anchors, and reference-checkout source paths.
//!
//! Every relative link must name a file that exists, and a `#fragment` must
//! match a heading of the target page by GitHub's anchor rules. Fenced code and
//! inline code are skipped and absolute URLs are never fetched. Tracked prose
//! also may not cite a source or config file inside a reference checkout
//! (`apps/web/`, `.spikes/`); evidence directories and captured fixtures stay
//! allowed.
use super::{ANCHOR, LINK, SOURCE, Violation};
use std::{
    collections::{BTreeSet, HashMap},
    fs,
    path::{Path, PathBuf},
};

/// Roots that hold read-only reference checkouts, never our source.
const REFERENCES: &[&str] = &["apps/web/", ".spikes/"];

/// A reference path is a source trace only when it names a source or config file.
const SOURCE_EXTENSIONS: &[&str] = &[
    "rs", "ts", "tsx", "js", "jsx", "mjs", "cjs", "vue", "svelte", "css", "scss", "sass", "less",
    "html", "htm", "py", "go", "java", "kt", "kts", "swift", "c", "h", "cc", "cpp", "cxx", "hpp",
    "cs", "rb", "php", "lua", "sh", "bash", "zsh", "ps1", "sql", "toml", "yaml", "yml", "json",
    "json5",
];

pub(super) fn check(root: &Path, tracked: &[String]) -> Result<BTreeSet<Violation>, String> {
    let mut violations = BTreeSet::new();
    let mut anchors: HashMap<PathBuf, BTreeSet<String>> = HashMap::new();
    for path in tracked {
        let text = fs::read_to_string(root.join(path)).map_err(|e| format!("{path}: {e}"))?;
        for (_, line) in prose_lines(&text) {
            for target in targets(line) {
                let Some(target) = relative(&target) else {
                    continue;
                };
                let Some((file, fragment)) = resolve(path, target) else {
                    violations.insert(Violation {
                        path: path.clone(),
                        rule: LINK,
                        detail: format!("relative link `{target}` leaves the repository"),
                    });
                    continue;
                };
                let full = root.join(&file);
                if !full.exists() {
                    violations.insert(Violation {
                        path: path.clone(),
                        rule: LINK,
                        detail: format!("relative link `{target}` names no file"),
                    });
                    continue;
                }
                if fragment.is_empty() || !is_markdown(&file) {
                    continue;
                }
                if !anchors.contains_key(&file) {
                    let anchor_text = fs::read_to_string(&full)
                        .map_err(|e| format!("{}: {e}", file.display()))?;
                    anchors.insert(file.clone(), anchors_of(&anchor_text));
                }
                if !anchors
                    .get(&file)
                    .is_some_and(|set| set.contains(&fragment))
                {
                    violations.insert(Violation {
                        path: path.clone(),
                        rule: ANCHOR,
                        detail: format!(
                            "anchor `#{fragment}` matches no heading in `{}`",
                            file.to_string_lossy().replace('\\', "/")
                        ),
                    });
                }
            }
        }
        for reference in references(&text) {
            violations.insert(Violation {
                path: path.clone(),
                rule: SOURCE,
                detail: format!("reference checkout source path `{reference}`"),
            });
        }
    }
    Ok(violations)
}

/// Lines outside fenced code blocks with their 1-based line number.
fn prose_lines(text: &str) -> Vec<(usize, &str)> {
    let mut lines = Vec::new();
    let mut fence: Option<(char, usize)> = None;
    for (number, line) in text.lines().enumerate() {
        if let Some((marker, width)) = fence {
            if closes(line, marker, width) {
                fence = None;
            }
            continue;
        }
        if let Some(open) = opens(line) {
            fence = Some(open);
            continue;
        }
        lines.push((number + 1, line));
    }
    lines
}

fn opens(line: &str) -> Option<(char, usize)> {
    let line = indent(line)?;
    let marker = line.chars().next().filter(|c| matches!(c, '`' | '~'))?;
    let width = line.chars().take_while(|c| *c == marker).count();
    (width >= 3).then_some((marker, width))
}

fn closes(line: &str, marker: char, width: usize) -> bool {
    let Some(line) = indent(line) else {
        return false;
    };
    let count = line.chars().take_while(|c| *c == marker).count();
    count >= width && line.chars().skip(count).all(char::is_whitespace)
}

fn indent(line: &str) -> Option<&str> {
    let spaces = line.chars().take_while(|c| *c == ' ').count();
    (spaces <= 3).then(|| &line[spaces..])
}

/// Collects inline link targets and reference-definition targets on one line.
fn targets(line: &str) -> Vec<String> {
    let mut targets = Vec::new();
    let trimmed = line.trim_start();
    if let Some(rest) = trimmed.strip_prefix('[')
        && let Some((_, link)) = rest.split_once("]:")
        && let Some(target) = destination(link)
    {
        targets.push(target);
    }
    let chars: Vec<char> = line.chars().collect();
    let mut index = 0;
    while let Some(&character) = chars.get(index) {
        if character == '`'
            && let Some(end) = code_span(&chars, index)
        {
            index = end;
            continue;
        }
        if character == '['
            && let Some(link) = inline(&chars, index)
        {
            targets.push(link.target);
            index = link.end;
            continue;
        }
        index += 1;
    }
    targets
}

struct Inline {
    target: String,
    end: usize,
}

fn inline(chars: &[char], start: usize) -> Option<Inline> {
    let mut depth = 0;
    let mut index = start;
    let label_end = loop {
        match *chars.get(index)? {
            '\\' => index += 2,
            '[' => {
                depth += 1;
                index += 1;
            }
            ']' => {
                depth -= 1;
                index += 1;
                if depth == 0 {
                    break index - 1;
                }
            }
            _ => index += 1,
        }
    };
    if chars.get(label_end + 1) != Some(&'(') {
        return None;
    }
    let mut depth = 0;
    let mut index = label_end + 2;
    loop {
        match *chars.get(index)? {
            '\\' => index += 2,
            '(' => {
                depth += 1;
                index += 1;
            }
            ')' => {
                if depth == 0 {
                    let content: String = chars.get(label_end + 2..index)?.iter().collect();
                    return Some(Inline {
                        target: destination(&content)?,
                        end: index + 1,
                    });
                }
                depth -= 1;
                index += 1;
            }
            _ => index += 1,
        }
    }
}

fn destination(content: &str) -> Option<String> {
    let content = content.trim();
    if let Some(rest) = content.strip_prefix('<') {
        return rest
            .split('>')
            .next()
            .map(|target| target.trim().to_owned());
    }
    content.split_whitespace().next().map(str::to_owned)
}

/// Skips an inline code span, returning the index after its closing run.
fn code_span(chars: &[char], start: usize) -> Option<usize> {
    let width = chars
        .get(start..)?
        .iter()
        .take_while(|c| **c == '`')
        .count();
    let mut index = start + width;
    while let Some(&character) = chars.get(index) {
        if character == '`' {
            let run = chars
                .get(index..)?
                .iter()
                .take_while(|c| **c == '`')
                .count();
            if run == width {
                return Some(index + run);
            }
            index += run;
        } else {
            index += 1;
        }
    }
    None
}

/// Returns the target when it is a relative link, or `None` for absolute URLs.
fn relative(target: &str) -> Option<&str> {
    if target.starts_with("//") {
        return None;
    }
    if let Some((scheme, _)) = target.split_once(':')
        && !scheme.is_empty()
        && scheme
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
        && scheme.starts_with(|c: char| c.is_ascii_alphabetic())
    {
        return None;
    }
    Some(target)
}

/// Resolves a link target to a repository path and its decoded fragment.
fn resolve(source: &str, target: &str) -> Option<(PathBuf, String)> {
    let target = target.replace('\\', "/");
    let (path, fragment) = target.split_once('#').unwrap_or((&target, ""));
    let path = path.split('?').next().unwrap_or(path);
    if path.is_empty() {
        return Some((PathBuf::from(source), decode(fragment)));
    }
    let mut parts: Vec<&str> = if path.starts_with('/') {
        Vec::new()
    } else {
        let mut parts: Vec<&str> = source.split('/').collect();
        parts.pop();
        parts
    };
    for part in path.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop()?;
            }
            _ => parts.push(part),
        }
    }
    let file = if parts.is_empty() {
        PathBuf::from(".")
    } else {
        parts
            .iter()
            .fold(PathBuf::new(), |path, part| path.join(part))
    };
    Some((file, decode(fragment)))
}

fn decode(fragment: &str) -> String {
    let bytes = fragment.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while let Some(&byte) = bytes.get(index) {
        if byte == b'%'
            && let (Some(high), Some(low)) = (bytes.get(index + 1), bytes.get(index + 2))
            && let (Some(high), Some(low)) = (hex(*high), hex(*low))
        {
            out.push(high * 16 + low);
            index += 3;
        } else {
            out.push(byte);
            index += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn hex(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

fn is_markdown(path: &Path) -> bool {
    path.extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("md"))
}

/// Every heading anchor in a page, following GitHub's slugger.
fn anchors_of(text: &str) -> BTreeSet<String> {
    let mut anchors = BTreeSet::new();
    let mut counts: HashMap<String, usize> = HashMap::new();
    for (_, line) in prose_lines(text) {
        let Some(heading) = heading(line) else {
            continue;
        };
        let base = slug(&heading);
        let count = counts.entry(base.clone()).or_default();
        let mut anchor = if *count == 0 {
            base.clone()
        } else {
            format!("{base}-{count}")
        };
        while anchors.contains(&anchor) {
            *count += 1;
            anchor = format!("{base}-{count}");
        }
        *count += 1;
        anchors.insert(anchor);
    }
    anchors
}

fn heading(line: &str) -> Option<String> {
    let line = indent(line)?;
    let level = line.chars().take_while(|c| *c == '#').count();
    if !(1..=6).contains(&level) {
        return None;
    }
    let rest = line.get(level..)?;
    if !rest.is_empty() && !rest.starts_with(' ') && !rest.starts_with('\t') {
        return None;
    }
    let mut text = rest.trim();
    if let Some(position) = text.rfind([' ', '\t'])
        && text[position + 1..].chars().all(|c| c == '#')
    {
        text = text[..position].trim_end();
    }
    Some(text.to_owned())
}

/// GitHub anchor text: inline markup gone, lowercase, punctuation gone,
/// whitespace replaced by hyphens.
fn slug(text: &str) -> String {
    let mut slug = String::new();
    for character in plain(text).trim().to_lowercase().chars() {
        if character.is_whitespace() {
            slug.push('-');
        } else if matches!(character, '-' | '_') || character.is_alphanumeric() {
            slug.push(character);
        }
    }
    slug
}

fn plain(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::new();
    let mut index = 0;
    while let Some(&character) = chars.get(index) {
        if character == '`'
            && let Some(end) = code_span(&chars, index)
        {
            let width = chars
                .get(index..)
                .unwrap_or_default()
                .iter()
                .take_while(|c| **c == '`')
                .count();
            out.extend(chars.get(index + width..end - width).unwrap_or_default());
            index = end;
            continue;
        }
        if character == '!' && chars.get(index + 1) == Some(&'[') {
            index += 1;
            continue;
        }
        if character == '['
            && let Some(link) = inline(&chars, index)
        {
            let label_end = chars
                .get(index..)
                .unwrap_or_default()
                .iter()
                .position(|c| *c == ']')
                .map(|offset| index + offset)
                .unwrap_or(index);
            out.push_str(&plain(
                &chars
                    .get(index + 1..label_end)
                    .unwrap_or_default()
                    .iter()
                    .collect::<String>(),
            ));
            index = link.end;
            continue;
        }
        if matches!(character, '*' | '~') {
            index += 1;
            continue;
        }
        out.push(character);
        index += 1;
    }
    out
}

/// Collects source or config paths under a reference checkout.
fn references(text: &str) -> Vec<String> {
    let mut found = Vec::new();
    for (_, line) in prose_lines(text) {
        for root in REFERENCES {
            let mut from = 0;
            while let Some(offset) = line[from..].find(root) {
                let start = from + offset;
                if boundary(line, start)
                    && let Some(token) = token(&line[start..])
                    && is_source(&token)
                    && !found.contains(&token)
                {
                    found.push(token);
                }
                from = start + root.len();
            }
        }
    }
    found
}

fn boundary(line: &str, start: usize) -> bool {
    line[..start]
        .chars()
        .next_back()
        .is_none_or(|c| !(c.is_alphanumeric() || matches!(c, '_' | '.' | '-' | '\\')))
}

fn token(text: &str) -> Option<String> {
    let end = text
        .char_indices()
        .find(|(_, c)| {
            c.is_whitespace()
                || matches!(
                    c,
                    '`' | '\'' | '"' | ')' | ']' | '}' | ',' | ';' | '<' | '>' | '|'
                )
        })
        .map(|(index, _)| index)
        .unwrap_or(text.len());
    let token = text.get(..end)?.trim_end_matches(['.', ':']);
    Some(token.to_owned())
}

fn is_source(path: &str) -> bool {
    let file = path
        .rsplit('/')
        .next()
        .unwrap_or_default()
        .split(':')
        .next()
        .unwrap_or_default();
    file.rsplit_once('.').is_some_and(|(_, extension)| {
        SOURCE_EXTENSIONS
            .iter()
            .any(|known| extension.eq_ignore_ascii_case(known))
    })
}
