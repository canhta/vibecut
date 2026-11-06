use crate::exit_codes::CliError;
use serde_json::{Value, json};
use std::io;
use std::io::Write as _;
use std::path::Path;
use time::Tick;

/// One command result. Both forms are always built so `--json` never re-runs work.
/// A streamed command writes its own protocol and returns a silent result with
/// the process status of its final frame.
pub struct Output {
    pub(crate) value: Value,
    pub(crate) human: String,
    pub(crate) silent: bool,
    pub(crate) exit: u8,
}
impl Output {
    pub fn write(&self, json_output: bool) -> Result<(), CliError> {
        if self.silent {
            return Ok(());
        }
        let text = if json_output {
            serde_json::to_string_pretty(&self.value)?
        } else {
            self.human.clone()
        };
        let mut stdout = io::stdout().lock();
        writeln!(stdout, "{text}")?;
        Ok(())
    }
    pub fn exit_code(&self) -> u8 {
        self.exit
    }
}

pub(crate) fn json_path(path: &Path) -> Result<String, CliError> {
    path.to_str()
        .map(str::to_owned)
        .ok_or(CliError::Unsupported(
            "non-UTF-8 path cannot be represented in JSON",
        ))
}

pub(crate) fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        let high = DIGITS.get(usize::from(byte >> 4)).copied().unwrap_or(b'0');
        let low = DIGITS
            .get(usize::from(byte & 0x0f))
            .copied()
            .unwrap_or(b'0');
        output.push(char::from(high));
        output.push(char::from(low));
    }
    output
}

pub(crate) fn tick_json(tick: Tick) -> Value {
    json!({"ticks": tick.0, "seconds": tick.seconds()})
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::panic)]
    use super::*;
    #[test]
    fn hex_is_lowercase_and_fixed_width() {
        assert_eq!(hex(&[0x00, 0x0f, 0xa5, 0xff]), "000fa5ff");
    }
}
