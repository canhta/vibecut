//! Resolve stable source versions, fetch them and extract a pristine tree.
use super::{
    archives::{fetch, sha256},
    command::{capture, run},
    verify::{SourceError, pinned_zlib_sha256},
};
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

pub(super) const X264_VERSION: &str = "20191217-2245";
pub(super) const X264_URL: &str = "https://download.videolan.org/pub/videolan/x264/snapshots/x264-snapshot-20191217-2245-stable.tar.bz2";
pub(super) const ZLIB_REPOSITORY: &str = "https://github.com/madler/zlib.git";
pub(super) fn stable_tag(repository: &str, prefix: &str) -> Result<String, String> {
    let text = capture(Command::new("git").args(["ls-remote", "--tags", "--refs", repository]))?;
    text.lines()
        .filter_map(|line| line.split("refs/tags/").nth(1))
        .filter_map(|tag| {
            let version = tag.strip_prefix(prefix)?;
            let parts: Option<Vec<u64>> = version.split('.').map(|n| n.parse().ok()).collect();
            let mut parts = parts?;
            if parts.len() == 2 {
                parts.push(0);
            }
            if parts.len() == 3 {
                Some((parts, version.to_owned()))
            } else {
                None
            }
        })
        .max_by(|a, b| a.0.cmp(&b.0))
        .map(|(_, version)| version)
        .ok_or_else(|| format!("no stable release in {repository}"))
}

pub(super) fn cache_key_for(ffmpeg: &str, opus: &str, vpx: &str, zlib: &str) -> String {
    format!("ffmpeg{ffmpeg}-opus{opus}-vpx{vpx}-zlib{zlib}-x264{X264_VERSION}")
}

/// Resolve the versions used by provisioning before restoring its hosted cache.
pub fn cache_key() -> Result<(), String> {
    let version = stable_tag("https://github.com/FFmpeg/FFmpeg.git", "n")?;
    let opus = stable_tag("https://github.com/xiph/opus.git", "v")?;
    let vpx = stable_tag("https://github.com/webmproject/libvpx.git", "v")?;
    let zlib = stable_tag(ZLIB_REPOSITORY, "v")?;
    println!("{}", cache_key_for(&version, &opus, &vpx, &zlib));
    Ok(())
}
pub(super) fn source(
    base: &Path,
    name: &str,
    version: &str,
    url: &str,
    receipt: &mut String,
) -> Result<PathBuf, String> {
    let archive = base.join(format!("{name}-{version}.archive"));
    if !archive.exists() {
        fetch(url, &archive)?;
    }
    let measured = sha256(&archive)?;
    // FFmpeg distributes detached signatures rather than an archive checksum;
    // other upstreams do not consistently publish checksums for source snapshots.
    // Verify a published SHA-256 when the canonical archive has one.
    // Upstream FFmpeg releases publish detached OpenPGP signatures, and GitHub
    // source snapshots publish no SHA-256 sidecar. This measured hash records
    // bytes fetched over verified HTTPS; it is not an upstream checksum claim.
    let published = false;
    receipt.push_str(&format!("{name}: {version}\nsource: {url}\nsha256: {measured}\nupstream checksum verified: {published}\n\n"));
    let dir = base.join(format!("{name}-{version}"));
    if !dir.exists() {
        fs::create_dir(&dir).map_err(|e| e.to_string())?;
        run(Command::new("tar")
            .arg("-xf")
            .arg(&archive)
            .arg("--strip-components=1")
            .arg("-C")
            .arg(&dir))?;
    }
    Ok(dir)
}
/// Fetch, verify and extract zlib before any install prefix exists.
pub(super) fn zlib_source(
    base: &Path,
    version: &str,
    receipt: &mut String,
) -> Result<PathBuf, SourceError> {
    let expected = pinned_zlib_sha256(version)?;
    let sources = base.join("sources");
    fs::create_dir_all(&sources).map_err(|error| SourceError::Io {
        name: "zlib",
        detail: error.to_string(),
    })?;
    let archive = sources.join(format!("zlib-{version}.archive"));
    let source = format!("https://zlib.net/fossils/zlib-{version}.tar.gz");
    if !archive.exists() {
        fetch(&source, &archive).map_err(|detail| SourceError::Download {
            name: "zlib",
            version: version.to_owned(),
            source: source.clone(),
            detail,
        })?;
    }
    let measured = sha256(&archive).map_err(|detail| SourceError::Io {
        name: "zlib",
        detail,
    })?;
    if measured != expected {
        let extracted = sources.join(format!("zlib-{version}"));
        fs::remove_file(&archive).map_err(|error| SourceError::Io {
            name: "zlib",
            detail: format!("cannot remove the mismatched archive: {error}"),
        })?;
        if extracted.exists() {
            fs::remove_dir_all(&extracted).map_err(|error| SourceError::Io {
                name: "zlib",
                detail: format!("cannot remove the mismatched extraction: {error}"),
            })?;
        }
        return Err(SourceError::ChecksumMismatch {
            name: "zlib",
            version: version.to_owned(),
            expected,
            actual: measured,
        });
    }
    receipt.push_str(&format!(
        "zlib: {version}\nsource: {source}\nsha256: {measured}\nupstream checksum verified: true\n\n"
    ));
    let dir = sources.join(format!("zlib-{version}"));
    if !dir.exists() {
        fs::create_dir(&dir).map_err(|error| SourceError::Io {
            name: "zlib",
            detail: error.to_string(),
        })?;
        run(Command::new("tar")
            .arg("-xf")
            .arg(&archive)
            .arg("--strip-components=1")
            .arg("-C")
            .arg(&dir))
        .map_err(|detail| SourceError::Extract {
            name: "zlib",
            version: version.to_owned(),
            detail,
        })?;
    }
    Ok(dir)
}
