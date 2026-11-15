//! Keeps the Home brand kit page's state, edited through one store worker.
//!
//! Every listing and edit is a registry `brand.*` command (`services::assets::brand`) run on
//! one worker thread, because a listing hashes each kit file. Each edit answers with the kit
//! after it. Nothing is read until the page first opens.

use crate::services::assets::brand::{self, run_brand};
use engine::control::{BrandKitView, BrandMediaSection};
use libraries::brand_kit::BrandKit;
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
    sync::{
        Arc,
        mpsc::{self, Receiver, Sender, TryRecvError},
    },
};

/// One page section, in the reference's order. Fonts are families; the rest are files.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum Section {
    Videos,
    Images,
    Stickers,
    Fonts,
    Backgrounds,
    Music,
}

impl Section {
    pub(super) const ALL: [Self; 6] = [
        Self::Videos,
        Self::Images,
        Self::Stickers,
        Self::Fonts,
        Self::Backgrounds,
        Self::Music,
    ];

    /// The automation id segment.
    pub(super) const fn id(self) -> &'static str {
        match self {
            Self::Videos => "videos",
            Self::Images => "images",
            Self::Stickers => "stickers",
            Self::Fonts => "fonts",
            Self::Backgrounds => "backgrounds",
            Self::Music => "music",
        }
    }

    /// The stored section of a file section; `None` for Fonts.
    pub(super) const fn media(self) -> Option<BrandMediaSection> {
        match self {
            Self::Videos => Some(BrandMediaSection::Video),
            Self::Images => Some(BrandMediaSection::Image),
            Self::Stickers => Some(BrandMediaSection::Sticker),
            Self::Fonts => None,
            Self::Backgrounds => Some(BrandMediaSection::Background),
            Self::Music => Some(BrandMediaSection::Music),
        }
    }
}

enum Request {
    List,
    /// Files picked for one section, each with the unique entry name the page chose.
    AddMedia {
        section: BrandMediaSection,
        files: Vec<(PathBuf, String)>,
        kit_name: String,
    },
    RenameMedia {
        media: u64,
        name: String,
    },
    RemoveMedia {
        media: u64,
    },
    AddFont {
        family: String,
        kit_name: String,
    },
    RemoveFont {
        family: String,
    },
    Families,
}

enum Event {
    Listed(Result<Option<BrandKitView>, String>),
    /// An edit was refused; the listing that follows it still arrives.
    Failed(String),
    Families(Result<Vec<String>, String>),
}

/// The open rename dialog: the entry and the name typed so far.
pub(super) struct Rename {
    pub(super) media: u64,
    pub(super) name: String,
}

/// The Fonts section's family chooser: the search typed and the families once read.
#[derive(Default)]
pub(super) struct FontChooser {
    pub(super) query: String,
    pub(super) families: Option<Result<Vec<String>, String>>,
}

/// The page state. The store is lent each frame; the worker starts when the page first opens.
pub(super) struct Page {
    store: Option<Arc<BrandKit>>,
    request: Option<Sender<Request>>,
    events: Receiver<Event>,
    /// The active kit as last listed; `None` before the first listing or with no kit yet.
    pub(super) kit: Option<BrandKitView>,
    pub(super) loaded: bool,
    pub(super) pending: usize,
    /// The store did not open, or the last edit or listing failed.
    pub(super) error: Option<String>,
    pub(super) unavailable: bool,
    pub(super) intro_closed: bool,
    pub(super) collapsed: BTreeSet<Section>,
    pub(super) rename: Option<Rename>,
    pub(super) fonts: Option<FontChooser>,
}

impl Default for Page {
    fn default() -> Self {
        let (_, events) = mpsc::channel();
        Self {
            store: None,
            request: None,
            events,
            kit: None,
            loaded: false,
            pending: 0,
            error: None,
            unavailable: false,
            intro_closed: false,
            collapsed: BTreeSet::new(),
            rename: None,
            fonts: None,
        }
    }
}

impl Page {
    /// Take this frame's lent store. Nothing is read until the page opens.
    pub(super) fn attach(&mut self, store: Option<&Arc<BrandKit>>) {
        self.unavailable = store.is_none();
        if self.store.is_none() {
            self.store = store.cloned();
        }
    }

    /// The page opened: the first open starts the worker and the first listing, a later one
    /// lists again, since another window may have changed the kit.
    pub(super) fn open(&mut self, budget: &engine::ByteBudget) {
        if self.request.is_some() {
            self.refresh();
            return;
        }
        let Some(store) = self.store.clone() else {
            return;
        };
        let (request, requests) = mpsc::channel();
        let (sender, events) = wake::channel();
        let budget = budget.clone();
        std::thread::spawn(move || worker(&store, &budget, &requests, &sender));
        self.request = Some(request);
        self.events = events;
        self.send(Request::List);
    }

    /// Apply what the worker finished.
    pub(super) fn tick(&mut self) {
        loop {
            match self.events.try_recv() {
                Ok(Event::Listed(result)) => {
                    self.pending = self.pending.saturating_sub(1);
                    self.loaded = true;
                    match result {
                        Ok(kit) => self.kit = kit,
                        Err(error) => self.error = Some(error),
                    }
                    let ids: Vec<u64> = self
                        .kit
                        .iter()
                        .flat_map(|kit| kit.media.iter().map(|media| media.id))
                        .collect();
                    if self
                        .rename
                        .as_ref()
                        .is_some_and(|rename| !ids.contains(&rename.media))
                    {
                        self.rename = None;
                    }
                }
                Ok(Event::Failed(error)) => self.error = Some(error),
                Ok(Event::Families(result)) => {
                    if let Some(chooser) = &mut self.fonts {
                        chooser.families = Some(result);
                    }
                }
                Err(TryRecvError::Empty) => return,
                Err(TryRecvError::Disconnected) => {
                    self.request = None;
                    self.pending = 0;
                    return;
                }
            }
        }
    }

    pub(super) fn busy(&self) -> bool {
        self.pending > 0
    }

    fn send(&mut self, request: Request) {
        let counts = !matches!(request, Request::Families);
        let sent = self
            .request
            .as_ref()
            .is_some_and(|sender| sender.send(request).is_ok());
        if !sent {
            self.unavailable = true;
        } else if counts {
            self.pending += 1;
        }
    }

    /// Re-read the active kit.
    fn refresh(&mut self) {
        if self.request.is_some() && !self.busy() {
            self.send(Request::List);
        }
    }

    /// Files the section's picker returned. Each gets its file name, made unique among the
    /// kit's media names; with no active kit one is created under `kit_name`.
    pub(super) fn add_media(
        &mut self,
        section: BrandMediaSection,
        paths: Vec<PathBuf>,
        kit_name: String,
    ) {
        if paths.is_empty() {
            return;
        }
        let mut taken: BTreeSet<String> = self
            .kit
            .iter()
            .flat_map(|kit| kit.media.iter().map(|media| media.name.clone()))
            .collect();
        let files = paths
            .into_iter()
            .map(|path| {
                let name = unique_name(&path, &taken);
                taken.insert(name.clone());
                (path, name)
            })
            .collect();
        self.error = None;
        self.send(Request::AddMedia {
            section,
            files,
            kit_name,
        });
    }

    pub(super) fn rename_media(&mut self, media: u64, name: String) {
        self.error = None;
        self.send(Request::RenameMedia { media, name });
    }

    pub(super) fn remove_media(&mut self, media: u64) {
        self.error = None;
        self.send(Request::RemoveMedia { media });
    }

    pub(super) fn add_font(&mut self, family: String, kit_name: String) {
        self.error = None;
        self.send(Request::AddFont { family, kit_name });
    }

    pub(super) fn remove_font(&mut self, family: String) {
        self.error = None;
        self.send(Request::RemoveFont { family });
    }

    /// Open the Fonts chooser and read the font store's families once.
    pub(super) fn open_fonts(&mut self) {
        self.fonts = Some(FontChooser::default());
        self.send(Request::Families);
    }
}

/// The file's name, or `<stem> <n>.<ext>` with the first free number when the kit has it.
fn unique_name(path: &Path, taken: &BTreeSet<String>) -> String {
    let name = path
        .file_name()
        .map_or_else(String::new, |name| name.to_string_lossy().trim().to_owned());
    if !taken.contains(&name) {
        return name;
    }
    let stem = path
        .file_stem()
        .map_or_else(String::new, |stem| stem.to_string_lossy().trim().to_owned());
    let extension = path
        .extension()
        .map(|extension| format!(".{}", extension.to_string_lossy()));
    (2..=u32::MAX)
        .map(|number| format!("{stem} {number}{}", extension.as_deref().unwrap_or("")))
        .find(|candidate| !taken.contains(candidate))
        .unwrap_or(name)
}

/// One `brand.*` command over the store; its answer is the active kit after it.
fn command(
    store: &BrandKit,
    budget: &engine::ByteBudget,
    id: &str,
    params: serde_json::Value,
) -> Result<Option<BrandKitView>, String> {
    run_brand(store, budget, id, params)
        .and_then(brand::kit)
        .map_err(|error| error.to_string())
}

fn worker(
    store: &BrandKit,
    budget: &engine::ByteBudget,
    requests: &Receiver<Request>,
    events: &wake::Sender<Event>,
) {
    while let Ok(request) = requests.recv() {
        let edited = match request {
            Request::List => command(store, budget, "brand.list", serde_json::json!({})),
            Request::Families => {
                let families = libraries::fonts::FontStore::native()
                    .and_then(|fonts| fonts.list(None))
                    .map(|entries| entries.into_iter().map(|entry| entry.family).collect())
                    .map_err(|error| error.to_string());
                let _ = events.send(Event::Families(families));
                continue;
            }
            Request::AddMedia {
                section,
                files,
                kit_name,
            } => files
                .iter()
                .map(|(path, name)| {
                    command(
                        store,
                        budget,
                        "brand.add_media",
                        serde_json::json!({
                            "section": section,
                            "path": path,
                            "name": name,
                            "kit_name": kit_name,
                        }),
                    )
                })
                .collect::<Result<Vec<_>, _>>()
                .map(|kits| kits.into_iter().last().flatten()),
            Request::RenameMedia { media, name } => command(
                store,
                budget,
                "brand.rename_media",
                serde_json::json!({"media": media, "name": name}),
            ),
            Request::RemoveMedia { media } => command(
                store,
                budget,
                "brand.remove_media",
                serde_json::json!({"media": media}),
            ),
            Request::AddFont { family, kit_name } => command(
                store,
                budget,
                "brand.add_font",
                serde_json::json!({"family": family, "kit_name": kit_name}),
            ),
            Request::RemoveFont { family } => command(
                store,
                budget,
                "brand.remove_font",
                serde_json::json!({"family": family}),
            ),
        };
        // A refused edit still leaves the page with the kit as it now stands.
        let listed = match edited {
            Ok(kit) => Ok(kit),
            Err(error) => {
                let _ = events.send(Event::Failed(error));
                command(store, budget, "brand.list", serde_json::json!({}))
            }
        };
        if events.send(Event::Listed(listed)).is_err() {
            return;
        }
    }
}
