//! Prepares ASS caption imports for headless callers.
use super::{Error as SubtitleError, Result as SubtitleResult};

use crate::{Command, NfcText};
use captions::{AssWarning, LanguageTag, parse_ass};
use media::{AssetData, AssetId, ContentHash, SourceAsset, SourceProbe};
use std::io::Read;
use std::path::{Path, PathBuf};
use time::Tick;

/// Inputs needed to prepare one `captions.import` command.
#[derive(Clone, Debug)]
pub struct AssImportRequest {
    pub path: PathBuf,
    pub language: LanguageTag,
    pub start: Tick,
    /// Existing text track; absent allocates a new one named `name`.
    pub track: Option<project::TrackId>,
    pub name: Option<NfcText>,
    /// Caller-allocated media identity for the imported script file.
    pub asset_id: AssetId,
}

/// Parsed counts, warnings and the command to execute.
#[derive(Clone, Debug)]
pub struct AssImportReport {
    pub cues: usize,
    pub styles: usize,
    pub skipped_cues: usize,
    pub skipped_styles: usize,
    pub warnings: Vec<AssWarning>,
    pub suppressed_warnings: usize,
    pub command: Command,
}

/// Read at most [`captions::ass::MAX_ASS_BYTES`] and build the import command.
/// No project mutation happens here, so a parse failure leaves the caller's
/// session untouched.
pub fn prepare_ass_import(request: &AssImportRequest) -> SubtitleResult<AssImportReport> {
    let bytes = read_bounded(&request.path)?;
    let import = parse_ass(&bytes)?;
    if import.cues.is_empty() {
        return Err(SubtitleError::Edit(crate::session::EditError::Unsupported(
            "no captions in ASS file",
        )));
    }
    let end = import
        .cues
        .iter()
        .map(|cue| cue.range.end())
        .max()
        .ok_or(SubtitleError::Edit(crate::session::EditError::Unsupported(
            "no captions in ASS file",
        )))?;
    let path = std::fs::canonicalize(&request.path)?;
    let asset = SourceAsset::try_from(AssetData {
        ephemeral: true,
        id: request.asset_id,
        path,
        content_hash: ContentHash::from_bytes(&bytes),
        probe: SourceProbe {
            format: "ass".into(),
            duration: Some(end),
            streams: Vec::new(),
        },
    })?;
    let command = Command::ImportCaptions {
        asset,
        language: request.language.clone(),
        start: request.start,
        track: request.track,
        name: request.name.clone(),
        styles: import.styles.clone(),
        cues: import.cues.clone(),
    };
    Ok(AssImportReport {
        cues: import.cues.len(),
        styles: import.styles.len(),
        skipped_cues: import.skipped_cues,
        skipped_styles: import.skipped_styles,
        warnings: import.warnings,
        suppressed_warnings: import.suppressed_warnings,
        command,
    })
}

fn read_bounded(path: &Path) -> SubtitleResult<Vec<u8>> {
    let limit = u64::try_from(captions::ass::MAX_ASS_BYTES)
        .map_err(|_| SubtitleError::from(crate::session::wire::ControlError::Limit))?;
    let mut bytes = Vec::new();
    std::fs::File::open(path)?
        .take(limit.saturating_add(1))
        .read_to_end(&mut bytes)?;
    if bytes.len() > captions::ass::MAX_ASS_BYTES {
        return Err(SubtitleError::from(
            crate::session::wire::ControlError::Limit,
        ));
    }
    Ok(bytes)
}
