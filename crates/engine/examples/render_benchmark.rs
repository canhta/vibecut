//! Developer measurements through the actual engine worker and render hooks.
use engine::cache::{ArtifactKind, CacheKey, KeyInputs};
use engine::jobs::{
    JobBytes, JobKind, JobRunner, JobSpec, JobState, ResourceClass, RunnerConfig, SystemClock,
};
use engine::{ByteBudget, EditError};
use std::{
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};
use time::{Tick, TimeRange};
fn main() {
    if let Err(error) = run() {
        eprintln!("benchmark failed: {error:?}");
        std::process::exit(1);
    }
}
fn run() -> Result<(), Box<dyn std::error::Error>> {
    let mode = std::env::args().nth(1).unwrap_or_else(|| "text".into());
    let budget = ByteBudget::new(2 << 30)?;
    let mut runner = JobRunner::new(
        RunnerConfig {
            concurrency: [1; 3],
            queue: 1,
            retained: 1,
            network_attempts: 1,
            backoff: Duration::from_millis(100),
            progress_interval: Duration::from_millis(10),
        },
        budget.clone(),
        Arc::new(SystemClock::default()),
    )?;
    let key = CacheKey::new(KeyInputs {
        kind: ArtifactKind::Preview,
        source: [0; 32],
        provider: "cpu-reference",
        model: "render",
        version: env!("CARGO_PKG_VERSION"),
        settings: mode.as_bytes(),
        transcript: [0; 32],
        cues: [0; 32],
        clip_range: [0; 32],
        translation: [0; 32],
    })?;
    let (id, _) = runner.submit(
        JobSpec {
            kind: if mode.starts_with("text") {
                JobKind::Preview
            } else {
                JobKind::Export
            },
            class: ResourceClass::Cpu,
            input: key,
            bytes: JobBytes {
                result: 4096,
                ..Default::default()
            },
        },
        move |hooks| {
            let output = if mode.starts_with("text") {
                text_benchmark(hooks, mode == "text-uncached")?
            } else {
                export_benchmark(hooks, mode == "blur")?
            };
            serde_json::to_vec(&output)
                .map_err(|_| EditError::Control(engine::control::ControlError::InvalidParams))
        },
    )?;
    let mut charged_peak = 0u64;
    loop {
        runner.tick()?;
        charged_peak = charged_peak.max(budget.snapshot()?.used);
        if let Some(s) = runner.snapshot(id) {
            if s.state == JobState::Completed {
                let output = runner
                    .take_output(id)
                    .ok_or(EditError::Invalid("missing benchmark output"))?;
                println!("{}", std::str::from_utf8(output.bytes())?);
                println!("sampled aggregate charged peak: {charged_peak} bytes");
                break;
            }
            if matches!(s.state, JobState::Failed | JobState::Cancelled) {
                return match s.error {
                    Some(error) => Err(Box::new(error)),
                    None => Err("job failure".into()),
                };
            }
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    Ok(())
}
fn text_benchmark(
    hooks: &engine::jobs::JobContext,
    uncached: bool,
) -> Result<serde_json::Value, EditError> {
    let mut d = project::Project::empty(project::ProjectId(0), "".into())?.into_data();
    d.canvas.width = 640;
    d.canvas.height = 360;
    d.scene_mut().timeline.insert_track(project::Track {
        id: project::TrackId(1),
        name: "".into(),
        kind: project::TrackKind::Text,
        muted: false,
        hidden: false,
        locked: false,
        items: vec![project::Item {
            hidden: false,
            blend_mode: Default::default(),
            retime: Default::default(),
            masks: Vec::new(),
            effects: Vec::new(),
            animations: Default::default(),
            id: project::ItemId(0),
            source_asset: None,
            source_range: TimeRange::new(Tick::ZERO, Tick(time::TICKS_PER_SECOND))?,
            start: Tick::ZERO,
            content: project::ItemParams::Text(project::TextParams {
                content: "Tiếng Việt 中文".into(),
                font_size: 40.0,
                color: project::Color([255; 4]),
                ..Default::default()
            }),
        }],
    });
    let p = project::Project::new(d)?;
    let mut settings = render::RenderSettings::from_project(&p);
    if uncached {
        settings.text_cache_bytes = 0;
    }
    let mut renderer = render::Renderer::new(&p, settings)?;
    let mut sources = render::FileSources::default();
    let start = Instant::now();
    let reference = renderer.render_frame(Tick::ZERO, &mut sources, hooks)?;
    let cold = start.elapsed();
    let mut elapsed = Duration::ZERO;
    for _ in 0..100 {
        let start = Instant::now();
        let frame = renderer.render_frame(Tick::ZERO, &mut sources, hooks)?;
        elapsed += start.elapsed();
        if frame.pixels() != reference.pixels() {
            return Err(EditError::Invalid("text benchmark pixel drift"));
        }
    }
    let stats = renderer.text_cache_stats();
    Ok(
        serde_json::json!({"cache_enabled":!uncached,"cache_entries":stats.entries,"cache_bytes":stats.bytes,"cache_hits":stats.hits,"cache_misses":stats.misses,"mode":"text","canvas":[640,360],"font":"Inter Regular 40px with bundled CJK fallback","cold_ms":cold.as_secs_f64()*1000.0,"warm_100_ms":elapsed.as_secs_f64()*1000.0,"warm_mean_ms":elapsed.as_secs_f64()*10.0,"pixel_equality":true}),
    )
}
fn export_benchmark(
    hooks: &engine::jobs::JobContext,
    blur: bool,
) -> Result<serde_json::Value, EditError> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/fixtures/render");
    std::fs::create_dir_all(&root)?;
    let input = root.join("benchmark-60s-1080p.mp4");
    if !input.exists() {
        let output = std::process::Command::new("ffmpeg")
            .env_remove("DYLD_LIBRARY_PATH")
            .env_remove("LD_LIBRARY_PATH")
            .args([
                "-hide_banner",
                "-loglevel",
                "error",
                "-y",
                "-f",
                "lavfi",
                "-i",
                "testsrc2=size=1920x1080:rate=30:duration=60",
                "-an",
                "-c:v",
                "libx264",
                "-preset",
                "ultrafast",
                "-crf",
                "18",
                "-pix_fmt",
                "yuv420p",
                "-color_primaries",
                "bt709",
                "-color_trc",
                "bt709",
                "-colorspace",
                "bt709",
            ])
            .arg(&input)
            .output()?;
        if !output.status.success() {
            return Err(EditError::Invalid("benchmark fixture generation"));
        }
    }
    let asset =
        media::SourceAsset::import(media::AssetId(0), input).map_err(project::Error::from)?;
    let duration = asset
        .probe()
        .duration
        .ok_or(EditError::Invalid("fixture duration"))?;
    let mut d = project::Project::empty(project::ProjectId(0), "".into())?.into_data();
    d.assets.push(asset);
    d.scene_mut().timeline.insert_track(project::Track {
        id: project::TrackId(1),
        name: "".into(),
        kind: project::TrackKind::Video,
        muted: false,
        hidden: false,
        locked: false,
        items: vec![project::Item {
            hidden: false,
            blend_mode: Default::default(),
            retime: Default::default(),
            masks: Vec::new(),
            effects: Vec::new(),
            animations: Default::default(),
            id: project::ItemId(0),
            source_asset: Some(media::AssetId(0)),
            source_range: TimeRange::new(Tick::ZERO, duration)?,
            start: Tick::ZERO,
            content: project::ItemParams::Video(project::VideoParams {
                fades: Default::default(),
                source_audio_enabled: false,
                volume: 0.0,
                muted: false,
                crop: None,
            }),
        }],
    });
    if blur
        && let Some(item) = d
            .scene_mut()
            .timeline
            .overlays
            .first_mut()
            .and_then(|t| t.items.first_mut())
    {
        item.effects
            .push(project::effect_definition("blur")?.instance("blur"));
    }
    let p = project::Project::new(d)?;
    let mut settings = render::ExportSettings::from_project(&p);
    settings.render = if blur {
        render::RenderSettings::from_project(&p)
    } else {
        render::RenderSettings::portrait(&p)
    };
    settings.include_audio = false;
    settings.mode = render::ExportMode::Encode;
    settings.overwrite = project::atomic::Overwrite::Replace;
    let output = root.join(if blur {
        "benchmark-60s-blur.mp4"
    } else {
        "benchmark-60s-portrait.mp4"
    });
    let start = Instant::now();
    let receipt = render::export(
        &p,
        TimeRange::new(Tick::ZERO, duration)?,
        &settings,
        &output,
        hooks,
    )?;
    let elapsed = start.elapsed();
    Ok(
        serde_json::json!({"mode":if blur { "blur" } else { "export" },"input_seconds":duration.seconds(),"input_canvas":[1920,1080],"output_canvas":[settings.render.width,settings.render.height],"frames":receipt.video_frames,"elapsed_seconds":elapsed.as_secs_f64(),"fps":receipt.video_frames as f64/elapsed.as_secs_f64(),"working_frame_bytes":render::Image::bytes(settings.render.width,settings.render.height)?}),
    )
}
