//! Pinned archive digests and typed source-verification failures.
// zlib's release page publishes the archive digest; pin the digest per version
// so a compromised mirror or a moved release cannot alter the provisioned SDK.
pub(super) const ZLIB_SHA256: [(&str, &str); 1] = [(
    "1.4",
    "02dd159783677a749c04adbdd02b55b8a2bcc7e054f51b76b549199a916ad584",
)];
/// Distinct failure from verifying an archive that must match its published digest.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum SourceError {
    NoPinnedChecksum {
        name: &'static str,
        version: String,
    },
    Download {
        name: &'static str,
        version: String,
        source: String,
        detail: String,
    },
    ChecksumMismatch {
        name: &'static str,
        version: String,
        expected: &'static str,
        actual: String,
    },
    Extract {
        name: &'static str,
        version: String,
        detail: String,
    },
    Io {
        name: &'static str,
        detail: String,
    },
}

impl std::fmt::Display for SourceError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoPinnedChecksum { name, version } => write!(
                formatter,
                "{name} {version} has no pinned SHA-256; update xtask/src/ffmpeg.rs before provisioning"
            ),
            Self::Download {
                name,
                version,
                source,
                detail,
            } => write!(
                formatter,
                "downloading {name} {version} from {source} failed: {detail}"
            ),
            Self::ChecksumMismatch {
                name,
                version,
                expected,
                actual,
            } => write!(
                formatter,
                "{name} {version} source checksum mismatch: expected {expected}, got {actual}; removed the downloaded archive"
            ),
            Self::Extract {
                name,
                version,
                detail,
            } => write!(formatter, "extracting {name} {version} failed: {detail}"),
            Self::Io { name, detail } => write!(formatter, "{name} source I/O failed: {detail}"),
        }
    }
}

impl std::error::Error for SourceError {}
pub(super) fn pinned_zlib_sha256(version: &str) -> Result<&'static str, SourceError> {
    ZLIB_SHA256
        .iter()
        .find_map(|(pinned, digest)| (*pinned == version).then_some(*digest))
        .ok_or_else(|| SourceError::NoPinnedChecksum {
            name: "zlib",
            version: version.to_owned(),
        })
}
