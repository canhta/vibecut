//! Publishes chapter, title and description sidecars without editing the project.
//!
//! Both files are serialized before admission, then each is written through the
//! shared publication owner: a private same-directory temporary file is synced and
//! renamed only after project/source alias admission. Each file is atomic; a failure
//! before its fence preserves the previous destination and removes staging.
pub use crate::ChapterSidecars;
use crate::{Error as EngineError, Result};
use ai::chapters::ChapterDocument;
use project::Project;
use std::{
    io::Write,
    path::{Path, PathBuf},
};

/// `<stem>.txt` and `<stem>.json`, preserving a native file name exactly.
pub fn sidecar_paths(stem: &Path) -> Result<ChapterSidecars> {
    if stem.file_name().is_none_or(|name| name.is_empty()) {
        return Err(EngineError::Edit(crate::EditError::Invalid(
            "chapter sidecar stem",
        )));
    }
    let append = |suffix: &str| {
        let mut name = stem.as_os_str().to_owned();
        name.push(suffix);
        PathBuf::from(name)
    };
    Ok(ChapterSidecars {
        text: append(".txt"),
        json: append(".json"),
    })
}

/// Publish both sidecars for `stem`; `overwrite` authorizes replacing an existing file.
pub fn write(
    project: &Project,
    protected: &[PathBuf],
    hooks: &dyn render::ExecutionHooks,
    stem: &Path,
    document: &ChapterDocument,
    overwrite: project::atomic::Overwrite,
) -> Result<ChapterSidecars> {
    write_with_fs(
        project,
        protected,
        hooks,
        stem,
        document,
        overwrite,
        &project::atomic::Fs,
    )
}

/// Publish both sidecars into an existing directory under the project name.
///
/// The pair always refuses an existing file; no caller can authorize replacement.
pub fn write_to_directory(
    project: &Project,
    protected: &[PathBuf],
    directory: &Path,
    document: &ChapterDocument,
) -> Result<ChapterSidecars> {
    if !directory.is_dir() {
        return Err(EngineError::Edit(crate::EditError::Invalid(
            "chapter sidecar directory",
        )));
    }
    let name = crate::default_file_name(project.data().name.as_str(), "json");
    let stem = directory.join(name.trim_end_matches(".json"));
    let paths = sidecar_paths(&stem)?;
    if paths.text.exists() || paths.json.exists() {
        return Err(EngineError::Edit(crate::EditError::Invalid(
            "chapter sidecar destination exists",
        )));
    }
    write(
        project,
        protected,
        &PublishHooks,
        &stem,
        document,
        project::atomic::Overwrite::Refuse,
    )
}

struct PublishHooks;
struct NoLease;
impl render::Reservation for NoLease {}
impl render::ExecutionHooks for PublishHooks {
    fn cancelled(&self) -> bool {
        false
    }
    fn reserve(
        &self,
        _: render::MemoryKind,
        _: u64,
    ) -> render::Result<Box<dyn render::Reservation>> {
        Ok(Box::new(NoLease))
    }
}

#[allow(clippy::too_many_arguments)]
fn write_with_fs(
    project: &Project,
    protected: &[PathBuf],
    hooks: &dyn render::ExecutionHooks,
    stem: &Path,
    document: &ChapterDocument,
    overwrite: project::atomic::Overwrite,
    fs: &dyn project::atomic::AtomicFs,
) -> Result<ChapterSidecars> {
    let paths = sidecar_paths(stem)?;
    let text = document.to_text().map_err(EngineError::Ai)?;
    let json = document.to_json().map_err(EngineError::Ai)?;
    let text_publication = crate::publication::Publication {
        project,
        path: &paths.text,
        protected,
        hooks,
    };
    let json_publication = crate::publication::Publication {
        project,
        path: &paths.json,
        protected,
        hooks,
    };
    // Admit both destinations before either file exists, so a source/project alias
    // on either side publishes nothing.
    text_publication.admit()?;
    json_publication.admit()?;
    text_publication.write(
        overwrite,
        u64::try_from(text.len())
            .map_err(|_| EngineError::Control(crate::session::wire::ControlError::Limit))?,
        fs,
        |writer| {
            writer.write_all(text.as_bytes())?;
            Ok(())
        },
    )?;
    json_publication.write(
        overwrite,
        u64::try_from(json.len())
            .map_err(|_| EngineError::Control(crate::session::wire::ControlError::Limit))?,
        fs,
        |writer| {
            writer.write_all(&json)?;
            Ok(())
        },
    )?;
    Ok(paths)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::panic)]
mod tests {
    use super::*;
    use ai::chapters::{Chapter, ChapterDocument};
    use captions::{LanguageTag, NfcText};
    use time::{TICKS_PER_SECOND, Tick};

    struct FullDisk;
    impl project::atomic::AtomicFs for FullDisk {
        fn sync(&self, _: &std::fs::File) -> std::io::Result<()> {
            Err(std::io::Error::other("injected full disk"))
        }
    }

    struct FailFence;
    impl project::atomic::AtomicFs for FailFence {
        fn sync(&self, file: &std::fs::File) -> std::io::Result<()> {
            file.sync_all()
        }

        fn before_publish(&self) -> std::io::Result<()> {
            Err(std::io::Error::other("injected publication fence"))
        }
    }

    struct FailsOnSecond(std::sync::atomic::AtomicUsize);
    impl project::atomic::AtomicFs for FailsOnSecond {
        fn sync(&self, file: &std::fs::File) -> std::io::Result<()> {
            file.sync_all()
        }

        fn before_publish(&self) -> std::io::Result<()> {
            if self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst) == 0 {
                Ok(())
            } else {
                Err(std::io::Error::other("injected second publication fence"))
            }
        }
    }

    struct Hooks;
    struct Lease;
    impl render::Reservation for Lease {}
    impl render::ExecutionHooks for Hooks {
        fn cancelled(&self) -> bool {
            false
        }
        fn reserve(
            &self,
            _: render::MemoryKind,
            _: u64,
        ) -> render::Result<Box<dyn render::Reservation>> {
            Ok(Box::new(Lease))
        }
    }

    fn document() -> ChapterDocument {
        ChapterDocument::new(
            LanguageTag::parse("vi").unwrap(),
            Tick(TICKS_PER_SECOND),
            "Hành trình".into(),
            "Một cuộc trò chuyện.".into(),
            vec![Chapter {
                start: Tick::ZERO,
                segment_id: NfcText::from("s0"),
                title: "Mở đầu".into(),
            }],
            Vec::new(),
        )
        .unwrap()
    }

    fn project() -> Project {
        Project::empty(project::ProjectId(1), "Chương".into()).unwrap()
    }

    #[test]
    fn failed_publication_preserves_both_destinations_and_removes_staging() {
        for fs in [&FullDisk as &dyn project::atomic::AtomicFs, &FailFence] {
            let dir = tempfile::tempdir().unwrap();
            let stem = dir.path().join("Mục lục");
            let paths = sidecar_paths(&stem).unwrap();
            std::fs::write(&paths.text, b"previous text").unwrap();
            std::fs::write(&paths.json, b"previous json").unwrap();
            let error = write_with_fs(
                &project(),
                &[],
                &Hooks,
                &stem,
                &document(),
                project::atomic::Overwrite::Replace,
                fs,
            )
            .unwrap_err();
            assert!(matches!(
                error,
                EngineError::Edit(crate::EditError::Project(project::Error::Io(_)))
            ));
            assert_eq!(std::fs::read(&paths.text).unwrap(), b"previous text");
            assert_eq!(std::fs::read(&paths.json).unwrap(), b"previous json");
            assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 2);
        }
    }

    #[test]
    fn second_file_failure_leaves_the_first_complete_and_no_staging() {
        let dir = tempfile::tempdir().unwrap();
        let stem = dir.path().join("Mục lục");
        let paths = sidecar_paths(&stem).unwrap();
        std::fs::write(&paths.json, b"previous json").unwrap();
        let error = write_with_fs(
            &project(),
            &[],
            &Hooks,
            &stem,
            &document(),
            project::atomic::Overwrite::Replace,
            &FailsOnSecond(Default::default()),
        )
        .unwrap_err();
        assert!(matches!(
            error,
            EngineError::Edit(crate::EditError::Project(project::Error::Io(_)))
        ));
        assert!(
            std::fs::read(&paths.text)
                .unwrap()
                .starts_with(b"H\xc3\xa0nh")
        );
        assert_eq!(std::fs::read(&paths.json).unwrap(), b"previous json");
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 2);
    }
}
