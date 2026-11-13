//! Opens the brand kit store and applies each change under the writer lease.
use super::Result;
use super::files::{self, FileState};
use super::record::{self, BrandColor, Content, Index, Kit, MAX_KITS, Media};
use engine::BrandKitError;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};

/// One kit as a listing shows it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KitSummary {
    pub id: u64,
    pub name: String,
    pub active: bool,
}

/// The active kit with the verification state of each media file, in entry order.
pub(super) struct ActiveKit {
    pub id: u64,
    pub content: Content,
    pub media: Vec<FileState>,
}

/// The user's brand kits under one store root. Every change persists before it is visible,
/// and one writer lease serialises the store across processes.
pub struct BrandKit {
    root: PathBuf,
    _lease: project::WriterLease,
    state: Mutex<Index>,
}

impl BrandKit {
    /// Opens the store under `root`, creating it when absent, and takes its writer lease.
    pub fn open(root: &Path) -> Result<Self> {
        std::fs::create_dir_all(root)?;
        let root = std::fs::canonicalize(root)?;
        let lease = project::WriterLease::acquire(&root.join(files::LEASE))?;
        let index = files::load_index(&root)?;
        std::fs::create_dir_all(files::media_dir(&root))?;
        Ok(Self {
            root,
            _lease: lease,
            state: Mutex::new(index),
        })
    }

    pub fn kits(&self) -> Result<Vec<KitSummary>> {
        let index = self.lock()?;
        Ok(index
            .kits
            .iter()
            .map(|kit| KitSummary {
                id: kit.id,
                name: kit.content.name.clone(),
                active: index.active == Some(kit.id),
            })
            .collect())
    }

    pub fn create(&self, name: &str) -> Result<u64> {
        record::check_name(name)?;
        self.mutate(|index| {
            if index.kits.len() >= MAX_KITS {
                return Err(BrandKitError::Limit);
            }
            let id = index.next_id;
            index.next_id += 1;
            index.kits.push(Kit {
                id,
                content: Content::named(name),
            });
            Ok(id)
        })
    }

    pub fn rename(&self, id: u64, name: &str) -> Result<()> {
        record::check_name(name)?;
        self.mutate(|index| {
            index.kit_mut(id)?.content.name = name.to_owned();
            Ok(())
        })
    }

    /// Deletes a kit and its selection; media files no other kit cites are removed.
    pub fn delete(&self, id: u64) -> Result<()> {
        self.mutate(|index| {
            index.kit(id)?;
            index.kits.retain(|kit| kit.id != id);
            if index.active == Some(id) {
                index.active = None;
            }
            Ok(())
        })
    }

    /// Selects the active kit, or clears the selection with `None`.
    pub fn set_active(&self, id: Option<u64>) -> Result<()> {
        self.mutate(|index| {
            if let Some(id) = id {
                index.kit(id)?;
            }
            index.active = id;
            Ok(())
        })
    }

    pub fn add_color(&self, id: u64, name: &str, rgba: [u8; 4]) -> Result<()> {
        record::check_name(name)?;
        self.mutate(|index| {
            index.kit_mut(id)?.content.colors.push(BrandColor {
                name: name.to_owned(),
                rgba,
            });
            Ok(())
        })
    }

    pub fn remove_color(&self, id: u64, name: &str) -> Result<()> {
        self.mutate(|index| {
            remove_named(
                &mut index.kit_mut(id)?.content.colors,
                |c| c.name.as_str(),
                name,
            )
        })
    }

    pub fn add_font(&self, id: u64, family: &str) -> Result<()> {
        record::check_name(family)?;
        self.mutate(|index| {
            index.kit_mut(id)?.content.fonts.push(family.to_owned());
            Ok(())
        })
    }

    pub fn remove_font(&self, id: u64, family: &str) -> Result<()> {
        self.mutate(|index| {
            remove_named(
                &mut index.kit_mut(id)?.content.fonts,
                |f| f.as_str(),
                family,
            )
        })
    }

    /// Copies a media file into the store, verifies it by hash and records it under `name`.
    /// Returns the entry's identity within the kit.
    pub fn add_media(&self, id: u64, source: &Path, name: &str) -> Result<u64> {
        record::check_name(name)?;
        let mut state = self.lock()?;
        state.kit(id)?;
        let (sha256, size) = files::import_media(&self.root, source)?;
        self.commit(&mut state, |index| {
            let kit = index.kit_mut(id)?;
            let media_id = kit
                .content
                .media
                .iter()
                .map(|media| media.id)
                .max()
                .map_or(1, |max| max + 1);
            kit.content.media.push(Media {
                id: media_id,
                name: name.to_owned(),
                sha256,
                size,
            });
            Ok(media_id)
        })
    }

    pub fn remove_media(&self, id: u64, media_id: u64) -> Result<()> {
        self.mutate(|index| {
            let media = &mut index.kit_mut(id)?.content.media;
            let before = media.len();
            media.retain(|entry| entry.id != media_id);
            if media.len() == before {
                return Err(BrandKitError::NotFound);
            }
            Ok(())
        })
    }

    /// Adds a text preset value. Its name, not its identity, is the key used by removal.
    pub fn add_text_preset(&self, id: u64, preset: project::SavedTextPreset) -> Result<()> {
        self.mutate(|index| {
            index.kit_mut(id)?.content.text_presets.push(preset);
            Ok(())
        })
    }

    pub fn remove_text_preset(&self, id: u64, name: &str) -> Result<()> {
        self.mutate(|index| {
            remove_named(
                &mut index.kit_mut(id)?.content.text_presets,
                |p| p.name.as_str(),
                name,
            )
        })
    }

    pub fn add_caption_preset(&self, id: u64, preset: project::SavedCaptionPreset) -> Result<()> {
        self.mutate(|index| {
            index.kit_mut(id)?.content.caption_presets.push(preset);
            Ok(())
        })
    }

    pub fn remove_caption_preset(&self, id: u64, name: &str) -> Result<()> {
        self.mutate(|index| {
            remove_named(
                &mut index.kit_mut(id)?.content.caption_presets,
                |p| p.name.as_str(),
                name,
            )
        })
    }

    /// The active kit and the state of each media file. Missing or corrupt files stay listed.
    pub(super) fn active(&self) -> Result<Option<ActiveKit>> {
        let index = self.lock()?;
        let Some(active) = index.active else {
            return Ok(None);
        };
        let kit = index.kit(active)?;
        let media = kit
            .content
            .media
            .iter()
            .map(|media| files::file_state(&self.root, media))
            .collect();
        Ok(Some(ActiveKit {
            id: kit.id,
            content: kit.content.clone(),
            media,
        }))
    }

    pub(super) fn content(&self, id: u64) -> Result<Content> {
        Ok(self.lock()?.kit(id)?.content.clone())
    }

    pub(super) fn root(&self) -> &Path {
        &self.root
    }

    /// Adds an imported kit under a new local identity. The caller holds `state` across its
    /// media imports, so no other change can prune those files before this commit.
    pub(super) fn adopt(&self, state: &mut MutexGuard<'_, Index>, content: Content) -> Result<u64> {
        self.commit(state, |index| {
            if index.kits.len() >= MAX_KITS {
                return Err(BrandKitError::Limit);
            }
            let id = index.next_id;
            index.next_id += 1;
            index.kits.push(Kit { id, content });
            Ok(id)
        })
    }

    pub(super) fn lock(&self) -> Result<MutexGuard<'_, Index>> {
        self.state.lock().map_err(|_| BrandKitError::Integrity)
    }

    /// Applies `change` to a copy, validates and persists it, then publishes it. Any failure
    /// leaves the published index unchanged and removes media files no kit still cites.
    fn mutate<T>(&self, change: impl FnOnce(&mut Index) -> Result<T>) -> Result<T> {
        let mut state = self.lock()?;
        self.commit(&mut state, change)
    }

    fn commit<T>(
        &self,
        state: &mut MutexGuard<'_, Index>,
        change: impl FnOnce(&mut Index) -> Result<T>,
    ) -> Result<T> {
        let mut next = state.clone();
        let result = change(&mut next).and_then(|value| {
            next.validate()?;
            files::persist(&self.root, &next)?;
            Ok(value)
        });
        match result {
            Ok(value) => {
                files::prune_media(&self.root, &next.referenced_media());
                **state = next;
                Ok(value)
            }
            Err(error) => {
                files::prune_media(&self.root, &state.referenced_media());
                Err(error)
            }
        }
    }
}

/// Removes the entry called `name`; an absent name is `NotFound`, never a silent no-op.
fn remove_named<T>(items: &mut Vec<T>, name_of: impl Fn(&T) -> &str, name: &str) -> Result<()> {
    let before = items.len();
    items.retain(|item| name_of(item) != name);
    if items.len() == before {
        Err(BrandKitError::NotFound)
    } else {
        Ok(())
    }
}
