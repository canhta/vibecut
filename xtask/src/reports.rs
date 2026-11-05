//! Copy reports from ticket worktrees into the main checkout without losing
//! newer local work.

use crate::orca::{self, CommandLine};
use std::fs;
use std::path::{Path, PathBuf};
use std::process;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::SystemTime;

#[derive(Debug, Default, PartialEq, Eq)]
struct Options {
    environment: Option<String>,
    ssh: Option<String>,
    dry_run: bool,
    targets: Vec<String>,
}

fn parse(args: &[String]) -> Result<Options, String> {
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
            "--ssh" => {
                if options.ssh.is_some() {
                    return Err("--ssh may be given only once".into());
                }
                let value = args.next().ok_or("--ssh requires a host")?;
                if value.is_empty()
                    || value.starts_with('-')
                    || value.chars().any(char::is_whitespace)
                {
                    return Err(format!("invalid ssh host `{value}`"));
                }
                options.ssh = Some(value.clone());
            }
            "--" => {
                for value in args.by_ref() {
                    orca::worktree_name(value)?;
                    options.targets.push(value.to_owned());
                }
            }
            value if value.starts_with("--") => {
                return Err(format!("unknown option `{value}`"));
            }
            value => {
                orca::worktree_name(value)?;
                options.targets.push(value.to_owned());
            }
        }
    }
    if options.targets.is_empty() {
        return Err(
            "usage: cargo xtask pull-reports [--environment NAME] [--ssh HOST] [--dry-run] <worktree-name>..."
                .into(),
        );
    }
    if options.environment.is_some() && options.ssh.is_none() {
        return Err(
            "pull-reports --environment needs --ssh HOST: Orca exposes no file-transfer command"
                .into(),
        );
    }
    Ok(options)
}

#[derive(Debug, Default, PartialEq, Eq)]
struct Merge {
    copied: usize,
    kept_newer: usize,
    skipped: usize,
}

fn modified(path: &Path) -> Result<SystemTime, String> {
    fs::symlink_metadata(path)
        .and_then(|metadata| metadata.modified())
        .map_err(|error| format!("{}: {error}", path.display()))
}

fn ensure_directory(path: &Path, dry_run: bool) -> Result<(), String> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            Err(format!("{}: refusing to follow a symlink", path.display()))
        }
        Ok(metadata) if metadata.is_dir() => Ok(()),
        Ok(_) => Err(format!("{}: not a directory", path.display())),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            if dry_run {
                Ok(())
            } else {
                fs::create_dir_all(path).map_err(|error| format!("{}: {error}", path.display()))
            }
        }
        Err(error) => Err(format!("{}: {error}", path.display())),
    }
}

fn merge_file(
    source: &Path,
    destination: &Path,
    dry_run: bool,
    merge: &mut Merge,
) -> Result<(), String> {
    let source_modified = modified(source)?;
    match fs::symlink_metadata(destination) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            return Err(format!(
                "{}: refusing to replace a local symlink",
                destination.display()
            ));
        }
        Ok(metadata) if !metadata.is_file() => {
            return Err(format!(
                "{}: local path is not a regular file",
                destination.display()
            ));
        }
        Ok(metadata) => {
            let local_modified = metadata
                .modified()
                .map_err(|error| format!("{}: {error}", destination.display()))?;
            if source_modified <= local_modified {
                merge.kept_newer += 1;
                return Ok(());
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(format!("{}: {error}", destination.display())),
    }
    if dry_run {
        println!("copy {} -> {}", source.display(), destination.display());
    } else {
        fs::copy(source, destination)
            .map_err(|error| format!("{}: {error}", destination.display()))?;
    }
    merge.copied += 1;
    Ok(())
}

fn merge_directory(
    source: &Path,
    destination: &Path,
    dry_run: bool,
    merge: &mut Merge,
) -> Result<(), String> {
    ensure_directory(destination, dry_run)?;
    let entries = fs::read_dir(source).map_err(|error| format!("{}: {error}", source.display()))?;
    for entry in entries {
        let entry = entry.map_err(|error| format!("{}: {error}", source.display()))?;
        let kind = entry
            .file_type()
            .map_err(|error| format!("{}: {error}", entry.path().display()))?;
        let target = destination.join(entry.file_name());
        if kind.is_symlink() {
            merge.skipped += 1;
        } else if kind.is_dir() {
            merge_directory(&entry.path(), &target, dry_run, merge)?;
        } else if kind.is_file() {
            merge_file(&entry.path(), &target, dry_run, merge)?;
        } else {
            merge.skipped += 1;
        }
    }
    Ok(())
}

fn merge_tree(source: &Path, destination: &Path, dry_run: bool) -> Result<Merge, String> {
    if !source.is_dir() {
        return Err(format!("{}: no report directory", source.display()));
    }
    let mut merge = Merge::default();
    merge_directory(source, destination, dry_run, &mut merge)?;
    Ok(merge)
}

fn find<'a>(worktrees: &'a [orca::Worktree], name: &str) -> Option<&'a orca::Worktree> {
    worktrees.iter().find(|worktree| {
        worktree.name == name
            || worktree
                .path
                .file_name()
                .and_then(|file| file.to_str())
                .is_some_and(|file| file == name)
    })
}

fn staging_directory() -> PathBuf {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    std::env::temp_dir().join(format!(
        "vibecut-pull-reports-{}-{}",
        process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ))
}

fn scp_command(ssh: &str, source: &Path, staging: &Path) -> CommandLine {
    // `-p` preserves the remote modification times. Without it every staged
    // report looks freshly downloaded and the merge below would treat stale
    // remote content as newer than a local edit.
    CommandLine::new(
        "scp",
        [
            "-p".to_owned(),
            "-r".to_owned(),
            format!("{ssh}:{}", source.display()),
            staging.to_string_lossy().into_owned(),
        ],
    )
}

fn merge_download(staging: &Path, destination: &Path, dry_run: bool) -> Result<Merge, String> {
    merge_tree(&staging.join(".scratch"), destination, dry_run)
}

fn pull_remote(
    options: &Options,
    worktree: &orca::Worktree,
    destination: &Path,
) -> Result<Merge, String> {
    let ssh = options
        .ssh
        .as_deref()
        .ok_or("--ssh is required for a paired environment")?;
    let source = worktree.path.join(".scratch");
    let staging = staging_directory();
    let scp = scp_command(ssh, &source, &staging);
    if options.dry_run {
        println!("{}", scp.render());
        println!(
            "merge {}/.scratch -> {}",
            staging.display(),
            destination.display()
        );
        return Ok(Merge::default());
    }
    fs::create_dir_all(&staging).map_err(|error| format!("{}: {error}", staging.display()))?;
    let result =
        orca::perform(&scp, false).and_then(|()| merge_download(&staging, destination, false));
    if let Err(error) = fs::remove_dir_all(&staging) {
        eprintln!("[pull-reports] {}: {error}", staging.display());
    }
    result
}

fn pull_one(
    options: &Options,
    worktrees: &[orca::Worktree],
    main: &Path,
    name: &str,
) -> Result<Merge, String> {
    let worktree =
        find(worktrees, name).ok_or_else(|| format!("no Orca worktree named `{name}`"))?;
    let destination = main.join(".scratch");
    if options.environment.is_none() {
        let source = worktree.path.join(".scratch");
        if source == destination {
            return Err("refusing to pull the main checkout into itself".into());
        }
        return merge_tree(&source, &destination, options.dry_run);
    }
    pull_remote(options, worktree, &destination)
}

pub fn run(root: &Path, args: &[String]) -> Result<(), String> {
    let options = parse(args)?;
    let main = crate::checkout::main(root)?;
    let worktrees = orca::worktrees(options.environment.as_deref())?;
    let mut failures = 0usize;
    for name in &options.targets {
        match pull_one(&options, &worktrees, &main, name) {
            Ok(merge) => {
                if options.dry_run {
                    println!("[pull-reports] {name}: planned");
                } else {
                    println!(
                        "[pull-reports] {name}: copied {}, kept {} newer local, skipped {}",
                        merge.copied, merge.kept_newer, merge.skipped
                    );
                }
            }
            Err(error) => {
                eprintln!("[pull-reports] {name}: {error}");
                failures += 1;
            }
        }
    }
    if failures == 0 {
        Ok(())
    } else {
        Err(format!("{failures} worktree(s) failed"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::File;
    use std::time::Duration;

    struct Directory(PathBuf);
    impl Directory {
        fn new() -> Self {
            static NEXT: AtomicUsize = AtomicUsize::new(0);
            let path = std::env::temp_dir().join(format!(
                "vibecut-reports-test-{}-{}",
                process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            let _ = fs::remove_dir_all(&path);
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }
    }
    impl Drop for Directory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn set_modified(path: &Path, offset: i64) {
        let now = SystemTime::now();
        let time = if offset >= 0 {
            now + Duration::from_secs(offset.unsigned_abs())
        } else {
            now - Duration::from_secs(offset.unsigned_abs())
        };
        File::options()
            .write(true)
            .open(path)
            .unwrap()
            .set_modified(time)
            .unwrap();
    }

    #[test]
    fn arguments_require_a_transfer_target_for_paired_environments() {
        let args = |items: &[&str]| {
            items
                .iter()
                .map(|item| (*item).to_owned())
                .collect::<Vec<_>>()
        };
        assert_eq!(
            parse(&args(&[
                "--environment",
                "Window",
                "--ssh",
                "builder@host",
                "ticket-58"
            ]))
            .unwrap(),
            Options {
                environment: Some("Window".into()),
                ssh: Some("builder@host".into()),
                dry_run: false,
                targets: vec!["ticket-58".into()],
            }
        );
        assert!(
            parse(&args(&["--environment", "Window", "ticket-58"]))
                .unwrap_err()
                .contains("--ssh")
        );
        assert!(
            parse(&args(&["../escape"]))
                .unwrap_err()
                .contains("worktree")
        );
        assert!(parse(&args(&[])).unwrap_err().contains("usage:"));
        assert!(
            parse(&args(&["--ssh", "-oProxyCommand=x", "ticket-58"]))
                .unwrap_err()
                .contains("invalid ssh host")
        );
    }

    #[test]
    fn the_scp_command_keeps_the_remote_report_path_in_one_argument() {
        assert_eq!(
            scp_command(
                "builder@host",
                Path::new("C:/work/VibeCut/ticket-58/.scratch"),
                Path::new("/tmp/staging")
            )
            .render(),
            "scp -p -r builder@host:C:/work/VibeCut/ticket-58/.scratch /tmp/staging"
        );
    }

    #[test]
    fn staged_download_preserves_remote_times_so_a_newer_local_report_wins() {
        let root = Directory::new();
        let staging = root.0.join("staging");
        let destination = root.0.join("main/.scratch");
        fs::create_dir_all(staging.join(".scratch")).unwrap();
        fs::create_dir_all(&destination).unwrap();
        let remote = staging.join(".scratch/report.md");
        let local = destination.join("report.md");
        fs::write(&remote, b"stale remote").unwrap();
        fs::write(&local, b"newer local").unwrap();
        set_modified(&remote, -60);
        set_modified(&local, 0);

        let scp = scp_command("builder@host", Path::new("remote/.scratch"), &staging);
        assert!(
            scp.args.iter().any(|arg| arg == "-p"),
            "scp must preserve remote modification times"
        );
        let merge = merge_download(&staging, &destination, false).unwrap();
        assert_eq!(merge.copied, 0);
        assert_eq!(merge.kept_newer, 1);
        assert_eq!(fs::read_to_string(local).unwrap(), "newer local");
    }

    #[test]
    fn merge_copies_missing_and_newer_files_but_keeps_newer_local_ones() {
        let root = Directory::new();
        let source = root.0.join("remote");
        let destination = root.0.join("local");
        fs::create_dir_all(source.join("reviews")).unwrap();
        fs::create_dir_all(destination.join("reviews")).unwrap();
        fs::write(source.join("reviews/new.md"), b"new").unwrap();
        fs::write(source.join("reviews/local-newer.md"), b"remote").unwrap();
        fs::write(destination.join("reviews/local-newer.md"), b"local").unwrap();
        fs::write(source.join("reviews/older-local.md"), b"fresh remote").unwrap();
        fs::write(destination.join("reviews/older-local.md"), b"stale local").unwrap();
        set_modified(&source.join("reviews/local-newer.md"), -60);
        set_modified(&destination.join("reviews/local-newer.md"), 0);
        set_modified(&source.join("reviews/older-local.md"), 0);
        set_modified(&destination.join("reviews/older-local.md"), -60);

        let merge = merge_tree(&source, &destination, false).unwrap();
        assert_eq!(merge.copied, 2);
        assert_eq!(merge.kept_newer, 1);
        assert_eq!(
            fs::read_to_string(destination.join("reviews/new.md")).unwrap(),
            "new"
        );
        assert_eq!(
            fs::read_to_string(destination.join("reviews/local-newer.md")).unwrap(),
            "local"
        );
        assert_eq!(
            fs::read_to_string(destination.join("reviews/older-local.md")).unwrap(),
            "fresh remote"
        );
    }

    #[test]
    fn dry_run_reports_copies_without_creating_them() {
        let root = Directory::new();
        let source = root.0.join("remote");
        let destination = root.0.join("local");
        fs::create_dir_all(&source).unwrap();
        fs::write(source.join("report.md"), b"report").unwrap();
        let merge = merge_tree(&source, &destination, true).unwrap();
        assert_eq!(merge.copied, 1);
        assert!(!destination.exists());
    }
}
