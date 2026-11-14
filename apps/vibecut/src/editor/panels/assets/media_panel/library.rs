//! Applies the media tile menu's Rename and New folder requests.
use crate::editor::{Port, error_detail};
use crate::editor::panels::assets::State;
use crate::editor::panels::assets::state::MediaRename;
use crate::i18n;
use crate::i18n::Text;
use engine::Command;
use media::AssetId;
use std::sync::Arc;

impl State {
    /// Open the Rename dialog on `asset` with its current display name typed in.
    pub(in crate::editor) fn request_media_rename(
        &mut self,
        session: &engine::Session,
        asset: AssetId,
    ) {
        let data = session.project().data();
        let Some(record) = data
            .assets
            .iter()
            .find(|record| record.id() == asset && !record.ephemeral())
        else {
            return;
        };
        self.media_rename = Some(MediaRename {
            asset,
            draft: data.media_library.display_name(record),
        });
    }
    pub(in crate::editor) fn cancel_media_rename(&mut self) {
        self.media_rename = None;
    }
    /// Apply the typed name through `media.rename`; a refusal reports as a toast.
    pub(in crate::editor) fn confirm_media_rename(&mut self, port: &mut Port<'_>) {
        let Some(rename) = self.media_rename.take() else {
            return;
        };
        let before = port.session.project().clone();
        let command = Command::RenameAsset {
            asset: rename.asset,
            name: rename.draft.trim().to_owned().into(),
        };
        match port.session.apply(command) {
            Ok(_) => {
                if !Arc::ptr_eq(&before, port.session.project()) {
                    port.sync();
                }
            }
            Err(error) => {
                let detail = error_detail(&error);
                let text = i18n::text(port.locale(), Text::MediaRenameFailed);
                port.toast(text, Some(detail));
            }
        }
    }
    /// Group `ids` into a new folder through `media.create_folder`, named after the locale's
    /// default and numbered when that name is taken.
    pub(in crate::editor) fn create_media_folder(&mut self, port: &mut Port<'_>, ids: Vec<AssetId>) {
        let base = i18n::text(port.locale(), Text::MediaFolderDefault);
        let library = &port.session.project().data().media_library;
        let taken = |candidate: &str| {
            library
                .folders
                .iter()
                .any(|folder| folder.name.as_str() == candidate)
        };
        let name = if taken(base) {
            (2_u64..)
                .map(|number| format!("{base} {number}"))
                .find(|candidate| !taken(candidate))
                .unwrap_or_else(|| base.to_owned())
        } else {
            base.to_owned()
        };
        let before = port.session.project().clone();
        let command = Command::CreateMediaFolder {
            name: name.into(),
            assets: ids,
        };
        match port.session.apply(command) {
            Ok(_) => {
                self.media_selection.clear();
                self.media_anchor = None;
                if !Arc::ptr_eq(&before, port.session.project()) {
                    port.sync();
                }
            }
            Err(error) => {
                let detail = error_detail(&error);
                let text = i18n::text(port.locale(), Text::MediaNewFolderFailed);
                port.toast(text, Some(detail));
            }
        }
    }
}
