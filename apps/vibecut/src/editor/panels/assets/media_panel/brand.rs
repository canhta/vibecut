//! Owns the Brand assets page's work on the lent brand kit store.
//!
//! Both are registry commands (`brand.list`, `brand.copy_media`) sent through
//! `services::assets::brand`; a listing hashes every kit file and a copy reads a whole file, so
//! both run on a thread the frame polls. A click, or a drag released over the timeline, copies
//! the kit file into the project's media folder and hands that copy to the paste import, which
//! inserts it where the click (the playhead) or the drop (its time and landing) asked: the
//! project gets its own file and never names the kit's.
use crate::editor::Port;
use crate::editor::context::{DragSource, MediaDrag};
use crate::editor::panels::assets::{AssetsTab, MediaSection, State};
use crate::i18n::Text;
use crate::services::assets::brand::{self, copy_media, run_brand};
use engine::control::{BrandEntryState, BrandKitView, BrandMediaSection, BrandMediaView};
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

/// Where a copied kit file inserts once it is imported.
type Landing = (Tick, engine::Placement);

/// The lent store and the work the page started on it.
pub(in crate::editor) struct BrandAssets {
    store: Arc<BrandKit>,
    pub(in crate::editor) listing: Listing,
    load: Option<Receiver<Result<Option<BrandKitView>, String>>>,
    /// The copy a click or drop started and where its file inserts.
    copy: Option<(Receiver<Result<PathBuf, String>>, Landing)>,
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

    /// A clicked or dropped file is still being copied into the project.
    pub(in crate::editor) fn copying(&self) -> bool {
        self.copy.is_some()
    }

    /// The listed entry `media`, when it is present and verified.
    fn available(&self, media: u64) -> Option<&BrandMediaView> {
        let Listing::Ready(Some(kit)) = &self.listing else {
            return None;
        };
        kit.media
            .iter()
            .find(|entry| entry.id == media && entry.state == BrandEntryState::Available)
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
            Some((copy, landing)) => match copy.try_recv() {
                Ok(result) => Some((result, *landing)),
                Err(TryRecvError::Disconnected) => Some((Err(String::new()), *landing)),
                Err(TryRecvError::Empty) => None,
            },
            None => None,
        };
        let Some((result, (at, placement))) = copied else {
            return;
        };
        brand.copy = None;
        let started = result
            .map_err(|detail| (!detail.is_empty()).then_some(detail))
            .and_then(|path| {
                self.paste_files(port, vec![path], at, placement)
                    .map_err(|error| Some(error.to_string()))
            });
        if let Err(detail) = started {
            let text = port.tr(Text::BrandAssetsInsertFailed);
            port.toast(text, detail);
        }
    }

    /// A row click: the kit file inserts at the current playhead, placed automatically.
    pub(in crate::editor) fn click_brand_media(&mut self, port: &mut Port<'_>, media: u64) {
        match port.playhead() {
            Ok(at) => self.insert_brand_media(port, media, at, engine::Placement::Auto),
            Err(error) => {
                let text = port.tr(Text::BrandAssetsInsertFailed);
                port.toast(text, Some(error.to_string()));
            }
        }
    }

    /// Record the drag a press on an available kit row starts; the timeline resolves its
    /// landing like a Media drag. A music file lands on audio tracks, every other section on
    /// video tracks, as its imported copy will.
    pub(in crate::editor) fn begin_brand_drag(&mut self, media: u64) {
        let Some(entry) = self.brand.as_ref().and_then(|brand| brand.available(media)) else {
            return;
        };
        let kind = match entry.section {
            BrandMediaSection::Music => project::TrackKind::Audio,
            BrandMediaSection::Video
            | BrandMediaSection::Image
            | BrandMediaSection::Sticker
            | BrandMediaSection::Background => project::TrackKind::Video,
        };
        self.media_drag = Some(MediaDrag {
            source: DragSource::Brand {
                media,
                name: entry.name.clone(),
            },
            kind,
        });
    }

    /// Copy one kit file into the project's media folder through `brand.copy_media`; when the
    /// copy lands, it is imported and inserted at `at` with `placement`. A file the kit reports
    /// missing or damaged, or a copy or import already running, refuses with a toast.
    pub(in crate::editor) fn insert_brand_media(
        &mut self,
        port: &mut Port<'_>,
        media: u64,
        at: Tick,
        placement: engine::Placement,
    ) {
        let importing = self.importing();
        let started = (|| -> crate::Result<()> {
            let brand = self
                .brand
                .as_mut()
                .ok_or(crate::Error::Invalid("brand kit unavailable"))?;
            if brand.copying() || importing {
                return Err(crate::Error::Invalid("brand asset insert in progress"));
            }
            if brand.available(media).is_none() {
                return Err(crate::Error::Invalid("brand asset unavailable"));
            }
            let directory = project::media_dir(port.session.saved_path());
            let (store, budget) = (brand.store.clone(), port.read.budget.clone());
            let (send, receive) = wake::channel();
            std::thread::spawn(move || {
                let copied = copy_media(&store, &budget, directory, media)
                    .map_err(|error| error.to_string());
                let _ = send.send(copied);
            });
            brand.copy = Some((receive, (at, placement)));
            Ok(())
        })();
        if let Err(error) = started {
            let text = port.tr(Text::BrandAssetsInsertFailed);
            port.toast(text, Some(error.to_string()));
        }
    }
}
