//! Cleanup arguments: dispatch/worktree classification and option parsing.
use crate::orca;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Target {
    Dispatch(String),
    Worktree(String),
}

#[derive(Debug, Default, PartialEq, Eq)]
pub(super) struct Options {
    pub(super) environment: Option<String>,
    pub(super) dry_run: bool,
    pub(super) targets: Vec<Target>,
}

pub(super) fn target(value: &str) -> Result<Target, String> {
    if let Some(dispatch) = value.strip_prefix("ctx_") {
        if dispatch.is_empty()
            || !dispatch
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-'))
        {
            return Err(format!("invalid dispatch id `{value}`"));
        }
        return Ok(Target::Dispatch(value.to_owned()));
    }
    orca::worktree_name(value)?;
    Ok(Target::Worktree(value.to_owned()))
}

pub(super) fn parse(args: &[String]) -> Result<Options, String> {
    let mut options = Options::default();
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--dry-run" => options.dry_run = true,
            "--environment" => {
                if options.environment.is_some() {
                    return Err("--environment may be given only once".into());
                }
                let value = args
                    .next()
                    .ok_or("--environment requires a saved environment name")?;
                if value.is_empty() || value.starts_with('-') {
                    return Err(format!("invalid environment `{value}`"));
                }
                options.environment = Some(value.clone());
            }
            "--" => {
                for value in args.by_ref() {
                    options.targets.push(target(value)?);
                }
            }
            value if value.starts_with("--") => {
                return Err(format!("unknown option `{value}`"));
            }
            value => options.targets.push(target(value)?),
        }
    }
    if options.targets.is_empty() {
        return Err(
            "usage: cargo xtask cleanup [--environment NAME] [--dry-run] <ctx_dispatch|worktree-name>..."
                .into(),
        );
    }
    Ok(options)
}
