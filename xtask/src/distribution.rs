//! Native distribution assembly and clean-host acceptance; never publishes reports.

use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::env;
use std::ffi::{OsStr, OsString};
use std::fs;
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

type Result<T> = std::result::Result<T, String>;
fn arg(value: impl AsRef<OsStr>) -> OsString {
    value.as_ref().to_owned()
}
macro_rules! run {
    ($tools:expr, $program:expr $(, $argument:expr)* $(,)?) => {
        $tools.run($program, vec![$($crate::distribution::arg($argument)),*])
    };
}

mod icons;
mod linux;
mod macos;
mod package;
mod publish;
mod signing;
mod smoke;
mod universal;
mod windows;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Host {
    Mac,
    Windows,
    Linux,
    Other,
}
impl Host {
    fn current() -> Self {
        match env::consts::OS {
            "macos" => Self::Mac,
            "windows" => Self::Windows,
            "linux" => Self::Linux,
            _ => Self::Other,
        }
    }
}
struct Captured {
    code: Option<i32>,
    stdout: String,
    stderr: String,
}
type Runner = Box<dyn FnMut(&mut Command, Duration) -> Result<Captured>>;
struct Tools {
    root: PathBuf,
    host: Host,
    env: BTreeMap<String, String>,
    commands: Vec<Vec<String>>,
    secrets: Vec<String>,
    runner: Runner,
    runner_temp: PathBuf,
}
impl Tools {
    fn new(root: &Path) -> Self {
        Self {
            root: root.to_owned(),
            host: Host::current(),
            env: env::vars_os()
                .filter_map(|(key, value)| {
                    Some((key.into_string().ok()?, value.into_string().ok()?))
                })
                .collect(),
            commands: Vec::new(),
            secrets: Vec::new(),
            runner: Box::new(capture),
            runner_temp: env::var_os("RUNNER_TEMP")
                .map(PathBuf::from)
                .unwrap_or_else(env::temp_dir),
        }
    }
    fn redact(&self, text: &str) -> String {
        let mut result = text.to_owned();
        let mut secrets: Vec<_> = self
            .secrets
            .iter()
            .chain(
                self.env
                    .iter()
                    .filter(|(key, _)| {
                        key.starts_with("VIBECUT_MAC_")
                            || key.starts_with("VIBECUT_NOTARY_")
                            || key.starts_with("VIBECUT_WINDOWS_")
                            || key.starts_with("MACOS_CERT_")
                            || key.starts_with("APPLE_")
                            || *key == "GH_TOKEN"
                    })
                    .map(|(_, value)| value),
            )
            .filter(|value| !value.is_empty() && value.as_str() != "-")
            .collect();
        secrets.sort_by_key(|value| std::cmp::Reverse(value.len()));
        for value in secrets {
            result = result.replace(value, "<private signing value>");
        }
        result
    }
    fn execute(&mut self, command: &mut Command, timeout: Duration) -> Result<Captured> {
        let record = std::iter::once(command.get_program())
            .chain(command.get_args())
            .map(|value| self.redact(&value.to_string_lossy()))
            .collect();
        self.commands.push(record);
        (self.runner)(command, timeout)
    }
    fn run(&mut self, program: &str, args: Vec<OsString>) -> Result<String> {
        let mut command = Command::new(program);
        command.args(args).current_dir(&self.root);
        let timeout =
            if program == "xcrun" && command.get_args().next() == Some(OsStr::new("notarytool")) {
                Duration::from_secs(1800)
            } else {
                Duration::from_secs(180)
            };
        let result = self.execute(&mut command, timeout)?;
        if result.code != Some(0) {
            if program == "security"
                && result
                    .stderr
                    .contains("MAC verification failed during PKCS12 import")
            {
                return Err("Developer ID certificate import rejected PKCS#12 encryption or password; verify MACOS_CERT_PASSWORD matches MACOS_CERT_P12 and use macOS-compatible PKCS#12 encryption (native diagnostics suppressed)".into());
            }
            let operation = if program == "security" {
                command
                    .get_args()
                    .next()
                    .and_then(OsStr::to_str)
                    .unwrap_or("operation")
            } else {
                "operation"
            };
            return Err(format!(
                "{program} {operation} failed ({:?}); {} bytes of sensitive tool diagnostics suppressed",
                result.code,
                result.stderr.len()
            ));
        }
        if program == "lipo" {
            print!("{}", self.redact(&result.stdout));
        }
        Ok(result.stdout)
    }
    fn version(&self) -> &'static str {
        env!("CARGO_PKG_VERSION")
    }
    fn variable(&self, key: &str) -> Option<&str> {
        if let Some(value) = self.env.get(key) {
            return Some(value.as_str());
        }
        if self.host == Host::Windows {
            self.env
                .iter()
                .find(|(name, _)| name.eq_ignore_ascii_case(key))
                .map(|(_, value)| value.as_str())
        } else {
            None
        }
    }
}
fn capture(command: &mut Command, timeout: Duration) -> Result<Captured> {
    let mut child = command
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| e.to_string())?;
    let stdout = child.stdout.take().ok_or("missing child stdout")?;
    let stderr = child.stderr.take().ok_or("missing child stderr")?;
    fn read(mut stream: impl Read) -> Result<String> {
        let mut bytes = Vec::new();
        let mut overflow = false;
        let mut block = [0; 8192];
        loop {
            let count = stream.read(&mut block).map_err(|e| e.to_string())?;
            if count == 0 {
                break;
            }
            if bytes.len().saturating_add(count) <= 8 * 1024 * 1024 {
                bytes.extend_from_slice(block.get(..count).ok_or("output block bounds")?);
            } else {
                // Keep draining so a verbose child cannot deadlock on a full pipe.
                overflow = true;
            }
        }
        if overflow {
            return Err("tool output exceeded 8 MiB".into());
        }
        Ok(String::from_utf8_lossy(&bytes).into_owned())
    }
    let output = std::thread::spawn(move || read(stdout));
    let errors = std::thread::spawn(move || read(stderr));
    let started = Instant::now();
    let status = loop {
        match child.try_wait().map_err(|e| e.to_string())? {
            Some(status) => break status,
            None if started.elapsed() >= timeout => {
                let _ = child.kill();
                let _ = child.wait();
                let _ = output.join();
                let _ = errors.join();
                return Err("tool timed out and was terminated".into());
            }
            None => std::thread::sleep(Duration::from_millis(20)),
        }
    };
    Ok(Captured {
        code: status.code(),
        stdout: output.join().map_err(|_| "stdout reader failed")??,
        stderr: errors.join().map_err(|_| "stderr reader failed")??,
    })
}
fn field<'a>(value: &'a Value, pointer: &str) -> &'a Value {
    value.pointer(pointer).unwrap_or(&Value::Null)
}
fn insert(value: &mut Value, key: &str, data: Value) -> Result<()> {
    value
        .as_object_mut()
        .ok_or("receipt must be an object")?
        .insert(key.to_owned(), data);
    Ok(())
}
fn write_json(path: &Path, value: &Value) -> Result<()> {
    let mut bytes = serde_json::to_vec_pretty(value).map_err(|e| e.to_string())?;
    bytes.push(b'\n');
    fs::write(path, bytes).map_err(|e| e.to_string())
}
fn text(path: &Path) -> Result<String> {
    fs::read_to_string(path).map_err(|e| format!("read {}: {e}", path.display()))
}
fn digest(path: &Path) -> Result<String> {
    let mut file = fs::File::open(path).map_err(|e| e.to_string())?;
    let mut hash = Sha256::new();
    let mut block = [0; 65536];
    loop {
        let count = file.read(&mut block).map_err(|e| e.to_string())?;
        if count == 0 {
            break;
        }
        hash.update(block.get(..count).ok_or("hash block bounds")?);
    }
    Ok(hash
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect())
}
fn fresh(path: &Path) -> Result<PathBuf> {
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    fs::create_dir(path).map_err(|e| format!("output must be new: {}: {e}", path.display()))?;
    canonical(path)
}
fn canonical(path: &Path) -> Result<PathBuf> {
    let canonical = fs::canonicalize(path).map_err(|e| e.to_string())?;
    #[cfg(windows)]
    {
        native_windows_path(&canonical)
    }
    #[cfg(not(windows))]
    {
        Ok(canonical)
    }
}
#[cfg(windows)]
fn native_windows_path(path: &Path) -> Result<PathBuf> {
    use std::path::{Component, Prefix};
    let mut components = path.components();
    let Some(Component::Prefix(prefix)) = components.next() else {
        return Ok(path.to_owned());
    };
    let mut output = match prefix.kind() {
        Prefix::VerbatimDisk(drive) => PathBuf::from(format!("{}:\\", char::from(drive))),
        Prefix::VerbatimUNC(server, share) => {
            let mut name = OsString::from("\\\\");
            name.push(server);
            name.push("\\");
            name.push(share);
            PathBuf::from(name)
        }
        Prefix::Verbatim(_) | Prefix::DeviceNS(_) => {
            return Err("native distribution tools require a drive or UNC path".into());
        }
        _ => return Ok(path.to_owned()),
    };
    for component in components {
        if component != Component::RootDir {
            output.push(component.as_os_str());
        }
    }
    Ok(output)
}
fn entries(path: &Path) -> Result<Vec<PathBuf>> {
    let mut result = fs::read_dir(path)
        .map_err(|e| e.to_string())?
        .map(|entry| entry.map(|e| e.path()).map_err(|e| e.to_string()))
        .collect::<Result<Vec<_>>>()?;
    result.sort();
    Ok(result)
}
fn copy(source: &Path, destination: &Path) -> Result<()> {
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    fs::copy(source, destination).map_err(|e| format!("copy {}: {e}", source.display()))?;
    Ok(())
}
fn symlink(source: &Path, destination: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(source, destination).map_err(|e| e.to_string())
    }
    #[cfg(not(unix))]
    {
        let _ = (source, destination);
        Err("symlink is only used by macOS packaging".into())
    }
}
fn tree(source: &Path, destination: &Path) -> Result<()> {
    fs::create_dir_all(destination).map_err(|e| e.to_string())?;
    for path in entries(source)? {
        let target = destination.join(path.file_name().ok_or("file name")?);
        let kind = fs::symlink_metadata(&path)
            .map_err(|e| e.to_string())?
            .file_type();
        if kind.is_symlink() {
            return Err("SDK inputs must contain regular files, not symlinks".into());
        } else if kind.is_dir() {
            tree(&path, &target)?;
        } else if kind.is_file() {
            copy(&path, &target)?;
        } else {
            return Err("special files are not admitted in distributions".into());
        }
    }
    Ok(())
}
fn manifest(root: &Path) -> Result<Value> {
    fn walk(root: &Path, path: &Path, result: &mut serde_json::Map<String, Value>) -> Result<()> {
        for path in entries(path)? {
            let name = path
                .strip_prefix(root)
                .map_err(|e| e.to_string())?
                .to_str()
                .ok_or("manifest path is not UTF-8")?
                .replace('\\', "/");
            let kind = fs::symlink_metadata(&path)
                .map_err(|e| e.to_string())?
                .file_type();
            if kind.is_symlink() {
                result.insert(
                    name,
                    json!({"symlink":fs::read_link(&path).map_err(|e| e.to_string())?}),
                );
            } else if kind.is_dir() {
                walk(root, &path, result)?;
            } else if kind.is_file() {
                result.insert(name, json!(digest(&path)?));
            }
        }
        Ok(())
    }
    let mut result = serde_json::Map::new();
    walk(root, root, &mut result)?;
    Ok(Value::Object(result))
}
fn executable(path: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o755)).map_err(|e| e.to_string())?;
    }
    let _ = path;
    Ok(())
}
struct Temp(PathBuf);
impl Temp {
    fn new(base: &Path) -> Result<Self> {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|e| e.to_string())?
            .as_nanos();
        let next = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let path = base.join(format!(
            "vibecut-distribution-{}-{stamp}-{next}",
            std::process::id()
        ));
        fs::create_dir(&path).map_err(|e| e.to_string())?;
        Ok(Self(path))
    }
}
impl Drop for Temp {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
struct Options {
    values: BTreeMap<String, String>,
    flags: Vec<String>,
}
impl Options {
    fn parse(args: &[String], flags: &[&str], values: &[&str]) -> Result<Self> {
        let mut result = Self {
            values: BTreeMap::new(),
            flags: Vec::new(),
        };
        let mut args = args.iter();
        while let Some(key) = args.next() {
            if flags.contains(&key.as_str()) {
                if result.flags.contains(key) {
                    return Err("duplicate flag".into());
                }
                result.flags.push(key.clone());
            } else if values.contains(&key.as_str()) {
                let value = args
                    .next()
                    .filter(|v| !v.starts_with("--"))
                    .ok_or("option needs a value")?;
                if result.values.insert(key.clone(), value.clone()).is_some() {
                    return Err("duplicate option".into());
                }
            } else {
                return Err(format!("unknown distribution option: {key}"));
            }
        }
        Ok(result)
    }
    fn get(&self, key: &str) -> Result<&str> {
        self.values
            .get(key)
            .map(String::as_str)
            .ok_or_else(|| format!("missing {key}"))
    }
    fn path(&self, key: &str) -> Result<PathBuf> {
        Ok(PathBuf::from(self.get(key)?))
    }
    fn has(&self, key: &str) -> bool {
        self.flags.iter().any(|flag| flag == key)
    }
}
fn build(tools: &mut Tools, prefix: &Path, output: &Path, debug: bool) -> Result<()> {
    let output = fresh(output)?;
    let prefix = canonical(prefix)?;
    let mut command = Command::new(env::var_os("CARGO").unwrap_or_else(|| "cargo".into()));
    command
        .args([
            "build",
            "--locked",
            "--message-format=json-render-diagnostics",
            "-p",
            "vibecut",
            "-p",
            "vibecut-cli",
        ])
        .current_dir(&tools.root);
    if !debug {
        command.arg("--release");
    }
    if tools.host == Host::Mac {
        command.env("MACOSX_DEPLOYMENT_TARGET", macos::MINIMUM);
    }
    if tools.host == Host::Linux {
        // The package keeps its LGPL libraries beside the binaries.
        crate::ffmpeg::append_encoded_flag(
            &mut command,
            "-C\x1flink-arg=-Wl,-rpath,$ORIGIN/../lib",
        );
    }
    crate::ffmpeg::environment_at(&mut command, &prefix)?;
    for key in tools.env.keys().filter(|key| {
        key.starts_with("MACOS_CERT_")
            || key.starts_with("APPLE_")
            || key.starts_with("VIBECUT_MAC_")
            || key.starts_with("VIBECUT_NOTARY_")
    }) {
        command.env_remove(key);
    }
    let recorded: Vec<_> = command
        .get_args()
        .map(|arg| arg.to_string_lossy().into_owned())
        .collect();
    let mut child = command
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .map_err(|e| e.to_string())?;
    let result = (|| {
        let stdout = child.stdout.take().ok_or("cargo stdout")?;
        let mut artifacts = BTreeMap::new();
        for line in BufReader::new(stdout).lines() {
            let message: Value = serde_json::from_str(&line.map_err(|e| e.to_string())?)
                .map_err(|e| e.to_string())?;
            if let (Some(executable), Some(manifest)) = (
                message.get("executable").and_then(Value::as_str),
                message.get("manifest_path").and_then(Value::as_str),
            ) {
                let package = Path::new(manifest)
                    .parent()
                    .and_then(Path::file_name)
                    .and_then(OsStr::to_str)
                    .ok_or("artifact package")?;
                if matches!(package, "vibecut" | "vibecut-cli") {
                    artifacts.insert(package.to_owned(), PathBuf::from(executable));
                }
            }
        }
        if !child.wait().map_err(|e| e.to_string())?.success() || artifacts.len() != 2 {
            return Err("distribution Cargo build failed or omitted executable artifacts".into());
        }
        for (name, source) in artifacts {
            let target = output.join(format!(
                "{name}{}",
                if tools.host == Host::Windows {
                    ".exe"
                } else {
                    ""
                }
            ));
            copy(&source, &target)?;
        }
        let revision = run!(tools, "git", "rev-parse", "HEAD")?;
        write_json(
            &output.join("build.json"),
            &json!({"command":recorded,"revision":revision.trim(),"profile":if debug {"debug"}else{"release"}}),
        )
    })();
    if result.is_err() {
        let _ = child.kill();
        let _ = child.wait();
    }
    result
}

pub fn run(root: &Path, args: &[String]) -> Result<()> {
    let mut tools = Tools::new(root);
    let (command, args) = args
        .split_first()
        .ok_or("usage: cargo xtask dist COMMAND [OPTIONS]")?;
    match command.as_str() {
        "build" => {
            let o = Options::parse(args, &["--debug"], &["--ffmpeg", "--output"])?;
            build(
                &mut tools,
                &o.path("--ffmpeg")?,
                &o.path("--output")?,
                o.has("--debug"),
            )
        }
        "package" => {
            let o = Options::parse(
                args,
                &[],
                &["--os", "--arch", "--ffmpeg", "--binaries", "--output"],
            )?;
            let config = signing::Config::environment(&mut tools)?;
            package::package(&mut tools, &package::Request::options(&o)?, &config)
        }
        "native" => universal::native(
            &mut tools,
            &Options::parse(args, &["--signed"], &["--arch"])?,
        ),
        "universal" => universal::universal(&mut tools, &Options::parse(args, &["--signed"], &[])?),
        "export" => {
            let o = Options::parse(args, &[], &["--ffmpeg", "--binaries", "--output"])?;
            universal::export(
                &mut tools,
                &o.path("--ffmpeg")?,
                &o.path("--binaries")?,
                &o.path("--output")?,
            )
        }
        "smoke" => smoke::run(
            &mut tools,
            &Options::parse(
                args,
                &["--signed", "--observe"],
                &["--installed", "--output", "--media"],
            )?,
        ),
        "install-smoke" => smoke::install(&mut tools, &Options::parse(args, &["--signed"], &[])?),
        "icons" => {
            Options::parse(args, &[], &[])?;
            icons::run(&mut tools)
        }
        "replace" => macos::replace(
            &mut tools,
            &Options::parse(args, &[], &["--app-copy", "--libraries"])?,
        ),
        "signing" => signing::run(&mut tools, args),
        "publish" => publish::run(&mut tools, args),
        "validate" => {
            let o = Options::parse(args, &[], &["--version"])?;
            if o.get("--version")? != tools.version() {
                Err("release version must match workspace.package.version".into())
            } else {
                Ok(())
            }
        }
        _ => Err("unknown distribution command".into()),
    }
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod process_environment_tests {
    #[cfg(windows)]
    use super::native_windows_path;
    use super::{Captured, Host, Tools, arg};
    use std::path::Path;
    #[cfg(windows)]
    use std::path::PathBuf;
    #[test]
    fn windows_environment_lookup_is_case_insensitive() {
        let mut tools = Tools::new(Path::new("."));
        tools.env.clear();
        tools.host = Host::Windows;
        tools.env.insert("SYSTEMROOT".into(), "C:\\Windows".into());
        assert_eq!(tools.variable("SystemRoot"), Some("C:\\Windows"));
        tools.host = Host::Mac;
        assert_eq!(tools.variable("SystemRoot"), None);
    }
    #[test]
    fn certificate_password_diagnostic_does_not_expose_native_output() {
        let mut tools = Tools::new(Path::new("."));
        tools.runner = Box::new(|_, _| {
            Ok(Captured{code:Some(1),stdout:String::new(),stderr:"security: SecKeychainItemImport: MAC verification failed during PKCS12 import (wrong password?) PRIVATE-IDENTITY".into()})
        });
        let message = tools
            .run("security", vec![arg("import")])
            .expect_err("password refusal");
        assert!(message.contains("MACOS_CERT_PASSWORD"));
        assert!(!message.contains("PRIVATE-IDENTITY"));
    }
    #[cfg(windows)]
    #[test]
    fn native_compiler_paths_do_not_use_extended_length_namespaces() {
        for (input, expected) in [
            (
                r"\\?\C:\SDK with spaces\Tiếng Việt",
                r"C:\SDK with spaces\Tiếng Việt",
            ),
            (
                r"\\?\UNC\server\share\SDK with spaces",
                r"\\server\share\SDK with spaces",
            ),
            (r"C:\SDK", r"C:\SDK"),
        ] {
            assert_eq!(
                native_windows_path(Path::new(input)).expect("native path"),
                PathBuf::from(expected)
            );
        }
    }
}
