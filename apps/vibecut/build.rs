//! Embeds the Windows manifest and icon resource and generates the About dialog's licence list.
use std::{
    collections::BTreeMap,
    error::Error,
    fs,
    path::{Path, PathBuf},
};

fn main() -> Result<(), Box<dyn Error>> {
    #[cfg(windows)]
    embed_resource::compile("vibecut.rc", embed_resource::NONE).manifest_required()?;
    let assets = Path::new("../../assets");
    println!("cargo:rerun-if-changed={}", assets.display());
    let mut sidecars = Vec::new();
    collect(assets, &mut sidecars)?;
    sidecars.sort();
    // One row per author and licence; the first sidecar in path order names the source.
    let mut notices: BTreeMap<(String, String), String> = BTreeMap::new();
    for path in &sidecars {
        let text = fs::read_to_string(path)?;
        let field = |name: &str| {
            text.lines()
                .find_map(|line| line.strip_prefix(name)?.strip_prefix(": "))
                .map(str::trim)
        };
        let (Some(author), Some(licence), Some(source)) =
            (field("author"), field("license"), field("source"))
        else {
            continue;
        };
        if source == "original work" {
            continue;
        }
        notices
            .entry((author.to_owned(), licence.to_owned()))
            .or_insert_with(|| source.to_owned());
    }
    let mut generated = String::from("&[\n");
    for ((author, licence), source) in &notices {
        generated.push_str(&format!(
            "    Notice {{ author: {author:?}, licence: {licence:?}, source: {source:?} }},\n"
        ));
    }
    generated.push_str("]\n");
    let out = PathBuf::from(std::env::var("OUT_DIR")?).join("licences.rs");
    fs::write(out, generated)?;
    Ok(())
}

/// Every `.attribution` sidecar under `dir`.
fn collect(dir: &Path, found: &mut Vec<PathBuf>) -> Result<(), Box<dyn Error>> {
    for entry in fs::read_dir(dir)? {
        let path = entry?.path();
        if path.is_dir() {
            collect(&path, found)?;
        } else if path.extension().is_some_and(|ext| ext == "attribution") {
            found.push(path);
        }
    }
    Ok(())
}
