#![allow(clippy::unwrap_used, clippy::panic)]
use project::atomic::{AtomicFs, Overwrite, StreamingStage};
use std::{
    fs::File,
    io::{Seek, SeekFrom, Write},
    path::PathBuf,
};
struct Fault {
    sync: bool,
    publish: bool,
    race: Option<PathBuf>,
}
impl AtomicFs for Fault {
    fn sync(&self, file: &File) -> std::io::Result<()> {
        if self.sync {
            Err(std::io::Error::new(
                std::io::ErrorKind::WriteZero,
                "injected disk full",
            ))
        } else {
            file.sync_all()
        }
    }
    fn before_publish(&self) -> std::io::Result<()> {
        if let Some(path) = &self.race {
            std::fs::write(path, b"racing writer")?;
        }
        if self.publish {
            Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "injected rename failure",
            ))
        } else {
            Ok(())
        }
    }
}
#[test]
fn streaming_bounds_finalize_failures_and_drop_preserve_previous_output() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("movie.mp4");
    std::fs::write(&path, b"previous").unwrap();
    assert!(StreamingStage::new(&path, Overwrite::Refuse, 100).is_err());
    for fault in [
        Fault {
            sync: true,
            publish: false,
            race: None,
        },
        Fault {
            sync: false,
            publish: true,
            race: None,
        },
    ] {
        let stage = StreamingStage::new(&path, Overwrite::Replace, 10).unwrap();
        let mut writer = stage.writer().unwrap();
        writer.write_all(b"complete").unwrap();
        drop(writer);
        match stage.finalize_with(&fault) {
            Err(_) => assert!(fault.sync),
            Ok(finalized) => assert!(finalized.publish_with(&fault).is_err()),
        }
        assert_eq!(std::fs::read(&path).unwrap(), b"previous");
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    }
    let stage = StreamingStage::new(&path, Overwrite::Replace, 4).unwrap();
    let mut writer = stage.writer().unwrap();
    assert!(writer.write_all(b"too long").is_err());
    assert!(writer.seek(SeekFrom::Start(5)).is_err());
    writer.write_all(b"full").unwrap();
    drop(writer);
    drop(stage.finalize().unwrap());
    assert_eq!(std::fs::read(&path).unwrap(), b"previous");
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
}
#[test]
fn publication_is_no_clobber_under_race_and_does_not_promote_crash_orphans() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("movie.mp4");
    let orphan = dir.path().join(".vibecut-export-crashed");
    std::fs::write(&orphan, b"partial orphan").unwrap();
    let stage = StreamingStage::new(&path, Overwrite::Refuse, 100).unwrap();
    stage.writer().unwrap().write_all(b"finished").unwrap();
    let final_stage = stage.finalize().unwrap();
    assert!(
        final_stage
            .publish_with(&Fault {
                sync: false,
                publish: false,
                race: Some(path.clone())
            })
            .is_err()
    );
    assert_eq!(std::fs::read(&path).unwrap(), b"racing writer");
    assert_eq!(std::fs::read(&orphan).unwrap(), b"partial orphan");
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 2);
    let stage = StreamingStage::new(&path, Overwrite::Replace, 100).unwrap();
    stage.writer().unwrap().write_all(b"finished").unwrap();
    stage.finalize().unwrap().publish().unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), b"finished");
    assert_eq!(std::fs::read(&orphan).unwrap(), b"partial orphan");
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 2);
}

#[test]
fn stage_refuses_concurrent_writers_and_finalize_with_an_open_writer() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("movie.mp4");
    std::fs::write(&path, b"previous").unwrap();
    let stage = StreamingStage::new(&path, Overwrite::Replace, 100).unwrap();
    let mut writer = stage.writer().unwrap();
    assert!(stage.writer().is_err());
    writer.write_all(b"staged").unwrap();
    assert!(stage.finalize().is_err());
    writer.write_all(b"private").unwrap();
    drop(writer);
    assert_eq!(std::fs::read(path).unwrap(), b"previous");
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
}
