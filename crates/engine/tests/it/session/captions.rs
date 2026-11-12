#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::unreachable
)]
use crate::support::session::{get, project, range, session};
use engine::{Command, CreatedId, Session};
use media::{AssetData, AssetId, ContentHash, SourceAsset, SourceProbe};
use project::{Color, Item, ItemId, ItemParams, Project, Track, TrackId, TrackKind};
use proptest::prelude::*;
use serde_json::json;
use std::collections::BTreeMap;
use std::sync::Arc;
use time::Tick;

fn preset_style(boxed: bool) -> captions::CueStyle {
    captions::CueStyle {
        font_family: "Inter".into(),
        font_size_ratio: 48.0 / 1080.0,
        primary_color: [255; 4],
        outline_color: [0, 0, 0, 255],
        background_color: if boxed { [0, 0, 0, 255] } else { [0; 4] },
        border_style: if boxed { 3 } else { 1 },
        alignment: captions::CueAlignment::from_numpad(2).unwrap(),
        margin_left_ratio: 0.05,
        margin_right_ratio: 0.05,
        margin_vertical_ratio: 0.05,
        bold: true,
        italic: false,
        underline: false,
        strikeout: false,
    }
}
fn full_style(boxed: bool) -> captions::CaptionStyle {
    render::style_from_cue(&preset_style(boxed))
}
fn apply_style(style: engine::CaptionStyleApply) -> Command {
    Command::ApplyCaptionStyle {
        item: ItemId(1),
        style,
    }
}

#[test]
fn caption_bank_selection_overrides_noops_and_ids_round_trip() {
    use engine::CaptionStyleApply;
    use project::{CaptionPresetId, SavedCaptionPreset};
    let mut data = style_source_project().into_data();
    data.caption_presets = vec![SavedCaptionPreset {
        id: CaptionPresetId(100),
        name: "Box".into(),
        style: full_style(true),
    }];
    let mut other = data.scenes.main().clone();
    other.id = project::SceneId(50);
    other.is_main = false;
    let mut next = 60;
    for track in other.timeline.iter_mut() {
        track.id = TrackId(next);
        next += 1;
        track.locked = true;
        for item in &mut track.items {
            item.id = ItemId(next);
            next += 1;
        }
    }
    let mut scenes: Vec<_> = data.scenes.iter().cloned().collect();
    scenes.push(other);
    data.scenes = scenes.try_into().unwrap();
    let mut s = Session::new(Project::new(data).unwrap(), 30).unwrap();
    let original = s.project().clone();
    s.execute(
        "captions.apply_style",
        json!({"item":1,"style":{"kind":"saved","preset":100}}),
    )
    .unwrap();
    let mut expected = original.data().clone();
    let ItemParams::Caption(params) =
        &mut expected.scene_mut().timeline.overlays[0].items[0].content
    else {
        panic!("caption")
    };
    // Selecting a saved look sets only the live reference: no overrides, same cues.
    params.preset = Some(CaptionPresetId(100));
    assert!(params.overrides.is_empty());
    let unchanged = params.cues.clone();
    assert_eq!(s.project().data(), &expected);
    assert_eq!(saved_cues(&s), &unchanged, "selection keeps cue values");
    s.undo().unwrap();
    assert_eq!(s.project(), &original);
    s.redo().unwrap();
    let styled = s.project().clone();
    // A sparse patch re-applies over the same live reference.
    let overrides = captions::CaptionOverrides {
        size: Some(72.0),
        color: Some([250, 240, 20, 200]),
        outline: Some(None),
        ..Default::default()
    };
    assert!(
        s.apply(Command::ApplyCaptionStyle {
            item: ItemId(1),
            style: CaptionStyleApply::Saved {
                preset: CaptionPresetId(100),
                overrides: overrides.clone(),
            },
        })
        .unwrap()
        .changed
    );
    let patched = s.project().clone();
    let ItemParams::Caption(p) = &get(&s, 1).content else {
        panic!("caption")
    };
    assert_eq!(p.preset, Some(CaptionPresetId(100)));
    assert_eq!(p.overrides, overrides);
    s.undo().unwrap();
    assert_eq!(s.project(), &styled);
    s.redo().unwrap();
    assert_eq!(s.project(), &patched);
    // Save captures the effective full style, not the untouched cue table.
    let out = s
        .execute(
            "captions.save_preset",
            json!({"item":1,"name":"Tie\u{302}\u{301}ng Vie\u{323}\u{302}t"}),
        )
        .unwrap();
    assert_eq!(
        out.created,
        vec![CreatedId::CaptionPreset(CaptionPresetId(101))]
    );
    let mut effective = full_style(true);
    overrides.apply(&mut effective);
    expected = patched.data().clone();
    expected.caption_presets.push(SavedCaptionPreset {
        id: CaptionPresetId(101),
        name: "Tiếng Việt".into(),
        style: effective.clone(),
    });
    assert_eq!(s.project().data(), &expected, "save changes only bank");
    let before_duplicate = s.project().clone();
    let history = s.history_len();
    let revision = s.revision();
    assert!(matches!(
        s.execute(
            "captions.save_preset",
            json!({"item":1,"name":"Tie\u{302}\u{301}ng Vie\u{323}\u{302}t"})
        ),
        Err(engine::EditError::Invalid(_))
    ));
    assert_eq!(s.project(), &before_duplicate);
    assert_eq!(s.history_len(), history);
    assert_eq!(s.revision(), revision);
    s.apply(Command::SaveCaptionPreset {
        item: ItemId(1),
        name: "Same values".into(),
    })
    .unwrap();
    let saved = s.project().clone();
    s.execute(
        "captions.rename_preset",
        json!({"preset":101,"name":"Renamed"}),
    )
    .unwrap();
    s.undo().unwrap();
    s.take_change();
    let revision = s.revision();
    let history = s.history_len();
    assert!(
        !s.execute(
            "captions.rename_preset",
            json!({"preset":101,"name":"Tiếng Việt"})
        )
        .unwrap()
        .changed
    );
    assert!(
        !s.execute(
            "captions.apply_style",
            json!({"item":1,"style":{"kind":"saved","preset":100,"overrides":{"size":72.0,"color":[250,240,20,200],"outline":null}}})
        )
        .unwrap()
        .changed
    );
    assert!(Arc::ptr_eq(&saved, s.project()));
    assert_eq!(s.revision(), revision);
    assert_eq!(s.history_len(), history);
    assert!(s.take_change().is_none());
    let before_bad = s.project().clone();
    let history = s.history_len();
    for style in [
        CaptionStyleApply::Saved {
            preset: CaptionPresetId(999),
            overrides: captions::CaptionOverrides::default(),
        },
        CaptionStyleApply::Saved {
            preset: CaptionPresetId(100),
            overrides: captions::CaptionOverrides {
                size: Some(1.0),
                ..Default::default()
            },
        },
    ] {
        assert!(matches!(
            s.apply(Command::ApplyCaptionStyle {
                item: ItemId(1),
                style,
            }),
            Err(engine::EditError::Invalid(_))
        ));
        assert_eq!(s.project(), &before_bad);
        assert_eq!(s.history_len(), history);
    }
    // A live selection is never silently detached or deleted.
    assert!(matches!(
        s.execute("captions.delete_preset", json!({"preset":100})),
        Err(engine::EditError::Invalid(_))
    ));
    assert_eq!(s.project(), &saved);
    s.execute("captions.delete_preset", json!({"preset":101}))
        .unwrap();
    assert_eq!(
        s.project()
            .data()
            .caption_presets
            .iter()
            .map(|p| p.id.0)
            .collect::<Vec<_>>(),
        // Saving appends and deleting keeps the remaining order.
        [100, 102]
    );
    let bytes = project::encode(s.project(), false).unwrap();
    assert_eq!(
        &project::decode(&bytes).unwrap().project,
        s.project().as_ref()
    );
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("bank.vibecut");
    project::WriterLease::acquire(&path)
        .unwrap()
        .save(s.project())
        .unwrap();
    assert_eq!(&project::load(&path).unwrap().project, s.project().as_ref());
    for group in [false, true] {
        let before = s.project().clone();
        let command = Command::SaveCaptionPreset {
            item: ItemId(1),
            name: "Cancelled".into(),
        };
        let id = if group {
            let mut id = None;
            assert!(
                s.transaction(|s| {
                    id = Some(s.apply(command)?.created[0]);
                    Err::<(), _>(engine::EditError::Invalid("rollback"))
                })
                .is_err()
            );
            id.unwrap()
        } else {
            s.begin_gesture().unwrap();
            let id = s.apply(command).unwrap().created[0];
            s.cancel_gesture().unwrap();
            id
        };
        assert_eq!(s.project(), &before);
        let next = s
            .apply(Command::SaveCaptionPreset {
                item: ItemId(1),
                name: "After cancel".into(),
            })
            .unwrap()
            .created[0];
        let (CreatedId::CaptionPreset(id), CreatedId::CaptionPreset(next)) = (id, next) else {
            panic!("preset IDs")
        };
        assert!(next > id, "accepted then cancelled identities stay retired");
        s.undo().unwrap();
    }
    // Manage is project metadata, even with no caption target and every track locked.
    s.apply(Command::Delete {
        items: vec![ItemId(1), ItemId(2)],
    })
    .unwrap();
    s.apply(Command::SetTrackLocked {
        track: TrackId(1),
        locked: true,
    })
    .unwrap();
    s.execute(
        "captions.rename_preset",
        json!({"preset":102,"name":"Without target"}),
    )
    .unwrap();
    s.execute("captions.delete_preset", json!({"preset":102}))
        .unwrap();
}

#[test]
fn caption_bank_refusals_roll_back_history_sources_and_global_allocator() {
    use engine::EditError;
    use project::{CaptionPresetId, SavedCaptionPreset};
    for case in [
        "mixed",
        "unstyled",
        "empty",
        "missing",
        "kind",
        "locked",
        "scene",
        "name",
        "duplicate",
        "cap",
        "exhausted",
        "batch",
    ] {
        let mut data = style_source_project().into_data();
        if case != "mixed" {
            let ItemParams::Caption(p) =
                &mut data.scene_mut().timeline.overlays[0].items[0].content
            else {
                panic!("caption")
            };
            for cue in &mut p.cues.cues {
                cue.style = Some(captions::CueStyleId(0));
            }
            if case == "unstyled" {
                p.cues.cues[0].style = None;
            }
            if case == "empty" {
                p.cues.cues.clear();
            }
        }
        if case == "kind" {
            let item = &mut data.scene_mut().timeline.overlays[0].items[0];
            item.content = ItemParams::Text(project::TextParams::new("Title"));
            item.source_asset = None;
        }
        if case == "locked" {
            data.scene_mut().timeline.overlays[0].locked = true;
        }
        let max = if case == "exhausted" { u64::MAX } else { 100 };
        data.caption_presets.push(SavedCaptionPreset {
            id: CaptionPresetId(max),
            name: "Taken".into(),
            style: full_style(false),
        });
        if case == "cap" {
            for id in 101..164 {
                data.caption_presets.push(SavedCaptionPreset {
                    id: CaptionPresetId(id),
                    name: format!("Look {id}").into(),
                    style: full_style(true),
                });
            }
        }
        let mut s = Session::new(Project::new(data).unwrap(), 10).unwrap();
        if case == "scene" {
            let created = s
                .apply(Command::CreateScene {
                    name: "Other".into(),
                })
                .unwrap();
            let CreatedId::Scene(scene) = created.created[0] else {
                panic!("scene")
            };
            s.apply(Command::SwitchScene { scene }).unwrap();
        }
        s.take_change();
        let before = s.project().clone();
        let revision = s.revision();
        let history = s.history_len();
        let save = Command::SaveCaptionPreset {
            item: ItemId(if case == "missing" { 999 } else { 1 }),
            name: (match case {
                "name" => " Bad",
                "duplicate" => "Taken",
                _ => "New",
            })
            .into(),
        };
        let command = if case == "batch" {
            Command::Batch {
                commands: vec![
                    save,
                    Command::Delete {
                        items: vec![ItemId(999)],
                    },
                ],
            }
        } else {
            save
        };
        let error = s.apply(command).unwrap_err();
        assert!(
            match case {
                "locked" => matches!(error, EditError::Locked(TrackId(1))),
                "scene" => matches!(error, EditError::SceneBoundary),
                "missing" | "batch" => matches!(error, EditError::NoItem(ItemId(999))),
                "cap" => matches!(
                    error,
                    EditError::Control(engine::control::ControlError::Limit)
                ),
                "exhausted" =>
                    matches!(error, EditError::Session(engine::SessionError::IdExhausted)),
                _ => matches!(error, EditError::Invalid(_)),
            },
            "{case}: {error}"
        );
        assert_eq!(s.project(), &before);
        assert_eq!(s.revision(), revision);
        assert_eq!(s.history_len(), history);
        assert!(s.take_change().is_none());
        if case == "batch" {
            assert_eq!(
                s.apply(Command::SaveCaptionPreset {
                    item: ItemId(1),
                    name: "After".into()
                })
                .unwrap()
                .created,
                vec![CreatedId::CaptionPreset(CaptionPresetId(101))]
            );
        }
        for (id, params) in [
            ("captions.save_preset", json!({"item":1})),
            (
                "captions.save_preset",
                json!({"item":1,"name":"A","extra":0}),
            ),
            ("captions.rename_preset", json!({"preset":100})),
            ("captions.delete_preset", json!({"preset":100,"extra":0})),
            (
                "captions.delete_preset",
                json!({"preset":9007199254740992_u64}),
            ),
            (
                "captions.apply_style",
                json!({"item":1,"style":{"kind":"saved","preset":100,"extra":0}}),
            ),
        ] {
            assert!(s.execute(id, params).is_err());
        }
    }
}
fn saved_cues(s: &Session) -> &captions::CueTrack {
    let ItemParams::Caption(params) = &get(s, 1).content else {
        panic!("caption")
    };
    &params.cues
}

#[test]
fn caption_bank_reads_hidden_trim_sources_and_applies_copies_before_split() {
    use project::{CaptionPresetId, SavedCaptionPreset};
    let mut data = style_source_project().into_data();
    data.caption_presets.push(SavedCaptionPreset {
        id: CaptionPresetId(100),
        name: "Box".into(),
        style: full_style(true),
    });
    let mut s = Session::new(Project::new(data).unwrap(), 20).unwrap();
    let original = s.project().clone();
    s.begin_gesture().unwrap();
    s.apply(caption_trim(false, 1050)).unwrap(); // Only the first of two different source styles is visible.
    assert!(
        s.apply(Command::SaveCaptionPreset {
            item: ItemId(1),
            name: "Hidden mixed".into()
        })
        .is_err()
    );
    s.apply(apply_style(engine::CaptionStyleApply::Saved {
        preset: CaptionPresetId(100),
        overrides: captions::CaptionOverrides::default(),
    }))
    .unwrap();
    s.apply(caption_trim(false, 1100)).unwrap();
    let ItemParams::Caption(selected) = &get(&s, 1).content else {
        panic!("caption")
    };
    assert_eq!(selected.preset, Some(CaptionPresetId(100)));
    assert!(selected.overrides.is_empty());
    let ItemParams::Caption(old) = &original.data().scene().timeline.overlays[0].items[0].content
    else {
        panic!("caption")
    };
    assert_eq!(saved_cues(&s).styles, old.cues.styles);
    for (new, old) in saved_cues(&s).cues.iter().zip(&old.cues.cues) {
        assert_eq!(new.texts, old.texts);
        assert_eq!(new.word_ranges, old.word_ranges);
        assert_eq!(new.range, old.range);
        assert_eq!(new.id, old.id);
    }
    s.apply(Command::SaveCaptionPreset {
        item: ItemId(1),
        name: "Complete".into(),
    })
    .unwrap();
    s.commit_gesture().unwrap();
    assert_eq!(s.history_len(), (1, 0));
    let complete = s.project().clone();
    s.undo().unwrap();
    assert_eq!(s.project(), &original);
    s.redo().unwrap();
    assert_eq!(s.project(), &complete);
    let split = s
        .apply(Command::SplitAt {
            item: ItemId(1),
            at: Tick(1050),
        })
        .unwrap();
    let CreatedId::Item(right) = split.created[0] else {
        panic!("split")
    };
    let ItemParams::Caption(params) = &get(&s, right.0).content else {
        panic!("caption")
    };
    assert_eq!(params.preset, Some(CaptionPresetId(100)));
    assert!(params.overrides.is_empty());
    assert_eq!(params.cues.styles, old.cues.styles);
    s.apply(Command::SetTrackLocked {
        track: TrackId(1),
        locked: true,
    })
    .unwrap();
    for command in [
        apply_style(engine::CaptionStyleApply::Saved {
            preset: CaptionPresetId(999),
            overrides: captions::CaptionOverrides::default(),
        }),
        apply_style(engine::CaptionStyleApply::Saved {
            preset: CaptionPresetId(100),
            overrides: captions::CaptionOverrides::default(),
        }),
        Command::SaveCaptionPreset {
            item: ItemId(1),
            name: "Complete".into(),
        },
    ] {
        assert!(matches!(
            s.apply(command),
            Err(engine::EditError::Locked(TrackId(1)))
        ));
    }
    let items = s.project().data().scene().clone();
    s.apply(Command::RenameCaptionPreset {
        preset: CaptionPresetId(100),
        name: "Rename only".into(),
    })
    .unwrap();
    assert!(matches!(
        s.apply(Command::DeleteCaptionPreset {
            preset: CaptionPresetId(100),
        }),
        Err(engine::EditError::Invalid(_))
    ));
    assert_eq!(s.project().data().scene(), &items);
    s.apply(Command::SetTrackLocked {
        track: TrackId(1),
        locked: false,
    })
    .unwrap();
    s.apply(Command::Delete {
        items: vec![ItemId(1), right],
    })
    .unwrap();
    s.apply(Command::DeleteCaptionPreset {
        preset: CaptionPresetId(100),
    })
    .unwrap();
}

#[test]
fn caption_bank_mixed_ass_import_refuses_save_and_undo_restores_exact_styles() {
    let mut s = session();
    let mut command = ass_import();
    let Command::ImportCaptions { styles, cues, .. } = &mut command else {
        panic!("import")
    };
    let mut alternate = ass_style();
    alternate.primary_color = [10, 90, 220, 190];
    styles.push(alternate);
    cues[1].style = captions::CueStyleId(1);
    let out = s.apply(command).unwrap();
    let item = out
        .created
        .iter()
        .find_map(|id| {
            if let CreatedId::Item(id) = id {
                Some(*id)
            } else {
                None
            }
        })
        .unwrap();
    let mixed = s.project().clone();
    assert!(matches!(
        s.apply(Command::SaveCaptionPreset {
            item,
            name: "Mixed".into()
        }),
        Err(engine::EditError::Invalid(_))
    ));
    assert_eq!(s.project(), &mixed);
    s.apply(Command::ApplyCaptionStyle {
        item,
        style: engine::CaptionStyleApply::Custom {
            style: preset_style(true),
        },
    })
    .unwrap();
    s.apply(Command::SaveCaptionPreset {
        item,
        name: "Uniform".into(),
    })
    .unwrap();
    assert_eq!(
        s.project().data().caption_presets[0].style,
        full_style(true)
    );
    s.undo().unwrap();
    s.undo().unwrap();
    assert_eq!(s.project(), &mixed);
}

fn style_source_project() -> Project {
    let mut data = caption_project().into_data();
    let item = &mut data.scene_mut().timeline.overlays[0].items[0];
    item.animations
        .set(
            project::AnimationPath::new("opacity").unwrap(),
            project::Param::new(project::ParamValue::Scalar(0.8)).unwrap(),
        )
        .unwrap();
    let ItemParams::Caption(p) = &mut item.content else {
        panic!("caption")
    };
    let mut other = ass_style();
    other.primary_color = [20, 60, 240, 255];
    p.cues.styles = vec![ass_style(), other];
    p.cues.cues[0].range = range(110, 150);
    p.cues.cues[0].word_ranges = vec![range(110, 130), range(130, 150)];
    p.cues.cues[0].style = Some(captions::CueStyleId(0));
    let mut second = p.cues.cues[0].clone();
    second.id = "source/1".into();
    second.range = range(150, 190);
    second.word_ranges = vec![range(150, 170), range(170, 190)];
    second.style = Some(captions::CueStyleId(1));
    p.cues.cues.push(second);
    let mut other = item.clone();
    other.id = ItemId(2);
    other.start = Tick(2000);
    data.scene_mut().timeline.overlays[0].items.push(other);
    Project::new(data).unwrap()
}

#[test]
fn caption_style_values_noop_history_and_saved_file_use_real_commands() {
    use engine::{CaptionPreset, CaptionStyleApply};
    for boxed in [false, true] {
        let mut s = Session::new(style_source_project(), 20).unwrap();
        s.set_selection([ItemId(1)].into()).unwrap();
        let before = s.project().clone();
        let files = |cues: &captions::CueTrack| {
            [captions::Format::Srt, captions::Format::WebVtt].map(|format| {
                captions::export(cues, format, &[captions::LanguageTag::parse("vi").unwrap()])
                    .unwrap()
                    .text
            })
        };
        let text_file = files(saved_cues(&s));
        let expected = preset_style(boxed);
        let outcome = if boxed {
            s.execute(
                "captions.apply_style",
                json!({"item":1,"style":{"kind":"preset","preset":"box"}}),
            )
            .unwrap()
        } else {
            s.apply(apply_style(CaptionStyleApply::Preset {
                preset: CaptionPreset::Outline,
            }))
            .unwrap()
        };
        assert!(outcome.changed && outcome.created.is_empty());
        assert_eq!(s.history_len(), (1, 0));
        let mut original = before.data().clone();
        let ItemParams::Caption(params) =
            &mut original.scene_mut().timeline.overlays[0].items[0].content
        else {
            panic!("caption")
        };
        params.cues.styles = vec![expected.clone()];
        for cue in &mut params.cues.cues {
            cue.style = Some(captions::CueStyleId(0));
        }
        assert_eq!(
            s.project().data(),
            &original,
            "only style values and refs change"
        );
        assert_eq!(s.selection(), &[ItemId(1)].into());
        assert_eq!(files(saved_cues(&s)), text_file);
        let styled = s.project().clone();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("styled.vibecut");
        project::WriterLease::acquire(&path)
            .unwrap()
            .save(s.project())
            .unwrap();
        assert_eq!(&project::load(&path).unwrap().project, styled.as_ref());
        let mut bytes: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        for version in [project::SCHEMA_VERSION - 1, project::SCHEMA_VERSION + 1] {
            bytes["schema_version"] = json!(version);
            assert!(matches!(
                project::decode(&serde_json::to_vec(&bytes).unwrap()),
                Err(project::Error::TooOld { .. } | project::Error::TooNew { .. })
            ));
        }
        s.undo().unwrap();
        assert_eq!(s.project(), &before);
        s.redo().unwrap();
        assert_eq!(s.project(), &styled);
        let mut custom = expected.clone();
        custom.primary_color = [30, 240, 80, 255];
        s.execute(
            "captions.apply_style",
            json!({"item":1,"style":{"kind":"custom","style":custom}}),
        )
        .unwrap();
        assert_eq!(saved_cues(&s).styles, vec![custom]);
        s.undo().unwrap();
        s.take_change();
        let revision = s.revision();
        assert!(
            !s.apply(apply_style(CaptionStyleApply::Custom {
                style: expected.clone()
            }))
            .unwrap()
            .changed
        );
        assert_eq!(s.revision(), revision);
        assert_eq!(s.history_len(), (1, 1));
        assert!(s.take_change().is_none());
        // Equal referenced entries and an unused different entry must not compact on a no-op.
        let mut d = s.project().data().clone();
        let ItemParams::Caption(p) = &mut d.scene_mut().timeline.overlays[0].items[0].content
        else {
            panic!("caption")
        };
        p.cues.styles = vec![expected.clone(), expected.clone(), ass_style()];
        p.cues.cues[0].style = Some(captions::CueStyleId(1));
        let mut s = Session::new(Project::new(d).unwrap(), 10).unwrap();
        let original = s.project().clone();
        assert!(
            !s.apply(apply_style(CaptionStyleApply::Custom { style: expected }))
                .unwrap()
                .changed
        );
        assert!(Arc::ptr_eq(&original, s.project()));
        assert_eq!(s.history_len(), (0, 0));
    }
}

#[test]
fn caption_style_refusals_and_batch_leave_the_entire_session_unchanged() {
    use engine::{CaptionStyleApply, EditError};
    for case in [
        "locked",
        "missing",
        "wrong-kind",
        "empty",
        "invalid",
        "invalid-family",
        "invalid-margin",
        "invalid-alignment",
        "invalid-size",
        "other-scene",
        "batch",
    ] {
        let mut d = caption_project().into_data();
        if case == "locked" {
            d.scene_mut().timeline.overlays[0].locked = true;
            let ItemParams::Caption(p) = &mut d.scene_mut().timeline.overlays[0].items[0].content
            else {
                panic!("caption")
            };
            p.cues.styles = vec![preset_style(false)];
            p.cues.cues[0].style = Some(captions::CueStyleId(0));
        }
        if case == "wrong-kind" {
            let value = &mut d.scene_mut().timeline.overlays[0].items[0];
            value.content = ItemParams::Text(project::TextParams::new("title"));
            value.source_asset = None;
        }
        if case == "empty" {
            let ItemParams::Caption(p) = &mut d.scene_mut().timeline.overlays[0].items[0].content
            else {
                panic!("caption")
            };
            p.cues.cues.clear();
        }
        let mut s = Session::new(Project::new(d).unwrap(), 10).unwrap();
        if case == "other-scene" {
            let result = s
                .apply(Command::CreateScene {
                    name: "Other".into(),
                })
                .unwrap();
            let CreatedId::Scene(scene) = result.created[0] else {
                panic!("scene")
            };
            s.apply(Command::SwitchScene { scene }).unwrap();
        }
        let mut style = preset_style(false);
        if case == "invalid" {
            style.font_size_ratio = f32::NAN;
        }
        match case {
            "invalid-family" => style.font_family = " ".into(),
            "invalid-margin" => style.margin_left_ratio = 10.01,
            "invalid-alignment" => style.alignment.numpad = 9,
            "invalid-size" => style.font_size_ratio = 0.0,
            _ => {}
        }
        let command = if case == "batch" {
            Command::Batch {
                commands: vec![
                    apply_style(CaptionStyleApply::Custom { style }),
                    Command::Delete {
                        items: vec![ItemId(999)],
                    },
                ],
            }
        } else {
            Command::ApplyCaptionStyle {
                item: ItemId(if case == "missing" { 999 } else { 1 }),
                style: CaptionStyleApply::Custom { style },
            }
        };
        s.take_change();
        let before = s.project().clone();
        let revision = s.revision();
        let history = s.history_len();
        let error = s.apply(command).unwrap_err();
        assert!(
            match case {
                "locked" => matches!(error, EditError::Locked(TrackId(1))),
                "missing" | "batch" => matches!(error, EditError::NoItem(ItemId(999))),
                "other-scene" => matches!(error, EditError::SceneBoundary),
                _ => matches!(error, EditError::Invalid(_)),
            },
            "{case}: {error}"
        );
        assert_eq!(s.project(), &before);
        assert_eq!(s.revision(), revision);
        assert_eq!(s.history_len(), history);
        assert!(s.take_change().is_none());
    }
    let mut s = Session::new(caption_project(), 10).unwrap();
    let before = s.project().clone();
    for style in [
        json!({"kind":"preset","preset":"unknown"}),
        json!({"kind":"preset","preset":"box","extra":0}),
        json!({"kind":"custom","style":{"font_family":"Inter"}}),
        json!({"kind":"other"}),
    ] {
        assert!(
            s.execute("captions.apply_style", json!({"item":1,"style":style}))
                .is_err()
        );
        assert_eq!(s.project(), &before);
        assert_eq!(s.history_len(), (0, 0));
    }
    let mut style = serde_json::to_value(preset_style(false)).unwrap();
    style["extra"] = json!(0);
    assert!(
        s.execute(
            "captions.apply_style",
            json!({"item":1,"style":{"kind":"custom","style":style}})
        )
        .is_err()
    );
}

#[test]
fn caption_style_retained_trim_cancel_commit_and_text_edit_preserve_source_data() {
    use engine::CaptionStyleApply;
    for commit in [false, true] {
        let mut s = Session::new(style_source_project(), 10).unwrap();
        let before = s.project().clone();
        s.begin_gesture().unwrap();
        s.apply(caption_trim(false, 1005)).unwrap(); // No visible cue, but a retained source cue.
        s.apply(apply_style(CaptionStyleApply::Custom {
            style: preset_style(true),
        }))
        .unwrap();
        s.apply(caption_trim(false, 1100)).unwrap();
        assert_eq!(saved_cues(&s).styles, vec![preset_style(true)]);
        assert_eq!(saved_cues(&s).cues[0].texts, {
            let ItemParams::Caption(p) =
                &before.data().scene().timeline.overlays[0].items[0].content
            else {
                panic!("caption")
            };
            p.cues.cues[0].texts.clone()
        });
        if commit {
            s.commit_gesture().unwrap();
            assert_eq!(s.history_len(), (1, 0));
            let styled = s.project().clone();
            s.undo().unwrap();
            assert_eq!(s.project(), &before);
            s.redo().unwrap();
            assert_eq!(s.project(), &styled);
            let mut content = get(&s, 1).content.clone();
            let ItemParams::Caption(p) = &mut content else {
                panic!("caption")
            };
            p.cues.cues[0].texts.values_mut().next().unwrap().text = "Edited".into();
            s.apply(Command::SetItemParam {
                item: ItemId(1),
                content,
            })
            .unwrap();
            assert_eq!(saved_cues(&s).styles, vec![preset_style(true)]);
        } else {
            s.cancel_gesture().unwrap();
            assert_eq!(s.project(), &before);
            assert_eq!(s.history_len(), (0, 0));
        }
    }
}

#[test]
fn caption_splits_and_trims_reuse_mapping_and_preserve_source_text_metadata() {
    let mut s = Session::new(caption_project(), 10).unwrap();
    let ItemParams::Caption(params) = &get(&s, 1).content else {
        panic!("caption")
    };
    let cue = params.cues.cues[0].clone();
    let original = s.project().clone();
    let outcome = s
        .apply(Command::SplitAt {
            item: ItemId(1),
            at: Tick(1050),
        })
        .unwrap();
    let CreatedId::Item(right) = *outcome.created.first().unwrap() else {
        panic!("item ID")
    };
    for (id, expected) in [(ItemId(1), range(110, 150)), (right, range(150, 190))] {
        let ItemParams::Caption(params) = &get(&s, id.0).content else {
            panic!("caption")
        };
        let clipped = params.cues.cues.first().unwrap();
        assert_eq!(clipped.range, expected);
        assert_eq!(clipped.id, cue.id);
        assert_eq!(clipped.texts, cue.texts);
        assert_eq!(clipped.source_lang, cue.source_lang);
    }
    s.apply(Command::TrimEnd {
        item: ItemId(1),
        at: Tick(1020),
    })
    .unwrap();
    let ItemParams::Caption(params) = &get(&s, 1).content else {
        panic!("caption")
    };
    assert_eq!(params.cues.cues.first().unwrap().range, range(110, 120));
    assert_eq!(params.cues.cues.first().unwrap().id, cue.id);
    s.undo().unwrap();
    s.undo().unwrap();
    assert!(Arc::ptr_eq(&original, s.project()));
}
fn caption_project() -> Project {
    use captions::{Cue, CueText, CueTrack, LanguageTag};
    use project::CaptionParams;
    let mut data = project().into_data();
    data.scene_mut().timeline.overlays.clear();
    data.assets.push(
        SourceAsset::try_from(AssetData {
            ephemeral: false,
            id: AssetId(1),
            path: std::env::temp_dir().join("caption source"),
            content_hash: ContentHash([0; 32]),
            probe: SourceProbe {
                format: "fixture".into(),
                duration: None,
                streams: vec![],
            },
        })
        .unwrap(),
    );
    let lang = LanguageTag::parse("vi").unwrap();
    let cue = Cue {
        word_timing: captions::CueWordTiming::Unverified,
        id: "source/0".into(),
        range: range(110, 190),
        source_lang: lang.clone(),
        texts: BTreeMap::from([(
            lang,
            CueText {
                text: "Tiếng Việt".into(),
                edited: true,
                translation_ref: Some("ref".into()),
            },
        )]),
        word_ranges: Vec::new(),
        style: None,
    };
    data.scene_mut().timeline.insert_track(Track {
        id: TrackId(1),
        name: "".into(),
        kind: TrackKind::Text,
        muted: false,
        hidden: false,
        locked: false,
        items: vec![Item {
            hidden: false,
            blend_mode: Default::default(),
            retime: Default::default(),
            masks: Vec::new(),
            effects: Vec::new(),
            animations: Default::default(),
            id: ItemId(1),
            source_asset: Some(AssetId(1)),
            source_range: range(100, 200),
            start: Tick(1000),
            content: ItemParams::Caption(CaptionParams {
                emphasis: Default::default(),
                word_highlight: false,
                preset: None,
                overrides: captions::CaptionOverrides::default(),
                cues: CueTrack {
                    styles: Vec::new(),
                    cues: vec![cue.clone()],
                },
            }),
        }],
    });
    Project::new(data).unwrap()
}
fn caption_trim(start: bool, at: i64) -> Command {
    if start {
        Command::TrimStart {
            item: ItemId(1),
            at: Tick(at),
        }
    } else {
        Command::TrimEnd {
            item: ItemId(1),
            at: Tick(at),
        }
    }
}
#[test]
fn caption_drag_inward_and_back_preserves_original_content_and_redo() {
    for start in [false, true] {
        // Exercise partial clipping and a preview that removes the cue entirely.
        for inward in if start { [1050, 1095] } else { [1050, 1005] } {
            let mut s = Session::new(caption_project(), 10).unwrap();
            let original = s.project().clone();
            s.apply(Command::BookmarkAdd {
                start: Tick(0),
                duration: Tick(0),
                note: "redo".into(),
                color: Color([0, 0, 0, 255]),
            })
            .unwrap();
            let redo = s.project().clone();
            s.undo().unwrap();
            s.take_change();
            s.begin_gesture().unwrap();
            s.apply(caption_trim(start, inward)).unwrap();
            s.apply(caption_trim(start, if start { 1000 } else { 1100 }))
                .unwrap();
            assert_eq!(s.project().data(), original.data());
            assert!(!s.commit_gesture().unwrap().changed);
            assert_eq!(s.history_len(), (0, 1));
            assert!(s.take_change().is_none());
            s.redo().unwrap();
            assert_eq!(s.project().data(), redo.data());
        }
    }
}
#[test]
fn caption_drag_commit_cancel_undo_redo_and_group_paths() {
    for start in [false, true] {
        for mode in ["gesture", "group", "batch", "merged"] {
            let mut s = Session::new(caption_project(), 10).unwrap();
            let original = s.project().clone();
            let inward = if start { 1095 } else { 1005 };
            let outward = if start { 1020 } else { 1080 };
            let mut direct = Session::new(caption_project(), 10).unwrap();
            direct.apply(caption_trim(start, outward)).unwrap();
            let drag = |s: &mut Session| -> engine::control::Result<()> {
                for at in [inward, outward] {
                    if mode == "merged" {
                        s.apply_merged(caption_trim(start, at), Some("trim"))?;
                    } else {
                        s.apply(caption_trim(start, at))?;
                    }
                }
                Ok(())
            };
            match mode {
                "gesture" => {
                    s.begin_gesture().unwrap();
                    drag(&mut s).unwrap();
                    assert!(s.take_change().is_none());
                    s.cancel_gesture().unwrap();
                    assert_eq!(s.project().data(), original.data());
                    assert_eq!(s.history_len(), (0, 0));
                    assert!(s.take_change().is_none());
                    s.begin_gesture().unwrap();
                    drag(&mut s).unwrap();
                    assert!(s.commit_gesture().unwrap().changed);
                }
                "group" => s.transaction(drag).unwrap(),
                "batch" => {
                    s.apply(Command::Batch {
                        commands: vec![caption_trim(start, inward), caption_trim(start, outward)],
                    })
                    .unwrap();
                }
                _ => drag(&mut s).unwrap(),
            }
            assert_eq!(s.project().data(), direct.project().data());
            assert_eq!(s.history_len(), (1, 0));
            assert!(s.take_change().is_some());
            s.undo().unwrap();
            assert_eq!(s.project().data(), original.data());
            s.redo().unwrap();
            assert_eq!(s.project().data(), direct.project().data());
            // A boundary discards transient source retention: later independent
            // edits cannot resurrect content discarded by the committed trim.
            s.end_merge();
            s.apply(caption_trim(start, if start { 1000 } else { 1100 }))
                .unwrap();
            let ItemParams::Caption(params) = &get(&s, 1).content else {
                panic!("caption")
            };
            let ItemParams::Caption(expected) = &get(&direct, 1).content else {
                panic!("caption")
            };
            assert_eq!(params, expected);
        }
    }
}
#[test]
fn explicit_caption_edits_replace_retained_gesture_content_and_refusals_roll_back() {
    for group in [false, true] {
        let mut s = Session::new(caption_project(), 10).unwrap();
        let run = |s: &mut Session| -> engine::control::Result<()> {
            s.apply(caption_trim(false, 1050))?;
            let mut content = get(s, 1).content.clone();
            let ItemParams::Caption(params) = &mut content else {
                panic!("caption")
            };
            let lang = captions::LanguageTag::parse("en").unwrap();
            params.cues.cues[0].texts.insert(
                lang,
                captions::CueText {
                    text: "Edited translation".into(),
                    edited: true,
                    translation_ref: Some("new".into()),
                },
            );
            // The replacement must not survive a refused batch.
            assert!(
                s.apply(Command::Batch {
                    commands: vec![
                        Command::SetItemParam {
                            item: ItemId(1),
                            content: content.clone()
                        },
                        Command::Delete {
                            items: vec![ItemId(999)]
                        },
                    ]
                })
                .is_err()
            );
            s.apply(caption_trim(false, 1100))?;
            assert_eq!(s.project().data(), caption_project().data());
            s.apply(caption_trim(false, 1050))?;
            s.apply(Command::SetItemParam {
                item: ItemId(1),
                content: content.clone(),
            })?;
            s.apply(caption_trim(false, 1100))?;
            assert_eq!(get(s, 1).content, content);
            Ok(())
        };
        if group {
            s.transaction(run).unwrap();
        } else {
            s.begin_gesture().unwrap();
            run(&mut s).unwrap();
            s.commit_gesture().unwrap();
        }
    }
}
proptest! {
    #![proptest_config(ProptestConfig::with_cases(128))]

#[test]
    fn caption_gesture_previews_depend_only_on_final_edge(
        start in any::<bool>(),
        edges in prop::collection::vec(1001i64..1100, 1..40),
    ) {
        let mut s = Session::new(caption_project(), 10).unwrap();
        s.begin_gesture().unwrap();
        for at in &edges {
            s.apply(caption_trim(start, *at)).unwrap();
            let mut direct = Session::new(caption_project(), 10).unwrap();
            direct.apply(caption_trim(start, *at)).unwrap();
            prop_assert_eq!(s.project().data(), direct.project().data());
        }
        let committed = s.project().clone();
        s.commit_gesture().unwrap();
        s.undo().unwrap();
        let original = caption_project();
        prop_assert_eq!(s.project().data(), original.data());
        s.redo().unwrap();
        prop_assert_eq!(s.project().data(), committed.data());
    }
}
#[test]
fn merged_caption_roundtrip_and_equal_content_replacement() {
    let mut s = Session::new(caption_project(), 10).unwrap();
    let original = s.project().clone();
    for at in [1005, 1100] {
        s.apply_merged(caption_trim(false, at), Some("trim"))
            .unwrap();
    }
    assert_eq!(s.project().data(), original.data());
    s.end_merge();
    s.begin_gesture().unwrap();
    s.apply(caption_trim(false, 1050)).unwrap();
    let replacement = get(&s, 1).content.clone();
    assert!(
        !s.apply(Command::SetItemParam {
            item: ItemId(1),
            content: replacement.clone(),
        })
        .unwrap()
        .changed
    );
    s.apply(caption_trim(false, 1100)).unwrap();
    assert_eq!(get(&s, 1).content, replacement);
    s.commit_gesture().unwrap();
}
fn ass_style() -> captions::CueStyle {
    captions::CueStyle {
        font_family: "Arial".into(),
        font_size_ratio: 0.05,
        primary_color: [255, 255, 255, 255],
        outline_color: [0, 0, 0, 255],
        background_color: [0, 0, 0, 0],
        border_style: 1,
        alignment: captions::CueAlignment {
            numpad: 2,
            horizontal: captions::HorizontalAlignment::Center,
            vertical: captions::VerticalAlignment::Bottom,
        },
        margin_left_ratio: 0.0,
        margin_right_ratio: 0.0,
        margin_vertical_ratio: 0.0,
        bold: true,
        italic: false,
        underline: false,
        strikeout: false,
    }
}
fn ass_asset() -> SourceAsset {
    SourceAsset::try_from(AssetData {
        ephemeral: true,
        id: AssetId(30),
        path: std::env::temp_dir().join("captions.ass"),
        content_hash: ContentHash::from_bytes(b"ass"),
        probe: SourceProbe {
            format: "ass".into(),
            duration: Some(Tick(254_016_000_000)),
            streams: Vec::new(),
        },
    })
    .unwrap()
}
fn ass_import() -> Command {
    Command::ImportCaptions {
        asset: ass_asset(),
        language: captions::LanguageTag::parse("en").unwrap(),
        start: Tick(100),
        track: None,
        name: Some("Imported captions".into()),
        styles: vec![ass_style()],
        cues: vec![
            captions::AssCue {
                range: range(0, 254_016_000_000),
                text: "Hello".into(),
                style: captions::CueStyleId(0),
            },
            captions::AssCue {
                range: range(508_032_000_000, 762_048_000_000),
                text: "World".into(),
                style: captions::CueStyleId(0),
            },
        ],
    }
}
#[test]
fn captions_import_is_one_undo_step_and_persists_style() {
    let mut s = session();
    let before = s.history_len();
    let outcome = s.apply(ass_import()).unwrap();
    assert!(outcome.changed);
    assert_eq!(s.history_len(), (before.0 + 1, 0));
    assert_eq!(outcome.placements.len(), 1);
    assert!(
        outcome
            .created
            .iter()
            .any(|id| matches!(id, CreatedId::Track(_)))
    );
    assert!(
        outcome
            .created
            .iter()
            .any(|id| matches!(id, CreatedId::Item(_)))
    );

    let item = s
        .project()
        .data()
        .scene()
        .timeline
        .iter()
        .flat_map(|track| &track.items)
        .find(|item| matches!(item.content, ItemParams::Caption(_)))
        .unwrap();
    let ItemParams::Caption(params) = &item.content else {
        panic!("expected caption item")
    };
    assert_eq!(params.cues.cues.len(), 2);
    assert_eq!(
        params.cues.cues[0]
            .texts
            .values()
            .next()
            .unwrap()
            .text
            .as_str(),
        "Hello"
    );
    assert_eq!(params.cues.styles.len(), 1);
    assert_eq!(params.cues.cues[0].style, Some(captions::CueStyleId(0)));
    assert_eq!(
        params
            .cues
            .style_of(&params.cues.cues[0])
            .unwrap()
            .font_family
            .as_str(),
        "Arial"
    );
    assert_eq!(item.start, Tick(100));
    assert_eq!(item.source_range, range(0, 762_048_000_000));

    let encoded = project::encode(s.project().as_ref(), false).unwrap();
    let loaded = project::decode(&encoded).unwrap();
    assert_eq!(loaded.project, **s.project());
    let ItemParams::Caption(reloaded) = &loaded
        .project
        .data()
        .scene()
        .timeline
        .iter()
        .flat_map(|track| &track.items)
        .find(|item| matches!(item.content, ItemParams::Caption(_)))
        .unwrap()
        .content
    else {
        panic!("expected caption item")
    };
    assert_eq!(reloaded.cues.cues[0].style, Some(captions::CueStyleId(0)));
    assert_eq!(
        reloaded.cues.style_of(&reloaded.cues.cues[0]).unwrap(),
        &ass_style()
    );

    s.apply(Command::Undo {}).unwrap();
    assert!(
        !s.project()
            .data()
            .scene()
            .timeline
            .iter()
            .any(|track| track.kind == TrackKind::Text)
    );
    assert!(
        !s.project()
            .data()
            .assets
            .iter()
            .any(|asset| asset.id() == AssetId(30))
    );

    s.apply(Command::Redo {}).unwrap();
    let item = s
        .project()
        .data()
        .scene()
        .timeline
        .iter()
        .flat_map(|track| &track.items)
        .find(|item| matches!(item.content, ItemParams::Caption(_)))
        .unwrap();
    let ItemParams::Caption(params) = &item.content else {
        panic!("expected caption item")
    };
    assert_eq!(params.cues.styles, vec![ass_style()]);
    assert_eq!(params.cues.cues[0].style, Some(captions::CueStyleId(0)));
}
#[test]
fn captions_import_command_admits_json_and_rejects_bad_cues_atomically() {
    let mut s = session();
    let command = ass_import();
    let Command::ImportCaptions {
        asset,
        language,
        start,
        track,
        name,
        styles,
        cues,
    } = command
    else {
        panic!("expected import command")
    };
    let params = json!({
        "asset": serde_json::to_value(&asset).unwrap(),
        "language": language.as_str(),
        "start": start.0,
        "track": track,
        "name": name,
        "styles": serde_json::to_value(&styles).unwrap(),
        "cues": serde_json::to_value(&cues).unwrap(),
    });
    let before = s.project().clone();
    let outcome = crate::wire::run(&mut s, "captions.import", params.clone()).unwrap();
    assert!(outcome.changed);
    assert_eq!(s.project().data().scene().timeline.track_count(), 4);

    let mut bad = params.clone();
    bad["cues"][0]["range"]["end"] = json!(0);
    let after = s.project().clone();
    assert!(crate::wire::run(&mut s, "captions.import", bad).is_err());
    assert_eq!(s.project(), &after);
    assert_ne!(after, before);

    // A cue that names a style outside the table, or a style the model would
    // refuse, leaves the project untouched.
    let mut dangling = params.clone();
    dangling["cues"][0]["style"] = json!(7);
    let after = s.project().clone();
    assert!(s.execute("captions.import", dangling).is_err());
    assert_eq!(s.project(), &after);
    let mut invalid = params.clone();
    invalid["styles"][0]["font_size_ratio"] = json!(0.0);
    assert!(s.execute("captions.import", invalid).is_err());
    assert_eq!(s.project(), &after);

    // Hostile placement, count and style bursts are refused whole, never a
    // panic or an unsaveable project.
    let make = |count: usize, start: Tick| {
        let Command::ImportCaptions {
            asset,
            language,
            track,
            name,
            ..
        } = ass_import()
        else {
            panic!("expected import command")
        };
        let cues = (0..count)
            .map(|index| captions::AssCue {
                range: range(index as i64 * 1_000_000, index as i64 * 1_000_000 + 500_000),
                text: format!("line {index}").into(),
                style: captions::CueStyleId(0),
            })
            .collect();
        Command::ImportCaptions {
            asset,
            language,
            start,
            track,
            name,
            styles: vec![ass_style()],
            cues,
        }
    };
    let make_unique_styles = |count: usize| {
        let Command::ImportCaptions {
            asset,
            language,
            track,
            name,
            ..
        } = ass_import()
        else {
            panic!("expected import command")
        };
        let styles: Vec<_> = (0..count)
            .map(|index| captions::CueStyle {
                font_size_ratio: 0.001 + index as f32 * 0.0001,
                ..ass_style()
            })
            .collect();
        let cues = (0..count)
            .map(|index| captions::AssCue {
                range: range(index as i64 * 1_000_000, index as i64 * 1_000_000 + 500_000),
                text: format!("line {index}").into(),
                style: captions::CueStyleId(index as u16),
            })
            .collect();
        Command::ImportCaptions {
            asset,
            language,
            start: Tick(0),
            track,
            name,
            styles,
            cues,
        }
    };
    for (count, start) in [(1, Tick(i64::MAX - 10)), (1, Tick(-1))] {
        let mut fresh = session();
        let untouched = fresh.project().clone();
        assert!(
            fresh.apply(make(count, start)).is_err(),
            "{count} cues at {start:?}"
        );
        assert_eq!(fresh.project(), &untouched);
    }
    let mut fresh = session();
    fresh.apply(make(20_000, Tick(0))).unwrap();
    let encoded = project::encode(fresh.project().as_ref(), false).unwrap();
    let loaded = project::decode(&encoded).unwrap();
    let ItemParams::Caption(params) = &loaded
        .project
        .data()
        .scene()
        .timeline
        .iter()
        .flat_map(|track| &track.items)
        .find(|item| matches!(item.content, ItemParams::Caption(_)))
        .unwrap()
        .content
    else {
        panic!("expected caption item")
    };
    assert_eq!(params.cues.cues.len(), 20_000);
    assert_eq!(params.cues.styles.len(), 1);
    assert!(
        params
            .cues
            .cues
            .iter()
            .all(|cue| cue.style == Some(captions::CueStyleId(0)))
    );

    let mut fresh = session();
    let untouched = fresh.project().clone();
    assert!(matches!(
        fresh.apply(make_unique_styles(captions::MAX_CUE_STYLES + 1)),
        Err(engine::EditError::Control(
            engine::control::ControlError::Limit
        ))
    ));
    assert_eq!(fresh.project(), &untouched);
    assert!(matches!(
        session().apply(make(captions::ass::MAX_ASS_CUES + 1, Tick(0))),
        Err(engine::EditError::Control(
            engine::control::ControlError::Limit
        ))
    ));
}

#[test]
fn word_highlight_is_an_undoable_locked_atomic_opt_in_without_style_or_timing_edits() {
    let mut s = Session::new(style_source_project(), 30).unwrap();
    let original = s.project().clone();
    s.execute(
        "captions.set_word_highlight",
        json!({"item":1,"enabled":true}),
    )
    .unwrap();
    let mut expected = original.data().clone();
    let ItemParams::Caption(params) =
        &mut expected.scene_mut().timeline.overlays[0].items[0].content
    else {
        panic!("caption");
    };
    params.word_highlight = true;
    assert_eq!(s.project().data(), &expected);
    let enabled = s.project().clone();
    assert!(
        !s.execute(
            "captions.set_word_highlight",
            json!({"item":1,"enabled":true})
        )
        .unwrap()
        .changed
    );
    s.undo().unwrap();
    assert_eq!(s.project(), &original);
    s.redo().unwrap();
    assert_eq!(s.project(), &enabled);
    let locked_track = s.project().data().scene().timeline.overlays[0].id;
    s.apply(Command::SetTrackLocked {
        track: locked_track,
        locked: true,
    })
    .unwrap();
    let locked = s.project().clone();
    assert!(matches!(
        s.execute(
            "captions.set_word_highlight",
            json!({"item":1,"enabled":true})
        ),
        Err(engine::EditError::Locked(_))
    ));
    assert!(Arc::ptr_eq(&locked, s.project()));
    s.undo().unwrap();
    let before = s.project().clone();
    assert!(
        s.apply(Command::Batch {
            commands: vec![
                Command::SetCaptionWordHighlight {
                    item: ItemId(1),
                    enabled: false
                },
                Command::SetCaptionWordHighlight {
                    item: ItemId(u64::MAX),
                    enabled: true
                }
            ]
        })
        .is_err()
    );
    assert!(Arc::ptr_eq(&before, s.project()));
}

fn emphasis_project() -> Project {
    use captions::{CueText, CueWordTiming, LanguageTag, TimingProvenance, VerifiedCueWord};
    let mut data = caption_project().into_data();
    let ItemParams::Caption(params) = &mut data.scene_mut().timeline.overlays[0].items[0].content
    else {
        panic!("caption");
    };
    let text = "Quarter revenue grew 40 percent worldwide";
    let en = LanguageTag::parse("en").unwrap();
    let records = text
        .split(' ')
        .scan(0, |at, word| {
            let start = *at;
            *at += word.len() + 1;
            Some(start..start + word.len())
        })
        .enumerate()
        .map(|(index, bytes)| {
            let timed = range(110 + 10 * index as i64, 120 + 10 * index as i64);
            VerifiedCueWord {
                bytes,
                source_range: timed,
                projection: captions::WordProjection::Visible(timed),
                provenance: TimingProvenance::Native,
            }
        })
        .collect();
    let cue = &mut params.cues.cues[0];
    cue.source_lang = en.clone();
    cue.texts = BTreeMap::from([
        (
            en.clone(),
            CueText {
                text: text.into(),
                edited: false,
                translation_ref: None,
            },
        ),
        (
            LanguageTag::parse("vi").unwrap(),
            CueText {
                text: "Doanh thu quý tăng 40 phần trăm".into(),
                edited: false,
                translation_ref: Some("ref".into()),
            },
        ),
    ]);
    cue.word_timing = CueWordTiming::Words {
        language: en,
        text: text.into(),
        records,
    };
    Project::new(data).unwrap()
}
fn emphasis_of(s: &Session) -> BTreeMap<captions::NfcText, Vec<u32>> {
    let ItemParams::Caption(params) =
        &s.project().data().scene().timeline.overlays[0].items[0].content
    else {
        panic!("caption");
    };
    params.emphasis.clone()
}

#[test]
fn caption_emphasis_is_a_verified_subset_undoable_persistent_and_dropped_on_retranslation() {
    // A cue without verified word timing never gets an invented selection.
    let mut plain = Session::new(caption_project(), 30).unwrap();
    let before = plain.project().clone();
    for command in [
        (
            "captions.select_emphasis",
            json!({"item":1,"cue_ids":["source/0"]}),
        ),
        (
            "captions.set_emphasis",
            json!({"item":1,"cue_id":"source/0","word_ids":[0]}),
        ),
    ] {
        assert!(matches!(
            plain.execute(command.0, command.1),
            Err(engine::EditError::Captions(captions::Error::NoWordTiming))
        ));
        assert!(Arc::ptr_eq(&before, plain.project()));
    }

    let mut s = Session::new(emphasis_project(), 30).unwrap();
    let original = s.project().clone();
    // Lexicon: the number and the longest content word win; stopwords never score.
    s.execute(
        "captions.select_emphasis",
        json!({"item":1,"cue_ids":["source/0"],"mode":"lexicon"}),
    )
    .unwrap();
    assert_eq!(
        emphasis_of(&s),
        BTreeMap::from([("source/0".into(), vec![3, 5])])
    );
    let selected = s.project().clone();
    // Same gesture again is deterministic and changes nothing.
    assert!(
        !s.execute(
            "captions.select_emphasis",
            json!({"item":1,"cue_ids":["source/0"]})
        )
        .unwrap()
        .changed
    );
    // Explicit ids: refusals leave the project untouched and name nothing outside the cue.
    for ids in [
        json!([5, 3]),
        json!([3, 3]),
        json!([0, 1, 2, 3]),
        json!([6]),
    ] {
        assert!(
            s.execute(
                "captions.set_emphasis",
                json!({"item":1,"cue_id":"source/0","word_ids":ids})
            )
            .is_err()
        );
        assert!(Arc::ptr_eq(&selected, s.project()));
    }
    assert!(
        s.execute(
            "captions.set_emphasis",
            json!({"item":1,"cue_id":"missing","word_ids":[1]})
        )
        .is_err()
    );
    s.execute(
        "captions.set_emphasis",
        json!({"item":1,"cue_id":"source/0","word_ids":[1]}),
    )
    .unwrap();
    assert_eq!(
        emphasis_of(&s),
        BTreeMap::from([("source/0".into(), vec![1])])
    );
    // One undo step per gesture, and the saved file round-trips the selection.
    let manual = s.project().clone();
    assert_eq!(
        project::decode(&project::encode(&manual, false).unwrap())
            .unwrap()
            .project,
        *manual
    );
    s.undo().unwrap();
    assert_eq!(s.project(), &selected);
    s.undo().unwrap();
    assert_eq!(s.project(), &original);
    s.redo().unwrap();
    s.redo().unwrap();
    assert_eq!(s.project(), &manual);
    s.execute(
        "captions.set_emphasis",
        json!({"item":1,"cue_id":"source/0","word_ids":[]}),
    )
    .unwrap();
    assert!(emphasis_of(&s).is_empty());
    s.undo().unwrap();

    // Retranslation drops the selection; cue text and word timing stay as they were.
    let cue_before = {
        let ItemParams::Caption(p) =
            &s.project().data().scene().timeline.overlays[0].items[0].content
        else {
            panic!("caption");
        };
        p.cues.cues[0].clone()
    };
    s.execute(
        "retranslate",
        json!({"item":1,"target_language":"vi","texts":{"source/0":"Doanh thu tăng 40%"},
               "translation_ref":"again","force":true}),
    )
    .unwrap();
    assert!(emphasis_of(&s).is_empty());
    let ItemParams::Caption(after) =
        &s.project().data().scene().timeline.overlays[0].items[0].content
    else {
        panic!("caption");
    };
    assert_eq!(after.cues.cues[0].word_timing, cue_before.word_timing);
    assert_eq!(
        after.cues.cues[0]
            .texts
            .get(&captions::LanguageTag::parse("en").unwrap()),
        cue_before
            .texts
            .get(&captions::LanguageTag::parse("en").unwrap())
    );
}
