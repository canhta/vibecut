#![allow(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    clippy::panic,
    clippy::expect_used
)]
//! The agent chat panel over a scripted model: consent, a turn, the changes card, Apply,
//! Discard, Stop and the history. No network and no key.
use agent::Provider;
use ai::text::conversation::{
    ChatModel, Message, ScriptedChat, ScriptedReply, ScriptedStep, ToolCall, ToolInput,
};
use egui_kittest::{
    Harness,
    kittest::{By, NodeT, Queryable},
};
use engine::{ByteBudget, Session};
use serde_json::{Value, json};
use std::{
    collections::VecDeque,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
        mpsc,
    },
    time::{Duration, Instant},
};
use vibecut::{
    AgentBackend, AgentView, Editor, egui,
    preferences::Preferences,
    services::playback::{Files, Playback},
};

/// Hands out one scripted conversation per consent and counts how often it was asked.
struct Scripted {
    scripts: Mutex<VecDeque<Vec<ScriptedStep>>>,
    opened: AtomicUsize,
    keyed: bool,
}

impl Scripted {
    fn new(scripts: Vec<Vec<ScriptedStep>>) -> Arc<Self> {
        Arc::new(Self {
            scripts: Mutex::new(scripts.into()),
            opened: AtomicUsize::new(0),
            keyed: true,
        })
    }
    fn without_key() -> Arc<Self> {
        Arc::new(Self {
            scripts: Mutex::new(VecDeque::new()),
            opened: AtomicUsize::new(0),
            keyed: false,
        })
    }
    fn opened(&self) -> usize {
        self.opened.load(Ordering::SeqCst)
    }
}

impl AgentBackend for Scripted {
    fn has_key(&self, _: Provider) -> bool {
        self.keyed
    }
    fn open(&self, _: Provider) -> Result<Box<dyn ChatModel>, agent::Error> {
        self.opened.fetch_add(1, Ordering::SeqCst);
        let steps = self.scripts.lock().unwrap().pop_front().unwrap_or_default();
        Ok(Box::new(
            ScriptedChat::new(steps).map_err(agent::Error::Chat)?,
        ))
    }
}

fn call(id: &str, name: &str, input: Value) -> ToolCall {
    ToolCall {
        id: id.into(),
        name: name.into(),
        input: ToolInput::Valid(input),
    }
}

fn calls(calls: Vec<ToolCall>) -> ScriptedStep {
    ScriptedStep::Reply(ScriptedReply {
        text: String::new(),
        calls,
    })
}

fn say(text: &str) -> ScriptedStep {
    ScriptedStep::Reply(ScriptedReply {
        text: text.into(),
        calls: Vec::new(),
    })
}

/// The decoded tool results of the latest tool-result message.
fn last_results(messages: &[Message]) -> Vec<Value> {
    messages
        .iter()
        .rev()
        .find_map(|message| match message {
            Message::ToolResults(results) => Some(
                results
                    .iter()
                    .map(|result| serde_json::from_str(&result.content).unwrap())
                    .collect(),
            ),
            _ => None,
        })
        .unwrap()
}

/// One turn that reads the timeline, hides the earliest clip and says so.
fn hide_first_turn() -> Vec<ScriptedStep> {
    vec![
        calls(vec![call("r1", "timeline_read", json!({}))]),
        ScriptedStep::Compose(Box::new(|messages: &[Message]| {
            let results = last_results(messages);
            let read = &results[0];
            let mut items: Vec<(u64, i64)> = read["items"]
                .as_array()
                .unwrap()
                .iter()
                .map(|item| {
                    (
                        item["id"].as_u64().unwrap(),
                        item["start"].as_i64().unwrap(),
                    )
                })
                .collect();
            items.sort_by_key(|(_, start)| *start);
            ScriptedReply {
                text: String::new(),
                calls: vec![call(
                    "c1",
                    "command_run",
                    json!({
                        "command": "timeline.set_hidden",
                        "params": {"items": [items[0].0], "hidden": true},
                    }),
                )],
            }
        })),
        say("Hid the first clip."),
    ]
}

fn editor(backend: &Arc<Scripted>, history: &std::path::Path) -> Editor {
    let session: Session = agent::fixture().unwrap();
    let mut playback = Playback::new(
        &session,
        ByteBudget::new(256 << 20).unwrap(),
        Arc::new(Files),
    )
    .unwrap();
    playback.set_device_available(true);
    let mut editor = Editor::new(session, playback, Preferences::default());
    editor.set_agent_backend(backend.clone());
    editor.set_agent_history(history.to_owned());
    editor
}

fn build(editor: Editor) -> Harness<'static, Editor> {
    crate::gpu::builder()
        .with_size(egui::vec2(1440.0, 900.0))
        .build_ui_state(|ui, editor: &mut Editor| editor.ui(ui), editor)
}

fn node<'a>(harness: &'a Harness<Editor>, id: &'static str) -> egui_kittest::Node<'a> {
    harness.get(By::new().predicate(move |node| node.author_id() == Some(id)))
}

fn has(harness: &Harness<Editor>, id: &'static str) -> bool {
    harness
        .query(By::new().predicate(move |node| node.author_id() == Some(id)))
        .is_some()
}

fn click(harness: &mut Harness<Editor>, id: &'static str) {
    node(harness, id).click();
    harness.run_steps(3);
}

fn view(harness: &Harness<Editor>) -> AgentView {
    harness.state().inspect().unwrap().agent
}

fn wait_for(harness: &mut Harness<Editor>, what: &str, done: impl Fn(&AgentView) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        harness.run_steps(1);
        if done(&view(harness)) {
            harness.run_steps(2);
            return;
        }
        assert!(
            Instant::now() < deadline,
            "timed out waiting for {what}: {:?}",
            view(harness)
        );
        std::thread::sleep(Duration::from_millis(2));
    }
}

fn hidden(editor: &Editor) -> Vec<bool> {
    editor
        .session
        .project()
        .data()
        .scene()
        .timeline
        .iter()
        .flat_map(|track| track.items.iter())
        .map(|item| item.hidden)
        .collect()
}

/// Open the chat, accept the consent and send `text`, as a creator does.
fn ask(harness: &mut Harness<Editor>, text: &str) {
    harness.state_mut().open_agent();
    harness.run_steps(3);
    if view(harness).stage == "consent" {
        click(harness, "agent.consent.accept");
    }
    node(harness, "agent.input").click();
    harness.run_steps(2);
    node(harness, "agent.input").type_text(text);
    harness.run_steps(2);
    click(harness, "agent.send");
}

#[test]
fn declining_the_consent_sends_nothing() {
    let backend = Scripted::new(vec![hide_first_turn()]);
    let dir = tempfile::tempdir().unwrap();
    let mut editor = editor(&backend, dir.path());
    let history = editor.session.history_len();
    editor.agent_prompt("hide the first clip".into());
    let mut harness = build(editor);
    harness.run_steps(3);

    let shown = view(&harness);
    assert!(shown.open && shown.queued);
    assert_eq!(
        shown.stage, "consent",
        "nothing is sent before the sheet is accepted"
    );
    assert!(has(&harness, "agent.consent.accept") && has(&harness, "agent.consent.frames"));
    assert!(!has(&harness, "agent.input"), "no input before consent");

    click(&mut harness, "agent.consent.decline");
    let after = view(&harness);
    assert!(!after.open && !after.queued);
    assert_eq!(backend.opened(), 0, "no model was opened");
    assert_eq!(after.history, 0);
    assert_eq!(harness.state().session.history_len(), history);
    assert_eq!(hidden(harness.state()), [false, false]);
}

#[test]
fn without_a_key_the_panel_explains_and_offers_no_input() {
    let backend = Scripted::without_key();
    let dir = tempfile::tempdir().unwrap();
    let mut harness = build(editor(&backend, dir.path()));
    harness.state_mut().open_agent();
    harness.run_steps(3);
    assert_eq!(view(&harness).stage, "no_key");
    assert!(!has(&harness, "agent.input") && !has(&harness, "agent.send"));
    assert!(!has(&harness, "agent.consent.accept"));
    assert_eq!(backend.opened(), 0);
}

#[test]
fn a_scripted_turn_shows_its_steps_and_the_card_and_discard_changes_nothing() {
    let backend = Scripted::new(vec![hide_first_turn()]);
    let dir = tempfile::tempdir().unwrap();
    let mut harness = build(editor(&backend, dir.path()));
    let before = harness.state().session.project().clone();
    // The fixture's own inserts are on the undo stack already.
    let history = harness.state().session.history_len();

    ask(&mut harness, "hide the first clip");
    wait_for(&mut harness, "the changes card", |view| {
        view.stage == "decide"
    });

    let shown = view(&harness);
    assert_eq!(
        shown.steps,
        [
            ("timeline_read".to_owned(), Some(true)),
            ("command_run".to_owned(), Some(true))
        ]
    );
    assert_eq!(shown.replies, ["Hid the first clip."]);
    assert_eq!(shown.changed, Some(1));
    assert!(shown.spent_micros > 0, "the ledger shows the spend");
    for id in ["agent.preview", "agent.apply", "agent.discard"] {
        assert!(has(&harness, id), "{id}");
    }
    assert!(
        node(&harness, "agent.send").accesskit_node().is_disabled(),
        "the next message waits for Apply or Discard"
    );
    assert_eq!(
        **harness.state().session.project(),
        *before,
        "the fork is not the project"
    );
    assert_eq!(harness.state().session.history_len(), history);

    click(&mut harness, "agent.discard");
    assert_eq!(view(&harness).stage, "ready");
    assert_eq!(**harness.state().session.project(), *before);
    assert_eq!(harness.state().session.history_len(), history);
    assert_eq!(hidden(harness.state()), [false, false]);
}

#[test]
fn preview_shows_the_fork_read_only_and_apply_is_one_undo_step() {
    let backend = Scripted::new(vec![hide_first_turn()]);
    let dir = tempfile::tempdir().unwrap();
    let mut harness = build(editor(&backend, dir.path()));
    let before = harness.state().session.project().clone();
    // The fixture's own inserts are on the undo stack already.
    let history = harness.state().session.history_len();

    ask(&mut harness, "hide the first clip");
    wait_for(&mut harness, "the changes card", |view| {
        view.stage == "decide"
    });

    assert!(harness.state().action_enabled("graphic.insert.rectangle"));
    click(&mut harness, "agent.preview");
    assert!(view(&harness).previewing);
    assert!(
        !harness.state().action_enabled("graphic.insert.rectangle"),
        "a previewed change is read only"
    );
    assert_eq!(
        **harness.state().session.project(),
        *before,
        "still the live project"
    );
    click(&mut harness, "agent.preview");
    assert!(!view(&harness).previewing);

    click(&mut harness, "agent.apply");
    assert_eq!(view(&harness).stage, "ready");
    assert_eq!(
        harness.state().session.history_len(),
        (history.0 + 1, 0),
        "one undo entry"
    );
    assert_eq!(
        hidden(harness.state())
            .iter()
            .filter(|hidden| **hidden)
            .count(),
        1
    );

    harness.state_mut().activate("edit.undo").unwrap();
    assert_eq!(
        **harness.state().session.project(),
        *before,
        "undo restores the project"
    );
    assert_eq!(harness.state().session.history_len(), (history.0, 1));
}

#[test]
fn stop_ends_a_running_turn_and_leaves_the_project_alone() {
    let (release, hold) = mpsc::channel::<()>();
    let held = ScriptedStep::Compose(Box::new(move |_: &[Message]| {
        let _ = hold.recv_timeout(Duration::from_secs(20));
        ScriptedReply {
            text: String::new(),
            calls: vec![call("r1", "timeline_read", json!({}))],
        }
    }));
    let backend = Scripted::new(vec![vec![held, say("never shown")]]);
    let dir = tempfile::tempdir().unwrap();
    let mut harness = build(editor(&backend, dir.path()));
    let before = harness.state().session.project().clone();
    // The fixture's own inserts are on the undo stack already.
    let history = harness.state().session.history_len();

    ask(&mut harness, "hide the first clip");
    wait_for(&mut harness, "the running turn", |view| {
        view.stage == "running"
    });
    assert!(has(&harness, "agent.stop"));
    assert!(!has(&harness, "agent.send"));

    click(&mut harness, "agent.stop");
    release.send(()).unwrap();
    wait_for(&mut harness, "the stopped turn", |view| {
        view.stage == "ready"
    });

    let shown = view(&harness);
    assert_eq!(shown.ended.as_deref(), Some("cancelled"));
    assert_eq!(shown.changed, None);
    assert_eq!(**harness.state().session.project(), *before);
    assert_eq!(harness.state().session.history_len(), history);
}

#[test]
fn a_live_edit_makes_apply_stale_and_retry_runs_the_request_again() {
    let mut script = hide_first_turn();
    script.extend(hide_first_turn());
    let backend = Scripted::new(vec![script]);
    let dir = tempfile::tempdir().unwrap();
    let mut harness = build(editor(&backend, dir.path()));

    ask(&mut harness, "hide the first clip");
    wait_for(&mut harness, "the changes card", |view| {
        view.stage == "decide"
    });

    harness
        .state_mut()
        .session
        .apply(engine::Command::RenameProject {
            name: "Edited meanwhile".into(),
        })
        .unwrap();
    click(&mut harness, "agent.apply");
    let shown = view(&harness);
    assert!(shown.stale && shown.stage == "decide", "{shown:?}");
    assert!(has(&harness, "agent.retry") && !has(&harness, "agent.apply"));
    assert_eq!(
        hidden(harness.state()),
        [false, false],
        "a stale apply changes nothing"
    );

    click(&mut harness, "agent.retry");
    wait_for(&mut harness, "the second card", |view| {
        view.stage == "decide" && !view.stale
    });
    click(&mut harness, "agent.apply");
    assert_eq!(view(&harness).stage, "ready");
    assert_eq!(
        hidden(harness.state())
            .iter()
            .filter(|hidden| **hidden)
            .count(),
        1
    );
}

#[test]
fn history_is_kept_per_project_and_clear_removes_it() {
    let backend = Scripted::new(vec![hide_first_turn()]);
    let dir = tempfile::tempdir().unwrap();
    let mut harness = build(editor(&backend, dir.path()));
    ask(&mut harness, "hide the first clip");
    wait_for(&mut harness, "the changes card", |view| {
        view.stage == "decide"
    });
    click(&mut harness, "agent.discard");
    let lines = view(&harness).history;
    assert!(
        lines >= 4,
        "the question, two steps, the answer and the decision: {lines}"
    );
    drop(harness);

    // A new editor on the same project reads the transcript back.
    let again = Scripted::new(vec![]);
    let mut harness = build(editor(&again, dir.path()));
    harness.state_mut().open_agent();
    harness.run_steps(3);
    assert_eq!(view(&harness).history, lines);

    harness.state_mut().open_agent();
    harness.run_steps(2);
    click(&mut harness, "agent.consent.accept");
    click(&mut harness, "agent.clear");
    assert_eq!(view(&harness).history, 0);
    drop(harness);

    let mut harness = build(editor(&again, dir.path()));
    harness.run_steps(2);
    assert_eq!(view(&harness).history, 0, "cleared for good");
}

#[test]
fn expand_takes_the_right_column_and_restore_docks_the_chat_again() {
    let backend = Scripted::new(vec![]);
    let dir = tempfile::tempdir().unwrap();
    let mut harness = build(editor(&backend, dir.path()));
    harness.state_mut().open_agent();
    harness.run_steps(3);
    assert!(!view(&harness).expanded);
    assert!(has(&harness, "agent.expand") && !has(&harness, "agent.restore"));
    assert!(
        node(&harness, "agent.expand")
            .label()
            .is_some_and(|label| label == "Expand")
    );
    let docked = harness.state().panels().clone();

    click(&mut harness, "agent.expand");
    harness.run_steps(2);
    assert!(view(&harness).expanded);
    assert!(has(&harness, "agent.restore") && !has(&harness, "agent.expand"));
    assert!(
        node(&harness, "agent.restore")
            .label()
            .is_some_and(|label| label == "Restore")
    );
    let wide = harness.state().panels().clone();
    // The chat runs from the header to the bottom of the body, in the measured 534 wide column.
    assert_eq!(wide["agent"].top(), wide["header"].bottom());
    assert_eq!(wide["agent"].bottom(), docked["timeline"].bottom());
    assert_eq!(wide["agent"].right(), docked["agent"].right());
    assert_eq!(wide["agent"].width(), 534.0);
    // Media, player, properties and timeline keep the rest, a 7 gutter away from the chat.
    for id in ["inspector", "timeline"] {
        assert_eq!(wide["agent"].left() - wide[id].right(), 7.0, "{id}");
    }
    assert!(wide["preview"].width() < docked["preview"].width());
    assert!(wide["timeline"].width() < docked["timeline"].width());
    assert_eq!(wide["timeline"].top(), docked["timeline"].top());
    assert_eq!(wide["timeline"].height(), docked["timeline"].height());
    assert_eq!(wide["media"].left(), docked["media"].left());
    assert_eq!(wide["media"].height(), docked["media"].height());

    // Closing and opening again keeps the layout the creator chose.
    click(&mut harness, "agent.close");
    assert!(!view(&harness).open && view(&harness).expanded);
    harness.state_mut().open_agent();
    harness.run_steps(3);
    assert_eq!(harness.state().panels()["agent"], wide["agent"]);

    click(&mut harness, "agent.restore");
    harness.run_steps(2);
    assert!(!view(&harness).expanded);
    assert_eq!(harness.state().panels()["agent"], docked["agent"]);
    assert_eq!(harness.state().panels()["timeline"], docked["timeline"]);
    assert_eq!(harness.state().panels()["preview"], docked["preview"]);
}
