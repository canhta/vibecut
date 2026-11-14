//! Earlier runs of the shorts window, kept as managed projects of the process library.
//!
//! The composition root lends a [`RunStore`] (the managed library behind the catalog worker);
//! the window owns no library handle. Every call runs on its own short thread so the UI never
//! waits on the filesystem, and its result comes back through [`Runs::drain`].
use super::state::{ClipCandidate, GenerationRequest};
use crate::egui;
use engine::control::Result;
use ai::text::clips::ClipLabel;
use project::{
    MAX_RUN_ENERGY, Project, ProjectId, ShortsClip, ShortsEnergy, ShortsLabel, ShortsPiece,
    ShortsRequest, ShortsRole, ShortsRun, ShortsSegment, ShortsSource, Thumbnail,
};
use std::{
    collections::BTreeMap,
    sync::{Arc, atomic::AtomicBool, mpsc},
};
use time::{SourceRange, Tick};

/// What the entry screen shows of one earlier run.
#[derive(Clone, Debug)]
pub struct RunEntry {
    pub id: ProjectId,
    pub source_name: String,
    pub clips: usize,
    /// The first short's frame, at most 128 pixels per edge.
    pub poster: Thumbnail,
    pub modified_unix_ms: u64,
}

/// The persistence of runs. The shipped one is the managed project library; tests inject
/// an in-memory store. Calls block.
pub trait RunStore: Send + Sync {
    /// Runs, most recently saved first.
    fn list(&self) -> Result<Vec<RunEntry>>;
    /// Keep a run: a project carrying its `shorts_run`, and the first short's poster.
    fn save(&self, project: Project, poster: Thumbnail) -> Result<ProjectId>;
    fn load(&self, id: ProjectId) -> Result<Project>;
    /// Forget a run. Exported files are never touched.
    fn remove(&self, id: ProjectId) -> Result<()>;
}

pub(super) enum Event {
    Listed(Result<Vec<RunEntry>>),
    Saved(Result<ProjectId>),
    Loaded(Result<Box<Project>>),
    Removed(Result<()>),
}

pub(super) struct Runs {
    store: Option<Arc<dyn RunStore>>,
    send: mpsc::Sender<Event>,
    receive: mpsc::Receiver<Event>,
    pub(super) entries: Vec<RunEntry>,
    /// Uploaded posters by run, created on first paint.
    pub(super) textures: BTreeMap<ProjectId, egui::TextureHandle>,
    loading: bool,
}

impl Default for Runs {
    fn default() -> Self {
        let (send, receive) = mpsc::channel();
        Self {
            store: None,
            send,
            receive,
            entries: Vec::new(),
            textures: BTreeMap::new(),
            loading: false,
        }
    }
}

impl Runs {
    pub(super) fn lend(&mut self, store: Arc<dyn RunStore>) {
        self.store = Some(store);
        self.list();
    }
    pub(super) fn lent(&self) -> bool {
        self.store.is_some()
    }
    pub(super) fn loading(&self) -> bool {
        self.loading
    }
    fn spawn(&mut self, work: impl FnOnce(&dyn RunStore, &mpsc::Sender<Event>) + Send + 'static) {
        let Some(store) = self.store.clone() else {
            return;
        };
        let send = self.send.clone();
        // A thread that cannot start leaves the entry screen as it was.
        let started = std::thread::Builder::new()
            .name("shorts-runs".into())
            .spawn(move || work(store.as_ref(), &send));
        if started.is_err() {
            self.loading = false;
        }
    }
    pub(super) fn list(&mut self) {
        self.spawn(|store, send| {
            let _ = send.send(Event::Listed(store.list()));
        });
    }
    /// Save a finished run, then refresh the listing. The poster is decoded here, off the UI
    /// thread; a source that cannot give a frame saves the run without one.
    pub(super) fn save(&mut self, project: Project, asset: media::SourceAsset, at: Tick) {
        self.spawn(move |store, send| {
            let poster = poster_of(&asset, at);
            let saved = store.save(project, poster);
            let ok = saved.is_ok();
            let _ = send.send(Event::Saved(saved));
            if ok {
                let _ = send.send(Event::Listed(store.list()));
            }
        });
    }
    pub(super) fn load(&mut self, id: ProjectId) {
        self.loading = true;
        self.spawn(move |store, send| {
            let _ = send.send(Event::Loaded(store.load(id).map(Box::new)));
        });
    }
    pub(super) fn remove(&mut self, id: ProjectId) {
        self.spawn(move |store, send| {
            let removed = store.remove(id);
            let ok = removed.is_ok();
            let _ = send.send(Event::Removed(removed));
            if ok {
                let _ = send.send(Event::Listed(store.list()));
            }
        });
    }
    /// Finished calls, oldest first. Listings update the entries here.
    pub(super) fn drain(&mut self) -> Vec<Event> {
        let mut events = Vec::new();
        while let Ok(event) = self.receive.try_recv() {
            if matches!(event, Event::Loaded(_)) {
                self.loading = false;
            }
            if let Event::Listed(Ok(entries)) = &event {
                self.textures
                    .retain(|id, _| entries.iter().any(|entry| entry.id == *id));
                self.entries = entries.clone();
            }
            events.push(event);
        }
        events
    }
}

/// The frame the entry card shows: the first short's first frame, scaled to 128 pixels.
fn poster_of(asset: &media::SourceAsset, at: Tick) -> Thumbnail {
    match media::thumbnail(asset, at, 128, Arc::new(AtomicBool::new(false))) {
        Ok(frame) => Thumbnail::Ready {
            width: frame.width,
            height: frame.height,
            rgba: frame.pixels,
        },
        Err(_) => Thumbnail::Unavailable,
    }
}

fn role(role: engine::ClipRole) -> ShortsRole {
    match role {
        engine::ClipRole::Hook => ShortsRole::Hook,
        engine::ClipRole::Context => ShortsRole::Context,
        engine::ClipRole::Payoff => ShortsRole::Payoff,
    }
}
fn role_back(role: ShortsRole) -> engine::ClipRole {
    match role {
        ShortsRole::Hook => engine::ClipRole::Hook,
        ShortsRole::Context => engine::ClipRole::Context,
        ShortsRole::Payoff => engine::ClipRole::Payoff,
    }
}
fn label(label: ClipLabel) -> ShortsLabel {
    match label {
        ClipLabel::Hook => ShortsLabel::Hook,
        ClipLabel::OnTopic => ShortsLabel::OnTopic,
        ClipLabel::CleanStart => ShortsLabel::CleanStart,
        ClipLabel::Standalone => ShortsLabel::Standalone,
    }
}
fn label_back(label: ShortsLabel) -> ClipLabel {
    match label {
        ShortsLabel::Hook => ClipLabel::Hook,
        ShortsLabel::OnTopic => ClipLabel::OnTopic,
        ShortsLabel::CleanStart => ClipLabel::CleanStart,
        ShortsLabel::Standalone => ClipLabel::Standalone,
    }
}

/// The persisted run of one finished suggestion. Speech-energy windows beyond the persisted
/// limit merge in equal groups (spanning range, peak RMS), as the suggestion result does.
pub(super) fn to_run(
    request: &GenerationRequest,
    source_name: captions::NfcText,
    asset: media::AssetId,
    language: captions::LanguageTag,
    candidates: &[ClipCandidate],
    energy: &[media::SpeechEnergyWindow],
) -> ShortsRun {
    let group = energy.len().div_ceil(MAX_RUN_ENERGY).max(1);
    ShortsRun {
        asset,
        source_name,
        source_language: language,
        transcription_provider: request.transcription_provider.clone(),
        transcription_model: request.transcription_model.clone(),
        request: ShortsRequest {
            language: request.language.clone(),
            transcription_provider: request.transcription_provider.clone(),
            transcription_model: request.transcription_model.clone(),
            suggestion_provider: request.suggestion_provider.clone(),
            suggestion_model: request.suggestion_model.clone(),
            count: u32::try_from(request.count).unwrap_or(u32::MAX),
            min_seconds: request.min_seconds,
            max_seconds: request.max_seconds,
            max_segments: u32::try_from(request.max_segments).unwrap_or(u32::MAX),
        },
        clips: candidates
            .iter()
            .map(|candidate| ShortsClip {
                title: candidate.proposal.title.clone(),
                hook_text: candidate.proposal.hook_text.clone(),
                reason: candidate.proposal.reason.clone(),
                score: candidate.proposal.score,
                source: match candidate.source {
                    engine::ClipSource::Llm => ShortsSource::Llm,
                    engine::ClipSource::Heuristic => ShortsSource::Heuristic,
                },
                label: label(candidate.label),
                duration: candidate.duration,
                segments: candidate
                    .segments
                    .iter()
                    .map(|segment| ShortsSegment {
                        range: segment.range,
                        role: role(segment.role),
                        source_order: u32::try_from(segment.source_order).unwrap_or(u32::MAX),
                    })
                    .collect(),
                pieces: candidate
                    .proposal
                    .segments
                    .iter()
                    .map(|piece| ShortsPiece {
                        start_segment_id: piece.start_segment_id.clone(),
                        end_segment_id: piece.end_segment_id.clone(),
                        role: role(piece.role),
                    })
                    .collect(),
            })
            .collect(),
        energy: energy
            .chunks(group)
            .filter_map(|windows| {
                let range =
                    SourceRange::new(windows.first()?.range.start(), windows.last()?.range.end())
                        .ok()?;
                let rms = windows
                    .iter()
                    .map(|window| window.rms as f32)
                    .fold(0.0, f32::max);
                Some(ShortsEnergy { range, rms })
            })
            .collect(),
    }
}

/// The review's candidates and speech energy of a persisted run.
pub(super) fn from_run(run: &ShortsRun) -> (Vec<ClipCandidate>, Vec<media::SpeechEnergyWindow>) {
    let candidates = run
        .clips
        .iter()
        .map(|clip| ClipCandidate {
            proposal: engine::ClipProposal {
                segments: clip
                    .pieces
                    .iter()
                    .map(|piece| ai::text::clips::ClipPiece {
                        start_segment_id: piece.start_segment_id.clone(),
                        end_segment_id: piece.end_segment_id.clone(),
                        role: role_back(piece.role),
                    })
                    .collect(),
                title: clip.title.clone(),
                hook_text: clip.hook_text.clone(),
                reason: clip.reason.clone(),
                score: clip.score,
            },
            segments: clip
                .segments
                .iter()
                .map(|segment| engine::ClipSourceSegment {
                    range: segment.range,
                    role: role_back(segment.role),
                    source_order: segment.source_order as usize,
                    repair: None,
                })
                .collect(),
            adjustments: Vec::new(),
            duration: clip.duration,
            label: label_back(clip.label),
            source: match clip.source {
                ShortsSource::Llm => engine::ClipSource::Llm,
                ShortsSource::Heuristic => engine::ClipSource::Heuristic,
            },
        })
        .collect();
    let energy = run
        .energy
        .iter()
        .map(|window| media::SpeechEnergyWindow {
            range: window.range,
            rms: f64::from(window.rms),
        })
        .collect();
    (candidates, energy)
}

/// The source-timed cues of a run project: its caption item's cue track.
pub(super) fn cues_of(project: &Project) -> Vec<captions::Cue> {
    project
        .data()
        .scenes
        .iter()
        .flat_map(|scene| scene.timeline.iter())
        .flat_map(|track| track.items.iter())
        .find_map(|item| match &item.content {
            project::ItemParams::Caption(params) => Some(params.cues.cues.clone()),
            _ => None,
        })
        .unwrap_or_default()
}
