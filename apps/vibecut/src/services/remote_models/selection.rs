//! Persists the user's remote model choice for each feature and service.
use crate::{Error, Result};
use ai::Operation;
use serde::{Deserialize, Serialize};

pub const MAX_RECENT_MODELS: usize = 4;
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelChoice {
    pub provider: String,
    pub feature: Operation,
    pub model: String,
    pub recent: Vec<String>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AiPreferences {
    pub choices: Vec<ModelChoice>,
}
impl Default for AiPreferences {
    fn default() -> Self {
        Self {
            choices: ai::defaults::DEFAULTS
                .iter()
                .filter(|row| row.shipped_default)
                .map(|row| ModelChoice {
                    provider: row.provider.into(),
                    feature: row.feature,
                    model: row.model.into(),
                    recent: Vec::new(),
                })
                .collect(),
        }
    }
}
impl AiPreferences {
    pub fn validate(&self) -> Result<()> {
        let defaults = Self::default();
        if self.choices.len() != defaults.choices.len() {
            return Err(Error::Invalid("AI model settings"));
        }
        for (index, choice) in self.choices.iter().enumerate() {
            if ai::defaults::model(&choice.provider, choice.feature).is_empty()
                || self.choices.iter().take(index).any(|other| {
                    other.provider == choice.provider && other.feature == choice.feature
                })
                || choice.recent.len() > MAX_RECENT_MODELS
            {
                return Err(Error::Invalid("AI model settings"));
            }
            ai::defaults::validate_model(&choice.model).map_err(engine::EditError::from)?;
            for (i, model) in choice.recent.iter().enumerate() {
                ai::defaults::validate_model(model).map_err(engine::EditError::from)?;
                if choice
                    .recent
                    .iter()
                    .take(i)
                    .any(|previous| previous == model)
                {
                    return Err(Error::Invalid("recent AI models"));
                }
            }
        }
        Ok(())
    }
    pub fn model(&self, provider: &str, feature: Operation) -> &str {
        self.choices
            .iter()
            .find(|row| row.provider == provider && row.feature == feature)
            .map_or_else(
                || ai::defaults::model(provider, feature),
                |row| row.model.as_str(),
            )
    }
    pub fn select(&mut self, provider: &str, feature: Operation, model: &str) -> Result<()> {
        ai::defaults::validate_model(model).map_err(engine::EditError::from)?;
        let choice = self
            .choices
            .iter_mut()
            .find(|row| row.provider == provider && row.feature == feature)
            .ok_or(Error::Invalid("AI model settings"))?;
        choice.recent.retain(|previous| previous != model);
        choice.recent.insert(0, model.into());
        choice.recent.truncate(MAX_RECENT_MODELS);
        choice.model = model.into();
        Ok(())
    }
    /// Default first, then the current key's fetched ids, then recent/custom choices.
    pub fn options(
        &self,
        provider: &str,
        feature: Operation,
        fetched: &[captions::NfcText],
    ) -> Vec<String> {
        let mut choices = vec![ai::defaults::model(provider, feature).to_owned()];
        let row = self
            .choices
            .iter()
            .find(|row| row.provider == provider && row.feature == feature);
        for id in fetched
            .iter()
            .map(captions::NfcText::as_str)
            .chain(
                row.into_iter()
                    .flat_map(|row| row.recent.iter().map(String::as_str)),
            )
            .chain(std::iter::once(self.model(provider, feature)))
        {
            if !choices.iter().any(|previous| previous == id) {
                choices.push(id.into());
            }
        }
        choices
    }
}
#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]
    use super::*;
    #[test]
    fn default_first_recent_bound_and_one_persisted_remote_choice_round_trip() {
        let mut settings = AiPreferences::default();
        for row in ai::defaults::DEFAULTS
            .iter()
            .filter(|row| row.shipped_default)
        {
            let fetched = vec!["fixture-fetched".into(), row.model.into()];
            for index in 0..6 {
                settings
                    .select(row.provider, row.feature, &format!("fixture-{index}"))
                    .unwrap();
            }
            let options = settings.options(row.provider, row.feature, &fetched);
            assert_eq!(options[0], row.model);
            assert_eq!(options[1], "fixture-fetched");
            assert_eq!(options[2], "fixture-5");
            assert_eq!(options.len(), 6);
            assert_eq!(settings.model(row.provider, row.feature), "fixture-5");
        }
        settings.validate().unwrap();
        let restored: AiPreferences =
            serde_json::from_slice(&serde_json::to_vec(&settings).unwrap()).unwrap();
        assert_eq!(restored, settings);
    }
    #[test]
    fn model_text_bounds_are_bytes_and_invalid_edits_leave_the_selection() {
        let mut preferences = AiPreferences::default();
        for value in ["", " ", "line\nbreak"] {
            assert!(
                preferences
                    .select("openai", Operation::Translate, value)
                    .is_err()
            );
        }
        let valid = "é".repeat(128);
        preferences
            .select("openai", Operation::Translate, &valid)
            .unwrap();
        assert!(
            preferences
                .select("openai", Operation::Translate, &format!("{valid}a"))
                .is_err()
        );
        assert_eq!(preferences.model("openai", Operation::Translate), valid);
        let mut duplicate = preferences.clone();
        duplicate.choices[1] = duplicate.choices[0].clone();
        assert!(duplicate.validate().is_err());
        let mut missing = preferences.clone();
        missing.choices.pop();
        assert!(missing.validate().is_err());
    }
}
