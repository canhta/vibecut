//! Monotone tone-curve points on the shared 1/4096 input grid.
use crate::{Error, Result};
use serde::{
    Deserialize, Deserializer, Serialize, Serializer,
    de::{self, SeqAccess, Visitor},
};
use std::fmt;

/// A curve has at least the two fixed endpoints and at most this many points.
pub const CURVE_MIN_POINTS: usize = 2;
pub const CURVE_MAX_POINTS: usize = 32;
/// Every authored input coordinate lies on this exact grid.
pub const CURVE_GRID: f64 = 4096.0;

/// Piecewise-linear tone curve with fixed `(0,0)`/`(1,1)` endpoints.
///
/// Points are ordered, strictly increasing in `x` on [`CURVE_GRID`] and
/// nondecreasing in `y`, so the map is monotone without overshoot. The two
/// endpoints are the identity and emit no effect pass.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ToneCurve {
    points: [[f64; 2]; CURVE_MAX_POINTS],
    len: u8,
}
const IDENTITY_POINTS: [[f64; 2]; CURVE_MAX_POINTS] = {
    let mut points = [[0.0; 2]; CURVE_MAX_POINTS];
    points[1] = [1.0, 1.0];
    points
};
impl ToneCurve {
    /// The neutral curve, exactly the two fixed endpoints.
    pub const IDENTITY: Self = Self {
        points: IDENTITY_POINTS,
        len: CURVE_MIN_POINTS as u8,
    };

    /// Validate and copy authored points. Nothing is sorted, repaired or
    /// deduplicated: a hostile or impossible shape refuses whole.
    pub fn new(points: &[[f64; 2]]) -> Result<Self> {
        validate(points)?;
        let mut stored = [[0.0; 2]; CURVE_MAX_POINTS];
        for (slot, point) in stored.iter_mut().zip(points) {
            *slot = *point;
        }
        Ok(Self {
            points: stored,
            len: points.len() as u8,
        })
    }

    /// The active points in authored order, endpoints included.
    pub fn points(&self) -> &[[f64; 2]] {
        self.points
            .get(..usize::from(self.len))
            .unwrap_or(&IDENTITY_POINTS[..CURVE_MIN_POINTS])
    }

    pub fn is_identity(&self) -> bool {
        self.points() == &[[0.0, 0.0], [1.0, 1.0]][..]
    }

    /// Piecewise-linear map of one finite `0..=1` value. Values outside the
    /// domain clamp to the matching endpoint; there is no HDR extrapolation.
    pub fn map(&self, value: f32) -> f32 {
        let x = value.clamp(0.0, 1.0);
        let Some(first) = self.points().first() else {
            return x;
        };
        let mut low = *first;
        for high in self.points().iter().skip(1) {
            if x <= high[0] as f32 {
                let x0 = low[0] as f32;
                let x1 = high[0] as f32;
                let span = x1 - x0;
                if span <= 0.0 {
                    return high[1] as f32;
                }
                let t = (x - x0) / span;
                return (low[1] as f32 + (high[1] as f32 - low[1] as f32) * t).clamp(0.0, 1.0);
            }
            low = *high;
        }
        low[1] as f32
    }
}
impl Default for ToneCurve {
    fn default() -> Self {
        Self::IDENTITY
    }
}
fn validate(points: &[[f64; 2]]) -> Result<()> {
    let invalid = || Error::Invalid("tone curve");
    if !(CURVE_MIN_POINTS..=CURVE_MAX_POINTS).contains(&points.len()) {
        return Err(invalid());
    }
    let Some((first, rest)) = points.split_first() else {
        return Err(invalid());
    };
    let Some((last, middle)) = rest.split_last() else {
        return Err(invalid());
    };
    if *first != [0.0, 0.0] || *last != [1.0, 1.0] {
        return Err(invalid());
    }
    let mut previous = *first;
    for point in middle {
        check_point(*point)?;
        let [x, y] = *point;
        if x - previous[0] < 1.0 / CURVE_GRID || y < previous[1] {
            return Err(invalid());
        }
        previous = *point;
    }
    if last[0] - previous[0] < 1.0 / CURVE_GRID || last[1] < previous[1] {
        return Err(invalid());
    }
    Ok(())
}
fn check_point([x, y]: [f64; 2]) -> Result<()> {
    let ticks = x * CURVE_GRID;
    if !x.is_finite()
        || !y.is_finite()
        || !(0.0..=1.0).contains(&x)
        || !(0.0..=1.0).contains(&y)
        || ticks.round() != ticks
    {
        return Err(Error::Invalid("tone curve"));
    }
    Ok(())
}

impl Serialize for ToneCurve {
    fn serialize<S: Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        self.points().serialize(serializer)
    }
}
struct CurveVisitor;
impl<'de> Visitor<'de> for CurveVisitor {
    type Value = ToneCurve;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a tone curve with 2 to 32 ordered [x, y] points")
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> std::result::Result<ToneCurve, A::Error> {
        let mut points = [[0.0; 2]; CURVE_MAX_POINTS];
        let mut len = 0usize;
        while let Some(point) = seq.next_element::<[f64; 2]>()? {
            let Some(slot) = points.get_mut(len) else {
                return Err(de::Error::custom("tone curve points"));
            };
            *slot = point;
            len += 1;
        }
        ToneCurve::new(points.get(..len).unwrap_or_default()).map_err(de::Error::custom)
    }
}
impl<'de> Deserialize<'de> for ToneCurve {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> std::result::Result<Self, D::Error> {
        deserializer.deserialize_seq(CurveVisitor)
    }
}
impl schemars::JsonSchema for ToneCurve {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "ToneCurve".into()
    }
    fn json_schema(_: &mut schemars::SchemaGenerator) -> schemars::Schema {
        schemars::json_schema!({
            "type": "array",
            "minItems": 2,
            "maxItems": 32,
            "items": {
                "type": "array",
                "minItems": 2,
                "maxItems": 2,
                "items": {"type": "number", "minimum": 0.0, "maximum": 1.0}
            }
        })
    }
}
