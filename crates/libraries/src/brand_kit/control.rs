//! Serves the `brand.*` control commands from the store through a lent `BrandAccess`.
use super::Result;
use super::files::FileState;
use super::store::BrandKit;
use engine::BrandKitError;
use engine::control::{
    BrandAccess, BrandColorView, BrandEdit, BrandEntryState, BrandKitView, BrandMediaView,
};
use std::path::{Path, PathBuf};

impl BrandKit {
    /// Lend this store to `serve` as the access object the `brand.*` commands use. Every edit
    /// answers with the active kit after it, so a caller never needs a second listing.
    pub fn with_control<T>(&self, serve: impl FnOnce(&BrandAccess<'_>) -> T) -> T {
        let list = || -> engine::control::Result<Option<BrandKitView>> { Ok(self.active_view()?) };
        let edit = |edit: BrandEdit| -> engine::control::Result<Option<BrandKitView>> {
            self.apply(edit)?;
            Ok(self.active_view()?)
        };
        let copy = |media: u64, dir: &Path| -> engine::control::Result<PathBuf> {
            Ok(self.copy_active_media(media, dir)?)
        };
        let access = BrandAccess::new(&list, &edit, &copy);
        serve(&access)
    }

    /// The active kit as `brand.list` reports it, or `None` when no kit is active.
    pub fn active_view(&self) -> Result<Option<BrandKitView>> {
        let Some(active) = self.active()? else {
            return Ok(None);
        };
        let content = active.content;
        let media = content
            .media
            .into_iter()
            .zip(active.media)
            .map(|(media, state)| BrandMediaView {
                id: media.id,
                name: media.name,
                section: media.section,
                sha256: media.sha256,
                size: media.size,
                state: state.into(),
            })
            .collect();
        Ok(Some(BrandKitView {
            id: active.id,
            name: content.name,
            colors: content
                .colors
                .into_iter()
                .map(|color| BrandColorView {
                    name: color.name,
                    rgba: color.rgba,
                })
                .collect(),
            fonts: content.fonts,
            media,
            text_presets: content.text_presets,
            caption_presets: content.caption_presets,
        }))
    }

    /// Apply one command edit to the active kit; an add first makes `kit_name` active when no
    /// kit is.
    fn apply(&self, edit: BrandEdit) -> Result<()> {
        match edit {
            BrandEdit::AddFont { family, kit_name } => {
                let kit = self.active_or_create(&kit_name)?;
                self.add_font(kit, &family)
            }
            BrandEdit::AddMedia {
                section,
                path,
                name,
                kit_name,
            } => {
                let kit = self.active_or_create(&kit_name)?;
                self.add_media(kit, section, &path, &name).map(|_| ())
            }
            BrandEdit::RenameMedia { media, name } => {
                self.rename_media(self.active_id()?, media, &name)
            }
            BrandEdit::RemoveMedia { media } => self.remove_media(self.active_id()?, media),
            BrandEdit::RemoveFont { family } => self.remove_font(self.active_id()?, &family),
        }
    }

    fn active_id(&self) -> Result<u64> {
        self.kits()?
            .into_iter()
            .find(|kit| kit.active)
            .map(|kit| kit.id)
            .ok_or(BrandKitError::NotFound)
    }

    /// The active kit; with none active, the kit called `name` (created when absent) becomes
    /// the active one.
    fn active_or_create(&self, name: &str) -> Result<u64> {
        let kits = self.kits()?;
        if let Some(kit) = kits.iter().find(|kit| kit.active) {
            return Ok(kit.id);
        }
        let id = match kits.iter().find(|kit| kit.name == name) {
            Some(kit) => kit.id,
            None => self.create(name)?,
        };
        self.set_active(Some(id))?;
        Ok(id)
    }

    /// Copy one active-kit file into `dir` under its entry name, made safe for every
    /// filesystem and unique in the folder, and verify the copy. Returns the new path.
    fn copy_active_media(&self, media: u64, dir: &Path) -> Result<PathBuf> {
        let kit = self.active_id()?;
        let name = self
            .content(kit)?
            .media
            .into_iter()
            .find(|entry| entry.id == media)
            .ok_or(BrandKitError::NotFound)?
            .name;
        std::fs::create_dir_all(dir)?;
        let safe: String = name
            .chars()
            .map(|c| {
                if matches!(c, '<' | '>' | ':' | '"' | '|' | '?' | '*') {
                    '_'
                } else {
                    c
                }
            })
            .collect();
        let first = dir.join(&safe);
        let dest = if first.exists() {
            let path = Path::new(&safe);
            let stem = path
                .file_stem()
                .map_or_else(String::new, |stem| stem.to_string_lossy().into_owned());
            let extension = path
                .extension()
                .map_or_else(String::new, |ext| format!(".{}", ext.to_string_lossy()));
            (2..=u32::MAX)
                .map(|number| dir.join(format!("{stem} {number}{extension}")))
                .find(|candidate| !candidate.exists())
                .ok_or(BrandKitError::Limit)?
        } else {
            first
        };
        self.copy_media(kit, media, &dest)?;
        Ok(dest)
    }
}

impl From<FileState> for BrandEntryState {
    fn from(state: FileState) -> Self {
        match state {
            FileState::Available => Self::Available,
            FileState::Missing => Self::Missing,
            FileState::Corrupt => Self::Corrupt,
        }
    }
}
