#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]
use engine::cache::{ArtifactKind, CacheKey, KeyInputs};
use engine::jobs::{
    JobBytes, JobKind, JobRunner, JobSpec, JobState, ResourceClass, RunnerConfig, SystemClock,
};
use engine::{ByteBudget, control::Error as EngineError};
use render::ExecutionHooks;
use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};

fn execute(
    limit: u64,
    task: impl Fn(&engine::jobs::JobContext) -> Result<(), EngineError> + Send + Sync + 'static,
) {
    let budget = ByteBudget::new(limit).unwrap();
    let mut runner = JobRunner::new(
        RunnerConfig {
            concurrency: [1; 3],
            queue: 1,
            retained: 1,
            network_attempts: 1,
            backoff: Duration::from_millis(10),
            progress_interval: Duration::from_millis(10),
        },
        budget.clone(),
        Arc::new(SystemClock::default()),
    )
    .unwrap();
    let input = CacheKey::new(KeyInputs {
        kind: ArtifactKind::Preview,
        source: [0; 32],
        provider: "render",
        model: "kernels",
        version: "1",
        settings: b"",
        transcript: [0; 32],
        cues: [0; 32],
        clip_range: [0; 32],
        translation: [0; 32],
    })
    .unwrap();
    let (id, _) = runner
        .submit::<engine::control::Error>(
            JobSpec {
                kind: JobKind::Preview,
                class: ResourceClass::Cpu,
                input,
                bytes: JobBytes::default(),
            },
            move |h| {
                task(h)?;
                Ok(Vec::new())
            },
        )
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        runner.tick().unwrap();
        let state = runner.snapshot(id).unwrap();
        if matches!(state.state, JobState::Failed | JobState::Cancelled) {
            panic!("{:?}", state.error);
        }
        if state.state == JobState::Completed {
            break;
        }
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(1));
    }
    runner.take_output(id).unwrap();
    assert_eq!(budget.snapshot().unwrap().used, 0);
}

#[test]
fn pixel_tasks_join_errors_and_unwinds_and_release_charges() {
    execute(16 << 20, |h| {
        for panic in [false, true] {
            let completed = AtomicUsize::new(0);
            let mut tasks: Vec<_> = (0..h.parallelism())
                .map(|i| {
                    let completed = &completed;
                    move || {
                        if i == 0 {
                            if panic {
                                panic!("injected pixel panic");
                            }
                            return Err(render::Error::Cancelled);
                        }
                        std::thread::sleep(Duration::from_millis(10));
                        completed.fetch_add(1, Ordering::SeqCst);
                        Ok(())
                    }
                })
                .collect();
            let mut borrowed: Vec<&mut (dyn FnMut() -> render::Result<()> + Send)> =
                tasks.iter_mut().map(|t| t as _).collect();
            let result = h.run_tasks(&mut borrowed);
            assert!(if panic {
                matches!(result, Err(render::Error::WorkerPanicked))
            } else {
                matches!(result, Err(render::Error::Cancelled))
            });
            assert_eq!(completed.load(Ordering::SeqCst), h.parallelism() - 1);
        }
        Ok(())
    });
}

#[test]
fn pixel_tasks_refuse_oversized_batches_and_fall_back_before_stack_allocation() {
    execute(128 << 10, |h| {
        let calls = AtomicUsize::new(0);
        let mut work: Vec<_> = (0..9)
            .map(|_| {
                || {
                    calls.fetch_add(1, Ordering::SeqCst);
                    Ok(())
                }
            })
            .collect();
        let mut tasks: Vec<&mut (dyn FnMut() -> render::Result<()> + Send)> =
            work.iter_mut().map(|t| t as _).collect();
        assert!(matches!(
            h.run_tasks(&mut tasks),
            Err(render::Error::Invalid(_))
        ));
        if h.parallelism() > 1 {
            h.run_tasks(&mut tasks[..2]).unwrap();
        }
        assert_eq!(
            calls.load(Ordering::SeqCst),
            if h.parallelism() > 1 { 2 } else { 0 }
        );
        Ok(())
    });
}

struct Sequential;
struct Lease;
impl render::Reservation for Lease {}
impl ExecutionHooks for Sequential {
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
#[test]
fn parallel_ingress_and_both_pack_modes_are_bit_exact_for_partial_row_chunks() {
    execute(32 << 20, |h| {
        let mut rgba = Vec::new();
        for i in 0..(517 * 263) {
            let a = (i % 256) as u8;
            rgba.extend_from_slice(&[
                (i * 31 % 256) as u8,
                (i * 17 % 256) as u8,
                (i * 7 % 256) as u8,
                a,
            ]);
        }
        for premultiplied in [false, true] {
            if premultiplied {
                for pixel in rgba.chunks_mut(4) {
                    for k in 0..3 {
                        pixel[k] = pixel[k].min(pixel[3]);
                    }
                }
            }
            for transfer in [1, 8, 13] {
                let expected = render::Image::from_rgba8(
                    517,
                    263,
                    &rgba,
                    premultiplied,
                    transfer,
                    &Sequential,
                )?;
                let actual =
                    render::Image::from_rgba8(517, 263, &rgba, premultiplied, transfer, h)?;
                assert_eq!(actual.pixels(), expected.pixels());
                assert_eq!(
                    actual.rgba8(transfer, h)?.bytes,
                    expected.rgba8(transfer, &Sequential)?.bytes
                );
                assert_eq!(
                    actual.video_rgba8(transfer, h)?.bytes,
                    expected.video_rgba8(transfer, &Sequential)?.bytes
                );
            }
        }
        Ok(())
    });
}

struct Pattern(Vec<u8>);
impl render::SourceProvider for Pattern {
    fn status(
        &mut self,
        _: &media::SourceAsset,
        _: bool,
        _: &dyn ExecutionHooks,
    ) -> render::Result<Option<render::UnavailableReason>> {
        Ok(None)
    }
    fn video(
        &mut self,
        _: &media::SourceAsset,
        _: project::ItemId,
        _: time::Tick,
        h: &dyn ExecutionHooks,
    ) -> render::Result<Option<render::Image>> {
        Ok(Some(render::Image::from_rgba8(
            521, 269, &self.0, false, 13, h,
        )?))
    }
    fn audio(
        &mut self,
        _: &media::SourceAsset,
        _: project::ItemId,
        _: i64,
        n: usize,
        rate: u32,
        h: &dyn ExecutionHooks,
    ) -> render::Result<render::AudioBlock> {
        render::AudioBlock::new(rate, 2, n, render::MemoryKind::Decoded, h)
    }
}
#[test]
fn parallel_draw_matches_sequential_at_chunk_edges_and_half_reductions() {
    execute(32 << 20, |h| {
        let duration = time::Tick(time::TICKS_PER_SECOND);
        let asset = testkit::source::asset(1, std::env::temp_dir().join("kernel-source"))
            .format("synthetic")
            .duration(duration)
            .stream(
                testkit::source::video()
                    .codec("rgba")
                    .time_base(media::TimeBase::new(1, 30).unwrap())
                    .duration(duration)
                    .start(time::Tick::ZERO)
                    .frame_count(30)
                    .variable_frame_rate(false)
                    .size(521, 269)
                    .build(),
            )
            .build();
        let mut data = project::Project::empty(project::ProjectId(0), "".into())
            .unwrap()
            .into_data();
        data.canvas.background = project::Color([0; 4]);
        data.assets.push(asset);
        data.scene_mut().timeline.insert_track(project::Track {
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
                source_asset: Some(media::AssetId(1)),
                source_range: time::TimeRange::new(time::Tick::ZERO, duration).unwrap(),
                start: time::Tick::ZERO,
                content: project::ItemParams::Video(project::VideoParams {
                    fades: Default::default(),
                    source_audio_enabled: false,
                    volume: 0.0,
                    muted: false,
                    crop: None,
                    cleanup: Default::default(),
                    reduce_noise: false,
                }),
            }],
        });
        let project = project::Project::new(data).unwrap();
        let mut rgba = Vec::new();
        for i in 0..521 * 269 {
            rgba.extend_from_slice(&[
                (i * 31 % 256) as u8,
                (i * 17 % 256) as u8,
                (i * 7 % 256) as u8,
                if i % 2 == 0 { 255 } else { (i % 256) as u8 },
            ]);
        }
        for (width, height, fit, crop) in [
            (
                513,
                257,
                render::Fit::Cover,
                project::Crop {
                    x: 0.13,
                    y: 0.21,
                    width: 0.71,
                    height: 0.61,
                },
            ),
            (513, 257, render::Fit::Contain, project::Crop::default()),
            (257, 263, render::Fit::Stretch, project::Crop::default()),
        ] {
            for opacity in [0.73, 1.0] {
                let mut settings = render::RenderSettings::from_project(&project);
                settings.width = width;
                settings.height = height;
                settings.placements.insert(
                    project::ItemId(0),
                    render::Placement {
                        destination: render::Rect {
                            x: -0.2,
                            y: -0.03,
                            width: 1.21,
                            height: 1.2,
                        },
                        crop,
                        fit,
                        opacity,
                    },
                );
                let mut reference = render::Renderer::new(&project, settings.clone())?;
                let mut actual = render::Renderer::new(&project, settings)?;
                let expected = reference.render_frame(
                    time::Tick::ZERO,
                    &mut Pattern(rgba.clone()),
                    &Sequential,
                )?;
                let result =
                    actual.render_frame(time::Tick::ZERO, &mut Pattern(rgba.clone()), h)?;
                assert_eq!(result.pixels(), expected.pixels());
            }
        }
        Ok(())
    });
}
