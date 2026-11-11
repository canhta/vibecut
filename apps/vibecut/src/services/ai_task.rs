//! Bounded progress and exact per-upload consent shared by desktop AI workflows.
use engine::providers::{ConsentPrompt, UploadRequest};
use engine::transcript::{Cancellation, Progress, Stage};
use std::{
    sync::{Mutex, mpsc},
    time::Duration,
};
pub(crate) struct Upload {
    pub(crate) request: UploadRequest,
    pub(crate) reply: mpsc::SyncSender<bool>,
}
#[derive(Default)]
pub(crate) struct EventState {
    pub(crate) stage: Option<Stage>,
    pub(crate) fraction: Option<f32>,
    pub(crate) upload: Option<Upload>,
}
/// One coalescing progress slot and one consent request. No unbounded event queue.
pub struct TaskEvents {
    pub(crate) state: Mutex<EventState>,
    cancel: Cancellation,
}
impl TaskEvents {
    pub(crate) fn new(cancel: Cancellation) -> Self {
        Self {
            state: Mutex::new(EventState::default()),
            cancel,
        }
    }
}
impl Progress for TaskEvents {
    fn stage(&self, stage: Stage) {
        if let Ok(mut state) = self.state.lock() {
            state.stage = Some(stage);
            state.fraction = None;
        }
    }
    fn job(&self, snapshot: &engine::jobs::JobSnapshot) {
        if let Ok(mut state) = self.state.lock() {
            state.fraction = Some(snapshot.progress);
        }
    }
}
impl ConsentPrompt for TaskEvents {
    fn approve(&self, request: &UploadRequest) -> std::result::Result<(), engine::EditError> {
        let (send, receive) = mpsc::sync_channel(1);
        self.state
            .lock()
            .map_err(|_| engine::RuntimeError::Poisoned)?
            .upload = Some(Upload {
            request: request.clone(),
            reply: send,
        });
        loop {
            if self.cancel.cancelled() {
                return Err(engine::EditError::Cancelled);
            }
            match receive.recv_timeout(Duration::from_millis(20)) {
                Ok(true) => return Ok(()),
                Ok(false) | Err(mpsc::RecvTimeoutError::Disconnected) => {
                    return Err(engine::providers::Error::ConsentRequired.into());
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
            }
        }
    }
}
