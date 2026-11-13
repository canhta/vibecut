//! The filter looks as data: each one is a colour-grade parameter set with a shelf.
use super::{EffectInstance, EffectValue, effect_definition};
use crate::{Item, ItemParams, Result, ToneCurve};

/// Where a look is offered in the Filters tab. `Tools` holds the technical grades that are not
/// looks and are not counted among them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FilterShelf {
    Featured,
    Movies,
    Retro,
    Night,
    Mono,
    Life,
    Tools,
}
impl FilterShelf {
    /// The shelves in the order the tab lists them.
    pub const ALL: [Self; 7] = [
        Self::Featured,
        Self::Movies,
        Self::Retro,
        Self::Night,
        Self::Mono,
        Self::Tools,
        Self::Life,
    ];
    pub const fn id(self) -> &'static str {
        match self {
            Self::Featured => "featured",
            Self::Movies => "movies",
            Self::Retro => "retro",
            Self::Night => "night",
            Self::Mono => "mono",
            Self::Life => "life",
            Self::Tools => "tools",
        }
    }
}

/// One look: the `color_grade` parameters it changes from their neutral defaults.
#[derive(Clone, Copy, Debug)]
pub struct FilterDefinition {
    pub id: &'static str,
    pub name: &'static str,
    pub shelf: FilterShelf,
    /// Scalar `color_grade` controls, each inside its definition's range.
    pub scalars: &'static [(&'static str, f64)],
    /// Tone curves by `color_grade` control id; endpoints are the fixed `(0,0)` and `(1,1)`.
    pub curves: &'static [(&'static str, &'static [[f64; 2]])],
}
impl FilterDefinition {
    /// Whether this entry counts among the 20 looks; the Tools shelf does not.
    pub const fn is_look(&self) -> bool {
        !matches!(self.shelf, FilterShelf::Tools)
    }
    /// The colour-grade instance that carries this look at full strength.
    ///
    /// # Errors
    ///
    /// Returns an error when a catalogued value is outside its parameter's range.
    pub fn instance(&self, id: impl Into<String>) -> Result<EffectInstance> {
        let mut instance = effect_definition("color_grade")?.instance(id);
        for (key, value) in self.scalars {
            instance.set_scalar(key, *value)?;
        }
        for (key, points) in self.curves {
            instance = instance.with(key, EffectValue::Curve(ToneCurve::new(points)?))?;
        }
        Ok(instance)
    }
}

const fn look(
    id: &'static str,
    name: &'static str,
    shelf: FilterShelf,
    scalars: &'static [(&'static str, f64)],
) -> FilterDefinition {
    FilterDefinition {
        id,
        name,
        shelf,
        scalars,
        curves: &[],
    }
}

use FilterShelf as S;

/// Our own procedural grades (no third-party LUTs). The first twelve adapt the eight procedural
/// looks and four creative presets of the reference implementation, the next eight are authored
/// here, and the last four are the Tools shelf. Values are new decisions, not measured parity.
static DEFINITIONS: &[FilterDefinition] = &[
    // Procedural looks.
    look(
        "teal_amber",
        "Teal and Amber",
        S::Movies,
        &[
            ("lift_g", 6.0),
            ("lift_b", 8.0),
            ("gain_r", 108.0),
            ("gain_g", 102.0),
            ("gain_b", 94.0),
            ("contrast", 15.0),
        ],
    ),
    look(
        "warm_haze",
        "Warm Haze",
        S::Retro,
        &[
            ("gain_r", 106.0),
            ("gain_b", 90.0),
            ("lift_r", 2.0),
            ("saturation", -8.0),
            ("fade", 12.0),
            ("temperature", 10.0),
        ],
    ),
    look(
        "cool_steel",
        "Cool Steel",
        S::Featured,
        &[
            ("gain_r", 90.0),
            ("gain_g", 98.0),
            ("gain_b", 110.0),
            ("lift_b", 2.0),
            ("contrast", 10.0),
        ],
    ),
    look(
        "silver_bleach",
        "Silver Bleach",
        S::Featured,
        &[("saturation", -28.0), ("contrast", 30.0)],
    ),
    look(
        "matte_soft",
        "Matte Soft",
        S::Featured,
        &[
            ("lift_r", 6.0),
            ("lift_g", 6.0),
            ("lift_b", 6.0),
            ("gain_r", 92.0),
            ("gain_g", 92.0),
            ("gain_b", 92.0),
            ("saturation", -12.0),
        ],
    ),
    look(
        "mono_punch",
        "Mono Punch",
        S::Mono,
        &[("saturation", -50.0), ("contrast", 20.0)],
    ),
    look(
        "amber_film",
        "Amber Film",
        S::Retro,
        &[
            ("gain_r", 110.0),
            ("gain_g", 102.0),
            ("gain_b", 82.0),
            ("lift_r", 3.0),
            ("lift_g", 1.0),
            ("contrast", 10.0),
        ],
    ),
    look(
        "deep_blue",
        "Deep Blue",
        S::Night,
        &[("gain_r", 60.0), ("gain_g", 68.0), ("gain_b", 92.0)],
    ),
    // Creative presets.
    look(
        "day_for_night",
        "Day for Night",
        S::Night,
        &[
            ("temperature", -30.0),
            ("brightness", -12.0),
            ("contrast", 10.0),
            ("saturation", -15.0),
            ("lift_b", 6.0),
        ],
    ),
    look(
        "late_gold",
        "Late Gold",
        S::Life,
        &[
            ("temperature", 28.0),
            ("highlight", -10.0),
            ("saturation", 8.0),
            ("gain_r", 106.0),
            ("gain_b", 92.0),
        ],
    ),
    look(
        "faded_print",
        "Faded Print",
        S::Retro,
        &[("fade", 22.0), ("contrast", -8.0), ("saturation", -8.0)],
    ),
    FilterDefinition {
        id: "cross_process",
        name: "Cross Process",
        shelf: S::Featured,
        scalars: &[("lift_b", 8.0)],
        curves: &[
            ("curve_red", &[[0.0, 0.0], [0.5, 0.56], [1.0, 1.0]]),
            ("curve_green", &[[0.0, 0.0], [0.5, 0.52], [1.0, 1.0]]),
            ("curve_blue", &[[0.0, 0.0], [0.5, 0.44], [1.0, 1.0]]),
        ],
    },
    // Authored looks.
    look(
        "sepia_print",
        "Sepia Print",
        S::Retro,
        &[
            ("saturation", -35.0),
            ("temperature", 35.0),
            ("gain_b", 85.0),
            ("fade", 10.0),
        ],
    ),
    look(
        "instant_fade",
        "Instant Fade",
        S::Retro,
        &[
            ("fade", 28.0),
            ("temperature", 12.0),
            ("hsl_green_saturation", -20.0),
            ("vignette", 15.0),
            ("grain", 10.0),
        ],
    ),
    look(
        "soft_mono",
        "Soft Mono",
        S::Mono,
        &[("saturation", -50.0), ("contrast", -10.0), ("fade", 10.0)],
    ),
    look(
        "noir",
        "Noir",
        S::Mono,
        &[
            ("saturation", -50.0),
            ("contrast", 40.0),
            ("shadow", -15.0),
            ("vignette", 25.0),
            ("grain", 8.0),
        ],
    ),
    look(
        "moonlit",
        "Moonlit",
        S::Night,
        &[
            ("temperature", -35.0),
            ("brightness", -10.0),
            ("saturation", -10.0),
            ("gain_b", 108.0),
            ("vignette", 15.0),
        ],
    ),
    look(
        "neon_city",
        "Neon City",
        S::Night,
        &[
            ("contrast", 20.0),
            ("saturation", 25.0),
            ("shadow", -10.0),
            ("lift_r", 3.0),
            ("lift_b", 6.0),
            ("hsl_magenta_saturation", 15.0),
            ("hsl_cyan_saturation", 15.0),
        ],
    ),
    look(
        "fresh_day",
        "Fresh Day",
        S::Life,
        &[
            ("brightness", 6.0),
            ("saturation", 15.0),
            ("temperature", 8.0),
            ("highlight", -8.0),
            ("shine", 5.0),
        ],
    ),
    look(
        "blockbuster",
        "Blockbuster",
        S::Movies,
        &[
            ("contrast", 22.0),
            ("saturation", -8.0),
            ("gain_r", 104.0),
            ("gain_b", 96.0),
            ("lift_b", 4.0),
            ("vignette", 15.0),
            ("grain", 5.0),
        ],
    ),
    // Tools: technical grades, not counted among the looks.
    look(
        "lift_shadows",
        "Lift Shadows",
        S::Tools,
        &[("shadow", 40.0)],
    ),
    look(
        "protect_highlights",
        "Protect Highlights",
        S::Tools,
        &[("highlight", -40.0)],
    ),
    look(
        "half_saturation",
        "Half Saturation",
        S::Tools,
        &[("saturation", -25.0)],
    ),
    look(
        "boost_colour",
        "Boost Colour",
        S::Tools,
        &[("saturation", 20.0)],
    ),
];

/// The id of the colour-grade instance a filter clip's stack holds.
pub const FILTER_INSTANCE_ID: &str = "filter";

/// Whether the item is a filter clip: an effect item whose stack holds the filter instance.
pub fn is_filter_clip(item: &Item) -> bool {
    matches!(item.content, ItemParams::Effect(_))
        && item
            .effects
            .iter()
            .any(|effect| effect.id == FILTER_INSTANCE_ID)
}

/// The catalogued look a filter clip carries, when its instance still equals one.
pub fn filter_look(item: &Item) -> Option<&'static FilterDefinition> {
    let instance = item
        .effects
        .iter()
        .find(|effect| effect.id == FILTER_INSTANCE_ID)?;
    DEFINITIONS
        .iter()
        .find(|definition| definition.instance(FILTER_INSTANCE_ID).ok().as_ref() == Some(instance))
}

/// Every filter, looks first and then the Tools shelf.
pub fn filter_definitions() -> &'static [FilterDefinition] {
    DEFINITIONS
}

/// The filter with this id.
pub fn filter_definition(id: &str) -> Option<&'static FilterDefinition> {
    DEFINITIONS.iter().find(|definition| definition.id == id)
}
