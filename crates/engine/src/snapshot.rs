//! Current-frame PNG snapshots over the production compositor.
//!
//! Rendering is headless and independent of app transport/zoom state. Publication
//! reuses project's same-directory staging owner: a private temp file is written,
//! synced and renamed over the destination only after source/project alias
//! admission succeeds. A refusal or failed write leaves no partial destination.
use crate::{ByteBudget, ByteOwner, EditError, Result};
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

/// Published snapshot receipt; the PNG itself stays in `SnapshotImage`.
#[derive(Clone, Debug, serde::Serialize, schemars::JsonSchema)]
pub struct SnapshotResult {
    pub path: PathBuf,
    pub time: Tick,
    pub timecode: String,
    pub width: u32,
    pub height: u32,
}

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
) -> Result<SnapshotResult> {
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
) -> Result<SnapshotResult> {
    let hooks = SnapshotHooks(budget);
    let image = render(project, time, &hooks)?;
    publish(
        path,
        &image.png,
        project,
        protected,
        &hooks,
        &project::atomic::Fs,
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
pub fn image(project: &Project, time: Tick) -> Result<SnapshotImage> {
    let budget = local_budget(project)?;
    render(project, time, &SnapshotHooks(&budget))
}

/// Default save name: `<project>-<timecode>.png`, reserved characters replaced
/// and `snapshot` used when the project name is empty after sanitizing.
pub fn default_name(project: &Project, time: Tick) -> Result<String> {
    let at = render_time(project, time)?;
    let timecode = timecode(project, at)?.replace(':', "-");
    Ok(format!(
        "{}-{timecode}.png",
        safe_project_name(project.data().name.as_str())
    ))
}

/// The frame actually captured: empty projects refuse, negative times refuse,
/// and a playhead at/past the end clamps to the last frame.
pub fn render_time(project: &Project, time: Tick) -> Result<Tick> {
    let duration = project.duration();
    if duration == Tick::ZERO {
        return Err(EditError::EmptyProject);
    }
    if time < Tick::ZERO {
        return Err(EditError::Invalid("snapshot time"));
    }
    let frame = project.data().canvas.fps.frame_duration()?;
    Ok(time.min(duration.saturating_sub(frame)))
}

fn local_budget(project: &Project) -> Result<ByteBudget> {
    let canvas = &project.data().canvas;
    let bytes = render::Image::bytes(canvas.width, canvas.height)?;
    let limit = bytes
        .saturating_mul(4)
        .saturating_add(64 << 20)
        .min(MAX_SNAPSHOT_MEMORY);
    ByteBudget::new(limit)
}

fn render(
    project: &Project,
    time: Tick,
    hooks: &dyn render::ExecutionHooks,
) -> Result<SnapshotImage> {
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

fn timecode(project: &Project, at: Tick) -> Result<String> {
    let rate = project.data().canvas.fps;
    let drop_frame = matches!(
        rate,
        time::FrameRate::FPS_29_97 | time::FrameRate::FPS_59_94
    );
    Ok(time::format_timecode(at, rate, drop_frame)?)
}

fn safe_project_name(name: &str) -> String {
    let mut sanitized = String::with_capacity(name.len());
    for character in name.chars() {
        if character.is_control()
            || matches!(
                character,
                '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*'
            )
        {
            sanitized.push('-');
        } else {
            sanitized.push(character);
        }
    }
    let trimmed = sanitized.trim().trim_end_matches(['.', ' ']).trim();
    if trimmed.is_empty() {
        "snapshot".into()
    } else {
        trimmed.into()
    }
}

fn encode_png(rgba: &[u8], width: u32, height: u32) -> Result<Vec<u8>> {
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
        Err(_) if writer.exceeded => Err(EditError::Limit),
        Err(_) => Err(EditError::Invalid("snapshot encoding")),
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

fn publish(
    path: &Path,
    png: &[u8],
    project: &Project,
    protected: &[PathBuf],
    hooks: &dyn render::ExecutionHooks,
    fs: &dyn project::atomic::AtomicFs,
) -> Result<()> {
    admit_destination(project, path, protected, hooks)?;
    let max = u64::try_from(png.len()).map_err(|_| EditError::Limit)?;
    let stage =
        project::atomic::StreamingStage::new(path, project::atomic::Overwrite::Replace, max)?;
    {
        let mut writer = stage.writer()?;
        writer.write_all(png)?;
    }
    let finalized = stage.finalize_with(fs)?;
    // Re-admit immediately before the rename so a destination swapped while
    // staging is still refused; the stage is dropped (and removed) on refusal.
    admit_destination(project, path, protected, hooks)?;
    finalized.publish_with(fs)?;
    Ok(())
}

fn admit_destination(
    project: &Project,
    path: &Path,
    protected: &[PathBuf],
    hooks: &dyn render::ExecutionHooks,
) -> Result<()> {
    // Source media aliases (direct, normalized, symlink and hard link).
    render::validate_output_path(project, path, hooks)?;
    // Writer-control names and Windows-normalized spellings.
    project::atomic::admit_destination(path)?;
    let normalized = normalize(path)?;
    for protected in protected {
        let normalized_protected = normalize(protected).unwrap_or_else(|_| protected.clone());
        if normalized == normalized_protected
            || (protected.is_dir() && normalized.starts_with(&normalized_protected))
        {
            return Err(EditError::Invalid("snapshot aliases a project file"));
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            if let (Ok(destination), Ok(protected)) =
                (std::fs::metadata(path), std::fs::metadata(protected))
                && destination.dev() == protected.dev()
                && destination.ino() == protected.ino()
            {
                return Err(EditError::Invalid("snapshot aliases a project file"));
            }
        }
        #[cfg(not(unix))]
        if path.is_file()
            && protected.is_file()
            && media::ContentHash::read_with_cancel(path, || hooks.cancelled())?
                == media::ContentHash::read_with_cancel(protected, || hooks.cancelled())?
        {
            return Err(EditError::Invalid("snapshot aliases a project file"));
        }
    }
    Ok(())
}

fn normalize(path: &Path) -> Result<PathBuf> {
    if path.exists() {
        Ok(std::fs::canonicalize(path)?)
    } else {
        let parent = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        Ok(std::fs::canonicalize(parent)?.join(
            path.file_name()
                .ok_or(EditError::Invalid("snapshot filename"))?,
        ))
    }
}

struct SnapshotHooks<'a>(&'a ByteBudget);

impl render::ExecutionHooks for SnapshotHooks<'_> {
    fn cancelled(&self) -> bool {
        false
    }

    fn reserve(
        &self,
        kind: render::MemoryKind,
        bytes: u64,
    ) -> render::Result<Box<dyn render::Reservation>> {
        let owner = match kind {
            render::MemoryKind::Decoded => ByteOwner::DecodedFrames,
            render::MemoryKind::Compositor => ByteOwner::Compositor,
            render::MemoryKind::Export => ByteOwner::Export,
        };
        self.0
            .reserve(owner, bytes)
            .map(|lease| Box::new(lease) as Box<dyn render::Reservation>)
            .map_err(|error| match error {
                EditError::Io(source) => render::Error::Io(source),
                _ => render::Error::Budget {
                    requested: bytes,
                    limit: self
                        .0
                        .snapshot()
                        .map_or(0, |snapshot| snapshot.limit.saturating_sub(snapshot.used)),
                },
            })
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::panic)]
mod tests {
    use super::*;

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

    fn hooks() -> SnapshotHooks<'static> {
        static BUDGET: std::sync::OnceLock<ByteBudget> = std::sync::OnceLock::new();
        SnapshotHooks(BUDGET.get_or_init(|| ByteBudget::new(1 << 20).unwrap()))
    }

    #[test]
    fn failed_publication_preserves_destination_and_removes_staging() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("snapshot.png");
        std::fs::write(&path, b"previous").unwrap();
        let error = publish(&path, b"new png", &project(), &[], &hooks(), &FullDisk).unwrap_err();
        assert!(matches!(error, EditError::Project(project::Error::Io(_))));
        assert_eq!(std::fs::read(&path).unwrap(), b"previous");
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    }

    #[test]
    fn failed_publication_fence_preserves_destination_and_removes_staging() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("snapshot.png");
        std::fs::write(&path, b"previous").unwrap();
        let error =
            publish(&path, b"new png", &project(), &[], &hooks(), &FailPublish).unwrap_err();
        assert!(matches!(error, EditError::Project(project::Error::Io(_))));
        assert_eq!(std::fs::read(&path).unwrap(), b"previous");
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    }

    #[test]
    fn publication_replaces_atomically_without_staging_leftovers() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("snapshot.png");
        std::fs::write(&path, b"previous").unwrap();
        publish(
            &path,
            b"new png",
            &project(),
            &[],
            &hooks(),
            &project::atomic::Fs,
        )
        .unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"new png");
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    }

    #[test]
    fn safe_project_name_replaces_reserved_characters_and_falls_back() {
        assert_eq!(safe_project_name("My: Project?"), "My- Project-");
        assert_eq!(
            safe_project_name("D\u{1ef1} \u{00e1}n"),
            "D\u{1ef1} \u{00e1}n"
        );
        assert_eq!(safe_project_name("  "), "snapshot");
        assert_eq!(safe_project_name("..."), "snapshot");
    }
}
