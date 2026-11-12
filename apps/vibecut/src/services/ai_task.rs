//! Bounded progress and exact per-upload consent shared by desktop AI workflows.
use engine::providers::{ConsentPrompt, UploadRequest};
use engine::transcript::{Cancellation, Progress, Stage};
use std::{
    sync::{Mutex, mpsc},
    time::Duration,
};
pub(crate) struct Upload {
    pub(crate) request: UploadRequest,
    pub(crate) price_acknowledged: bool,
    pub(crate) reply: mpsc::SyncSender<bool>,
}
#[derive(Default)]
pub(crate) struct EventState {
    pub(crate) stage: Option<Stage>,
    pub(crate) fraction: Option<f32>,
    pub(crate) upload: Option<Upload>,
    acknowledged: Vec<(ai::ProviderConfig, engine::providers::CatalogOperation)>,
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
impl TaskEvents {
    pub(crate) fn answer(&self, approved: bool) -> crate::Result<()> {
        if self.cancel.cancelled() {
            return Err(crate::Error::Cancelled);
        }
        let mut state = self.state.lock().map_err(|_| crate::Error::Poisoned)?;
        let upload = state
            .upload
            .as_ref()
            .ok_or(crate::Error::Invalid("no upload request"))?;
        if approved
            && matches!(upload.request.cost, engine::providers::Cost::Unknown)
            && !upload.price_acknowledged
        {
            return Err(ai::Error::Cost.into());
        }
        let upload = state
            .upload
            .take()
            .ok_or(crate::Error::Invalid("no upload request"))?;
        if approved && matches!(upload.request.cost, engine::providers::Cost::Unknown) {
            let identity = (upload.request.config.clone(), upload.request.operation);
            if !state.acknowledged.contains(&identity) {
                if state.acknowledged.len() == 16 {
                    state.acknowledged.remove(0);
                }
                state.acknowledged.push(identity);
            }
        }
        upload
            .reply
            .send(approved)
            .map_err(|_| crate::Error::Cancelled)
    }
    fn request(&self, request: &UploadRequest) -> engine::providers::Result<()> {
        let (send, receive) = mpsc::sync_channel(1);
        let mut state = self
            .state
            .lock()
            .map_err(|_| engine::RuntimeError::Poisoned)?;
        let price_acknowledged = state
            .acknowledged
            .contains(&(request.config.clone(), request.operation));
        state.upload = Some(Upload {
            request: request.clone(),
            price_acknowledged,
            reply: send,
        });
        drop(state);
        loop {
            if self.cancel.cancelled() {
                return Err(engine::providers::Error::Runtime(
                    engine::RuntimeError::Cancelled,
                ));
            }
            match receive.recv_timeout(Duration::from_millis(20)) {
                Ok(true) => return Ok(()),
                Ok(false) | Err(mpsc::RecvTimeoutError::Disconnected) => {
                    return Err(engine::providers::Error::ConsentRequired);
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
            }
        }
    }
}

impl ConsentPrompt for TaskEvents {
    fn approve(&self, request: &UploadRequest) -> engine::providers::Result<()> {
        self.request(request)
    }
    fn approve_unknown_price(&self, request: &UploadRequest) -> engine::providers::Result<()> {
        self.request(request)
    }
}
#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::panic)]
    use super::*;
    use engine::providers::{CatalogOperation, Cost};
    use std::sync::Arc;
    fn upload(model: &str, operation: CatalogOperation) -> UploadRequest {
        UploadRequest {
            config: ai::chapters::OpenAiChapterGenerator::provider_config(model).unwrap(),
            operation,
            provider: "OpenAI".into(),
            model: model.into(),
            languages: "en".into(),
            audio_seconds: 0.0,
            upload_bytes: 10,
            cost: Cost::Unknown,
            terms: vec![],
        }
    }
    fn pending(events: &TaskEvents) {
        let deadline = std::time::Instant::now() + Duration::from_secs(3);
        loop {
            if events.state.lock().unwrap().upload.is_some() {
                return;
            }
            assert!(std::time::Instant::now() < deadline);
            std::thread::yield_now();
        }
    }
    #[test]
    fn unknown_price_checkbox_is_required_and_reused_only_for_exact_model_and_job_kind() {
        let events = Arc::new(TaskEvents::new(Cancellation::new()));
        for (model, operation, expected) in [
            ("fixture-a", CatalogOperation::GenerateChapters, false),
            ("fixture-a", CatalogOperation::GenerateChapters, true),
            ("fixture-b", CatalogOperation::GenerateChapters, false),
            ("fixture-b", CatalogOperation::ClassifyFillers, false),
        ] {
            let request = upload(model, operation);
            let task = events.clone();
            let worker = std::thread::spawn(move || task.approve_unknown_price(&request));
            pending(&events);
            assert_eq!(
                events
                    .state
                    .lock()
                    .unwrap()
                    .upload
                    .as_ref()
                    .unwrap()
                    .price_acknowledged,
                expected
            );
            if !expected {
                assert!(events.answer(true).is_err());
                events
                    .state
                    .lock()
                    .unwrap()
                    .upload
                    .as_mut()
                    .unwrap()
                    .price_acknowledged = true;
            }
            events.answer(true).unwrap();
            worker.join().unwrap().unwrap();
        }
        for disconnected in [false, true] {
            let denied = Arc::new(TaskEvents::new(Cancellation::new()));
            let task = denied.clone();
            let worker = std::thread::spawn(move || {
                task.approve(&upload(
                    "fixture-denied",
                    CatalogOperation::GenerateChapters,
                ))
            });
            pending(&denied);
            if disconnected {
                denied.state.lock().unwrap().upload.take();
            } else {
                denied.answer(false).unwrap();
            }
            assert!(matches!(
                worker.join().unwrap(),
                Err(engine::providers::Error::ConsentRequired)
            ));
        }
        let cancelled = Arc::new(TaskEvents::new(Cancellation::new()));
        let task = cancelled.clone();
        let worker = std::thread::spawn(move || {
            task.approve_unknown_price(&upload("fixture-a", CatalogOperation::GenerateChapters))
        });
        pending(&cancelled);
        cancelled.cancel.cancel();
        assert!(cancelled.answer(true).is_err());
        assert!(matches!(
            worker.join().unwrap(),
            Err(engine::providers::Error::Runtime(
                engine::RuntimeError::Cancelled
            ))
        ));
    }
}
