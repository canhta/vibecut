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
    pub branch: Option<String>,
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
            branch: row
                .get("branch")
                .and_then(serde_json::Value::as_str)
                .and_then(|branch| branch.strip_prefix("refs/heads/"))
                .filter(|branch| !branch.is_empty())
                .map(str::to_owned),
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
    pub state: Option<String>,
    pub terminal: Option<String>,
    pub terminal_state: Option<String>,
    pub host: Option<String>,
    pub owner_dispatch_id: Option<String>,
    pub release_state: Option<String>,
}

impl Worker {
    /// A worker state that has stopped producing work; its terminal may still
    /// be alive and owned by this or a follow-up dispatch.
    pub fn is_settled(&self) -> bool {
        matches!(
            self.state.as_deref(),
            Some("succeeded" | "failed" | "stopped" | "abandoned")
        )
    }

    pub fn already_released(&self) -> bool {
        matches!(
            self.terminal_state.as_deref(),
            Some("released" | "already_released")
        ) || matches!(self.release_state.as_deref(), Some("released"))
    }
}

fn row_text<'a>(row: &'a serde_json::Value, path: &[&str]) -> Option<&'a str> {
    path.iter()
        .try_fold(row, |value, key| value.get(*key))
        .and_then(serde_json::Value::as_str)
}

pub fn workers(environment: Option<&str>) -> Result<Vec<Worker>, String> {
    // `--full` exposes the terminal resource (owner and release state); the
    // concise shape is still parsed so an older runtime remains usable.
    let value = json(
        environment,
        &[
            "orchestration",
            "worker-list",
            "--json",
            "--full",
            "--limit",
            "100",
        ],
    )?;
    let rows = value
        .get("result")
        .and_then(|result| result.get("workers"))
        .and_then(serde_json::Value::as_array)
        .ok_or("orca worker-list returned no workers array")?;
    let mut workers = Vec::with_capacity(rows.len());
    for row in rows {
        workers.push(worker_row(row)?);
    }
    Ok(workers)
}

fn worker_row(row: &serde_json::Value) -> Result<Worker, String> {
    let dispatch_id = row
        .get("dispatchId")
        .and_then(serde_json::Value::as_str)
        .ok_or("orca worker row has no dispatchId")?;
    let text = |path: &[&str]| row_text(row, path).map(str::to_owned);
    Ok(Worker {
        dispatch_id: dispatch_id.to_owned(),
        state: text(&["state"]).or_else(|| text(&["workerState"])),
        terminal: text(&["terminal"]).or_else(|| text(&["agentTerminalHandle"])),
        terminal_state: text(&["terminalState"])
            .or_else(|| text(&["resource", "terminalState"]))
            .or_else(|| text(&["projection", "resource", "terminalState"])),
        host: text(&["host"]).or_else(|| text(&["projection", "host", "id"])),
        owner_dispatch_id: text(&["resource", "ownerDispatchId"])
            .or_else(|| text(&["projection", "resource", "ownerDispatchId"])),
        release_state: text(&["resource", "releaseState"])
            .or_else(|| text(&["projection", "resource", "releaseState"])),
    })
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
    fn worker_rows_parse_concise_and_full_receipts() {
        let concise = serde_json::json!({
            "dispatchId": "ctx_a",
            "state": "succeeded",
            "terminal": null,
            "terminalState": "released",
            "host": "local"
        });
        assert_eq!(
            worker_row(&concise).unwrap(),
            Worker {
                dispatch_id: "ctx_a".into(),
                state: Some("succeeded".into()),
                terminal: None,
                terminal_state: Some("released".into()),
                host: Some("local".into()),
                owner_dispatch_id: None,
                release_state: None,
            }
        );
        let full = serde_json::json!({
            "dispatchId": "ctx_b",
            "workerState": "failed",
            "agentTerminalHandle": "term_x",
            "resource": {
                "ownerDispatchId": "ctx_c",
                "releaseState": "retained",
                "terminalState": "retained"
            },
            "projection": {"host": {"kind": "remote", "id": "Window"}}
        });
        assert_eq!(
            worker_row(&full).unwrap(),
            Worker {
                dispatch_id: "ctx_b".into(),
                state: Some("failed".into()),
                terminal: Some("term_x".into()),
                terminal_state: Some("retained".into()),
                host: Some("Window".into()),
                owner_dispatch_id: Some("ctx_c".into()),
                release_state: Some("retained".into()),
            }
        );
        assert!(worker_row(&serde_json::json!({"state": "succeeded"})).is_err());
        assert!(
            !Worker {
                dispatch_id: "ctx_d".into(),
                state: None,
                terminal: None,
                terminal_state: None,
                host: None,
                owner_dispatch_id: None,
                release_state: None,
            }
            .is_settled()
        );
    }

    #[test]
    fn workspaces_dir_uses_the_parent_of_a_linked_worktree() {
        let worktrees = vec![
            Worktree {
                path: PathBuf::from("/repo"),
                name: "repo".into(),
                branch: Some("main".into()),
                is_main: true,
            },
            Worktree {
                path: PathBuf::from("/workspaces/repo/ticket-58"),
                name: "ticket-58".into(),
                branch: Some("fix/ticket-58".into()),
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
