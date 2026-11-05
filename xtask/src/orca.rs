//! Orca CLI command lines and host-owned worktree/worker discovery.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

/// One fully resolved process invocation. Keeping commands as data lets the
/// maintenance commands render `--dry-run` plans without touching a live Orca.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandLine {
    pub program: String,
    pub args: Vec<String>,
}

impl CommandLine {
    pub fn new(program: impl Into<String>, args: impl IntoIterator<Item = String>) -> Self {
        Self {
            program: program.into(),
            args: args.into_iter().collect(),
        }
    }

    /// Render a command that can be pasted into a POSIX shell. Windows command
    /// lines are printed for inspection only; execution never goes through a shell.
    pub fn render(&self) -> String {
        let mut rendered = quote(&self.program);
        for arg in &self.args {
            rendered.push(' ');
            rendered.push_str(&quote(arg));
        }
        rendered
    }

    pub fn run(&self) -> Result<Output, String> {
        Command::new(&self.program)
            .args(&self.args)
            .output()
            .map_err(|error| format!("{}: {error}", self.render()))
    }
}

/// Run a command, or print it unchanged in dry-run mode.
pub fn perform(command: &CommandLine, dry_run: bool) -> Result<(), String> {
    if dry_run {
        println!("{}", command.render());
        return Ok(());
    }
    let output = command.run()?;
    if output.status.success() {
        Ok(())
    } else {
        Err(format!(
            "{}: {}",
            command.render(),
            String::from_utf8_lossy(&output.stderr).trim()
        ))
    }
}

fn quote(value: &str) -> String {
    let safe = !value.is_empty()
        && value.chars().all(|c| {
            c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '/' | ':' | '@' | '=' | '+')
        });
    if safe {
        value.to_owned()
    } else {
        format!("'{}'", value.replace('\'', "'\\''"))
    }
}

/// Build an `orca` invocation. `--environment` is appended as a normal command
/// option, so the same builder serves local and paired-server invocations.
pub fn command(environment: Option<&str>, args: &[&str]) -> CommandLine {
    let mut arguments: Vec<String> = args.iter().map(|arg| (*arg).to_owned()).collect();
    if let Some(environment) = environment {
        arguments.push("--environment".into());
        arguments.push(environment.to_owned());
    }
    CommandLine::new("orca", arguments)
}

fn json(environment: Option<&str>, args: &[&str]) -> Result<serde_json::Value, String> {
    command_json(&command(environment, args))
}

/// Execute an Orca command and require its structured receipt.
pub fn command_json(command: &CommandLine) -> Result<serde_json::Value, String> {
    let output = command.run()?;
    if !output.status.success() {
        return Err(format!(
            "{}: {}",
            command.render(),
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    serde_json::from_slice(&output.stdout)
        .map_err(|error| format!("{}: invalid JSON: {error}", command.render()))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Worktree {
    pub path: PathBuf,
    pub name: String,
    pub is_main: bool,
}

fn worktree_list(environment: Option<&str>) -> Result<Vec<Worktree>, String> {
    let value = json(environment, &["worktree", "list", "--json"])?;
    let rows = value
        .get("result")
        .and_then(|result| result.get("worktrees"))
        .and_then(serde_json::Value::as_array)
        .ok_or("orca worktree list returned no worktrees array")?;
    let mut worktrees = Vec::with_capacity(rows.len());
    for row in rows {
        let path = row
            .get("path")
            .and_then(serde_json::Value::as_str)
            .ok_or("orca worktree row has no path")?;
        let name = match row
            .get("displayName")
            .and_then(serde_json::Value::as_str)
            .filter(|name| !name.is_empty())
        {
            Some(name) => name.to_owned(),
            None => PathBuf::from(path)
                .file_name()
                .and_then(|name| name.to_str())
                .map(str::to_owned)
                .ok_or_else(|| format!("orca worktree {path} has no usable name"))?,
        };
        worktrees.push(Worktree {
            path: PathBuf::from(path),
            name,
            is_main: row
                .get("isMainWorktree")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false),
        });
    }
    Ok(worktrees)
}

/// Worktrees visible to this host, or to a paired Orca server.
pub fn worktrees(environment: Option<&str>) -> Result<Vec<Worktree>, String> {
    worktree_list(environment)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Worker {
    pub dispatch_id: String,
    pub terminal: Option<String>,
}

pub fn workers(environment: Option<&str>) -> Result<Vec<Worker>, String> {
    let value = json(
        environment,
        &["orchestration", "worker-list", "--json", "--limit", "100"],
    )?;
    let rows = value
        .get("result")
        .and_then(|result| result.get("workers"))
        .and_then(serde_json::Value::as_array)
        .ok_or("orca worker-list returned no workers array")?;
    let mut workers = Vec::with_capacity(rows.len());
    for row in rows {
        let dispatch_id = row
            .get("dispatchId")
            .and_then(serde_json::Value::as_str)
            .ok_or("orca worker row has no dispatchId")?;
        workers.push(Worker {
            dispatch_id: dispatch_id.to_owned(),
            terminal: row
                .get("terminal")
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned),
        });
    }
    Ok(workers)
}

/// A worktree name is one path component: no traversal or host-specific
/// separators. Dispatch IDs use their own `ctx_` classifier.
pub fn worktree_name(value: &str) -> Result<(), String> {
    if value.is_empty()
        || matches!(value, "." | "..")
        || value
            .chars()
            .any(|c| matches!(c, '/' | '\\' | ':' | '\0') || c.is_control())
    {
        return Err(format!("invalid worktree name `{value}`"));
    }
    Ok(())
}

/// Parent directory Orca creates linked worktrees in, derived from the listed
/// checkouts so no maintainer-specific path is compiled into the tool.
pub fn workspaces_dir(worktrees: &[Worktree]) -> Option<PathBuf> {
    worktrees
        .iter()
        .find(|worktree| !worktree.is_main)
        .and_then(|worktree| worktree.path.parent())
        .map(Path::to_path_buf)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_appends_environment_and_quotes_only_unsafe_arguments() {
        let list = command(Some("Window"), &["worktree", "list", "--json"]);
        assert_eq!(
            list.render(),
            "orca worktree list --json --environment Window"
        );
        let close = command(None, &["terminal", "close", "--terminal", "term a"]);
        assert_eq!(close.render(), "orca terminal close --terminal 'term a'");
    }

    #[test]
    fn dry_run_prints_instead_of_spawning_the_command() {
        let missing = CommandLine::new("vibecut-missing-command", Vec::new());
        assert!(perform(&missing, true).is_ok());
        assert!(missing.run().is_err());
    }

    #[test]
    fn workspaces_dir_uses_the_parent_of_a_linked_worktree() {
        let worktrees = vec![
            Worktree {
                path: PathBuf::from("/repo"),
                name: "repo".into(),
                is_main: true,
            },
            Worktree {
                path: PathBuf::from("/workspaces/repo/ticket-58"),
                name: "ticket-58".into(),
                is_main: false,
            },
        ];
        assert_eq!(
            workspaces_dir(&worktrees),
            Some(PathBuf::from("/workspaces/repo"))
        );
        assert_eq!(workspaces_dir(&worktrees[..1]), None);
    }
}
