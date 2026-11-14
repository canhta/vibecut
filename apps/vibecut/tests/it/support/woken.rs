//! The app loop as the native shell runs it: a pass only when something woke it.
//!
//! The editor keeps no repaint timer. A worker that publishes a result must wake the loop, or
//! the result stays unseen until the next input. A test that polls in a loop of its own hides
//! that hole; these helpers run a pass only after a wake, so a result published without one
//! fails the test.
#![allow(clippy::panic)]
use std::{
    sync::mpsc,
    time::{Duration, Instant},
};
use vibecut::{Editor, egui};

/// How long the loop may sleep with no wake before the test fails.
const QUIET: Duration = Duration::from_secs(10);
/// How long one wait may run in all.
const LIMIT: Duration = Duration::from_secs(60);

/// A loop that runs passes only when woken.
pub struct Woken {
    wake: wake::Wake,
    send: mpsc::Sender<Duration>,
    asks: mpsc::Receiver<Duration>,
}

impl Woken {
    /// Bind a fresh wake to this thread, as the app does before it starts a worker. Bind before
    /// building the editor or playback: a worker keeps the wake it was started under. Until
    /// [`Woken::attach`], each wake asks this loop for a pass directly.
    pub fn bind() -> Self {
        let (send, asks) = mpsc::channel();
        let wake = wake::Wake::new();
        let direct = send.clone();
        wake.set(move |delay| {
            let _ = direct.send(delay);
        });
        wake.bind();
        Self { wake, send, asks }
    }

    /// Run the loop of `ctx`, as the native shell does: a wake becomes a repaint request of the
    /// root viewport, and only the requests egui passes on to its integration ask for a pass,
    /// the UI's own requests included. Every pass must then run one `ctx` frame.
    pub fn attach(&self, ctx: &egui::Context) {
        let target = ctx.clone();
        self.wake.set(move |delay| {
            target.request_repaint_after_for(delay, egui::ViewportId::ROOT);
        });
        let ask = self.send.clone();
        ctx.set_request_repaint_callback(move |info| {
            let _ = ask.send(info.delay);
        });
    }

    /// Run `pass` now and then once after each wake, after the delay the wake asked for, until
    /// it returns a value. Fails when nothing wakes the loop while `pass` still waits.
    #[track_caller]
    pub fn until<T>(&self, what: &str, mut pass: impl FnMut() -> Option<T>) -> T {
        let limit = Instant::now() + LIMIT;
        loop {
            if let Some(done) = pass() {
                return done;
            }
            let delay = self
                .asks
                .recv_timeout(QUIET)
                .unwrap_or_else(|_| panic!("{what}: nothing woke the loop in {QUIET:?}"));
            // A sooner wake that arrives meanwhile cuts a delayed one short, as in egui.
            let mut due = Instant::now() + delay.min(QUIET);
            while let Ok(sooner) = self
                .asks
                .recv_timeout(due.saturating_duration_since(Instant::now()))
            {
                due = due.min(Instant::now() + sooner.min(QUIET));
            }
            assert!(
                Instant::now() < limit,
                "{what}: still waiting after {LIMIT:?}"
            );
        }
    }

    /// [`Woken::until`] over an attached kittest harness: each pass is one frame.
    #[track_caller]
    pub fn step_until<S>(
        &self,
        harness: &mut egui_kittest::Harness<'_, S>,
        what: &str,
        mut done: impl FnMut(&S) -> bool,
    ) {
        self.until(what, || {
            harness.step();
            done(harness.state()).then_some(())
        });
    }
}

/// The editor's last frame registered the widget `id`.
pub fn shows(editor: &Editor, id: &str) -> bool {
    editor
        .inspect()
        .unwrap_or_else(|error| panic!("inspect: {error:?}"))
        .widgets
        .iter()
        .any(|widget| widget.id == id)
}
