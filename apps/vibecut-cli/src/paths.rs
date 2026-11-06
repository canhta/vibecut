use crate::exit_codes::CliError;
use std::io;
use std::path::Path;
use std::path::PathBuf;

pub(crate) fn same_target(left: &Path, right: &Path) -> Result<bool, CliError> {
    Ok(resolve_target(left)? == resolve_target(right)?)
}

pub(crate) fn resolve_target(path: &Path) -> Result<PathBuf, CliError> {
    match std::fs::canonicalize(path) {
        Ok(path) => Ok(path),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            let name = path
                .file_name()
                .ok_or(CliError::Usage("path has no file name".into()))?;
            let parent = path
                .parent()
                .filter(|parent| !parent.as_os_str().is_empty())
                .unwrap_or(Path::new("."));
            Ok(std::fs::canonicalize(parent)?.join(name))
        }
        Err(error) => Err(error.into()),
    }
}
