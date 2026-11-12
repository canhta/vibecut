#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]
use serde_json::{Value, json};
use std::{
    io::{BufRead, BufReader, Read as _, Write},
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};

struct Process(Child);
impl Drop for Process {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[cfg(windows)]
fn foreground() -> String {
    use std::os::windows::process::CommandExt;
    let output = Command::new("powershell.exe").args(["-NoProfile", "-NonInteractive", "-Command", "Add-Type 'using System; using System.Runtime.InteropServices; public class Focus { [DllImport(\"user32.dll\")] public static extern IntPtr GetForegroundWindow(); }'; [Focus]::GetForegroundWindow().ToInt64()"])
        .creation_flags(0x08000000).output().unwrap();
    assert!(output.status.success());
    String::from_utf8(output.stdout).unwrap().trim().to_owned()
}

#[test]
fn mcp_real_app_background_attach_never_creates_a_window_or_takes_focus() {
    socket_edits_activate_editor();
    let root = tempfile::tempdir().unwrap();
    let project = root.path().join("project.json");
    let snapshot = project::Project::empty(project::ProjectId(42), "background".into()).unwrap();
    std::fs::write(&project, project::encode(&snapshot, true).unwrap()).unwrap();
    let endpoint = if cfg!(windows) {
        std::path::PathBuf::from(format!("vibecut-background-{}", std::process::id()))
    } else {
        root.path().join("private/control.sock")
    };
    #[cfg(windows)]
    let focus = foreground();
    let mut launch = Command::new(env!("CARGO_BIN_EXE_vibecut"));
    launch
        .arg("--control-background")
        .arg(&project)
        .arg(&endpoint)
        .arg(root.path().join("recovery"))
        .arg("20")
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        launch.creation_flags(0x08000000);
    }
    let mut app = Process(launch.spawn().unwrap());
    let cli = std::path::Path::new(env!("CARGO_BIN_EXE_vibecut"))
        .with_file_name(format!("vibecut-cli{}", std::env::consts::EXE_SUFFIX));
    assert!(
        cli.exists(),
        "run cargo test -p vibecut-cli -p vibecut mcp to build both binaries"
    );
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        if let Some(status) = app.0.try_wait().unwrap() {
            let mut diagnostics = String::new();
            if let Some(mut stderr) = app.0.stderr.take() {
                stderr.read_to_string(&mut diagnostics).unwrap();
            }
            panic!("background app exited early: {status}: {diagnostics}");
        }
        let mut child = Process(
            Command::new(&cli)
                .args(["mcp", "--attach", "--socket"])
                .arg(&endpoint)
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .spawn()
                .unwrap(),
        );
        let mut input = child.0.stdin.take().unwrap();
        if writeln!(input, "{}", json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"app-test","version":"1"}}})).is_ok() {
            let output = child.0.stdout.take().unwrap();
            let mut reader = BufReader::new(output); let mut line = String::new();
            if reader.read_line(&mut line).unwrap_or(0) > 0 {
                let initialized: Value = serde_json::from_str(&line).unwrap(); assert!(initialized.get("result").is_some());
                writeln!(input, "{}", json!({"jsonrpc":"2.0","method":"notifications/initialized"})).unwrap();
                writeln!(input, "{}", json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"inspect","arguments":{"request_id":"inspect","command":"inspect","params":{}}}})).unwrap();
                line.clear(); reader.read_line(&mut line).unwrap();
                let response: Value = serde_json::from_str(&line).unwrap();
                assert_eq!(response.pointer("/result/structuredContent/value/status"), Some(&json!("result")), "{response}");
                #[cfg(windows)] assert_eq!(foreground(), focus, "background attach stole focus");
                break;
            }
        }
        assert!(
            Instant::now() < deadline,
            "background app socket did not become ready"
        );
        std::thread::sleep(Duration::from_millis(25));
    }
}

// Additional rows of the control behaviour: real socket -> inbox -> editor,
// while a kittest caption field still owns its uncommitted draft.
fn socket_edits_activate_editor() {
    use egui_kittest::kittest::{By, Queryable};
    use vibecut::{egui, preferences::Preferences, workspace::Workspace};

    for (command, draft) in [("delete", false), ("project.rename", true)] {
        let root = tempfile::tempdir().unwrap();
        let mut workspace =
            Workspace::new(root.path().join("recovery"), Preferences::default(), None).unwrap();
        let editor = caption_editor();
        let item = editor
            .session
            .project()
            .data()
            .scene()
            .timeline
            .iter()
            .flat_map(|track| &track.items)
            .next()
            .unwrap()
            .id;
        let id = workspace.register_editor(Box::new(editor));
        let mut h = crate::gpu::builder()
            .with_size(egui::vec2(1440.0, 1000.0))
            .build_ui_state(
                |ui, workspace: &mut Workspace| workspace.focused_editor_mut().unwrap().ui(ui),
                workspace,
            );
        h.run_steps(3);
        h.state_mut()
            .editor_mut(id)
            .unwrap()
            .activate(&format!("timeline.item.{}", item.0))
            .unwrap();
        let expected = if draft {
            let field = "subtitles.text.cue-0.vi";
            h.get(By::new().predicate(|n| n.author_id() == Some("subtitles.cue.cue-0")))
                .click_accesskit();
            h.run_steps(3);
            h.get(By::new().predicate(|n| n.author_id() == Some(field)))
                .focus();
            h.run_steps(2);
            h.get(By::new().predicate(|n| n.author_id() == Some(field)))
                .type_text(" pending draft");
            h.run_steps(2);
            Some(
                h.get(By::new().predicate(|n| n.author_id() == Some(field)))
                    .value()
                    .unwrap()
                    .to_owned(),
            )
        } else {
            None
        };
        let editor = h.state_mut().editor_mut(id).unwrap();
        let generation = editor.playback.generation();
        let revision = editor.session.revision();
        let endpoint = if cfg!(windows) {
            std::path::PathBuf::from(format!(
                "vibecut-activation-{}-{command}",
                std::process::id()
            ))
        } else {
            root.path().join("private/control.sock")
        };
        let mut server = mcp::Server::bind_with_media(
            &endpoint,
            Default::default(),
            editor.playback.budget().clone(),
            None,
        )
        .unwrap();
        let cli = std::path::Path::new(env!("CARGO_BIN_EXE_vibecut"))
            .with_file_name(format!("vibecut-cli{}", std::env::consts::EXE_SUFFIX));
        let params = if command == "delete" {
            json!({"items":[item]})
        } else {
            json!({"name":"Agent rename"})
        };
        let mut child = Process(
            Command::new(cli)
                .args(["mcp", "--attach", "--socket"])
                .arg(&endpoint)
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .spawn()
                .unwrap(),
        );
        let mut input = child.0.stdin.take().unwrap();
        writeln!(input, "{}", json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"activation-test","version":"1"}}})).unwrap();
        let mut reader = BufReader::new(child.0.stdout.take().unwrap());
        let mut line = String::new();
        reader.read_line(&mut line).unwrap();
        writeln!(
            input,
            "{}",
            json!({"jsonrpc":"2.0","method":"notifications/initialized"})
        )
        .unwrap();
        writeln!(input, "{}", json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"command_run","arguments":{"request_id":"activation","base_rev":revision,"idempotency_key":"activation","command":command,"params":params,"project_id":id}}})).unwrap();
        let (send, receive) = std::sync::mpsc::channel();
        let response = std::thread::spawn(move || {
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            send.send(serde_json::from_str::<Value>(&line).unwrap())
                .unwrap();
            reader
        });
        let deadline = Instant::now() + Duration::from_secs(10);
        let response_value = loop {
            vibecut::control::poll_workspace(h.state_mut(), &mut server.inbox).unwrap();
            if let Ok(value) = receive.try_recv() {
                break value;
            }
            assert!(Instant::now() < deadline, "control request timed out");
            std::thread::sleep(Duration::from_millis(2));
        };
        let mut reader = response.join().unwrap();
        let editor = h.state_mut().editor_mut(id).unwrap();
        assert!(
            editor.playback.generation() > generation,
            "{command}: stale preview"
        );
        assert_eq!(
            editor.inspect().unwrap().selection,
            editor
                .session
                .selection()
                .iter()
                .copied()
                .collect::<Vec<_>>(),
            "{command}: stale selection"
        );
        if let Some(expected) = expected {
            let cues = editor
                .session
                .project()
                .data()
                .scene()
                .timeline
                .iter()
                .flat_map(|track| &track.items)
                .find_map(|item| match &item.content {
                    project::ItemParams::Caption(p) => Some(&p.cues),
                    _ => None,
                })
                .unwrap();
            assert_eq!(
                cues.cues[0].texts[&captions::LanguageTag::parse("vi").unwrap()]
                    .text
                    .as_str(),
                expected,
                "socket edit skipped caption draft"
            );
            assert_eq!(
                response_value.pointer("/result/structuredContent/value/value"),
                Some(&json!("stale_revision")),
                "flushing the draft must preserve control revision admission: {response_value}"
            );
            assert_eq!(editor.session.project().data().name.as_str(), "Captions");
            editor.activate("edit.undo").unwrap();
            let cues = editor
                .session
                .project()
                .data()
                .scene()
                .timeline
                .iter()
                .flat_map(|track| &track.items)
                .find_map(|item| match &item.content {
                    project::ItemParams::Caption(p) => Some(&p.cues),
                    _ => None,
                })
                .unwrap();
            assert_ne!(
                cues.cues[0].texts[&captions::LanguageTag::parse("vi").unwrap()]
                    .text
                    .as_str(),
                expected,
                "draft must be its own undoable edit"
            );
        } else {
            assert_eq!(
                response_value.pointer("/result/structuredContent/value/status"),
                Some(&json!("result")),
                "{response_value}"
            );
            assert!(
                editor.inspect().unwrap().selection.is_empty(),
                "deleted item remains selected"
            );
            // Real persistence moves the sole editor onto its worker. Requests
            // arriving before that boundary settles keep the same refusal code.
            h.state_mut()
                .persist(
                    Some(root.path().join("saving.vibecut")),
                    false,
                    false,
                    false,
                )
                .unwrap();
            assert!(h.state().windows[&id].editor.is_none());
            writeln!(input, "{}", json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"command_run","arguments":{"request_id":"saving","command":"inspect","params":{},"project_id":id}}})).unwrap();
            let (send, receive) = std::sync::mpsc::channel();
            let response = std::thread::spawn(move || {
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                send.send(serde_json::from_str::<Value>(&line).unwrap())
                    .unwrap();
            });
            let deadline = Instant::now() + Duration::from_secs(10);
            let refused = loop {
                vibecut::control::poll_workspace(h.state_mut(), &mut server.inbox).unwrap();
                if let Ok(value) = receive.try_recv() {
                    break value;
                }
                assert!(Instant::now() < deadline, "busy editor response timed out");
                std::thread::sleep(Duration::from_millis(2));
            };
            response.join().unwrap();
            assert_eq!(
                refused.pointer("/result/structuredContent/error/code"),
                Some(&json!("refused"))
            );
        }
    }
}

fn caption_editor() -> vibecut::Editor {
    use engine::{ByteBudget, Command, NewItem, Session};
    use time::{SourceRange, TICKS_PER_SECOND, Tick};
    let mut session = Session::new(
        project::Project::empty(project::ProjectId(9), "Captions".into()).unwrap(),
        30,
    )
    .unwrap();
    session
        .apply(Command::AddAsset {
            asset: testkit::source::asset(1, std::env::temp_dir().join("control-caption.srt"))
                .ephemeral(true)
                .format("srt")
                .content_hash(media::ContentHash::from_bytes(b"control caption"))
                .build(),
        })
        .unwrap();
    let language = captions::LanguageTag::parse("vi").unwrap();
    session
        .apply(Command::InsertItem {
            placement: engine::Placement::Auto,
            item: NewItem {
                retime: Default::default(),
                animations: Default::default(),
                source_asset: Some(media::AssetId(1)),
                source_range: SourceRange::new(Tick::ZERO, Tick(10 * TICKS_PER_SECOND)).unwrap(),
                start: Tick::ZERO,
                content: project::ItemParams::Caption(project::CaptionParams {
                    word_highlight: false,
                    preset: None,
                    overrides: captions::CaptionOverrides::default(),
                    cues: captions::CueTrack {
                        styles: Vec::new(),
                        cues: vec![captions::Cue {
                            word_timing: captions::CueWordTiming::Unverified,
                            id: "cue-0".into(),
                            range: SourceRange::new(
                                Tick(TICKS_PER_SECOND),
                                Tick(3 * TICKS_PER_SECOND),
                            )
                            .unwrap(),
                            source_lang: language.clone(),
                            texts: [(
                                language,
                                captions::CueText {
                                    text: "Cha\u{0300}o ba\u{0323}n".into(),
                                    edited: false,
                                    translation_ref: None,
                                },
                            )]
                            .into(),
                            word_ranges: Vec::new(),
                            style: None,
                        }],
                    },
                }),
            },
        })
        .unwrap();
    let playback = vibecut::services::playback::Playback::new(
        &session,
        ByteBudget::new(256 << 20).unwrap(),
        std::sync::Arc::new(vibecut::services::playback::Files),
    )
    .unwrap();
    let mut preferences = vibecut::preferences::Preferences {
        assets_tab: vibecut::AssetsTab::Captions,
        ..Default::default()
    };
    preferences.subtitles.source = "vi".into();
    let mut editor = vibecut::Editor::new(session, playback, preferences);
    editor.prepare_subtitles().unwrap();
    editor
}
