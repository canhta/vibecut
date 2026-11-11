//! Current-frame PNG snapshots over the production compositor.
//!
//! Rendering is headless and independent of app transport/zoom state. Publication
//! reuses project's same-directory staging owner: a private temp file is written,
//! synced and renamed over the destination only after source/project alias
//! admission succeeds. A refusal or failed write leaves no partial destination.
use super::{Error as ArtifactError, Result as ArtifactResult};
use crate::{ByteBudget, ByteBudgetHooks};
use image::ImageEncoder;
use project::Project;
use std::{
    io::Write,
    path::{Path, PathBuf},
};
use time::Tick;

/// Encoded PNG ceiling; protects the process from unbounded encoder output.
pub const MAX_SNAPSHOT_PNG_BYTES: usize = 32 << 20;
/// Local accounting ceiling for callers that do not supply the shared budget.
pub const MAX_SNAPSHOT_MEMORY: u64 = 1 << 30;

/// A rendered snapshot before publication. `png` is returned so the desktop app
/// can put the same frame image on the clipboard without a second render.
#[derive(Clone, Debug)]
pub struct SnapshotImage {
    pub time: Tick,
    pub timecode: String,
    pub width: u32,
    pub height: u32,
    pub png: Vec<u8>,
}

pub use crate::session::wire::SnapshotResult;

/// Render the current frame to a PNG and publish it atomically.
///
/// `protected` lists active project/metadata/recovery paths supplied by the
/// composition root, exactly like completed-export publication. Project source
/// aliases are always admitted from the captured project.
pub fn save(
    project: &Project,
    time: Tick,
    path: &Path,
    protected: &[PathBuf],
) -> ArtifactResult<SnapshotResult> {
    let budget = local_budget(project)?;
    save_with_budget(project, time, path, protected, &budget)
}

/// Same publication with the caller's shared byte budget.
pub fn save_with_budget(
    project: &Project,
    time: Tick,
    path: &Path,
    protected: &[PathBuf],
    budget: &ByteBudget,
) -> ArtifactResult<SnapshotResult> {
    save_with_overwrite(
        project,
        time,
        path,
        protected,
        budget,
        project::atomic::Overwrite::Replace,
    )
}

pub(crate) fn save_with_overwrite(
    project: &Project,
    time: Tick,
    path: &Path,
    protected: &[PathBuf],
    budget: &ByteBudget,
    overwrite: project::atomic::Overwrite,
) -> ArtifactResult<SnapshotResult> {
    let hooks = ByteBudgetHooks::new(budget, None);
    let image = render(project, time, &hooks)?;
    crate::publication::Publication {
        project,
        path,
        protected,
        hooks: &hooks,
    }
    .write(
        overwrite,
        u64::try_from(image.png.len())
            .map_err(|_| ArtifactError::from(crate::session::wire::ControlError::Limit))?,
        &project::atomic::Fs,
        |writer| -> crate::runtime::Result<()> {
            writer.write_all(&image.png)?;
            Ok(())
        },
    )?;
    Ok(SnapshotResult {
        path: path.to_owned(),
        time: image.time,
        timecode: image.timecode,
        width: image.width,
        height: image.height,
    })
}

/// Render the current frame as PNG without publishing a file (Copy action).
pub fn image(project: &Project, time: Tick) -> ArtifactResult<SnapshotImage> {
    let budget = local_budget(project)?;
    render(project, time, &ByteBudgetHooks::new(&budget, None))
}

/// Default save name: `<project>-<timecode>.png`, reserved characters replaced
/// and `snapshot` used when the project name is empty after sanitizing.
pub fn default_name(project: &Project, time: Tick) -> ArtifactResult<String> {
    let at = render_time(project, time)?;
    let timecode = timecode(project, at)?.replace(':', "-");
    let suffix = format!("-{timecode}.png");
    let base = crate::runtime::snapshot_project_name(
        project.data().name.as_str(),
        255_usize.saturating_sub(suffix.len()),
    );
    Ok(format!("{base}{suffix}"))
}

/// The frame actually captured: empty projects refuse, negative times refuse,
/// and a playhead at/past the end clamps to the last frame.
pub fn render_time(project: &Project, time: Tick) -> ArtifactResult<Tick> {
    let duration = project.duration();
    if duration == Tick::ZERO {
        return Err(ArtifactError::EmptyProject);
    }
    if time < Tick::ZERO {
        return Err(ArtifactError::Edit(crate::session::EditError::Invalid(
            "snapshot time",
        )));
    }
    if time < duration {
        return Ok(time);
    }
    Ok(project
        .data()
        .canvas
        .fps
        .snap(duration.saturating_sub(Tick(1)))?)
}

fn local_budget(project: &Project) -> ArtifactResult<ByteBudget> {
    let canvas = &project.data().canvas;
    let bytes = render::Image::bytes(canvas.width, canvas.height)?;
    let limit = bytes
        .saturating_mul(4)
        .saturating_add(64 << 20)
        .min(MAX_SNAPSHOT_MEMORY);
    Ok(ByteBudget::new(limit)?)
}

fn render(
    project: &Project,
    time: Tick,
    hooks: &dyn render::ExecutionHooks,
) -> ArtifactResult<SnapshotImage> {
    let at = render_time(project, time)?;
    let canvas = &project.data().canvas;
    let settings = render::RenderSettings::from_project(project);
    let mut renderer = render::Renderer::new(project, settings)?;
    let frame = renderer.render_frame(at, &mut render::FileSources::default(), hooks)?;
    // PNG is an sRGB image boundary; preserve straight alpha for a transparent canvas.
    let rgba = frame.rgba8(13, hooks)?;
    let png = encode_png(&rgba.bytes, canvas.width, canvas.height)?;
    Ok(SnapshotImage {
        time: at,
        timecode: timecode(project, at)?,
        width: canvas.width,
        height: canvas.height,
        png,
    })
}

fn timecode(project: &Project, at: Tick) -> crate::session::Result<String> {
    Ok(time::format_timecode(at, project.data().canvas.fps)?)
}

fn encode_png(rgba: &[u8], width: u32, height: u32) -> crate::session::Result<Vec<u8>> {
    let mut writer = PngWriter {
        bytes: Vec::new(),
        max: MAX_SNAPSHOT_PNG_BYTES,
        exceeded: false,
    };
    let encoded = image::codecs::png::PngEncoder::new(&mut writer).write_image(
        rgba,
        width,
        height,
        image::ExtendedColorType::Rgba8,
    );
    match encoded {
        Ok(()) => Ok(writer.bytes),
        Err(_) if writer.exceeded => Err(crate::session::EditError::from(
            crate::session::wire::ControlError::Limit,
        )),
        Err(_) => Err(crate::session::EditError::Invalid("snapshot encoding")),
    }
}

struct PngWriter {
    bytes: Vec<u8>,
    max: usize,
    exceeded: bool,
}

impl Write for PngWriter {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if self.bytes.len().saturating_add(bytes.len()) > self.max {
            self.exceeded = true;
            return Err(std::io::Error::other("snapshot PNG limit"));
        }
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::panic)]
mod tests {
    use super::*;

    fn publish(
        path: &Path,
        png: &[u8],
        project: &Project,
        protected: &[PathBuf],
        hooks: &dyn render::ExecutionHooks,
        fs: &dyn project::atomic::AtomicFs,
    ) -> ArtifactResult<()> {
        crate::publication::Publication {
            project,
            path,
            protected,
            hooks,
        }
        .write(
            project::atomic::Overwrite::Replace,
            png.len() as u64,
            fs,
            |writer| {
                writer.write_all(png)?;
                Ok(())
            },
        )
    }

    struct FullDisk;

    impl project::atomic::AtomicFs for FullDisk {
        fn sync(&self, _: &std::fs::File) -> std::io::Result<()> {
            Err(std::io::Error::other("injected full disk"))
        }
    }

    struct FailPublish;

    impl project::atomic::AtomicFs for FailPublish {
        fn sync(&self, file: &std::fs::File) -> std::io::Result<()> {
            file.sync_all()
        }

        fn before_publish(&self) -> std::io::Result<()> {
            Err(std::io::Error::other("injected publication fence"))
        }
    }

    fn project() -> Project {
        Project::empty(project::ProjectId(1), "Snapshot".into()).unwrap()
    }

    fn hooks() -> ByteBudgetHooks<'static> {
        static BUDGET: std::sync::OnceLock<ByteBudget> = std::sync::OnceLock::new();
        ByteBudgetHooks::new(
            BUDGET.get_or_init(|| ByteBudget::new(1 << 20).unwrap()),
            None,
        )
    }

    #[test]
    fn failed_publication_preserves_destination_and_removes_staging() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("snapshot.png");
        std::fs::write(&path, b"previous").unwrap();
        let error = publish(&path, b"new png", &project(), &[], &hooks(), &FullDisk).unwrap_err();
        assert!(matches!(
            error,
            ArtifactError::Runtime(crate::RuntimeError::Edit(
                crate::session::EditError::Project(project::Error::Io(_))
            ))
        ));
        assert_eq!(std::fs::read(&path).unwrap(), b"previous");
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
        let cause: std::sync::Arc<dyn crate::Failure> =
            std::sync::Arc::new(crate::control::Error::Ai(ai::Error::RateLimit {
                retry_after_seconds: Some(3),
            }));
        for unwinds in [false, true] {
            let project = project();
            let hooks = hooks();
            let returned = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                crate::publication::Publication {
                    project: &project,
                    path: &path,
                    protected: &[],
                    hooks: &hooks,
                }
                .write(
                    project::atomic::Overwrite::Replace,
                    64,
                    &project::atomic::Fs,
                    |writer| -> crate::control::Result<()> {
                        writer.write_all(b"partial callback bytes")?;
                        if unwinds {
                            panic!("injected mixed writer unwind");
                        }
                        Err(crate::recovery::Error::PersistenceFailed(cause.clone()).into())
                    },
                )
            }));
            if unwinds {
                assert!(returned.is_err());
            } else {
                let error = returned.unwrap().unwrap_err();
                use crate::Failure;
                assert_eq!(error.code(), crate::control::ErrorCode::Ai);
                assert!(!error.transient());
                assert!(!error.job_policy().terminal_cancelled);
                assert_eq!(error.job_policy().retry_after_seconds, None);
                let crate::control::Error::Recovery(crate::recovery::Error::PersistenceFailed(
                    original,
                )) = error
                else {
                    panic!("writer error was normalized")
                };
                assert!(std::sync::Arc::ptr_eq(&original, &cause));
            }
            assert_eq!(std::fs::read(&path).unwrap(), b"previous");
            assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
        }
    }

    #[test]
    fn failed_publication_fence_preserves_destination_and_removes_staging() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("snapshot.png");
        std::fs::write(&path, b"previous").unwrap();
        let error =
            publish(&path, b"new png", &project(), &[], &hooks(), &FailPublish).unwrap_err();
        assert!(matches!(
            error,
            ArtifactError::Runtime(crate::RuntimeError::Edit(
                crate::session::EditError::Project(project::Error::Io(_))
            ))
        ));
        assert_eq!(std::fs::read(&path).unwrap(), b"previous");
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    }

    #[test]
    fn publication_replaces_atomically_without_staging_leftovers() {
        struct CreateAtFence(PathBuf);
        impl project::atomic::AtomicFs for CreateAtFence {
            fn sync(&self, file: &std::fs::File) -> std::io::Result<()> {
                file.sync_all()
            }
            fn before_publish(&self) -> std::io::Result<()> {
                std::fs::write(&self.0, b"raced destination")
            }
        }
        for (overwrite, exists, race, succeeds) in [
            (project::atomic::Overwrite::Replace, true, false, true),
            (project::atomic::Overwrite::Refuse, false, false, true),
            (project::atomic::Overwrite::Refuse, true, false, false),
            (project::atomic::Overwrite::Refuse, false, true, false),
        ] {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("snapshot.png");
            if exists {
                std::fs::write(&path, b"previous").unwrap();
            }
            let fence = CreateAtFence(path.clone());
            let fs: &dyn project::atomic::AtomicFs =
                if race { &fence } else { &project::atomic::Fs };
            let project = project();
            let hooks = hooks();
            let result = crate::publication::Publication {
                project: &project,
                path: &path,
                protected: &[],
                hooks: &hooks,
            }
            .write(overwrite, 7, fs, |writer| -> crate::runtime::Result<()> {
                writer.write_all(b"new png")?;
                Ok(())
            });
            assert_eq!(
                result.is_ok(),
                succeeds,
                "{overwrite:?} existing={exists} race={race}"
            );
            let expected: &[u8] = if succeeds {
                b"new png"
            } else if race {
                b"raced destination"
            } else {
                b"previous"
            };
            assert_eq!(std::fs::read(&path).unwrap(), expected);
            assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
        }
    }

    #[test]
    fn safe_project_name_replaces_reserved_characters_and_falls_back() {
        assert_eq!(
            crate::runtime::snapshot_project_name("My: Project?", 255),
            "My- Project-"
        );
        assert_eq!(
            crate::runtime::snapshot_project_name("D\u{1ef1} \u{00e1}n", 255),
            "D\u{1ef1} \u{00e1}n"
        );
        assert_eq!(crate::runtime::snapshot_project_name("  ", 255), "snapshot");
        assert_eq!(
            crate::runtime::snapshot_project_name("...", 255),
            "snapshot"
        );
    }
}
