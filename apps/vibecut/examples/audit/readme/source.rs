//! Defines canonical README source provenance, hashes, tool constants.
#![allow(
    clippy::indexing_slicing,
    reason = "an offline audit capture indexes its own manifest and fixed-size frame buffers; a panic aborts the tool, never a person's project"
)]
use super::super::Failure;
use sha2::{Digest, Sha256};
use std::fmt::Write as _;
use std::fs;
use std::io::Read;
use std::path::Path;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;
use std::time::Duration;

pub(super) const README_WINDOW: [f32; 2] = [1440.0, 900.0];
pub(super) const README_GIF_FRAMES: usize = 80;
pub(super) const README_MAX_GIF: u64 = 5_000_000;
pub(super) const README_MAX_JPG: u64 = 512 * 1024;
pub(super) const README_SCRATCH: u64 = 8 << 30;
pub(super) const BBB_SOURCE_URL: &str =
    "https://www.peach.themazzone.com/big_buck_bunny_720p_h264.mov";
pub(super) const BBB_SOURCE_BYTES: u64 = 416_751_190;
pub(super) const BBB_SOURCE_SHA256: &str =
    "45c8bafeb9a53df7f491198d2e71529701bcf1cd51805782089fac1d32869f9b";
pub(super) const BBB_EXCERPT_BYTES: u64 = 4_544_843;
pub(super) const BBB_EXCERPT_SHA256: &str =
    "66518061bbabbde0c7d48b639675ce8c46a549182d32c3cc1203f2cded7c21a1";
pub(super) const BBB_CREDIT: &str = "(c) copyright 2008, Blender Foundation / www.bigbuckbunny.org";
pub(super) const TOOL_METADATA_TIMEOUT: Duration = Duration::from_secs(10);
pub(super) const TOOL_WORK_TIMEOUT: Duration = Duration::from_secs(120);
pub(super) const TOOL_STDOUT_CAP: usize = 16 << 20;
pub(super) const TOOL_STDERR_CAP: usize = 64 << 10;

pub(super) fn hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        write!(&mut out, "{byte:02x}").ok();
    }
    out
}

pub(super) fn sha256_bytes(bytes: &[u8]) -> String {
    hex(&Sha256::digest(bytes))
}

pub(super) struct ReadmeClock(pub(super) AtomicU64);

impl engine::jobs::Clock for ReadmeClock {
    fn now(&self) -> Duration {
        Duration::from_millis(self.0.load(Ordering::SeqCst))
    }
}

pub(super) fn stream_sha256(path: &Path) -> Result<String, Failure> {
    let metadata = fs::metadata(path)?;
    if metadata.len() != BBB_EXCERPT_BYTES {
        return Err(format!(
            "README source is not the admitted Big Buck Bunny excerpt: expected {BBB_EXCERPT_BYTES} bytes, found {}",
            metadata.len()
        )
        .into());
    }
    let mut file = fs::File::open(path)?;
    let mut buffer = [0u8; 1 << 16];
    let mut hasher = Sha256::new();
    let mut total = 0u64;
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        total = total.saturating_add(read as u64);
        if total > BBB_EXCERPT_BYTES {
            return Err("README source grew beyond the admitted excerpt".into());
        }
        hasher.update(&buffer[..read]);
    }
    if total != BBB_EXCERPT_BYTES {
        return Err("README source ended before the admitted excerpt".into());
    }
    let digest = hex(&hasher.finalize());
    if digest != BBB_EXCERPT_SHA256 {
        return Err(format!(
            "README source SHA-256 mismatch: expected {BBB_EXCERPT_SHA256}, found {digest}"
        )
        .into());
    }
    Ok(digest)
}
