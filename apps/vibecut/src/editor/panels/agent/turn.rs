//! Runs one agent turn on a worker thread and hands its events back to the panel.
use crate::egui;
use agent::{AgentEvent, CancelFlag, ForkTurnRequest, Turn, TurnLimits, run_turn_on_fork};
use ai::text::conversation::Conversation;
use engine::{
    ByteBudget, Fork,
    jobs::{JobRunner, RunnerConfig, SystemClock},
};
use std::{
    sync::{
        Arc,
        mpsc::{self, Receiver, TryRecvError},
    },
    thread::JoinHandle,
    time::Duration,
};

/// What the worker sends the panel.
pub(super) enum Message {
    Event(AgentEvent),
    Done(Box<Done>),
}

/// The end of a turn: the turn when it started, and the conversation either way.
pub(super) struct Done {
    pub(super) turn: Option<Turn>,
    pub(super) conversation: Conversation,
}

/// A turn in flight. Dropping it cancels the turn and waits for the worker.
pub(super) struct Running {
    events: Receiver<Message>,
    cancel: CancelFlag,
    worker: Option<JoinHandle<()>>,
}

impl Running {
    /// Ask the turn to stop; it ends with `EndReason::Cancelled` and keeps its fork.
    pub(super) fn cancel(&self) {
        self.cancel.cancel();
    }

    /// The next message, if one is ready. A worker that vanished without reporting counts
    /// as a turn that never started.
    pub(super) fn poll(&mut self) -> Option<PollResult> {
        match self.events.try_recv() {
            Ok(message) => Some(PollResult::Message(message)),
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => Some(PollResult::Lost),
        }
    }
}

pub(super) enum PollResult {
    Message(Message),
    Lost,
}

impl Drop for Running {
    fn drop(&mut self) {
        self.cancel.cancel();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

const RUNNER: RunnerConfig = RunnerConfig {
    concurrency: [1, 1, 2],
    queue: 8,
    retained: 8,
    network_attempts: 3,
    backoff: Duration::from_millis(250),
    progress_interval: Duration::from_millis(50),
};

/// Start a turn on `fork` with `prompt`. `None` means the thread could not start and
/// nothing was sent.
pub(super) fn spawn(
    fork: Fork,
    prompt: String,
    mut conversation: Conversation,
    budget: ByteBudget,
    ctx: Option<egui::Context>,
) -> Option<Running> {
    let (sender, events) = mpsc::channel();
    let cancel = CancelFlag::new();
    let flag = cancel.clone();
    let repaint = move |ctx: &Option<egui::Context>| {
        if let Some(ctx) = ctx {
            ctx.request_repaint();
        }
    };
    let worker = std::thread::Builder::new()
        .name("agent-turn".to_owned())
        .spawn(move || {
            let turn = match JobRunner::new(RUNNER, budget, Arc::new(SystemClock::default())) {
                Ok(mut runner) => {
                    let turn = run_turn_on_fork(
                        ForkTurnRequest {
                            fork,
                            prompt: &prompt,
                            conversation: &mut conversation,
                            runner: &mut runner,
                            limits: TurnLimits::default(),
                            cancel: flag,
                        },
                        &mut |event| {
                            let _ = sender.send(Message::Event(event));
                            repaint(&ctx);
                        },
                    )
                    .ok();
                    runner.shutdown();
                    turn
                }
                Err(_) => None,
            };
            let _ = sender.send(Message::Done(Box::new(Done { turn, conversation })));
            repaint(&ctx);
        })
        .ok()?;
    Some(Running {
        events,
        cancel,
        worker: Some(worker),
    })
}
