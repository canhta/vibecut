//! App-owned saved colour-adjustment presets.

use crate::{Error, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// The document bound for saved adjustment presets.
pub const MAX_ADJUSTMENT_PRESETS: usize = 32;
/// The longest preset name the adjustment assets tab accepts.
pub const MAX_ADJUSTMENT_PRESET_NAME: usize = 64;

/// One named copy of a `color_grade` parameter set. Presets belong to the app,
/// never to a project, and apply only to the selected video or image item.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AdjustmentPreset {
    pub name: String,
    pub params: BTreeMap<String, project::EffectValue>,
}

/// Structural admission for the whole saved bank. Names are non-empty, bounded
/// and unique; every parameter must belong to the current colour definition and
/// pass its typed range check.
pub fn validate_adjustment_presets(presets: &[AdjustmentPreset]) -> Result<()> {
    if presets.len() > MAX_ADJUSTMENT_PRESETS {
        return Err(Error::Invalid("adjustment presets"));
    }
    let definition = project::effect_definition("color_grade")?;
    let mut names = std::collections::BTreeSet::new();
    for preset in presets {
        if preset.name.is_empty()
            || preset.name.len() > MAX_ADJUSTMENT_PRESET_NAME
            || preset.name.chars().any(char::is_control)
            || !names.insert(preset.name.as_str())
            || preset.params.len() != definition.params.len()
        {
            return Err(Error::Invalid("adjustment preset"));
        }
        for parameter in definition.params {
            let value = preset
                .params
                .get(parameter.id)
                .ok_or(Error::Invalid("adjustment preset"))?;
            parameter.check(value)?;
        }
    }
    Ok(())
}
