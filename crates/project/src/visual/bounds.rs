//! JSON byte, nesting and string budgets applied before typed deserialization.
use crate::{Error, Result};
/// Bounds apply before any typed JSON allocation, including ignored fields.
pub const MAX_BYTES: usize = 16 << 20;
pub fn check(bytes: &[u8]) -> Result<()> {
    if bytes.len() > MAX_BYTES {
        return Err(Error::TooLarge("JSON bytes"));
    }
    let (mut depth, mut separators, mut quoted, mut escaped, mut string_bytes) =
        (0usize, 0usize, false, false, 0usize);
    for &b in bytes {
        if quoted {
            string_bytes += 1;
            if string_bytes > 1 << 20 {
                return Err(Error::TooLarge("JSON string"));
            }
            if escaped {
                escaped = false;
            } else if b == b'\\' {
                escaped = true;
            } else if b == b'"' {
                quoted = false;
            }
        } else {
            match b {
                b'"' => {
                    quoted = true;
                    string_bytes = 0;
                }
                b'[' | b'{' => {
                    depth += 1;
                    if depth > 64 {
                        return Err(Error::TooLarge("JSON nesting"));
                    }
                }
                b']' | b'}' => {
                    depth = depth.saturating_sub(1);
                }
                b',' | b':' => {
                    separators += 1;
                    if separators > 200000 {
                        return Err(Error::TooLarge("JSON structure"));
                    }
                }
                _ => {}
            }
        }
    }
    Ok(())
}
pub struct Writer(pub Vec<u8>);
impl std::io::Write for Writer {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if bytes.len() > MAX_BYTES.saturating_sub(self.0.len()) {
            return Err(std::io::Error::other("project output limit"));
        }
        self.0.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
