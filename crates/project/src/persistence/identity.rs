//! Identifies retained filesystem objects.
use std::{
    fs::{self, File},
    path::Path,
};

#[cfg(unix)]
pub(super) type Identity = (u64, u64);
#[cfg(windows)]
pub(super) type Identity = file_id::FileId;
#[cfg(not(any(unix, windows)))]
pub(super) type Identity = ();

#[cfg(unix)]
pub(super) fn held_identity(file: &File, _: &Path) -> std::io::Result<Identity> {
    use std::os::unix::fs::MetadataExt;
    let metadata = file.metadata()?;
    Ok((metadata.dev(), metadata.ino()))
}
#[cfg(unix)]
pub(super) fn named_identity(_: &Path, metadata: &fs::Metadata) -> std::io::Result<Identity> {
    use std::os::unix::fs::MetadataExt;
    Ok((metadata.dev(), metadata.ino()))
}
#[cfg(windows)]
pub(super) fn held_identity(_: &File, path: &Path) -> std::io::Result<Identity> {
    // The file is already open without delete sharing. Do not degrade to 64-bit
    // IDs: those do not establish object identity on ReFS.
    file_id::get_high_res_file_id(path)
}
#[cfg(windows)]
pub(super) fn named_identity(path: &Path, _: &fs::Metadata) -> std::io::Result<Identity> {
    file_id::get_high_res_file_id(path)
}
#[cfg(not(any(unix, windows)))]
pub(super) fn held_identity(_: &File, _: &Path) -> std::io::Result<Identity> {
    Err(std::io::ErrorKind::Unsupported.into())
}
#[cfg(not(any(unix, windows)))]
pub(super) fn named_identity(_: &Path, _: &fs::Metadata) -> std::io::Result<Identity> {
    Err(std::io::ErrorKind::Unsupported.into())
}
