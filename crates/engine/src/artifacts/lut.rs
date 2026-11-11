//! Executes user-selected LUT imports.
use crate::{
    Result,
    jobs::{JobBytes, JobId, JobKind, JobRunner, JobSpec, ResourceClass},
    store::luts::LutStore,
};
pub(crate) fn submit(
    runner: &mut JobRunner,
    store: LutStore,
    project: project::ProjectId,
    path: std::path::PathBuf,
    name: Option<captions::NfcText>,
    licence: Option<captions::NfcText>,
    expected: Option<project::LutRecord>,
    fingerprint: blake3::Hash,
) -> Result<JobId> {
    let input = crate::cache::CacheKey::lut_request(fingerprint);
    let (id, _) = runner.submit(
        JobSpec {
            kind: JobKind::ImportLut,
            class: ResourceClass::Cpu,
            input,
            bytes: JobBytes {
                result: 16384,
                ..Default::default()
            },
        },
        move |context| -> Result<Vec<u8>> {
            let record = store.import(
                project,
                &path,
                name.clone(),
                licence.clone(),
                expected.clone(),
                context,
            )?;
            crate::session::wire::bounded_json(&record, 16384).map_err(crate::Error::from)
        },
    )?;
    Ok(id)
}
