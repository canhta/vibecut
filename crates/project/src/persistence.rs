//! Versioned project JSON ingestion, publication and recovery snapshots.

pub mod atomic;
pub mod autosave;
mod bounds;
mod metadata;
mod streaming;
mod writer;

pub use atomic::atomic_write;
pub use bounds::MAX_SEPARATORS;
pub use metadata::{METADATA_VERSION, Metadata, Thumbnail, ViewState, path as metadata_path};
pub use writer::{SaveReceipt, WriterLease};

use crate::{Error, Project, Result};
use bounds::{MAX_BYTES, Writer, check};
use serde::{Deserialize, Serialize};
use std::{fs::File, io::Read, path::Path};
pub const FORMAT_ID: &str = "vibecut.project";
pub const SCHEMA_VERSION: u32 = 28;
#[derive(Serialize)]
struct EnvelopeOut<'a> {
    format: &'static str,
    schema_version: u32,
    generator: String,
    project: &'a Project,
}
#[derive(Deserialize)]
struct Header {
    format: String,
    schema_version: u32,
    generator: String,
    project: serde::de::IgnoredAny,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EnvelopeIn {
    #[serde(rename = "format")]
    _format: String,
    #[serde(rename = "schema_version")]
    _schema_version: u32,
    #[serde(rename = "generator")]
    _generator: String,
    project: crate::ProjectData,
}
pub struct Loaded {
    pub project: Project,
    pub schema_version: u32,
    // User-controlled generator text is not exposed through Debug/logs.
    generator: String,
}
impl Loaded {
    pub fn generator(&self) -> &str {
        &self.generator
    }
}
/// Encode a current-schema envelope; never silently produces empty bytes on failure.
pub fn encode(project: &Project, pretty: bool) -> Result<Vec<u8>> {
    let envelope = EnvelopeOut {
        format: FORMAT_ID,
        schema_version: SCHEMA_VERSION,
        generator: format!("VibeCut {}", env!("CARGO_PKG_VERSION")),
        project,
    };
    let mut writer = Writer(Vec::new());
    let result = if pretty {
        serde_json::to_writer_pretty(&mut writer, &envelope)
    } else {
        serde_json::to_writer(&mut writer, &envelope)
    };
    if let Err(error) = result {
        if error.is_io() {
            return Err(Error::TooLarge("serialized project"));
        }
        return Err(error.into());
    }
    check(&writer.0)?;
    Ok(writer.0)
}
/// Bounded JSON parse followed by complete invariant validation. Old/new schemas
/// are refused before parsing the body as the current model.
pub fn decode(bytes: &[u8]) -> Result<Loaded> {
    check(bytes)?;
    let header: Header = serde_json::from_slice(bytes)?;
    if header.format != FORMAT_ID {
        return Err(Error::Invalid("format identifier"));
    }
    if header.schema_version < SCHEMA_VERSION {
        return Err(Error::TooOld {
            found: header.schema_version,
            supported: SCHEMA_VERSION,
        });
    }
    if header.schema_version > SCHEMA_VERSION {
        return Err(Error::TooNew {
            found: header.schema_version,
            supported: SCHEMA_VERSION,
        });
    }
    let _ = header.project;
    let envelope: EnvelopeIn = serde_json::from_slice(bytes)?;
    Ok(Loaded {
        project: Project::new(envelope.project)?,
        schema_version: header.schema_version,
        generator: header.generator,
    })
}
/// Read-only ingestion. Writers acquire WriterLease before loading/editing.
pub fn load(path: &Path) -> Result<Loaded> {
    decode(&read_bytes(path)?)
}
pub(crate) fn read_bytes(path: &Path) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    File::open(path)?
        .take((MAX_BYTES + 1) as u64)
        .read_to_end(&mut bytes)?;
    if bytes.len() > MAX_BYTES {
        return Err(Error::TooLarge("project file"));
    }
    Ok(bytes)
}
/// Read optional bounded metadata without acquiring an editing lease (catalog display).
pub fn read_metadata(path: &Path) -> Metadata {
    metadata::load(path)
}

impl std::fmt::Debug for Loaded {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Loaded")
            .field("project", &self.project)
            .field("schema_version", &self.schema_version)
            .finish_non_exhaustive()
    }
}
