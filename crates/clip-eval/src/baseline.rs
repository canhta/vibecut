//! Pinned main-revision baseline: inputs and metrics for the 01 heuristic mode.
use crate::metrics::Metrics;
use crate::{Error, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BaselineInput {
    pub source_sha256: String,
    pub transcript_sha256: String,
    pub settings_sha256: String,
    pub language: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Baseline {
    pub schema: u32,
    pub revision: String,
    pub provider: String,
    pub note: String,
    pub inputs: BTreeMap<String, BaselineInput>,
    pub videos: BTreeMap<String, Metrics>,
}

impl Baseline {
    pub fn load(path: &Path) -> Result<Self> {
        let bytes = std::fs::read(path).map_err(|error| {
            if error.kind() == std::io::ErrorKind::NotFound {
                Error::BaselineMissing(path.display().to_string())
            } else {
                Error::Io(error)
            }
        })?;
        let baseline: Self = serde_json::from_slice(&bytes)?;
        baseline.validate()?;
        Ok(baseline)
    }

    pub fn validate(&self) -> Result<()> {
        if self.schema != 1
            || self.provider != "heuristic"
            || self.revision.trim().is_empty()
            || self.inputs.len() != self.videos.len()
        {
            return Err(Error::BaselineInvalid);
        }
        for (label, input) in &self.inputs {
            if !self.videos.contains_key(label)
                || input.source_sha256.len() != 64
                || input.transcript_sha256.len() != 64
                || input.settings_sha256.len() != 64
                || input.language.trim().is_empty()
            {
                return Err(Error::BaselineInvalid);
            }
        }
        Ok(())
    }

    pub fn validate_labels(&self, labels: &[String]) -> Result<()> {
        if self.inputs.len() != labels.len()
            || labels
                .iter()
                .any(|label| !self.inputs.contains_key(label) || !self.videos.contains_key(label))
        {
            return Err(Error::BaselineVideoSet);
        }
        Ok(())
    }

    pub fn validate_input(&self, label: &str, input: &BaselineInput) -> Result<()> {
        let expected = self
            .inputs
            .get(label)
            .ok_or_else(|| Error::BaselineMismatch(label.to_owned()))?;
        if expected != input {
            return Err(Error::BaselineMismatch(label.to_owned()));
        }
        Ok(())
    }

    pub fn metrics(&self, label: &str) -> Result<&Metrics> {
        self.videos
            .get(label)
            .ok_or_else(|| Error::BaselineMismatch(label.to_owned()))
    }

    pub fn aggregate(&self) -> Metrics {
        let mut aggregate = Metrics::default();
        for metrics in self.videos.values() {
            aggregate.merge(metrics);
        }
        aggregate
    }
}
