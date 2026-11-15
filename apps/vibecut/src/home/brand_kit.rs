//! Keeps the Home brand kit page's state, edited through one store worker.
//!
//! Every edit is a `libraries::brand_kit` store operation. A listing hashes each kit file, so
//! no store call runs on the UI thread: the page sends requests and applies the listing that
//! follows each one.

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
        kit: u64,
        media: u64,
        name: String,
    },
    RemoveMedia {
        kit: u64,
        media: u64,
    },
    AddFont {
        family: String,
        kit_name: String,
    },
    RemoveFont {
        kit: u64,
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

/// The page state. The store is lent each frame; the worker starts on the first lend.
pub(super) struct Page {
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
    /// Take this frame's lent store: the first lend starts the worker and the first listing.
    pub(super) fn attach(&mut self, store: Option<&Arc<BrandKit>>) {
        self.unavailable = store.is_none();
        if self.request.is_some() {
            return;
        }
        let Some(store) = store else {
            return;
        };
        let (request, requests) = mpsc::channel();
        let (sender, events) = wake::channel();
        let store = store.clone();
        std::thread::spawn(move || worker(&store, &requests, &sender));
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

    /// Re-read the active kit, for when another window may have changed it.
    pub(super) fn refresh(&mut self) {
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
        if let Some(kit) = &self.kit {
            let kit = kit.id;
            self.error = None;
            self.send(Request::RenameMedia { kit, media, name });
        }
    }

    pub(super) fn remove_media(&mut self, media: u64) {
        if let Some(kit) = &self.kit {
            let kit = kit.id;
            self.error = None;
            self.send(Request::RemoveMedia { kit, media });
        }
    }

    pub(super) fn add_font(&mut self, family: String, kit_name: String) {
        self.error = None;
        self.send(Request::AddFont { family, kit_name });
    }

    pub(super) fn remove_font(&mut self, family: String) {
        if let Some(kit) = &self.kit {
            let kit = kit.id;
            self.error = None;
            self.send(Request::RemoveFont { kit, family });
        }
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

/// The active kit's identity; with none active, the kit called `name` (created when absent)
/// becomes the active one.
fn active_kit(store: &BrandKit, name: &str) -> libraries::brand_kit::Result<u64> {
    let kits = store.kits()?;
    if let Some(kit) = kits.iter().find(|kit| kit.active) {
        return Ok(kit.id);
    }
    let id = match kits.iter().find(|kit| kit.name == name) {
        Some(kit) => kit.id,
        None => store.create(name)?,
    };
    store.set_active(Some(id))?;
    Ok(id)
}

fn worker(store: &BrandKit, requests: &Receiver<Request>, events: &wake::Sender<Event>) {
    while let Ok(request) = requests.recv() {
        let edit = match request {
            Request::List => Ok(()),
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
            } => active_kit(store, &kit_name).and_then(|kit| {
                files.iter().try_for_each(|(path, name)| {
                    store.add_media(kit, section, path, name).map(|_| ())
                })
            }),
            Request::RenameMedia { kit, media, name } => store.rename_media(kit, media, &name),
            Request::RemoveMedia { kit, media } => store.remove_media(kit, media),
            Request::AddFont { family, kit_name } => {
                active_kit(store, &kit_name).and_then(|kit| store.add_font(kit, &family))
            }
            Request::RemoveFont { kit, family } => store.remove_font(kit, &family),
        };
        if let Err(error) = edit {
            let _ = events.send(Event::Failed(error.to_string()));
        }
        let listed = store.active_view().map_err(|error| error.to_string());
        if events.send(Event::Listed(listed)).is_err() {
            return;
        }
    }
}
