#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]
use crate::support::*;
use project::atomic::Overwrite;
use render::{Error, ExportMode, ExportSettings, OutputOptions, SceneChoice, export};
use std::path::{Path, PathBuf};
use time::{Tick, TimeRange};

const COLOR: [u8; 4] = [70, 160, 210, 255];

fn settings(project: &project::Project) -> ExportSettings {
    let mut settings = ExportSettings::from_project(project);
    settings.image_sequence = true;
    settings
}

fn names(directory: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(directory)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .collect();
    names.sort();
    names
}

fn decode(path: &Path) -> png_codec::RgbaImage {
    let bytes = std::fs::read(path).unwrap();
    png_codec::load_from_memory_with_format(&bytes, png_codec::ImageFormat::Png)
        .unwrap()
        .to_rgba8()
}

fn near(pixel: &[u8], expected: [u8; 4]) -> bool {
    pixel
        .iter()
        .zip(expected)
        .all(|(found, wanted)| found.abs_diff(wanted) <= 1)
}

fn one_second() -> TimeRange {
    TimeRange::new(Tick::ZERO, tick(1, 1)).unwrap()
}

/// A sequence names its frames `<name>_%05d.png` from 1, promotes one directory and
/// leaves nothing else beside it.
#[test]
fn png_sequence_names_frames_and_promotes_one_directory() {
    let p = solid_project(1, 32, 24, COLOR);
    let parent = tempfile::tempdir().unwrap();
    let output = parent.path().join("take");
    let receipt = export(&p, one_second(), &settings(&p), &output, &Hooks::default()).unwrap();
    assert_eq!(receipt.video_frames, 30);
    assert_eq!(receipt.audio_samples, 0);
    assert!(!receipt.stream_copied);
    let frames = names(&output);
    assert_eq!(frames.len(), 30);
    assert_eq!(frames.first().unwrap(), "take_00001.png");
    assert_eq!(frames.last().unwrap(), "take_00030.png");
    assert_eq!(frames[1], render::sequence_frame_name("take", 2));
    for name in [frames.first().unwrap(), frames.last().unwrap()] {
        let image = decode(&output.join(name));
        assert_eq!((image.width(), image.height()), (32, 24));
        assert!(
            image.pixels().all(|pixel| near(&pixel.0, COLOR)),
            "{name} is the flat project colour"
        );
    }
    assert_eq!(names(parent.path()), ["take"], "no staging left beside it");
}

/// The chosen range, rate and canvas decide the frames.
#[test]
fn png_sequence_follows_range_rate_and_canvas() {
    let p = solid_project(1, 32, 24, COLOR);
    let parent = tempfile::tempdir().unwrap();
    let output = parent.path().join("half");
    let mut chosen = settings(&p);
    chosen.fps = time::FrameRate::FPS_24;
    chosen.render.width = 16;
    chosen.render.height = 12;
    let range = TimeRange::new(Tick::ZERO, tick(1, 2)).unwrap();
    let receipt = export(&p, range, &chosen, &output, &Hooks::default()).unwrap();
    assert_eq!(receipt.video_frames, 12);
    let frames = names(&output);
    assert_eq!(frames.len(), 12);
    assert_eq!(frames.last().unwrap(), "half_00012.png");
    let last = decode(&output.join("half_00012.png"));
    assert_eq!((last.width(), last.height()), (16, 12));
    assert!(last.pixels().all(|pixel| near(&pixel.0, COLOR)));
}

/// An existing destination survives every refusal and failure; only an earlier
/// sequence of the same name is replaced, and only on request.
#[test]
fn png_sequence_preserves_an_existing_destination() {
    let p = solid_project(1, 16, 12, COLOR);
    let parent = tempfile::tempdir().unwrap();
    let output = parent.path().join("take");

    // A directory, refused without consent.
    std::fs::create_dir(&output).unwrap();
    std::fs::write(output.join("keep.txt"), b"mine").unwrap();
    let result = export(&p, one_second(), &settings(&p), &output, &Hooks::default());
    assert!(matches!(result, Err(Error::Project(_))), "{result:?}");
    // Replace still refuses a directory that holds other files.
    let mut replace = settings(&p);
    replace.overwrite = Overwrite::Replace;
    let result = export(&p, one_second(), &replace, &output, &Hooks::default());
    assert!(matches!(result, Err(Error::Project(_))), "{result:?}");
    assert_eq!(names(&output), ["keep.txt"]);
    assert_eq!(std::fs::read(output.join("keep.txt")).unwrap(), b"mine");
    assert_eq!(names(parent.path()), ["take"]);

    // A file or a link cannot become a directory.
    std::fs::remove_dir_all(&output).unwrap();
    std::fs::write(&output, b"a file").unwrap();
    let result = export(&p, one_second(), &replace, &output, &Hooks::default());
    assert!(matches!(result, Err(Error::Project(_))), "{result:?}");
    assert_eq!(std::fs::read(&output).unwrap(), b"a file");
    std::fs::remove_file(&output).unwrap();

    // An earlier sequence of the same name is replaced whole, with no stale frames.
    std::fs::create_dir(&output).unwrap();
    std::fs::write(output.join("take_00099.png"), b"stale").unwrap();
    export(&p, one_second(), &replace, &output, &Hooks::default()).unwrap();
    let frames = names(&output);
    assert_eq!(frames.len(), 30);
    assert!(!frames.contains(&"take_00099.png".to_owned()));
    assert_eq!(names(parent.path()), ["take"]);
}

/// Cancelling during rendering or at the publication fence promotes nothing and leaves
/// neither a final nor a staging directory; a prior sequence is untouched.
#[test]
fn png_sequence_cancellation_leaves_no_partial_directory() {
    let p = solid_project(1, 16, 12, COLOR);
    let parent = tempfile::tempdir().unwrap();
    let output = parent.path().join("take");
    for at_publish in [false, true] {
        let hooks = Hooks::default();
        let state = hooks.state.clone();
        let renders = std::sync::atomic::AtomicU32::new(0);
        let hooks = Hooks {
            event: Some(std::sync::Arc::new(move |stage| {
                let cancel = if at_publish {
                    stage == render::Stage::Publish
                } else {
                    stage == render::Stage::Render
                        && renders.fetch_add(1, std::sync::atomic::Ordering::SeqCst) == 9
                };
                if cancel {
                    state.store(1, std::sync::atomic::Ordering::SeqCst);
                }
                Ok(())
            })),
            ..hooks
        };
        let result = export(&p, one_second(), &settings(&p), &output, &hooks);
        assert!(matches!(result, Err(Error::Cancelled)), "{result:?}");
        assert!(
            names(parent.path()).is_empty(),
            "{:?}",
            names(parent.path())
        );
    }
    // With a prior sequence in place, a cancelled replacement keeps it.
    std::fs::create_dir(&output).unwrap();
    std::fs::write(output.join("take_00001.png"), b"prior").unwrap();
    let hooks = Hooks::default();
    let state = hooks.state.clone();
    let hooks = Hooks {
        event: Some(std::sync::Arc::new(move |stage| {
            if stage == render::Stage::Publish {
                state.store(1, std::sync::atomic::Ordering::SeqCst);
            }
            Ok(())
        })),
        ..hooks
    };
    let mut replace = settings(&p);
    replace.overwrite = Overwrite::Replace;
    let result = export(&p, one_second(), &replace, &output, &hooks);
    assert!(matches!(result, Err(Error::Cancelled)), "{result:?}");
    assert_eq!(names(&output), ["take_00001.png"]);
    assert_eq!(
        std::fs::read(output.join("take_00001.png")).unwrap(),
        b"prior"
    );
    assert_eq!(names(parent.path()), ["take"]);
}

/// Hostile names and bounds refuse before any staging.
#[test]
fn png_sequence_refuses_hostile_destinations_and_settings() {
    let p = solid_project(1, 16, 12, COLOR);
    let parent = tempfile::tempdir().unwrap();
    for hostile in [".hidden", ".vibecut-export-x"] {
        let output = parent.path().join(hostile);
        let result = export(&p, one_second(), &settings(&p), &output, &Hooks::default());
        assert!(
            matches!(result, Err(Error::Invalid(_))),
            "{hostile}: {result:?}"
        );
    }
    // A missing parent never gets created.
    let result = export(
        &p,
        one_second(),
        &settings(&p),
        &parent.path().join("absent").join("take"),
        &Hooks::default(),
    );
    assert!(result.is_err());
    assert!(names(parent.path()).is_empty());

    // Audio, stream copy and more frames than five digits name are refused.
    let output = parent.path().join("take");
    let mut audio = settings(&p);
    audio.audio_only = true;
    assert!(matches!(
        export(&p, one_second(), &audio, &output, &Hooks::default()),
        Err(Error::Invalid(_))
    ));
    let mut copy = settings(&p);
    copy.mode = ExportMode::StreamCopy;
    assert!(matches!(
        export(&p, one_second(), &copy, &output, &Hooks::default()),
        Err(Error::Unsupported(_))
    ));
    let long = solid_project(3600, 16, 12, COLOR);
    let result = export(
        &long,
        TimeRange::new(Tick::ZERO, long.duration()).unwrap(),
        &settings(&long),
        &output,
        &Hooks::default(),
    );
    assert!(
        matches!(result, Err(Error::Invalid("image sequence frame count"))),
        "{result:?}"
    );
    assert!(names(parent.path()).is_empty());
}

/// The shared output options refuse video-only choices and several scenes for a sequence.
#[test]
fn png_sequence_options_refuse_video_choices() {
    let p = solid_project(1, 16, 12, COLOR);
    let mut sequence = settings(&p);
    OutputOptions::default().apply(&mut sequence).unwrap();
    OutputOptions::default()
        .check_sequence(&p, &sequence)
        .unwrap();
    for options in [
        OutputOptions {
            bit_rate: Some(1_000_000),
            ..OutputOptions::default()
        },
        OutputOptions {
            codec: render::VideoCodec::H264,
            ..OutputOptions::default()
        },
        OutputOptions {
            profile: media::H264Profile::High,
            ..OutputOptions::default()
        },
        OutputOptions {
            scene: SceneChoice::All,
            ..OutputOptions::default()
        },
    ] {
        assert!(
            matches!(
                options.apply(&mut sequence.clone()),
                Err(Error::Unsupported(_))
            ),
            "{options:?}"
        );
    }
    // Rate and canvas choices pass, and an explicit rate changes the frame bound.
    let mut chosen = settings(&p);
    OutputOptions {
        resolution: Some([8, 6]),
        fps: Some(time::FrameRate::FPS_60),
        ..OutputOptions::default()
    }
    .apply(&mut chosen)
    .unwrap();
    assert_eq!(
        (chosen.render.width, chosen.fps),
        (8, time::FrameRate::FPS_60)
    );
    let long = solid_project(1700, 16, 12, COLOR);
    let mut fast = settings(&long);
    let options = OutputOptions {
        fps: Some(time::FrameRate::FPS_60),
        ..OutputOptions::default()
    };
    options.apply(&mut fast).unwrap();
    assert!(matches!(
        options.check_sequence(&long, &fast),
        Err(Error::Invalid("image sequence frame count"))
    ));
}

/// `<name>_%05d.png` selects a sequence directory; anything else is not one.
#[test]
fn output_pattern_selects_the_sequence_directory() {
    let root = PathBuf::from("out");
    assert_eq!(
        render::sequence_destination(&root.join("shot_%05d.png")).unwrap(),
        Some(root.join("shot"))
    );
    for plain in ["shot.png", "shot_%05d.mp4", "shot.mp4", "shot_%04d.png"] {
        assert_eq!(
            render::sequence_destination(&root.join(plain)).unwrap(),
            None,
            "{plain}"
        );
    }
    for hostile in ["_%05d.png", "a%b_%05d.png", ".x_%05d.png"] {
        assert!(
            render::sequence_destination(&root.join(hostile)).is_err(),
            "{hostile}"
        );
    }
}

/// A destination directory that holds a project source is that source's alias.
#[test]
fn png_sequence_destination_holding_a_source_is_refused() {
    let parent = tempfile::tempdir().unwrap();
    let output = parent.path().join("take");
    std::fs::create_dir(&output).unwrap();
    let source = output.join("source.mp4");
    std::fs::write(&source, b"source bytes").unwrap();
    let mut data = project::Project::empty(project::ProjectId(0), "alias".into())
        .unwrap()
        .into_data();
    data.assets = vec![
        media::SourceAsset::try_from(media::AssetData {
            ephemeral: false,
            id: media::AssetId(1),
            path: std::fs::canonicalize(&source).unwrap(),
            content_hash: media::ContentHash::from_bytes(b"source bytes"),
            probe: media::SourceProbe {
                format: "fixture".into(),
                duration: None,
                streams: Vec::new(),
            },
        })
        .unwrap(),
    ];
    let p = project::Project::new(data).unwrap();
    let result = render::validate_output_path(&p, &output, &Hooks::default());
    assert!(
        matches!(result, Err(Error::Invalid("output aliases a source"))),
        "{result:?}"
    );
    assert_eq!(std::fs::read(&source).unwrap(), b"source bytes");
}
