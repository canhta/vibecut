#![allow(clippy::unwrap_used, reason = "validated test fixtures and assertions")]
use engine::Session;
use media::{AssetData, AssetId, ContentHash, SourceProbe};
use project::{
    AudioParams, Fade, FadeCurve, Item, ItemId, ItemParams, Project, ProjectId, Track, TrackId,
    TrackKind,
};
use serde_json::json;
use time::{SourceRange, Tick};

/// Largest integer the JSON control boundary admits.
const MAX_JSON: i64 = (1 << 53) - 1;

fn session() -> Session {
    let mut data = Project::empty(ProjectId(0), "fades".into())
        .unwrap()
        .into_data();
    data.assets.push(
        media::SourceAsset::try_from(AssetData {
            ephemeral: false,
            id: AssetId(7),
            path: std::env::temp_dir().join("fades.wav"),
            content_hash: ContentHash::from_bytes(&[7]),
            probe: SourceProbe {
                format: "wav".into(),
                duration: Some(Tick(1000)),
                streams: vec![media::SourceStream {
                    index: 0,
                    kind: media::StreamKind::Audio,
                    codec: "pcm_f32le".into(),
                    time_base: media::TimeBase::new(1, 48000).unwrap(),
                    start: None,
                    duration: Some(Tick(1000)),
                    frame_rate: None,
                    frame_count: None,
                    variable_frame_rate: None,
                    video: None,
                    audio: Some(media::AudioInfo {
                        sample_rate: 48000,
                        channels: 2,
                        initial_padding: 0,
                        trailing_padding: 0,
                    }),
                }],
            },
        })
        .unwrap(),
    );
    data.scene_mut().timeline.insert_track(Track {
        id: TrackId(8),
        name: "".into(),
        kind: TrackKind::Audio,
        hidden: false,
        muted: false,
        locked: false,
        items: vec![Item {
            id: ItemId(20),
            source_asset: Some(AssetId(7)),
            source_range: SourceRange::new(Tick::ZERO, Tick(1000)).unwrap(),
            start: Tick::ZERO,
            animations: Default::default(),
            effects: vec![],
            masks: vec![],
            blend_mode: Default::default(),
            content: ItemParams::Audio(AudioParams {
                fades: Default::default(),
                volume: 0.0,
                muted: false,
                source: Default::default(),
            }),
        }],
    });
    Session::new(Project::new(data).unwrap(), 100).unwrap()
}
fn fades(session: &Session, id: u64) -> project::Fades {
    session
        .project()
        .data()
        .scene()
        .timeline
        .iter()
        .flat_map(|t| &t.items)
        .find_map(|i| match &i.content {
            ItemParams::Audio(AudioParams { fades, .. }) if i.id == ItemId(id) => Some(*fades),
            _ => None,
        })
        .unwrap()
}
fn set(session: &mut Session, edge: &str, duration: i64, curve: &str) -> engine::Result<()> {
    session
        .execute(
            "audio.set_fade",
            json!({"item":20,"edge":edge,"duration":duration,"curve":curve}),
        )
        .map(|_| ())
}

#[test]
fn set_fade_is_undoable_redoable_and_persisted() {
    let mut s = session();
    assert_eq!(fades(&s, 20), project::Fades::default());
    for (edge, duration, curve) in [("in", 300, "linear"), ("out", 200, "exponential")] {
        let before = s.project().clone();
        set(&mut s, edge, duration, curve).unwrap();
        let after = s.project().clone();
        assert_ne!(before, after);
        s.undo().unwrap();
        assert_eq!(&before, s.project());
        s.redo().unwrap();
        assert_eq!(&after, s.project());
    }
    let expected = project::Fades {
        fade_in: Fade {
            duration: Tick(300),
            curve: FadeCurve::Linear,
        },
        fade_out: Fade {
            duration: Tick(200),
            curve: FadeCurve::Exponential,
        },
    };
    assert_eq!(fades(&s, 20), expected);
    let encoded = project::encode(s.project(), false).unwrap();
    let loaded = project::decode(&encoded).unwrap().project;
    assert_eq!(&loaded, s.project().as_ref());
}

#[test]
fn set_fade_clamps_to_clip_without_overlapping_the_other_edge() {
    let mut s = session();
    set(&mut s, "out", 700, "equal_power").unwrap();
    set(&mut s, "in", 5000, "equal_power").unwrap();
    assert_eq!(fades(&s, 20).fade_in.duration, Tick(300));
    assert_eq!(fades(&s, 20).fade_out.duration, Tick(700));
    set(&mut s, "out", MAX_JSON, "linear").unwrap();
    assert_eq!(fades(&s, 20).fade_out.duration, Tick(700));
    set(&mut s, "in", 0, "linear").unwrap();
    set(&mut s, "out", MAX_JSON, "linear").unwrap();
    assert_eq!(fades(&s, 20).fade_out.duration, Tick(1000));
}

#[test]
fn set_fade_refuses_hostile_input_atomically() {
    let mut s = session();
    let before = s.project().clone();
    let history = s.history_len();
    assert!(set(&mut s, "in", -1, "linear").is_err());
    assert!(set(&mut s, "in", i64::MIN, "linear").is_err());
    assert!(set(&mut s, "in", 10, "cubic").is_err());
    assert!(set(&mut s, "middle", 10, "linear").is_err());
    assert!(
        s.execute(
            "audio.set_fade",
            json!({"item":999,"edge":"in","duration":1,"curve":"linear"})
        )
        .is_err()
    );
    assert!(
        s.execute(
            "audio.set_fade",
            json!({"item":20,"edge":"in","duration":1,"curve":"linear","extra":1})
        )
        .is_err()
    );
    assert!(
        s.execute("audio.set_fade", json!({"item":20,"edge":"in"}))
            .is_err()
    );
    assert_eq!(s.project(), &before);
    assert_eq!(s.history_len(), history);
    s.execute("track.set_locked", json!({"track":8,"locked":true}))
        .unwrap();
    let locked = s.project().clone();
    assert!(set(&mut s, "in", 10, "linear").is_err());
    assert_eq!(s.project(), &locked);
}

#[test]
fn split_divides_fades_at_the_cut_and_trim_keeps_them() {
    let mut s = session();
    set(&mut s, "in", 300, "linear").unwrap();
    set(&mut s, "out", 400, "exponential").unwrap();
    s.execute("split_at", json!({"item":20,"at":500})).unwrap();
    let left = fades(&s, 20);
    assert_eq!(left.fade_in.duration, Tick(300));
    assert_eq!(left.fade_in.curve, FadeCurve::Linear);
    assert_eq!(left.fade_out, Fade::default());
    let right_id = s
        .project()
        .data()
        .scene()
        .timeline
        .iter()
        .flat_map(|t| &t.items)
        .map(|i| i.id.0)
        .find(|id| *id != 20)
        .unwrap();
    let right = fades(&s, right_id);
    assert_eq!(right.fade_in, Fade::default());
    assert_eq!(right.fade_out.duration, Tick(400));
    assert_eq!(right.fade_out.curve, FadeCurve::Exponential);
    // Undo restores both fades on the original clip; no stray piece remains.
    s.undo().unwrap();
    assert_eq!(fades(&s, 20).fade_out.duration, Tick(400));
    s.redo().unwrap();
    // Trimming keeps the stored durations; evaluation clamps them to the clip.
    s.execute("trim_end", json!({"item":20,"at":100})).unwrap();
    assert_eq!(fades(&s, 20).fade_in.duration, Tick(300));
}

#[test]
fn registry_lists_set_fade_as_an_edit() {
    let spec = engine::registry()
        .iter()
        .find(|spec| spec.id == "audio.set_fade")
        .unwrap();
    assert_eq!(spec.effect, engine::Effect::Edit);
}
