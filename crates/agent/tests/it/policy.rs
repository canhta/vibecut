//! The refusal policy table against the engine registry.
use agent::{AgentPolicy, Reason, TABLE, Verdict, verdict};
use engine::{CommandKind, GrantClass, registry};
use std::collections::BTreeSet;

#[test]
fn every_registry_command_has_exactly_one_row_and_no_row_is_stale() {
    let ids: Vec<&str> = registry().iter().map(|spec| spec.id).collect();
    let rows: Vec<&str> = TABLE.iter().map(|(id, _)| *id).collect();
    let unique: BTreeSet<&str> = rows.iter().copied().collect();
    assert_eq!(unique.len(), rows.len(), "duplicate rows");
    let missing: Vec<&&str> = ids.iter().filter(|id| !unique.contains(**id)).collect();
    assert!(
        missing.is_empty(),
        "commands without a policy row, add each as allowed or refused: {missing:?}"
    );
    let stale: Vec<&&str> = rows.iter().filter(|id| !ids.contains(*id)).collect();
    assert!(
        stale.is_empty(),
        "rows for commands that no longer exist: {stale:?}"
    );
}

#[test]
fn a_row_never_widens_the_declared_grant() {
    for spec in registry() {
        let kind = CommandKind::from_id(spec.id).unwrap();
        let agent = kind.grant() == Some(GrantClass::Agent);
        let row = verdict(spec.id).unwrap();
        let allowed = matches!(row, Verdict::Allow | Verdict::Frames);
        assert!(
            !allowed || agent,
            "{} is allowed but its grant is {:?}",
            spec.id,
            kind.grant()
        );
        if !agent {
            assert!(
                AgentPolicy::new(true).check(spec.id).is_err(),
                "{}",
                spec.id
            );
        }
    }
}

#[test]
fn decision_five_commands_are_refused_with_their_reason() {
    let policy = AgentPolicy::new(false);
    for (id, reason) in [
        ("export", Reason::Export),
        ("export.clips", Reason::Export),
        ("export_subtitles", Reason::Export),
        ("burn_subtitles", Reason::Export),
        ("project.save", Reason::Save),
        ("project.save_as", Reason::Export),
        ("project.create", Reason::Project),
        ("project.delete", Reason::Project),
        ("project.duplicate", Reason::Project),
        ("project.import_bundle", Reason::Path),
        ("import.add_asset", Reason::Path),
        ("undo", Reason::History),
        ("redo", Reason::History),
        ("fonts.download", Reason::Download),
        ("fonts.import", Reason::Path),
        ("sounds.search", Reason::Library),
        ("music.add_to_timeline", Reason::Path),
        ("begin_gesture", Reason::Gesture),
        ("timeline.preview", Reason::Gesture),
        ("preview", Reason::Frames),
        ("inspect", Reason::UseTool),
        ("nonexistent.command", Reason::Unlisted),
    ] {
        assert_eq!(policy.check(id), Err(reason), "{id}");
    }
}

#[test]
fn editing_commands_are_allowed_and_frames_open_only_the_preview() {
    let policy = AgentPolicy::new(false);
    for id in [
        "batch",
        "timeline.trim",
        "timeline.add_track",
        "project.rename",
        "split_at",
        "delete",
        "job_status",
        "job_cancel",
    ] {
        assert_eq!(policy.check(id), Ok(()), "{id}");
    }
    assert_eq!(AgentPolicy::new(true).check("preview"), Ok(()));
    assert_eq!(
        AgentPolicy::new(true).check("export"),
        Err(Reason::Export),
        "frames never open an export"
    );
}

#[test]
fn the_preview_row_opens_with_frames_but_command_run_always_points_to_the_tool() {
    assert_eq!(verdict("preview"), Some(Verdict::Frames));
    assert_eq!(
        AgentPolicy::new(false).check("preview"),
        Err(Reason::Frames)
    );
    assert_eq!(AgentPolicy::new(true).check("preview"), Ok(()));
    for frames in [false, true] {
        assert_eq!(
            AgentPolicy::new(frames).check_command("preview"),
            Err(Reason::UseTool)
        );
        assert_eq!(
            AgentPolicy::new(frames).check_command("timeline.trim"),
            Ok(())
        );
    }
}
