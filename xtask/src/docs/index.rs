//! Every tracked page under `docs/` is listed in `docs/README.md`.
use super::{INDEX, Violation};
use std::{collections::BTreeSet, fs, path::Path};

pub(super) fn check(root: &Path, tracked: &[String]) -> Result<BTreeSet<Violation>, String> {
    let path = root.join("docs/README.md");
    let readme = fs::read_to_string(&path).unwrap_or_default();
    let mut violations = BTreeSet::new();
    for file in tracked {
        let Some(relative) = file.strip_prefix("docs/") else {
            continue;
        };
        if relative == "README.md" || listed(&readme, relative) {
            continue;
        }
        violations.insert(Violation {
            path: file.clone(),
            rule: INDEX,
            detail: "not listed in docs/README.md".into(),
        });
    }
    Ok(violations)
}

fn listed(readme: &str, relative: &str) -> bool {
    readme.split("](").skip(1).any(|rest| {
        let mut target = rest.split(')').next().unwrap_or_default();
        target = target.split('#').next().unwrap_or(target).trim();
        let target = target.strip_prefix("./").unwrap_or(target);
        let target = target.strip_prefix("docs/").unwrap_or(target);
        target == relative
    })
}
