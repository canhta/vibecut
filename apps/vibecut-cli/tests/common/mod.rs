use std::{
    ffi::OsStr,
    fs,
    path::Path,
    process::{Command, Stdio},
};
use time::{Tick, TimeRange};
fn os(value: &str) -> &OsStr {
    OsStr::new(value)
}

pub(super) fn tool_command(program: &str) -> Command {
    let mut command = Command::new(program);
    // Production's shared libraries must not replace the external oracle's.
    command
        .env_remove("DYLD_LIBRARY_PATH")
        .env_remove("LD_LIBRARY_PATH");
    if let (Some(prefix), Some(paths)) = (std::env::var_os("FFMPEG_DIR"), std::env::var_os("PATH"))
    {
        let production = std::path::PathBuf::from(prefix).join("bin");
        let paths: Vec<_> = std::env::split_paths(&paths)
            .filter(|path| path != &production)
            .collect();
        command.env("PATH", std::env::join_paths(paths).unwrap());
    }
    command
}

pub(super) fn tools() -> bool {
    let available = ["ffmpeg", "ffprobe"].iter().all(|tool| {
        match tool_command(tool).arg("-version").output() {
            Ok(output) => {
                assert!(output.status.success(), "found {tool} but it failed");
                true
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
            Err(error) => panic!("cannot run {tool}: {error}"),
        }
    });
    if !available {
        assert!(
            std::env::var_os("VIBECUT_REQUIRE_ORACLES").is_none(),
            "required ffmpeg/ffprobe missing"
        );
        eprintln!("SKIP CLI oracle: ffmpeg/ffprobe missing");
    }
    available
}
pub(super) fn ffmpeg(args: &[&OsStr]) {
    let status = tool_command("ffmpeg")
        .arg("-v")
        .arg("error")
        .arg("-y")
        .args(args)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .unwrap();
    assert!(status.success());
}
pub(super) fn generate_av(path: &Path) {
    ffmpeg(&[
        os("-f"),
        os("lavfi"),
        os("-i"),
        os("testsrc2=size=128x96:rate=30:duration=1"),
        os("-f"),
        os("lavfi"),
        os("-i"),
        os("sine=frequency=997:sample_rate=44100:duration=1"),
        os("-c:v"),
        os("libx264"),
        os("-pix_fmt"),
        os("yuv420p"),
        os("-g"),
        os("30"),
        os("-color_primaries"),
        os("bt709"),
        os("-color_trc"),
        os("bt709"),
        os("-colorspace"),
        os("bt709"),
        os("-c:a"),
        os("aac"),
        os("-ac"),
        os("1"),
        os("-shortest"),
        path.as_os_str(),
    ]);
}
pub(super) fn media_project(project_path: &Path, video: &Path, audio: Option<&Path>) {
    let video_asset = media::SourceAsset::import(media::AssetId(1), video.to_owned()).unwrap();
    let stream = video_asset
        .probe()
        .streams
        .iter()
        .find_map(|stream| stream.video.as_ref())
        .unwrap();
    let fps = video_asset
        .probe()
        .streams
        .iter()
        .find_map(|stream| stream.frame_rate)
        .unwrap();
    let duration = video_asset.probe().duration.unwrap();
    let mut data = project::Project::empty(project::ProjectId(0), "media".into())
        .unwrap()
        .into_data();
    data.canvas.width = stream.width;
    data.canvas.height = stream.height;
    data.canvas.fps = fps;
    data.assets.push(video_asset);
    data.timeline.insert_track(project::Track {
        id: project::TrackId(1),
        name: "Video".into(),
        kind: project::TrackKind::Video,
        muted: false,
        hidden: false,
        locked: false,
        items: vec![project::Item {
            effects: Vec::new(),
            animations: Default::default(),
            id: project::ItemId(0),
            source_asset: Some(media::AssetId(1)),
            source_range: TimeRange::new(Tick::ZERO, duration).unwrap(),
            start: Tick::ZERO,
            content: project::ItemParams::Video(project::VideoParams {
                source_audio_enabled: false,
            }),
        }],
    });
    if let Some(audio) = audio {
        let audio_asset = media::SourceAsset::import(media::AssetId(2), audio.to_owned()).unwrap();
        let audio_duration = audio_asset.probe().duration.unwrap();
        data.assets.push(audio_asset);
        data.timeline.insert_track(project::Track {
            id: project::TrackId(2),
            name: "Audio".into(),
            kind: project::TrackKind::Audio,
            muted: false,
            hidden: false,
            locked: false,
            items: vec![project::Item {
                effects: Vec::new(),
                animations: Default::default(),
                id: project::ItemId(1),
                source_asset: Some(media::AssetId(2)),
                source_range: TimeRange::new(Tick::ZERO, audio_duration).unwrap(),
                start: Tick::ZERO,
                content: project::ItemParams::Audio(project::AudioParams {
                    gain_db: 0.0,
                    muted: false,
                }),
            }],
        });
    }
    let project = project::Project::new(data).unwrap();
    fs::write(project_path, project::encode(&project, true).unwrap()).unwrap();
}
