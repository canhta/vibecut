//! Inserts bundled stickers from the Stickers tab and keeps the recent list.

use super::super::stickers::{self, Active, DownloadProblem, StickerDownloads};
use super::super::{CreatedId, TICKS_PER_SECOND, Text, Tick};
use super::State;
use crate::editor::Port;
use crate::i18n;
use engine::jobs::JobState;
use engine::{ByteBudget, Failure, Placement};
use libraries::stickers::Recents;
use std::collections::BTreeSet;

/// Stickers take the app's five-second insert length, as graphics and text do.
const STICKER_DURATION: Tick = Tick(5 * TICKS_PER_SECOND);

impl State {
    /// Insert one bundled sticker at the playhead through the same catalog edit `stickers.insert`
    /// applies, select it and move it to the front of the recent list. A refused edit records
    /// nothing.
    pub(in crate::editor) fn insert_sticker(&mut self, port: &mut Port<'_>, sticker_id: &str) {
        if !port.read.insertion_available {
            return;
        }
        let at = match port.playhead() {
            Ok(at) => at,
            Err(_error) => {
                tracing::warn!(category = "asset_position", "insertion unavailable");
                let text = i18n::text(port.locale(), Text::InsertFailed);
                port.toast(text, None);
                return;
            }
        };
        let outcome = libraries::stickers::insert_sticker(
            &mut *port.session,
            libraries::stickers::native_store(),
            sticker_id,
            Placement::Auto,
            at,
            STICKER_DURATION,
        );
        let created = outcome.ok().and_then(|outcome| {
            outcome.created.into_iter().find_map(|id| match id {
                CreatedId::Item(item) => Some(item),
                _ => None,
            })
        });
        let Some(item) = created else {
            tracing::warn!(category = "sticker_insert", "insertion failed");
            let text = i18n::text(port.locale(), Text::InsertFailed);
            port.toast(text, None);
            return;
        };
        port.select(BTreeSet::from([item]));
        port.reset_selection(true);
        self.remember_sticker(port, sticker_id);
    }

    /// Move one inserted id to the front of the saved recents. Stale saved ids drop out here.
    pub(in crate::editor) fn remember_sticker(&mut self, port: &mut Port<'_>, id: &str) {
        let saved = port.read.preferences.sticker_recents.clone();
        let mut recents = Recents::from_ids(stickers::known_ids(&saved));
        recents.record(id);
        port.store_preference("sticker_recents", serde_json::json!(recents.ids()));
    }

    /// Empty the saved recent list. Projects are untouched.
    pub(in crate::editor) fn clear_sticker_recents(&mut self, port: &mut Port<'_>) {
        port.store_preference("sticker_recents", serde_json::json!([]));
    }

    /// The sticker catalogue the tab reads: the injected store when one is set, else the per-user
    /// store.
    pub(in crate::editor) fn sticker_catalogue(
        &self,
    ) -> Option<&libraries::stickers::StickerStore> {
        self.sticker_store
            .as_deref()
            .or_else(libraries::stickers::native_store)
    }

    /// Start the explicit download of one collection. The card shows its licence credit before
    /// the button, so the button is the acknowledgement. One download runs at a time.
    pub(in crate::editor) fn download_sticker(&mut self, budget: &ByteBudget, collection: &str) {
        if self.sticker_downloads.is_none() {
            match StickerDownloads::new(budget.clone()) {
                Ok(downloads) => self.sticker_downloads = Some(downloads),
                Err(_) => {
                    tracing::warn!(category = "sticker_download", "download runner unavailable");
                    return;
                }
            }
        }
        let Some(downloads) = self.sticker_downloads.as_mut() else {
            return;
        };
        if downloads.active.is_some() {
            return;
        }
        downloads.problem = None;
        let injected = self.sticker_store.clone();
        let Some(store) = injected
            .as_deref()
            .or_else(libraries::stickers::native_store)
        else {
            if let Some(downloads) = self.sticker_downloads.as_mut() {
                downloads.problem = Some((collection.to_owned(), DownloadProblem::Failed));
            }
            return;
        };
        let Some(downloads) = self.sticker_downloads.as_mut() else {
            return;
        };
        match store.submit_download(&mut downloads.runner, collection, true) {
            Ok((job, cancel)) => {
                downloads.active = Some(Active {
                    collection: collection.to_owned(),
                    job,
                    cancel,
                    progress: 0.0,
                    cancelling: false,
                });
            }
            Err(error) => {
                let problem = if libraries::stickers::is_offline(&error) {
                    DownloadProblem::Offline
                } else {
                    DownloadProblem::Failed
                };
                downloads.problem = Some((collection.to_owned(), problem));
            }
        }
    }

    /// Ask the running collection download to stop. It leaves no cache entry.
    pub(in crate::editor) fn cancel_sticker_download(&mut self) {
        let Some(active) = self
            .sticker_downloads
            .as_mut()
            .and_then(|downloads| downloads.active.as_mut())
        else {
            return;
        };
        active.cancel.cancel();
        active.cancelling = true;
    }

    /// Pump the running download and record how it ended: a finished collection clears the
    /// card, a transport failure shows the offline state and any other failure the retry state.
    pub(in crate::editor) fn poll_sticker_download(&mut self) {
        let Some(downloads) = self.sticker_downloads.as_mut() else {
            return;
        };
        let Some(active) = downloads.active.as_ref() else {
            return;
        };
        let (job, collection) = (active.job, active.collection.clone());
        if downloads.runner.tick().is_err() {
            downloads.active = None;
            downloads.problem = Some((collection, DownloadProblem::Failed));
            return;
        }
        let Some(snapshot) = downloads.runner.snapshot(job) else {
            downloads.active = None;
            downloads.problem = Some((collection, DownloadProblem::Failed));
            return;
        };
        let ended = match snapshot.state {
            JobState::Completed | JobState::Cancelled => None,
            JobState::Failed => {
                let offline = snapshot
                    .error
                    .as_ref()
                    .is_some_and(|error| error.transient());
                Some(if offline {
                    DownloadProblem::Offline
                } else {
                    DownloadProblem::Failed
                })
            }
            _ => {
                if let Some(active) = downloads.active.as_mut() {
                    active.progress = snapshot.progress;
                }
                return;
            }
        };
        downloads.active = None;
        downloads.problem = ended.map(|problem| (collection, problem));
    }
}
