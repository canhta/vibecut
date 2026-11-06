//! Verifies native CLI process behaviour.
use serde_json::Value;
use std::ffi::OsStr;
use std::process::{Command, Output, Stdio};

pub(super) fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_vibecut-cli")
}
pub(super) fn os(value: &str) -> &OsStr {
    OsStr::new(value)
}
pub(super) fn cli(args: &[&OsStr]) -> Output {
    Command::new(bin()).args(args).output().unwrap()
}
pub(super) fn spawn(args: &[&OsStr]) -> std::process::Child {
    Command::new(bin())
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap()
}
pub(super) fn code(output: &Output) -> i32 {
    output.status.code().unwrap()
}
pub(super) fn stdout_json(output: &Output) -> Value {
    serde_json::from_slice(&output.stdout).unwrap()
}
pub(super) fn error_json(output: &Output) -> Value {
    let text = String::from_utf8_lossy(&output.stderr);
    let line = text
        .lines()
        .rev()
        .find(|line| line.starts_with('{'))
        .unwrap_or_else(|| panic!("no JSON error on stderr: {text}"));
    serde_json::from_str(line).unwrap()
}
