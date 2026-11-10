//! Main-checkout discovery, so linked worktrees read the main checkout's private names list.
use std::path::{Path, PathBuf};

pub fn main(root: &Path) -> Result<PathBuf, String> {
    let bytes = crate::process::checked(
        root,
        "git",
        &["rev-parse", "--path-format=absolute", "--git-common-dir"],
    )?;
    let common = PathBuf::from(
        String::from_utf8(bytes)
            .map_err(|_| "git common directory is not UTF-8")?
            .trim_end_matches(['\r', '\n']),
    );
    common
        .parent()
        .map(Path::to_path_buf)
        .ok_or_else(|| "git common directory has no parent".into())
}
