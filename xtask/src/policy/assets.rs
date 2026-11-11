//! Asset attribution policy: every shipped asset carries a populated sidecar.
use std::fs;
use std::path::Path;

pub(crate) fn check(root: &Path) -> Result<(), String> {
    fn walk(dir: &Path) -> Result<(), String> {
        if !dir.exists() {
            return Ok(());
        }
        for entry in fs::read_dir(dir).map_err(|e| e.to_string())? {
            let path = entry.map_err(|e| e.to_string())?.path();
            if path.is_dir() {
                walk(&path)?;
            } else if path.file_name().is_some_and(|n| n != "README.md")
                && path.extension().is_none_or(|e| e != "attribution")
            {
                let mut sidecar = path.as_os_str().to_os_string();
                sidecar.push(".attribution");
                let content = fs::read_to_string(&sidecar)
                    .map_err(|e| format!("{}: attribution missing: {e}", path.display()))?;
                for key in [
                    "asset",
                    "title",
                    "author",
                    "source",
                    "license",
                    "license-file",
                    "added",
                    "notes",
                ] {
                    if !content.lines().any(|line| {
                        line.strip_prefix(&format!("{key}:"))
                            .is_some_and(|v| !v.trim().is_empty())
                    }) {
                        return Err(format!(
                            "{}: empty/missing {key}",
                            Path::new(&sidecar).display()
                        ));
                    }
                }
            }
        }
        Ok(())
    }
    walk(&root.join("assets"))?;
    walk(&root.join("docs/brand"))?;
    walk(&root.join("docs/assets/readme"))?;
    eprintln!("assets: ok");
    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::panic)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn readme_media_requires_populated_attribution_but_ignores_its_readme() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let media = root.join("docs/assets/readme");
        fs::create_dir_all(&media).unwrap();
        fs::write(media.join("README.md"), "provenance owner").unwrap();
        fs::write(media.join("hero.jpg"), b"jpeg").unwrap();
        let missing = check(root).unwrap_err();
        assert!(missing.contains("attribution missing"), "{missing}");
        fs::write(media.join("hero.jpg.attribution"), "asset: hero.jpg\n").unwrap();
        let empty = check(root).unwrap_err();
        assert!(empty.contains("empty/missing title"), "{empty}");
        fs::write(
            media.join("hero.jpg.attribution"),
            "asset: hero.jpg\ntitle: Editor\nauthor: VibeCut contributors\nsource: capture\nlicense: GPL-3.0-only\nlicense-file: ../../../LICENSE\nadded: p0-core/22\nnotes: generated\n",
        )
        .unwrap();
        check(root).unwrap();
    }
}
