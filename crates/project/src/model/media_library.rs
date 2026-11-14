//! The project's media library organisation: names the user gave assets and the folders that group them.
use crate::{Error, Result};
use captions::NfcText;
use media::{AssetId, SourceAsset};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

id!(MediaFolderId);

/// Most folders one project keeps.
pub const MAX_MEDIA_FOLDERS: usize = 256;
/// Longest stored asset or folder name, in UTF-8 bytes.
pub const MAX_MEDIA_NAME_BYTES: usize = 255;

/// Whether a library name is storable: 1..=255 UTF-8 bytes, no controls, no leading or
/// trailing whitespace. Input is never trimmed.
pub fn valid_media_name(name: &NfcText) -> bool {
    let name = name.as_str();
    !name.is_empty()
        && name.len() <= MAX_MEDIA_NAME_BYTES
        && name.trim() == name
        && !name.chars().any(char::is_control)
}

/// The name the user gave one asset; the asset's file keeps its own name.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AssetName {
    pub asset: AssetId,
    pub name: NfcText,
}

/// A named group of assets in the media library; an asset sits in at most one folder.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MediaFolder {
    pub id: MediaFolderId,
    pub name: NfcText,
    /// At least one asset, in the order they were added.
    pub assets: Vec<AssetId>,
}

/// Library organisation. Presentation of the assets (thumbnails, sort) is not stored here.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MediaLibrary {
    pub names: Vec<AssetName>,
    pub folders: Vec<MediaFolder>,
}
impl MediaLibrary {
    /// The name the user gave `asset`, when it has one.
    pub fn name(&self, asset: AssetId) -> Option<&NfcText> {
        self.names
            .iter()
            .find(|entry| entry.asset == asset)
            .map(|entry| &entry.name)
    }
    /// The one name every surface shows for `asset`: the library name the user gave it, else
    /// the file name (the whole path when it has none).
    pub fn display_name(&self, asset: &SourceAsset) -> String {
        self.name(asset.id()).map_or_else(
            || {
                asset.path().file_name().map_or_else(
                    || asset.path().to_string_lossy().into_owned(),
                    |name| name.to_string_lossy().into_owned(),
                )
            },
            |name| name.as_str().to_owned(),
        )
    }
    /// The folder holding `asset`, when it sits in one.
    pub fn folder_of(&self, asset: AssetId) -> Option<MediaFolderId> {
        self.folders
            .iter()
            .find(|folder| folder.assets.contains(&asset))
            .map(|folder| folder.id)
    }
    /// Forget the places of `assets` in folders; a folder left without assets is dropped.
    pub fn unplace(&mut self, assets: &BTreeSet<AssetId>) {
        for folder in &mut self.folders {
            folder.assets.retain(|asset| !assets.contains(asset));
        }
        self.folders.retain(|folder| !folder.assets.is_empty());
    }
    /// Forget everything about `assets`: their names and their places in folders.
    pub fn forget(&mut self, assets: &BTreeSet<AssetId>) {
        self.names.retain(|entry| !assets.contains(&entry.asset));
        self.unplace(assets);
    }
    /// Whole-library invariants against the project's assets.
    pub(super) fn validate(&self, assets: &[SourceAsset]) -> Result<()> {
        let known: BTreeSet<AssetId> = assets.iter().map(SourceAsset::id).collect();
        let mut named = BTreeSet::new();
        for entry in &self.names {
            if !valid_media_name(&entry.name)
                || !known.contains(&entry.asset)
                || !named.insert(entry.asset)
            {
                return Err(Error::Invalid("asset name"));
            }
        }
        if self.folders.len() > MAX_MEDIA_FOLDERS {
            return Err(Error::TooLarge("media folders"));
        }
        let mut folder_ids = BTreeSet::new();
        let mut placed = BTreeSet::new();
        for folder in &self.folders {
            if !valid_media_name(&folder.name)
                || folder.assets.is_empty()
                || !folder_ids.insert(folder.id)
            {
                return Err(Error::Invalid("media folder"));
            }
            for asset in &folder.assets {
                if !known.contains(asset) || !placed.insert(*asset) {
                    return Err(Error::Invalid("media folder asset"));
                }
            }
        }
        Ok(())
    }
}
