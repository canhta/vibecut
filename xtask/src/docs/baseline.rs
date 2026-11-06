//! Reads the only-shrinking `xtask/docs-baseline.txt` violation list.
use super::{BASELINE, INDEX, MAP, TEXT, Violation};
use std::{collections::BTreeSet, fs, path::Path};

pub(super) fn baseline(root: &Path) -> Result<BTreeSet<Violation>, String> {
    let path = root.join(BASELINE);
    let text = fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut lines = Vec::new();
    let mut entries = BTreeSet::new();
    for (index, line) in text.lines().enumerate() {
        if line.is_empty() {
            continue;
        }
        let number = index + 1;
        let mut fields = line.splitn(3, '\t');
        let rule = fields
            .next()
            .and_then(rule_id)
            .ok_or_else(|| format!("{BASELINE}:{number}: unknown rule"))?;
        let file = fields
            .next()
            .ok_or_else(|| format!("{BASELINE}:{number}: expected `rule<TAB>path<TAB>detail`"))?;
        let detail = fields
            .next()
            .ok_or_else(|| format!("{BASELINE}:{number}: expected `rule<TAB>path<TAB>detail`"))?;
        if detail.contains('\t') {
            return Err(format!(
                "{BASELINE}:{number}: detail must not contain a tab"
            ));
        }
        entries.insert(Violation {
            path: file.to_owned(),
            rule,
            detail: detail.to_owned(),
        });
        lines.push(line.to_owned());
    }
    let mut sorted = lines.clone();
    sorted.sort();
    sorted.dedup();
    if lines != sorted {
        return Err(format!(
            "{BASELINE}: entries must be sorted by rule, path and detail without duplicates"
        ));
    }
    Ok(entries)
}

pub(super) fn rule_id(rule: &str) -> Option<&'static str> {
    match rule {
        INDEX => Some(INDEX),
        MAP => Some(MAP),
        TEXT => Some(TEXT),
        _ => None,
    }
}
