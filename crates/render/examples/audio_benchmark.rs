//! Sixty-second 1080p multi-track mix/export measurement, using production APIs.
use render::{ExecutionHooks, MemoryKind, Reservation};
use std::{path::PathBuf, time::Instant};
use time::{Rounding, Tick, TimeRange};
struct Hooks;
struct Lease;
impl Reservation for Lease {}
impl ExecutionHooks for Hooks {
    fn cancelled(&self) -> bool {
        false
    }
    fn reserve(&self, _: MemoryKind, bytes: u64) -> render::Result<Box<dyn Reservation>> {
        if bytes > 1 << 30 {
            return Err(render::Error::Budget {
                requested: bytes,
                limit: 1 << 30,
            });
        }
        Ok(Box::new(Lease))
    }
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let root = PathBuf::from(
        std::env::var_os("VIBECUT_AUDIO_BENCH_DIR").ok_or("set VIBECUT_AUDIO_BENCH_DIR")?,
    );
    let duration = Tick::from_units(60, 1, Rounding::Nearest)?;
    let mut data =
        project::Project::empty(project::ProjectId(1), "Mix benchmark".into())?.into_data();
    data.scene_mut().timeline.main.items.push(project::Item {
        hidden: false,
        id: project::ItemId(1),
        source_asset: None,
        source_range: TimeRange::new(Tick::ZERO, duration)?,
        start: Tick::ZERO,
        animations: Default::default(),
        effects: vec![],
        masks: vec![],
        blend_mode: Default::default(),
        retime: Default::default(),
        content: project::ItemParams::Overlay(project::OverlayParams {
            color: project::Color([32, 80, 128, 255]),
        }),
    });
    for (id, name) in [(2, "44100.wav"), (3, "48000.wav")] {
        data.assets.push(media::SourceAsset::import(
            media::AssetId(id),
            root.join(name),
        )?);
        data.scene_mut().timeline.insert_track(project::Track {
            id: project::TrackId(id),
            name: "".into(),
            kind: project::TrackKind::Audio,
            hidden: false,
            muted: false,
            locked: false,
            items: vec![project::Item {
                hidden: false,
                id: project::ItemId(id),
                source_asset: Some(media::AssetId(id)),
                source_range: TimeRange::new(Tick::ZERO, duration)?,
                start: Tick::ZERO,
                animations: Default::default(),
                effects: vec![],
                masks: vec![],
                blend_mode: Default::default(),
                retime: Default::default(),
                content: project::ItemParams::Audio(project::AudioParams {
                    fades: Default::default(),
                    volume: -6.0,
                    muted: false,
                    source: Default::default(),
                }),
            }],
        });
    }
    let project = project::Project::new(data)?;
    let mut settings = render::ExportSettings::from_project(&project);
    settings.mode = render::ExportMode::Encode;
    settings.overwrite = project::atomic::Overwrite::Replace;
    let start = Instant::now();
    let receipt = render::export(
        &project,
        TimeRange::new(Tick::ZERO, duration)?,
        &settings,
        &root.join("mixed.mp4"),
        &Hooks,
    )?;
    println!(
        "elapsed_seconds={} video_frames={} audio_samples={} fps={}",
        start.elapsed().as_secs_f64(),
        receipt.video_frames,
        receipt.audio_samples,
        receipt.video_frames as f64 / start.elapsed().as_secs_f64()
    );
    Ok(())
}
