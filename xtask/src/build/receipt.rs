//! Clean-tree CI receipts that let an identical tree reuse a green run.
use std::{
    ffi::OsString,
    path::{Path, PathBuf},
};

pub(super) fn clean_tree(root: &Path) -> Result<Option<String>, String> {
    if !crate::process::checked(
        root,
        "git",
        &["status", "--porcelain", "--untracked-files=normal"],
    )?
    .is_empty()
    {
        return Ok(None);
    }
    String::from_utf8(crate::process::checked(
        root,
        "git",
        &["rev-parse", "HEAD^{tree}"],
    )?)
    .map(|tree| Some(tree.trim().to_owned()))
    .map_err(|e| e.to_string())
}

pub(super) fn receipt(
    root: &Path,
    base: &Path,
    args: &[OsString],
) -> Result<Option<PathBuf>, String> {
    if args.first().is_none_or(|s| s != "cargo")
        || args.get(1).is_none_or(|s| s != "xtask")
        || args.get(2).is_none_or(|s| s != "ci")
    {
        return Ok(None);
    }
    let mode = match args.get(3..) {
        Some([]) => "plain",
        Some([flag]) if flag == "--require-oracles" => "oracles",
        _ => return Ok(None),
    };
    Ok(clean_tree(root)?.map(|tree| base.join(format!("ci-receipts/{tree}-{mode}"))))
}
