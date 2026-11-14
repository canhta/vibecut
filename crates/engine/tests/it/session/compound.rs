#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::unreachable
)]
use engine::{Command, CreatedId, EditError, ItemPasteSource, NewItem, Session};
use project::{
    Color, Item, ItemId, ItemParams, OverlayParams, Project, ProjectId, TextParams, Timeline,
    Track, TrackId, TrackKind,
};
use std::sync::Arc;
use time::{SourceRange, Tick};

/// Whether a refusal is the expected one.
type Refusal = fn(&EditError) -> bool;

fn clip(id: u64, content: ItemParams, start: i64, length: i64) -> Item {
    testkit::item::item(ItemId(id), content)
        .source(SourceRange::new(Tick::ZERO, Tick(length)).unwrap())
        .start(Tick(start))
        .build()
}

fn overlay(id: u64, start: i64, length: i64) -> Item {
    let color = Color([40, 80, 160, 255]);
    clip(
        id,
        ItemParams::Overlay(OverlayParams { color }),
        start,
        length,
    )
}

fn track(id: u64, kind: TrackKind, locked: bool, items: Vec<Item>) -> Track {
    Track {
        id: TrackId(id),
        name: format!("Track {id}").into(),
        kind,
        muted: false,
        hidden: false,
        locked,
        items,
    }
}

/// Main track 0: overlays 1 at 0-100 and 2 at 100-200. Text track 3: text 4 at 50-150.
/// Locked video track 5: overlay 6 at 0-100.
fn session() -> Session {
    let mut data = Project::empty(ProjectId(1), "Compound".into())
        .unwrap()
        .into_data();
    let timeline = &mut data.scene_mut().timeline;
    timeline.main.items = vec![overlay(1, 0, 100), overlay(2, 100, 100)];
    timeline.overlays = vec![
        track(5, TrackKind::Video, true, vec![overlay(6, 0, 100)]),
        track(
            3,
            TrackKind::Text,
            false,
            vec![clip(
                4,
                ItemParams::Text(TextParams::new("Xin chào")),
                50,
                100,
            )],
        ),
    ];
    Session::new(Project::new(data).unwrap(), 100).unwrap()
}

fn compound(s: &mut Session, items: &[u64]) -> ItemId {
    let out = s
        .apply(Command::TimelineCompound {
            items: items.iter().copied().map(ItemId).collect(),
        })
        .unwrap();
    out.created
        .iter()
        .find_map(|id| match id {
            CreatedId::Item(item) => Some(*item),
            _ => None,
        })
        .expect("compound clip id")
}

fn find(s: &Session, id: ItemId) -> Option<(TrackId, &Item)> {
    s.project()
        .data()
        .scene()
        .timeline
        .iter()
        .find_map(|track| {
            track
                .items
                .iter()
                .find(|item| item.id == id)
                .map(|item| (track.id, item))
        })
}

fn nested(item: &Item) -> &Timeline {
    let ItemParams::Compound(params) = &item.content else {
        panic!("compound clip expected");
    };
    &params.timeline
}

#[test]
fn compound_moves_members_into_one_clip_on_their_track_as_one_undo_step() {
    let mut s = session();
    let before = s.project().clone();
    let id = compound(&mut s, &[1, 2]);
    let (host, item) = find(&s, id).unwrap();
    assert_eq!(host, TrackId(0));
    assert_eq!(
        (item.start, item.duration().unwrap()),
        (Tick::ZERO, Tick(200))
    );
    assert!(find(&s, ItemId(1)).is_none() && find(&s, ItemId(2)).is_none());
    let members: Vec<_> = nested(item)
        .main
        .items
        .iter()
        .map(|member| (member.id, member.start))
        .collect();
    assert_eq!(members, [(ItemId(1), Tick::ZERO), (ItemId(2), Tick(100))]);
    assert_eq!(s.project().duration(), before.duration());
    let after = s.project().clone();
    s.undo().unwrap();
    assert!(Arc::ptr_eq(&before, s.project()));
    s.redo().unwrap();
    assert!(Arc::ptr_eq(&after, s.project()));
}

#[test]
fn compound_takes_a_new_video_track_when_its_span_is_occupied() {
    let mut s = session();
    // Overlay 2 stays on main inside the 0-150 span, so the clip moves above main.
    let id = compound(&mut s, &[1, 4]);
    let timeline = &s.project().data().scene().timeline;
    let (host, item) = find(&s, id).unwrap();
    assert_ne!(host, TrackId(0));
    assert_eq!(timeline.overlays.last().unwrap().id, host);
    assert_eq!(
        (item.start, item.duration().unwrap()),
        (Tick::ZERO, Tick(150))
    );
    let inner = nested(item);
    assert_eq!(inner.main.items[0].id, ItemId(1));
    assert_eq!(inner.overlays[0].kind, TrackKind::Text);
    assert_eq!(inner.overlays[0].items[0].start, Tick(50));
}

#[test]
fn uncompound_puts_members_back_at_their_times() {
    let mut s = session();
    let id = compound(&mut s, &[1, 2, 4]);
    let before = s.project().clone();
    s.apply(Command::TimelineUncompound { items: vec![id] })
        .unwrap();
    assert!(find(&s, id).is_none());
    for (member, start, kind) in [
        (1, 0, TrackKind::Video),
        (2, 100, TrackKind::Video),
        (4, 50, TrackKind::Text),
    ] {
        let (track, item) = find(&s, ItemId(member)).unwrap();
        assert_eq!(item.start, Tick(start), "member {member}");
        let track = s
            .project()
            .data()
            .scene()
            .timeline
            .iter()
            .find(|candidate| candidate.id == track)
            .unwrap();
        assert_eq!(track.kind, kind);
    }
    assert_eq!(find(&s, ItemId(1)).unwrap().0, TrackId(0));
    s.undo().unwrap();
    assert!(Arc::ptr_eq(&before, s.project()));
}

#[test]
fn compound_commands_refuse_typed_and_leave_the_project_unchanged() {
    let mut s = session();
    let before = s.project().clone();
    let cases: Vec<(Command, Refusal)> = vec![
        (
            Command::TimelineCompound {
                items: vec![ItemId(1), ItemId(6)],
            },
            |e| matches!(e, EditError::Locked(TrackId(5))),
        ),
        (Command::TimelineCompound { items: vec![] }, |e| {
            matches!(e, EditError::Invalid(_) | EditError::Control(_))
        }),
        (
            Command::TimelineCompound {
                items: vec![ItemId(99)],
            },
            |e| matches!(e, EditError::NoItem(ItemId(99))),
        ),
        (
            Command::TimelineUncompound {
                items: vec![ItemId(1)],
            },
            |e| matches!(e, EditError::NotCompound(ItemId(1))),
        ),
    ];
    for (command, check) in cases {
        let error = s.apply(command).unwrap_err();
        assert!(check(&error), "{error:?}");
        assert!(Arc::ptr_eq(&before, s.project()));
    }
    // Compound content enters only through `timeline.compound`.
    let id = compound(&mut s, &[1]);
    let content = find(&s, id).unwrap().1.content.clone();
    let insert = NewItem {
        animations: Default::default(),
        source_asset: None,
        source_range: SourceRange::new(Tick::ZERO, Tick(100)).unwrap(),
        start: Tick(400),
        retime: Default::default(),
        content,
    };
    assert!(matches!(
        s.apply(Command::InsertItem {
            placement: Default::default(),
            item: insert,
        }),
        Err(EditError::Unsupported(_))
    ));
}

#[test]
fn a_compound_clip_trims_splits_and_retimes_like_a_clip() {
    let mut s = session();
    s.apply(Command::SetTrackMagnet { enabled: false }).unwrap();
    let id = compound(&mut s, &[1, 2]);
    s.apply(Command::TrimStart {
        item: id,
        at: Tick(50),
    })
    .unwrap();
    let item = find(&s, id).unwrap().1;
    assert_eq!(
        (item.start, item.source_range),
        (Tick(50), SourceRange::new(Tick(50), Tick(200)).unwrap())
    );
    // The window cannot reach past the nested content.
    let before = s.project().clone();
    assert!(
        s.apply(Command::TrimEnd {
            item: id,
            at: Tick(300),
        })
        .is_err()
    );
    assert!(Arc::ptr_eq(&before, s.project()));
    let out = s
        .apply(Command::SplitAt {
            item: id,
            at: Tick(120),
        })
        .unwrap();
    let right = out
        .created
        .iter()
        .find_map(|created| match created {
            CreatedId::Item(item) => Some(*item),
            _ => None,
        })
        .unwrap();
    let (left, right_item) = (find(&s, id).unwrap().1, find(&s, right).unwrap().1);
    assert_eq!(
        left.source_range,
        SourceRange::new(Tick(50), Tick(120)).unwrap()
    );
    assert_eq!(
        (right_item.start, right_item.source_range),
        (Tick(120), SourceRange::new(Tick(120), Tick(200)).unwrap())
    );
    // Each half owns its copy of the nested timeline under its own ids.
    let ids = |item: &Item| -> Vec<ItemId> {
        nested(item)
            .all_tracks()
            .iter()
            .flat_map(|track| &track.items)
            .map(|member| member.id)
            .collect()
    };
    let (left_ids, right_ids) = (ids(left), ids(right_item));
    assert_eq!(left_ids.len(), right_ids.len());
    assert!(right_ids.iter().all(|member| !left_ids.contains(member)));
    s.apply(Command::SetSpeed {
        items: vec![right],
        speed: time::Speed::from_f64(2.0).unwrap(),
        maintain_pitch: true,
    })
    .unwrap();
    assert_eq!(find(&s, right).unwrap().1.duration().unwrap(), Tick(40));
}

#[test]
fn uncompound_refuses_a_clip_with_its_own_edits() {
    let mut s = session();
    s.apply(Command::SetTrackMagnet { enabled: false }).unwrap();
    let id = compound(&mut s, &[1, 2]);
    s.apply(Command::TrimEnd {
        item: id,
        at: Tick(150),
    })
    .unwrap();
    let before = s.project().clone();
    assert!(matches!(
        s.apply(Command::TimelineUncompound { items: vec![id] }),
        Err(EditError::CompoundHasOwnEdits(refused)) if refused == id
    ));
    assert!(Arc::ptr_eq(&before, s.project()));
    s.undo().unwrap();
    s.apply(Command::TimelineUncompound { items: vec![id] })
        .unwrap();
    assert_eq!(find(&s, ItemId(2)).unwrap().1.start, Tick(100));
}

#[test]
fn a_grouped_item_compounds_with_its_whole_group() {
    let mut s = session();
    s.apply(Command::TimelineGroup {
        items: vec![ItemId(1), ItemId(4)],
    })
    .unwrap();
    let id = compound(&mut s, &[1]);
    assert!(find(&s, ItemId(4)).is_none(), "the text joined its group");
    let members: Vec<&Item> = nested(find(&s, id).unwrap().1)
        .all_tracks()
        .into_iter()
        .flat_map(|track| &track.items)
        .collect();
    assert_eq!(members.len(), 2);
    assert!(members.iter().all(|member| member.group.is_none()));
    assert_eq!(find(&s, id).unwrap().1.group, None);
}

#[test]
fn copies_of_a_compound_clip_nest_fresh_ids() {
    let mut s = session();
    let id = compound(&mut s, &[1, 2]);
    let original: Vec<ItemId> = nested(find(&s, id).unwrap().1)
        .all_tracks()
        .iter()
        .flat_map(|track| &track.items)
        .map(|item| item.id)
        .collect();
    let duplicated = s
        .apply(Command::DuplicateItems { items: vec![id] })
        .unwrap();
    let CreatedId::Item(copy) = *duplicated
        .created
        .iter()
        .find(|created| matches!(created, CreatedId::Item(_)))
        .unwrap()
    else {
        unreachable!()
    };
    s.apply(Command::CopyItems { items: vec![id] }).unwrap();
    let pasted = s
        .apply(Command::PasteItems {
            at: Tick(400),
            source: ItemPasteSource::Stored,
            track: None,
        })
        .unwrap();
    let CreatedId::Item(paste) = *pasted
        .created
        .iter()
        .find(|created| matches!(created, CreatedId::Item(_)))
        .unwrap()
    else {
        unreachable!()
    };
    for copy in [copy, paste] {
        let item = find(&s, copy).unwrap().1;
        let ids: Vec<ItemId> = nested(item)
            .all_tracks()
            .iter()
            .flat_map(|track| &track.items)
            .map(|item| item.id)
            .collect();
        assert_eq!(ids.len(), original.len());
        assert!(ids.iter().all(|id| !original.contains(id)), "{ids:?}");
    }
}

#[test]
fn a_transition_joining_a_member_to_an_outside_clip_refuses() {
    let mut s = crate::session::transition_session(true);
    let before = s.project().clone();
    assert!(matches!(
        s.apply(Command::TimelineCompound {
            items: vec![ItemId(10)],
        }),
        Err(EditError::CompoundSplitsTransition(_))
    ));
    assert!(Arc::ptr_eq(&before, s.project()));
    // With both of its clips, the transition moves inside.
    let id = compound(&mut s, &[10, 11]);
    let timeline = &s.project().data().scene().timeline;
    assert!(timeline.transitions.is_empty());
    assert_eq!(nested(find(&s, id).unwrap().1).transitions.len(), 1);
}
