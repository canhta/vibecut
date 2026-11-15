//! Embeds the Windows manifest and icon resource and generates the About dialog's facts: its licence list and build commit.
use std::{
    collections::BTreeMap,
    error::Error,
    fs,
    path::{Path, PathBuf},
    process::Command,
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
    commit();
    Ok(())
}

/// Set `VIBECUT_COMMIT` to the checkout's short HEAD for the About dialog, rerunning when HEAD
/// or the branch it names moves. Outside a Git checkout the variable stays unset.
fn commit() {
    let git = |args: &[&str]| {
        Command::new("git")
            .args(args)
            .output()
            .ok()
            .filter(|output| output.status.success())
            .and_then(|output| String::from_utf8(output.stdout).ok())
            .map(|text| text.trim().to_owned())
            .filter(|text| !text.is_empty())
    };
    let Some(sha) = git(&["rev-parse", "--short", "HEAD"]) else {
        return;
    };
    let mut watched = vec!["HEAD".to_owned(), "packed-refs".to_owned()];
    watched.extend(git(&["symbolic-ref", "-q", "HEAD"]));
    for name in watched {
        // A missing path would rerun the script on every build; watch only what exists.
        if let Some(path) = git(&["rev-parse", "--path-format=absolute", "--git-path", &name])
            && Path::new(&path).exists()
        {
            println!("cargo:rerun-if-changed={path}");
        }
    }
    println!("cargo:rustc-env=VIBECUT_COMMIT={sha}");
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
