use super::super::Failure;
use std::fs;
use std::path::Path;
use std::path::PathBuf;

pub(super) fn publish(src: &Path, dst: &Path) -> Result<(), Failure> {
    if let Some(parent) = dst.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::copy(src, dst)?;
    fs::remove_file(src).ok();
    Ok(())
}

pub(super) fn owned_by_runner(outputs: &Path, name: &str, slot: &str) -> Result<bool, Failure> {
    let media = outputs.join(name);
    let sidecar = outputs.join(format!("{name}.attribution"));
    if !media.exists() && !sidecar.exists() {
        return Ok(false);
    }
    let text = fs::read_to_string(&sidecar).map_err(|_| {
        format!("refusing unrelated existing README output `{name}`: no runner sidecar")
    })?;
    let marker = format!("ui_audit.rs --readme {slot}");
    Ok(text
        .lines()
        .any(|line| line.trim() == format!("asset: {name}"))
        && text.lines().any(|line| line.trim() == "added: p0-core/22")
        && text
            .lines()
            .any(|line| line.starts_with("source:") && line.contains(&marker)))
}

#[derive(Debug)]
pub(crate) struct Publication {
    outputs: PathBuf,
    backup: PathBuf,
    published: Vec<String>,
    backed: Vec<String>,
    run: PathBuf,
}

impl Publication {
    fn commit(self) {
        fs::remove_dir_all(&self.backup).ok();
    }

    fn rollback(self, error: Failure) -> Failure {
        let mut rollback = Vec::new();
        for name in self.published.iter().rev() {
            let path = self.outputs.join(name);
            if path.exists() && fs::remove_file(&path).is_err() {
                rollback.push(format!("remove {name}"));
            }
        }
        for name in self.backed.iter().rev() {
            if fs::rename(self.backup.join(name), self.outputs.join(name)).is_err() {
                rollback.push(format!("restore {name}"));
            }
        }
        if rollback.is_empty() {
            fs::remove_dir_all(&self.backup).ok();
            error
        } else {
            fs::write(
                self.run.join("preserve-staging"),
                format!("rollback incomplete: {}", rollback.join(", ")),
            )
            .ok();
            format!(
                "{error}; rollback incomplete ({}) with backups at {}",
                rollback.join(", "),
                self.backup.display()
            )
            .into()
        }
    }
}

pub(crate) fn publish_outputs(
    stage: &Path,
    outputs: &Path,
    slot: &str,
    media_names: &[&str],
    all_names: &[&str],
    run: &Path,
    run_id: &str,
) -> Result<Publication, Failure> {
    for name in media_names {
        let exists =
            outputs.join(name).exists() || outputs.join(format!("{name}.attribution")).exists();
        if exists && !owned_by_runner(outputs, name, slot)? {
            return Err(format!(
                "refusing unrelated existing README output `{name}`: sidecar does not prove runner ownership"
            )
            .into());
        }
    }
    let backup = outputs.join(format!(".readme-media-backup-{run_id}"));
    if backup.exists() {
        return Err("README publication backup already exists".into());
    }
    fs::create_dir_all(&backup)?;
    let mut publication = Publication {
        outputs: outputs.to_owned(),
        backup: backup.clone(),
        published: Vec::new(),
        backed: Vec::new(),
        run: run.to_owned(),
    };
    let mut apply = || -> Result<(), Failure> {
        for name in all_names {
            let src = stage.join(name);
            if !src.is_file() {
                return Err(format!("staged README output `{name}` is missing").into());
            }
            let dst = outputs.join(name);
            if dst.exists() {
                fs::rename(&dst, backup.join(name))?;
                publication.backed.push((*name).to_owned());
            }
            let temp = outputs.join(format!(".{name}.tmp-{run_id}"));
            if temp.exists() {
                fs::remove_file(&temp)?;
            }
            fs::copy(&src, &temp)?;
            fs::rename(&temp, &dst)?;
            publication.published.push((*name).to_owned());
        }
        Ok(())
    };
    match apply() {
        Ok(()) => Ok(publication),
        Err(error) => Err(publication.rollback(error)),
    }
}

pub(crate) fn finish_publication(
    publication: Publication,
    manifest_path: &Path,
    manifest: &[u8],
) -> Result<(), Failure> {
    if let Err(error) = fs::write(manifest_path, manifest) {
        return Err(
            publication.rollback(format!("publication manifest write failed: {error}").into())
        );
    }
    publication.commit();
    Ok(())
}
