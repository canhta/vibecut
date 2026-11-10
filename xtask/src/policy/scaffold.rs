//! Admitted crate scaffolding from the architecture table.
use super::edges;
use std::fs;
use std::path::Path;

pub(crate) fn new_crate(root: &Path, name: &str) -> Result<(), String> {
    let rules = edges::policy(root)?;
    let path = &rules
        .get(name)
        .ok_or("name must be admitted in architecture.md")?
        .path;
    if !path.starts_with("crates/") {
        return Err("scaffold supports core libraries only".into());
    }
    let dir = root.join(path);
    fs::create_dir(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let result = (|| {
        fs::create_dir(dir.join("src")).map_err(|e| e.to_string())?;
        fs::write(
            dir.join("src/lib.rs"),
            format!("//! {name}: see the crate README for its contract.\n"),
        )
        .map_err(|e| e.to_string())?;
        fs::write(dir.join("Cargo.toml"), format!("[package]\nname = \"{name}\"\nversion.workspace = true\nedition.workspace = true\nlicense.workspace = true\npublish = false\n\n[lints]\nworkspace = true\n")).map_err(|e| e.to_string())?;
        fs::write(dir.join("README.md"), format!("# {name}\n\nThe [architecture](../../docs/engineering/architecture.md#crate-map) owns this crate's responsibility.\nBefore delivery, document purpose/exclusions, the public interface, the module map and each module's contract under the [README contract](../../docs/engineering/standards.md#public-apis-and-crate-readme).\n")).map_err(|e| e.to_string())
    })();
    if result.is_err() {
        fs::remove_dir_all(&dir).map_err(|e| e.to_string())?;
    }
    result
}
