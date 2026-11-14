#![allow(clippy::unwrap_used, clippy::panic)]
//! Mask scopes: the Adjust mask of a clip and the masks of an effect or adjustment layer all
//! run through the ordinary mask commands.
use engine::{Command, ItemPasteSource, Session};
use project::*;
use time::{FrameRate, SourceRange, Tick};

const VIDEO: ItemId = ItemId(2);
const LAYER: ItemId = ItemId(3);

fn session() -> Session {
    let mut data = Project::empty(ProjectId(1), "Scopes".into())
        .unwrap()
        .into_data();
    data.assets = vec![crate::support::session::pause_video_asset()];
    let range = SourceRange::new(Tick::ZERO, FrameRate::FPS_30.tick_of(30).unwrap()).unwrap();
    data.scene_mut().timeline.insert_track(Track {
        id: TrackId(1),
        name: "V".into(),
        kind: TrackKind::Video,
        muted: false,
        hidden: false,
        locked: false,
        items: vec![
            testkit::item::video(VIDEO)
                .asset(media::AssetId(2))
                .source(range)
                .build(),
        ],
    });
    data.scene_mut().timeline.insert_track(Track {
        id: TrackId(4),
        name: "Adjustment".into(),
        kind: TrackKind::Effect,
        muted: false,
        hidden: false,
        locked: false,
        items: vec![
            testkit::item::item(LAYER, ItemParams::Effect(EffectParams::default()))
                .source(range)
                .build(),
        ],
    });
    Session::new(Project::new(data).unwrap(), 20).unwrap()
}

fn mask(id: &str, scope: MaskScope) -> Mask {
    let mut mask = Mask::new(id, MaskShape::Ellipse);
    mask.scope = scope;
    mask
}

fn find(s: &Session, id: ItemId) -> Item {
    s.project()
        .data()
        .scene()
        .timeline
        .iter()
        .flat_map(|track| track.items.iter())
        .find(|item| item.id == id)
        .unwrap()
        .clone()
}

fn key(s: &mut Session, item: ItemId, mask: &Mask) -> AnimationPath {
    let path = mask.path("feather").unwrap();
    s.apply(Command::SetAnimatedParam {
        item,
        path: path.clone(),
        param: Param::new(ParamValue::Scalar(5.)).unwrap(),
    })
    .unwrap();
    path
}

#[test]
fn adjust_masks_use_the_mask_commands_and_undo_beside_the_clip_masks() {
    let mut s = session();
    let start = s.project().clone();
    s.apply(Command::AddMask {
        item: VIDEO,
        mask: mask("adjust1", MaskScope::Adjust),
    })
    .unwrap();
    let added = s.project().clone();
    s.apply(Command::Undo {}).unwrap();
    assert_eq!(s.project(), &start);
    s.apply(Command::Redo {}).unwrap();
    assert_eq!(s.project(), &added);

    // The clip's own mask sits beside it; each keeps its scope.
    s.apply(Command::AddMask {
        item: VIDEO,
        mask: mask("mask1", MaskScope::Clip),
    })
    .unwrap();
    let scopes: Vec<_> = find(&s, VIDEO)
        .masks
        .iter()
        .map(|m| (m.id.clone(), m.scope))
        .collect();
    assert_eq!(
        scopes,
        [
            ("adjust1".to_owned(), MaskScope::Adjust),
            ("mask1".to_owned(), MaskScope::Clip)
        ]
    );

    // An update replaces the shape and fields and keeps the scope it carries.
    let mut changed = find(&s, VIDEO).masks[0].clone();
    changed.shape = MaskShape::Star;
    changed.feather = 8.;
    s.apply(Command::UpdateMask {
        item: VIDEO,
        mask: changed.clone(),
    })
    .unwrap();
    assert_eq!(find(&s, VIDEO).masks[0], changed);
    assert_eq!(find(&s, VIDEO).masks[0].scope, MaskScope::Adjust);

    // Its parameters key like any mask's, and removing the mask drops the keys; undo restores
    // both.
    let path = key(&mut s, VIDEO, &changed);
    let keyed = s.project().clone();
    s.apply(Command::RemoveMask {
        item: VIDEO,
        mask: "adjust1".into(),
    })
    .unwrap();
    assert!(find(&s, VIDEO).animations.get(&path).is_none());
    assert_eq!(find(&s, VIDEO).masks.len(), 1);
    s.apply(Command::Undo {}).unwrap();
    assert_eq!(s.project(), &keyed);

    // The saved project keeps the scope.
    let saved = project::encode(s.project(), false).unwrap();
    assert_eq!(
        &project::decode(&saved).unwrap().project,
        s.project().as_ref()
    );
}

#[test]
fn an_adjust_mask_is_refused_on_an_effect_layer_without_changing_anything() {
    let mut s = session();
    let before = s.project().clone();
    let history = s.history_len();
    assert!(
        s.apply(Command::AddMask {
            item: LAYER,
            mask: mask("adjust1", MaskScope::Adjust),
        })
        .is_err()
    );
    assert_eq!(s.project(), &before);
    assert_eq!(s.history_len(), history);

    // Turning an existing layer mask into an Adjust mask is refused the same way.
    s.apply(Command::AddMask {
        item: LAYER,
        mask: mask("mask1", MaskScope::Clip),
    })
    .unwrap();
    let before = s.project().clone();
    let history = s.history_len();
    let mut turned = find(&s, LAYER).masks[0].clone();
    turned.scope = MaskScope::Adjust;
    assert!(
        s.apply(Command::UpdateMask {
            item: LAYER,
            mask: turned,
        })
        .is_err()
    );
    assert_eq!(s.project(), &before);
    assert_eq!(s.history_len(), history);
}

#[test]
fn effect_and_adjustment_layers_take_clip_masks_through_the_same_commands() {
    let mut s = session();
    let start = s.project().clone();
    s.apply(Command::AddMask {
        item: LAYER,
        mask: mask("mask1", MaskScope::Clip),
    })
    .unwrap();
    let added = s.project().clone();
    assert_eq!(find(&s, LAYER).masks.len(), 1);
    s.apply(Command::Undo {}).unwrap();
    assert_eq!(s.project(), &start);
    s.apply(Command::Redo {}).unwrap();
    assert_eq!(s.project(), &added);

    let mut changed = find(&s, LAYER).masks[0].clone();
    changed.shape = MaskShape::Heart;
    changed.inverted = true;
    changed.feather = 12.;
    s.apply(Command::UpdateMask {
        item: LAYER,
        mask: changed.clone(),
    })
    .unwrap();
    assert_eq!(find(&s, LAYER).masks[0], changed);
    let path = key(&mut s, LAYER, &changed);
    s.apply(Command::RemoveMask {
        item: LAYER,
        mask: "mask1".into(),
    })
    .unwrap();
    assert!(find(&s, LAYER).masks.is_empty());
    assert!(find(&s, LAYER).animations.get(&path).is_none());
    s.apply(Command::Undo {}).unwrap();
    assert_eq!(find(&s, LAYER).masks, [changed]);
    assert!(find(&s, LAYER).animations.get(&path).is_some());
}

#[test]
fn pasting_attributes_leaves_adjust_masks_and_their_keys_off_a_layer() {
    let mut s = session();
    let adjust = mask("adjust1", MaskScope::Adjust);
    let clip = mask("mask1", MaskScope::Clip);
    for mask in [&adjust, &clip] {
        s.apply(Command::AddMask {
            item: VIDEO,
            mask: mask.clone(),
        })
        .unwrap();
    }
    let adjust_path = key(&mut s, VIDEO, &adjust);
    let clip_path = key(&mut s, VIDEO, &clip);
    s.apply(Command::CopyItems { items: vec![VIDEO] }).unwrap();
    s.apply(Command::PasteAttributes {
        items: vec![LAYER],
        source: ItemPasteSource::Stored,
    })
    .unwrap();
    let layer = find(&s, LAYER);
    assert_eq!(
        layer
            .masks
            .iter()
            .map(|m| m.id.as_str())
            .collect::<Vec<_>>(),
        ["mask1"],
        "the layer takes the clip mask and not the grade mask"
    );
    assert!(layer.animations.get(&adjust_path).is_none());
    assert!(layer.animations.get(&clip_path).is_some());
}
