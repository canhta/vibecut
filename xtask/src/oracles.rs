//! External media oracle receipts for the required-oracle CI mode.
use sha2::{Digest, Sha256};
use std::env;
use std::fmt::Write as _;
use std::fs;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::process::Command;

const MINIMUM_MAJOR: u32 = 6;
const TOOLS: [&str; 2] = ["ffmpeg", "ffprobe"];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Version {
    pub major: u32,
    pub minor: u32,
}
impl std::fmt::Display for Version {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}.{}", self.major, self.minor)
    }
}

pub(super) struct Tool {
    name: &'static str,
    path: Option<PathBuf>,
    version: Option<Version>,
    problem: Option<String>,
}

pub(super) struct Fixture {
    pub relative: PathBuf,
    pub bytes: u64,
    pub sha256: String,
}

/// Print the CI mode, tool receipt and, in required mode, fail on any missing,
/// old or unusable external oracle.
pub(super) fn begin(required: bool) -> Result<(), String> {
    if required {
        eprintln!("ci: required-oracles mode: external ffmpeg/ffprobe must be present and usable");
    } else {
        eprintln!(
            "ci: fast/offline mode: external ffmpeg/ffprobe are optional; skipped oracle tests are not passes"
        );
    }
    let tools: Vec<Tool> = TOOLS.iter().map(|name| inspect(name)).collect();
    for tool in &tools {
        eprintln!("oracle: {}", receipt(tool));
    }
    if required {
        let failures: Vec<String> = tools
            .iter()
            .filter_map(|tool| {
                tool.problem
                    .as_ref()
                    .map(|problem| format!("{}: {problem}", tool.name))
            })
            .collect();
        if !failures.is_empty() {
            return Err(format!(
                "required media oracles unavailable: {}",
                failures.join("; ")
            ));
        }
    } else {
        for tool in &tools {
            if let Some(problem) = &tool.problem {
                eprintln!("oracle: skipped {} ({problem})", tool.name);
            }
        }
    }
    Ok(())
}

fn receipt(tool: &Tool) -> String {
    match (&tool.path, tool.version, &tool.problem) {
        (Some(path), Some(version), None) => {
            format!("{} path={} version={version}", tool.name, path.display())
        }
        _ => format!(
            "{} unavailable: {}",
            tool.name,
            tool.problem.as_deref().unwrap_or("unknown error")
        ),
    }
}

fn inspect(name: &'static str) -> Tool {
    let Some(path) = resolve(name) else {
        return Tool {
            name,
            path: None,
            version: None,
            problem: Some("not found on PATH".into()),
        };
    };
    match Command::new(&path).arg("-version").output() {
        Ok(output) if output.status.success() => {
            let text = String::from_utf8_lossy(&output.stdout);
            let version = parse_version(&text);
            let problem = match version {
                Some(version) if version.major >= MINIMUM_MAJOR => None,
                Some(version) => Some(format!("version {version} is older than {MINIMUM_MAJOR}.0")),
                None => Some("unrecognized -version output".into()),
            };
            Tool {
                name,
                path: Some(path),
                version,
                problem,
            }
        }
        Ok(output) => Tool {
            name,
            path: Some(path),
            version: None,
            problem: Some(format!("-version exited {}", output.status)),
        },
        Err(error) => Tool {
            name,
            path: Some(path),
            version: None,
            problem: Some(format!("cannot run: {error}")),
        },
    }
}

fn resolve(name: &str) -> Option<PathBuf> {
    let path = env::var_os("PATH")?;
    let mut candidates = vec![name.to_owned()];
    if cfg!(windows) {
        candidates.push(format!("{name}.exe"));
    }
    for directory in env::split_paths(&path) {
        for candidate in &candidates {
            let path = directory.join(candidate);
            if path.is_file() {
                return Some(path);
            }
        }
    }
    None
}

pub(super) fn parse_version(output: &str) -> Option<Version> {
    let line = output.lines().next()?;
    let rest = line.split("version ").nth(1)?;
    let token = rest.split_whitespace().next()?.trim_start_matches('n');
    let mut parts = token.split('.');
    let major = number(parts.next()?)?;
    let minor = parts.next().map(number).unwrap_or(Some(0))?;
    Some(Version { major, minor })
}

fn number(part: &str) -> Option<u32> {
    let digits: String = part
        .chars()
        .take_while(|value| value.is_ascii_digit())
        .collect();
    if digits.is_empty() {
        return None;
    }
    digits.parse().ok()
}

/// Print every fixture currently under `target/fixtures` with its size and hash.
pub(super) fn print_fixtures(root: &Path) -> Result<(), String> {
    let fixtures = inventory(root)?;
    eprintln!("fixtures: {} file(s) under target/fixtures", fixtures.len());
    for fixture in fixtures {
        eprintln!(
            "fixture: {} {} sha256:{}",
            fixture.relative.display(),
            fixture.bytes,
            fixture.sha256
        );
    }
    Ok(())
}

pub(super) fn inventory(root: &Path) -> Result<Vec<Fixture>, String> {
    let base = root.join("target/fixtures");
    let mut paths = Vec::new();
    walk(&base, &mut paths)?;
    paths.sort();
    paths
        .into_iter()
        .map(|path| fixture(&base, &path))
        .collect()
}

fn walk(directory: &Path, paths: &mut Vec<PathBuf>) -> Result<(), String> {
    let entries = match fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(format!("{}: {error}", directory.display())),
    };
    let mut children = Vec::new();
    for entry in entries {
        children.push(
            entry
                .map_err(|error| format!("{}: {error}", directory.display()))?
                .path(),
        );
    }
    children.sort();
    for child in children {
        let metadata = fs::symlink_metadata(&child)
            .map_err(|error| format!("{}: {error}", child.display()))?;
        if metadata.is_dir() {
            walk(&child, paths)?;
        } else if metadata.is_file() {
            paths.push(child);
        }
    }
    Ok(())
}

fn fixture(base: &Path, path: &Path) -> Result<Fixture, String> {
    let relative = path
        .strip_prefix(base)
        .map_err(|error| error.to_string())?
        .to_path_buf();
    let bytes = fs::metadata(path)
        .map_err(|error| format!("{}: {error}", path.display()))?
        .len();
    let mut file = fs::File::open(path).map_err(|error| format!("{}: {error}", path.display()))?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 << 10];
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|error| format!("{}: {error}", path.display()))?;
        if read == 0 {
            break;
        }
        hasher.update(
            buffer
                .get(..read)
                .ok_or_else(|| format!("{}: read bounds", path.display()))?,
        );
    }
    let mut sha256 = String::with_capacity(64);
    for byte in hasher.finalize() {
        write!(&mut sha256, "{byte:02x}")
            .map_err(|_| format!("{}: hash formatting", path.display()))?;
    }
    Ok(Fixture {
        relative,
        bytes,
        sha256,
    })
}
