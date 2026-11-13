#![allow(clippy::unwrap_used, reason = "validated test fixtures and assertions")]
use crate::support::session::{new_item, range};
use engine::{Command, NewItem, Session};
use media::{AssetData, AssetId, ContentHash, SourceAsset, SourceProbe};
use project::{Color, ItemParams, Project, ProjectId};
use serde_json::json;
use time::Tick;

#[test]
fn imported_asset_is_one_undo_step_and_restores_the_exact_record() {
    let mut s = Session::new(Project::empty(ProjectId(9), "Assets".into()).unwrap(), 8).unwrap();
    let asset = SourceAsset::try_from(AssetData {
        ephemeral: false,
        id: AssetId(3),
        path: std::env::temp_dir().join("import record.mp4"),
        content_hash: ContentHash([3; 32]),
        probe: SourceProbe {
            format: "mov,mp4".into(),
            duration: Some(Tick(2_000)),
            streams: vec![],
        },
    })
    .unwrap();
    s.apply(Command::AddAsset {
        asset: asset.clone(),
    })
    .unwrap();
    assert_eq!(s.project().data().assets, vec![asset.clone()]);
    s.apply(Command::Undo {}).unwrap();
    assert!(s.project().data().assets.is_empty());
    s.apply(Command::Redo {}).unwrap();
    assert_eq!(s.project().data().assets, vec![asset]);
}
fn policy_asset(id: u64, format: &str, rate: Option<time::FrameRate>) -> SourceAsset {
    policy_asset_sized(id, format, rate, (640, 360))
}
fn policy_asset_sized(
    id: u64,
    format: &str,
    rate: Option<time::FrameRate>,
    (width, height): (u32, u32),
) -> SourceAsset {
    let stream = testkit::source::video()
        .codec("synthetic")
        .duration(Tick(1000))
        .size(width, height);
    let stream = match rate {
        Some(rate) => stream.frame_rate(rate),
        None => stream.no_frame_rate(),
    };
    testkit::source::asset(id, std::env::temp_dir().join("policy source"))
        .ephemeral(true)
        .format(format)
        .duration(Tick(1000))
        .stream(stream.build())
        .build()
}
fn policy_insert(id: u64) -> Command {
    Command::InsertItem {
        placement: Default::default(),
        item: NewItem {
            retime: Default::default(),
            animations: Default::default(),
            source_asset: Some(AssetId(id)),
            source_range: range(0, 1000),
            start: Tick::ZERO,
            content: ItemParams::Video(project::VideoParams {
                fades: Default::default(),
                source_audio_enabled: true,
                volume: 0.0,
                muted: false,
                crop: None,
                cleanup: Default::default(),
                background: ClipBackground::None,
            }),
        },
    }
}
#[test]
fn import_policy_ratchets_exact_rates_and_refuses_invalid_metadata_atomically() {
    use time::FrameRate as F;
    for (initial, incoming, format, expected) in [
        (F::FPS_30, Some(F::FPS_60), "mp4", F::FPS_60),
        (F::FPS_60, Some(F::FPS_30), "mp4", F::FPS_60),
        (F::FPS_29_97, Some(F::FPS_30), "mp4", F::FPS_30),
        (F::FPS_30, Some(F::FPS_29_97), "mp4", F::FPS_30),
        (F::FPS_23_976, Some(F::FPS_29_97), "mp4", F::FPS_29_97),
        (F::FPS_30, Some(F::FPS_60), "png_pipe", F::FPS_30),
        (F::FPS_30, Some(F::FPS_60), "gif", F::FPS_30),
        (F::FPS_30, Some(F::FPS_60), "apng", F::FPS_30),
        (F::FPS_30, None, "mp4", F::FPS_30),
    ] {
        let mut data = Project::empty(ProjectId(1), "policy".into())
            .unwrap()
            .into_data();
        data.canvas.fps = initial;
        let before = Project::new(data).unwrap();
        let mut s = Session::new(before.clone(), 10).unwrap();
        s.apply(Command::AddAsset {
            asset: policy_asset(1, format, incoming),
        })
        .unwrap();
        assert_eq!(s.project().data().canvas.fps, expected);
        s.apply(Command::Undo {}).unwrap();
        assert_eq!(s.project().as_ref(), &before);
        s.apply(Command::Redo {}).unwrap();
        assert_eq!(s.project().data().canvas.fps, expected);
        let frozen = s.project().clone();
        for rate in [
            json!({"num":0,"den":1}),
            json!({"num":30,"den":0}),
            json!({"num":-30,"den":1}),
            json!({"num":29.97,"den":1}),
        ] {
            let mut asset = serde_json::to_value(policy_asset(2, "mp4", None)).unwrap();
            asset["probe"]["streams"][0]["frame_rate"] = rate;
            assert!(crate::wire::run(&mut s, "import.add_asset", json!({"asset":asset})).is_err());
            assert_eq!(s.project(), &frozen);
        }
        // A valid probe rational without exact canvas ticks is refused, never rounded.
        assert!(
            s.apply(Command::AddAsset {
                asset: policy_asset(2, "mp4", Some(F::new(1001, 1).unwrap()))
            })
            .is_err()
        );
        assert_eq!(s.project(), &frozen);
    }
    let mut audio = AssetData::from(policy_asset(1, "wav", Some(F::FPS_120)));
    audio.probe.streams.first_mut().unwrap().kind = media::StreamKind::Audio;
    audio.probe.streams.first_mut().unwrap().video = None;
    audio.probe.streams.first_mut().unwrap().audio = Some(media::AudioInfo {
        sample_rate: 48000,
        channels: 2,
        initial_padding: 0,
        trailing_padding: 0,
    });
    let mut s = Session::new(Project::empty(ProjectId(1), "".into()).unwrap(), 10).unwrap();
    s.apply(Command::AddAsset {
        asset: SourceAsset::try_from(audio).unwrap(),
    })
    .unwrap();
    assert_eq!(s.project().data().canvas.fps, F::FPS_30);
}
#[test]
fn first_media_canvas_is_atomic_persisted_and_batch_ordered() {
    use time::FrameRate as F;
    // Odd captured media sizes resolve the Original preset to the nearest even
    // size (ties up), never zero, in one undo step.
    for ((width, height), even) in [
        ((641, 361), (642, 362)),
        ((1, 1), (2, 2)),
        ((8191, 8191), (8192, 8192)),
    ] {
        let mut data = Project::empty(ProjectId(1), "odd".into())
            .unwrap()
            .into_data();
        data.assets.push(policy_asset_sized(
            1,
            "mp4",
            Some(F::FPS_30),
            (width, height),
        ));
        let before = Project::new(data).unwrap();
        let mut s = Session::new(before.clone(), 10).unwrap();
        s.apply(policy_insert(1)).unwrap();
        let size = project::CanvasSize {
            width: even.0,
            height: even.1,
        };
        let data = s.project().data();
        assert_eq!((data.canvas.width, data.canvas.height), even);
        assert_eq!(data.original_canvas_size, Some(size));
        assert_eq!(data.canvas.preset, Some(project::AspectPreset::Original));
        s.apply(Command::Undo {}).unwrap();
        assert_eq!(s.project().as_ref(), &before);
    }
    for format in ["mp4", "png_pipe"] {
        let mut data = Project::empty(ProjectId(1), "policy".into())
            .unwrap()
            .into_data();
        data.assets.push(policy_asset(1, format, Some(F::FPS_60)));
        let before = Project::new(data).unwrap();
        let mut s = Session::new(before.clone(), 10).unwrap();
        s.apply(policy_insert(1)).unwrap();
        let after = s.project().clone();
        assert_eq!(
            (after.data().canvas.width, after.data().canvas.height),
            (640, 360)
        );
        assert_eq!(
            after.data().original_canvas_size,
            Some(project::CanvasSize {
                width: 640,
                height: 360
            })
        );
        assert_eq!(
            after.data().canvas.preset,
            Some(project::AspectPreset::Original)
        );
        assert_eq!(
            after.data().canvas.fps,
            if format == "mp4" {
                F::FPS_60
            } else {
                F::FPS_30
            }
        );
        let loaded = project::decode(&project::encode(&after, false).unwrap()).unwrap();
        assert_eq!(&loaded.project, after.as_ref());
        assert!(loaded.project.data().assets.first().unwrap().ephemeral());
        s.apply(Command::Undo {}).unwrap();
        assert_eq!(s.project().as_ref(), &before);
        s.apply(Command::Redo {}).unwrap();
        assert_eq!(s.project(), &after);
    }
    let mut data = Project::empty(ProjectId(1), "policy".into())
        .unwrap()
        .into_data();
    data.assets.push(policy_asset(1, "mp4", Some(F::FPS_60)));
    let mut second = AssetData::from(policy_asset(2, "mp4", Some(F::FPS_120)));
    second
        .probe
        .streams
        .first_mut()
        .unwrap()
        .video
        .as_mut()
        .unwrap()
        .width = 1280;
    data.assets.push(SourceAsset::try_from(second).unwrap());
    let mut s = Session::new(Project::new(data).unwrap(), 10).unwrap();
    let before = s.project().as_ref().clone();
    // A later failure must roll back first-insert canvas/FPS as well as items.
    assert!(
        s.apply(Command::Batch {
            commands: vec![policy_insert(2), policy_insert(999)]
        })
        .is_err()
    );
    assert_eq!(s.project().as_ref(), &before);
    s.apply(Command::BeginGesture {}).unwrap();
    s.apply(policy_insert(2)).unwrap();
    s.apply(Command::CancelGesture {}).unwrap();
    assert_eq!(s.project().as_ref(), &before);
    s.apply(Command::Batch {
        commands: vec![policy_insert(2), policy_insert(1)],
    })
    .unwrap();
    assert_eq!(s.project().data().canvas.width, 1280);
    assert_eq!(s.project().data().canvas.fps, F::FPS_120);
    let ids = s
        .project()
        .data()
        .scene()
        .timeline
        .iter()
        .flat_map(|track| track.items.iter().map(|item| item.id))
        .collect();
    s.apply(Command::Delete { items: ids }).unwrap();
    s.apply(policy_insert(1)).unwrap();
    assert_eq!(s.project().data().canvas.width, 1280);
    assert_eq!(
        s.project()
            .data()
            .original_canvas_size
            .as_ref()
            .unwrap()
            .width,
        1280
    );
    s.apply(Command::Undo {}).unwrap();
    s.apply(Command::Undo {}).unwrap();
    s.apply(Command::Undo {}).unwrap();
    assert_eq!(s.project().as_ref(), &before);
    // Any existing timeline item prevents canvas initialization.
    s.apply(Command::InsertItem {
        placement: Default::default(),
        item: new_item(0, 10),
    })
    .unwrap();
    s.apply(policy_insert(1)).unwrap();
    assert_eq!(s.project().data().canvas.width, 1920);
    assert_eq!(s.project().data().original_canvas_size, None);
}
#[test]
fn canvas_settings_commit_one_step_and_keep_exact_rationals() {
    let original = project::CanvasSize {
        width: 1080,
        height: 1920,
    };
    let mut data = Project::empty(ProjectId(1), "Canvas".into())
        .unwrap()
        .into_data();
    data.original_canvas_size = Some(original);
    let mut s = Session::new(Project::new(data).unwrap(), 10).unwrap();
    let initial = s.project().clone();
    let mut steps = 0;
    let mut commit = |s: &mut Session, command: Command| {
        s.apply(command).unwrap();
        steps += 1;
        assert_eq!(s.history_len().0, steps);
    };

    commit(
        &mut s,
        Command::SetCanvas {
            width: 1080,
            height: 1920,
            preset: Some(project::AspectPreset::Portrait),
        },
    );
    assert_eq!(
        (
            s.project().data().canvas.width,
            s.project().data().canvas.height
        ),
        (1080, 1920)
    );
    assert_eq!(
        s.project().data().canvas.preset,
        Some(project::AspectPreset::Portrait)
    );

    commit(
        &mut s,
        Command::SetCanvas {
            width: 1000,
            height: 700,
            preset: None,
        },
    );
    assert_eq!(s.project().data().canvas.preset, None);
    assert_eq!(
        s.project().data().canvas.last_custom_size,
        project::CanvasSize {
            width: 1000,
            height: 700
        }
    );

    commit(
        &mut s,
        Command::SetCanvas {
            width: 1081,
            height: 721,
            preset: None,
        },
    );
    assert_eq!(
        (
            s.project().data().canvas.width,
            s.project().data().canvas.height
        ),
        (1082, 722)
    );
    assert_eq!(
        s.project().data().canvas.last_custom_size,
        project::CanvasSize {
            width: 1082,
            height: 722
        }
    );
    commit(&mut s, Command::SetFps { num: 48, den: 2 });
    assert_eq!(s.project().data().canvas.fps, time::FrameRate::FPS_24);
    commit(
        &mut s,
        Command::SetFps {
            num: 24000,
            den: 1001,
        },
    );
    assert_eq!(s.project().data().canvas.fps, time::FrameRate::FPS_23_976);
    assert_eq!(s.project().data().canvas.fps.label(), "23.976");
    commit(
        &mut s,
        Command::SetFps {
            num: 30000,
            den: 1001,
        },
    );
    assert_eq!(s.project().data().canvas.fps, time::FrameRate::FPS_29_97);
    assert_eq!(s.project().data().canvas.fps.label(), "29.97");

    commit(
        &mut s,
        Command::SetBackground {
            color: Color([12, 34, 56, 78]),
        },
    );
    assert_eq!(
        s.project().data().canvas.background,
        Color([12, 34, 56, 78])
    );

    commit(
        &mut s,
        Command::SetCanvas {
            width: 1080,
            height: 1920,
            preset: Some(project::AspectPreset::Original),
        },
    );
    assert_eq!(
        s.project().data().canvas.preset,
        Some(project::AspectPreset::Original)
    );
    assert_eq!(
        (
            s.project().data().canvas.width,
            s.project().data().canvas.height
        ),
        (original.width, original.height)
    );

    let saved = project::encode(s.project(), false).unwrap();
    let loaded = project::decode(&saved).unwrap().project;
    assert_eq!(&loaded, s.project().as_ref());
    for _ in 0..steps {
        s.undo().unwrap();
    }
    assert_eq!(s.project(), &initial);
}
#[test]
fn canvas_custom_sizes_commit_the_nearest_even_size() {
    // (requested, committed): odd axes take the nearest even value (ties up),
    // one never becomes zero, and the largest odd square stays inside the bound.
    for (requested, committed) in [
        ((1081, 721), (1082, 722)),
        ((1, 1), (2, 2)),
        ((1081, 1), (1082, 2)),
        ((8191, 8191), (8192, 8192)),
    ] {
        let mut s =
            Session::new(Project::empty(ProjectId(1), "Canvas".into()).unwrap(), 10).unwrap();
        let initial = s.project().clone();
        s.apply(Command::SetCanvas {
            width: requested.0,
            height: requested.1,
            preset: None,
        })
        .unwrap();
        assert_eq!(
            (
                s.project().data().canvas.width,
                s.project().data().canvas.height
            ),
            committed,
            "{requested:?}"
        );
        assert_eq!(s.project().data().canvas.preset, None);
        assert_eq!(
            s.project().data().canvas.last_custom_size,
            project::CanvasSize {
                width: committed.0,
                height: committed.1
            },
            "{requested:?}"
        );
        assert_eq!(s.history_len().0, 1, "{requested:?}");
        s.undo().unwrap();
        assert_eq!(s.project(), &initial, "{requested:?}");
    }
}
#[test]
fn canvas_settings_reject_invalid_sizes_rates_and_preset_mismatches() {
    let mut data = Project::empty(ProjectId(1), "Canvas".into())
        .unwrap()
        .into_data();
    data.original_canvas_size = Some(project::CanvasSize {
        width: 1920,
        height: 1080,
    });
    let mut s = Session::new(Project::new(data).unwrap(), 10).unwrap();
    let before = s.project().clone();
    for command in [
        Command::SetCanvas {
            width: 0,
            height: 1080,
            preset: None,
        },
        Command::SetCanvas {
            width: 1920,
            height: 0,
            preset: None,
        },
        Command::SetCanvas {
            width: 67_108_865,
            height: 1,
            preset: None,
        },
        Command::SetCanvas {
            width: 67_108_863,
            height: 1,
            preset: None,
        },
        Command::SetCanvas {
            width: 8193,
            height: 8193,
            preset: None,
        },
        Command::SetCanvas {
            width: u32::MAX,
            height: 1,
            preset: None,
        },
        Command::SetCanvas {
            width: 1,
            height: u32::MAX,
            preset: None,
        },
        Command::SetCanvas {
            width: u32::MAX,
            height: u32::MAX,
            preset: None,
        },
        Command::SetCanvas {
            width: 1080,
            height: 1920,
            preset: Some(project::AspectPreset::Landscape),
        },
        Command::SetCanvas {
            width: 1080,
            height: 1920,
            preset: Some(project::AspectPreset::Original),
        },
    ] {
        assert!(s.apply(command).is_err());
        assert_eq!(s.project(), &before);
    }
    assert!(
        crate::wire::run(
            &mut s,
            "project.set_canvas",
            json!({"width":-1,"height":1080})
        )
        .is_err()
    );
    assert_eq!(s.project(), &before);
    for command in [
        Command::SetFps { num: 0, den: 1 },
        Command::SetFps { num: 30, den: 0 },
        Command::SetFps { num: -30, den: 1 },
        Command::SetFps { num: 17, den: 1 },
    ] {
        assert!(s.apply(command).is_err());
        assert_eq!(s.project(), &before);
    }
}
