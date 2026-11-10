//! Forbidden-name policy: tracked files and branch commit messages carry no private names.
use crate::checkout;
use crate::process::{checked, output};
use std::fs;
use std::path::Path;

pub(crate) fn check(root: &Path) -> Result<(), String> {
    let patterns = checkout::main(root)?.join(".scratch/forbidden-names.txt");
    if !patterns.exists() {
        return Ok(());
    }
    let bytes = fs::read_to_string(&patterns).map_err(|e| e.to_string())?;
    let patterns: Vec<_> = bytes.lines().filter(|s| !s.trim().is_empty()).collect();
    let args: Vec<_> = patterns.iter().flat_map(|s| ["-e", *s]).collect();
    if patterns.is_empty() {
        return Ok(());
    }
    let mut grep_args = vec!["grep", "-l", "-i", "-E"];
    grep_args.extend(&args);
    grep_args.push("--");
    let out = output(root, "git", &grep_args)?;
    match out.status.code() {
        Some(1) => (),
        Some(0) => {
            return Err(format!(
                "forbidden tracked content:\n{}",
                String::from_utf8_lossy(&out.stdout)
            ));
        }
        _ => {
            return Err(format!(
                "name check: {}",
                String::from_utf8_lossy(&out.stderr)
            ));
        }
    }
    let base = checked(root, "git", &["merge-base", "main", "HEAD"])?;
    let base = String::from_utf8(base).map_err(|e| e.to_string())?;
    let head = checked(root, "git", &["rev-parse", "HEAD"])?;
    let range = if base.as_bytes() == head {
        "HEAD^!".into()
    } else {
        format!("{}..HEAD", base.trim())
    };
    let mut log_args = vec![
        "log",
        "--format=%H %B",
        "--regexp-ignore-case",
        "--extended-regexp",
    ];
    let filters: Vec<_> = patterns.iter().map(|s| format!("--grep={s}")).collect();
    log_args.extend(filters.iter().map(String::as_str));
    log_args.push(&range);
    let matches = checked(root, "git", &log_args)?;
    if !matches.is_empty() {
        return Err(format!(
            "forbidden branch commit message:\n{}",
            String::from_utf8_lossy(&matches)
        ));
    }
    eprintln!("names: ok");
    Ok(())
}
