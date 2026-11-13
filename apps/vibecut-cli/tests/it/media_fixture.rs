//! Creates synthetic media for subprocess oracle checks.
use std::{ffi::OsStr, fs, path::Path};
use time::{Tick, TimeRange};
fn os(value: &str) -> &OsStr {
    OsStr::new(value)
}

#[allow(unused_imports)]
pub use testkit::oracle::command as tool_command;
pub use testkit::oracle::ffmpeg;

pub(super) fn tools() -> bool {
    testkit::oracle::require("CLI oracle")
}
pub(super) fn generate_av(path: &Path, seconds: u32) {
    let video = format!("testsrc2=size=128x96:rate=30:duration={seconds}");
    let audio = format!("sine=frequency=997:sample_rate=44100:duration={seconds}");
    ffmpeg(&[
        os("-f"),
        os("lavfi"),
        os("-i"),
        os(&video),
        os("-f"),
        os("lavfi"),
        os("-i"),
        os(&audio),
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
    data.scene_mut().timeline.insert_track(project::Track {
        id: project::TrackId(1),
        name: "Video".into(),
        kind: project::TrackKind::Video,
        muted: false,
        hidden: false,
        locked: false,
        items: vec![
            testkit::item::video(project::ItemId(0))
                .asset(media::AssetId(1))
                .source(TimeRange::new(Tick::ZERO, duration).unwrap())
                .build(),
        ],
    });
    if let Some(audio) = audio {
        let audio_asset = media::SourceAsset::import(media::AssetId(2), audio.to_owned()).unwrap();
        let audio_duration = audio_asset.probe().duration.unwrap();
        data.assets.push(audio_asset);
        data.scene_mut().timeline.insert_track(project::Track {
            id: project::TrackId(2),
            name: "Audio".into(),
            kind: project::TrackKind::Audio,
            muted: false,
            hidden: false,
            locked: false,
            items: vec![
                testkit::item::audio(project::ItemId(1))
                    .asset(media::AssetId(2))
                    .source(TimeRange::new(Tick::ZERO, audio_duration).unwrap())
                    .build(),
            ],
        });
    }
    let project = project::Project::new(data).unwrap();
    fs::write(project_path, project::encode(&project, true).unwrap()).unwrap();
}
