//! The admitted transition catalogue shared by command validation, rendering and the browser.
use super::transition_params::{TransitionParamDef, TransitionParamKind};
use crate::{Color, Error, Result};

/// One admitted kind: stable id, our display name, grouping and typed parameters.
#[derive(Clone, Copy, Debug)]
pub struct TransitionDefinition {
    pub kind: &'static str,
    pub name: &'static str,
    pub family: &'static str,
    /// The browser shelf that lists the entry.
    pub shelf: &'static str,
    pub params: &'static [TransitionParamDef],
}
impl TransitionDefinition {
    pub fn param(&self, id: &str) -> Option<&'static TransitionParamDef> {
        self.params.iter().find(|p| p.id == id)
    }
}

const DIP_COLOR: TransitionParamDef = TransitionParamDef {
    id: "color",
    name: "Color",
    kind: TransitionParamKind::Color {
        default: Color([217, 51, 51, 255]),
    },
};
const DIP_HOLD: TransitionParamDef = TransitionParamDef {
    id: "hold",
    name: "Hold",
    kind: TransitionParamKind::Float {
        min: 0.,
        max: 90.,
        default: 0.,
        unit: "%",
    },
};

pub const TRANSITION_DEFINITIONS: &[TransitionDefinition] = &[
    TransitionDefinition {
        kind: "cross_dissolve",
        name: "Cross Dissolve",
        family: "Dissolve",
        shelf: "Basic",
        params: &[],
    },
    TransitionDefinition {
        kind: "dip_to_black",
        name: "Dip to Black",
        family: "Dissolve",
        shelf: "Basic",
        params: &[],
    },
    TransitionDefinition {
        kind: "dip_to_white",
        name: "Dip to White",
        family: "Dissolve",
        shelf: "Basic",
        params: &[],
    },
    TransitionDefinition {
        kind: "dip_to_color",
        name: "Dip to Colour",
        family: "Dissolve",
        shelf: "Basic",
        params: &[DIP_COLOR, DIP_HOLD],
    },
];

/// Look up an admitted kind; unsupported names return a typed error capped at 64 characters.
pub fn transition_definition(kind: &str) -> Result<&'static TransitionDefinition> {
    TRANSITION_DEFINITIONS
        .iter()
        .find(|d| d.kind == kind)
        .ok_or_else(|| Error::UnsupportedTransition(kind.chars().take(64).collect()))
}
