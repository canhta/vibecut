//! Owns the Brand assets page's work on the lent brand kit store.
//!
//! Both are registry commands (`brand.list`, `brand.copy_media`) sent through
//! `services::assets::brand`; a listing hashes every kit file and a copy reads a whole file, so
//! both run on a thread the frame polls. A click copies the kit file into the project's media
//! folder and hands that copy to the paste import, which inserts it at the playhead the click
//! saw: the project gets its own file and never names the kit's.
use crate::editor::Port;
use crate::editor::panels::assets::{AssetsTab, MediaSection, State};
use crate::i18n::Text;
use crate::services::assets::brand::{self, run_brand};
use engine::control::{BrandKitView, ControlResult};
use libraries::brand_kit::BrandKit;
use std::{
    path::PathBuf,
    sync::{
        Arc,
        mpsc::{Receiver, TryRecvError},
    },
};
use time::Tick;

/// What the page shows of the active kit.
pub(in crate::editor) enum Listing {
    Loading,
    /// The active kit, or `None` when no kit is active yet.
    Ready(Option<BrandKitView>),
    Failed,
}

/// The lent store and the work the page started on it.
pub(in crate::editor) struct BrandAssets {
    store: Arc<BrandKit>,
    pub(in crate::editor) listing: Listing,
    load: Option<Receiver<Result<Option<BrandKitView>, String>>>,
    /// The copy a click started and the playhead its file inserts at.
    copy: Option<(Receiver<Result<PathBuf, String>>, Tick)>,
    started: bool,
}

impl BrandAssets {
    fn new(store: Arc<BrandKit>) -> Self {
        Self {
            store,
            listing: Listing::Loading,
            load: None,
            copy: None,
            started: false,
        }
    }

    /// Read the active kit again through `brand.list`, off the UI thread.
    pub(in crate::editor) fn refresh(&mut self, budget: &engine::ByteBudget) {
        let (store, budget) = (self.store.clone(), budget.clone());
        let (send, receive) = wake::channel();
        std::thread::spawn(move || {
            let listed = run_brand(&store, &budget, "brand.list", serde_json::json!({}))
                .and_then(brand::kit)
                .map_err(|error| error.to_string());
            let _ = send.send(listed);
        });
        self.load = Some(receive);
        self.listing = Listing::Loading;
        self.started = true;
    }

    /// A clicked file is still being copied into the project.
    pub(in crate::editor) fn copying(&self) -> bool {
        self.copy.is_some()
    }
}

impl State {
    /// Lend the user's brand kits; the page lists them the first time it shows.
    pub(in crate::editor) fn set_brand_kit(&mut self, store: Arc<BrandKit>) {
        self.brand = Some(BrandAssets::new(store));
    }

    /// Start the first listing once the page shows, then apply a finished listing or copy.
    pub(in crate::editor) fn poll_brand(&mut self, port: &mut Port<'_>) {
        let shown = port.read.preferences.assets_tab == AssetsTab::Media
            && self.media_section == MediaSection::BrandAssets;
        let Some(brand) = &mut self.brand else {
            return;
        };
        if shown && !brand.started {
            brand.refresh(port.read.budget);
        }
        if let Some(load) = &brand.load {
            match load.try_recv() {
                Ok(Ok(view)) => {
                    brand.listing = Listing::Ready(view);
                    brand.load = None;
                }
                Ok(Err(_)) | Err(TryRecvError::Disconnected) => {
                    brand.listing = Listing::Failed;
                    brand.load = None;
                }
                Err(TryRecvError::Empty) => {}
            }
        }
        let copied = match &brand.copy {
            Some((copy, at)) => match copy.try_recv() {
                Ok(result) => Some((result, *at)),
                Err(TryRecvError::Disconnected) => Some((Err(String::new()), *at)),
                Err(TryRecvError::Empty) => None,
            },
            None => None,
        };
        let Some((result, at)) = copied else {
            return;
        };
        brand.copy = None;
        let started = result
            .map_err(|detail| (!detail.is_empty()).then_some(detail))
            .and_then(|path| {
                self.paste_files(port, vec![path], at)
                    .map_err(|error| Some(error.to_string()))
            });
        if let Err(detail) = started {
            let text = port.tr(Text::BrandAssetsInsertFailed);
            port.toast(text, detail);
        }
    }

    /// Copy one kit file into the project's media folder; when the copy lands, it is imported
    /// and inserted at the current playhead. A file the kit reports missing or damaged, or a
    /// copy already running, refuses with a toast.
    pub(in crate::editor) fn insert_brand_media(&mut self, port: &mut Port<'_>, media: u64) {
        let importing = self.importing();
        let started = (|| -> crate::Result<()> {
            let brand = self
                .brand
                .as_mut()
                .ok_or(crate::Error::Invalid("brand kit unavailable"))?;
            if brand.copying() || importing {
                return Err(crate::Error::Invalid("brand asset insert in progress"));
            }
            let Listing::Ready(Some(kit)) = &brand.listing else {
                return Err(crate::Error::Invalid("brand kit not listed"));
            };
            if !kit.media.iter().any(|entry| {
                entry.id == media && entry.state == engine::control::BrandEntryState::Available
            }) {
                return Err(crate::Error::Invalid("brand asset unavailable"));
            }
            let at = port.playhead()?;
            let directory = project::media_dir(port.session.saved_path());
            let (store, budget) = (brand.store.clone(), port.read.budget.clone());
            let (send, receive) = wake::channel();
            std::thread::spawn(move || {
                let params = serde_json::json!({"media": media, "dir": directory});
                let copied = match run_brand(&store, &budget, "brand.copy_media", params) {
                    Ok(ControlResult::BrandMediaCopied { path }) => Ok(path),
                    Ok(_) => Err(String::from("brand kit copy result")),
                    Err(error) => Err(error.to_string()),
                };
                let _ = send.send(copied);
            });
            brand.copy = Some((receive, at));
            Ok(())
        })();
        if let Err(error) = started {
            let text = port.tr(Text::BrandAssetsInsertFailed);
            port.toast(text, Some(error.to_string()));
        }
    }
}
