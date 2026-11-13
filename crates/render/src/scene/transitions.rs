//! Transition kinds over premultiplied linear-light frames, dispatched by catalogue id.
use crate::{Error, ExecutionHooks, Image, Result, check};
use project::{TransitionParamDef, TransitionParams, TransitionValue, transition_definition};
use rayon::prelude::*;

mod cards;
mod dissolve;
mod grunge;
mod lights;
mod motion;
mod pixels;
pub mod thumbnail;
mod wipe;

/// Rows evaluated between cancellation checks.
const ROW_GROUP: usize = 8;

/// Typed access to one transition's parameters: the persisted override, else the catalogue
/// default. A wrong id or type is a typed error, so a kind that misreads its own definition
/// fails the registry contract instead of rendering with a guess.
pub struct Params<'a> {
    defs: &'static [TransitionParamDef],
    set: &'a TransitionParams,
}
impl<'a> Params<'a> {
    pub fn new(defs: &'static [TransitionParamDef], set: &'a TransitionParams) -> Self {
        Self { defs, set }
    }
    fn value(&self, id: &str) -> Result<TransitionValue> {
        let def = self
            .defs
            .iter()
            .find(|d| d.id == id)
            .ok_or(Error::Invalid("transition parameter"))?;
        let value = self
            .set
            .get(id)
            .cloned()
            .unwrap_or_else(|| def.kind.default_value());
        def.kind
            .validate(&value)
            .map_err(|_| Error::Invalid("transition parameter value"))?;
        Ok(value)
    }
    fn validate(&self) -> Result<()> {
        for def in self.defs {
            self.value(def.id)?;
        }
        Ok(())
    }
    /// A float parameter in its own unit.
    pub fn num(&self, id: &str) -> Result<f32> {
        match self.value(id)? {
            TransitionValue::Float(v) => Ok(v as f32),
            _ => Err(Error::Invalid("transition parameter type")),
        }
    }
    /// A float parameter that is a percentage, as 0..1 of its range.
    pub fn fraction(&self, id: &str) -> Result<f32> {
        Ok(self.num(id)? / 100.0)
    }
    pub fn choice(&self, id: &str) -> Result<u32> {
        match self.value(id)? {
            TransitionValue::Choice(v) => Ok(v),
            _ => Err(Error::Invalid("transition parameter type")),
        }
    }
    pub fn flag(&self, id: &str) -> Result<bool> {
        match self.value(id)? {
            TransitionValue::Bool(v) => Ok(v),
            _ => Err(Error::Invalid("transition parameter type")),
        }
    }
    /// An opaque linear-light colour; the stored alpha is ignored.
    pub fn color(&self, id: &str) -> Result<[f32; 4]> {
        match self.value(id)? {
            TransitionValue::Color(c) => {
                let [r, g, b, _] = c.0;
                Ok(crate::image::color([r, g, b, 255]))
            }
            _ => Err(Error::Invalid("transition parameter type")),
        }
    }
    /// A point as fractions of the frame, origin top-left.
    pub fn point(&self, id: &str) -> Result<[f32; 2]> {
        match self.value(id)? {
            TransitionValue::Point([x, y]) => Ok([x as f32, y as f32]),
            _ => Err(Error::Invalid("transition parameter type")),
        }
    }
    /// An angle in radians, clockwise.
    pub fn angle(&self, id: &str) -> Result<f32> {
        match self.value(id)? {
            TransitionValue::Angle(v) => Ok((v as f32).to_radians()),
            _ => Err(Error::Invalid("transition parameter type")),
        }
    }
    pub fn seed(&self, id: &str) -> Result<u32> {
        match self.value(id)? {
            TransitionValue::Seed(v) => Ok(v),
            _ => Err(Error::Invalid("transition parameter type")),
        }
    }
}

/// Everything a kind reads: both frames, progress strictly inside (0, 1), the pixel scale and
/// the parameters. `scale` converts pixel-denominated parameters from sequence pixels to the
/// pixels of these frames (0.5 for a half-resolution preview).
pub struct Tx<'a> {
    pub outgoing: &'a Image,
    pub incoming: &'a Image,
    pub progress: f32,
    pub width: usize,
    pub height: usize,
    pub scale: f32,
    pub params: Params<'a>,
}
impl Tx<'_> {
    /// The outgoing pixel at `(x, y)`.
    pub fn outgoing_at(&self, x: usize, y: usize) -> [f32; 4] {
        pixel(self.outgoing, self.width, x, y)
    }
    /// The incoming pixel at `(x, y)`.
    pub fn incoming_at(&self, x: usize, y: usize) -> [f32; 4] {
        pixel(self.incoming, self.width, x, y)
    }
}
fn pixel(image: &Image, width: usize, x: usize, y: usize) -> [f32; 4] {
    y.checked_mul(width)
        .and_then(|row| row.checked_add(x))
        .and_then(|n| image.pixels.get(n))
        .copied()
        .unwrap_or([0.; 4])
}

/// The destination frame. `paint` evaluates every pixel in parallel, checking cancellation
/// once per row group.
pub struct Out<'a> {
    pixels: &'a mut [[f32; 4]],
    width: usize,
    hooks: &'a dyn ExecutionHooks,
}
impl Out<'_> {
    /// Set each pixel to `f(x, y)` with integer pixel coordinates. The written value is clamped
    /// to the working-image contract: finite, 0..=1 and RGB at most the alpha.
    pub fn paint(&mut self, f: impl Fn(usize, usize) -> [f32; 4] + Sync) -> Result<()> {
        let hooks = self.hooks;
        self.pixels
            .par_chunks_mut(self.width)
            .enumerate()
            .try_for_each(|(y, row)| {
                if y.is_multiple_of(ROW_GROUP) {
                    check(hooks)?;
                }
                for (x, px) in row.iter_mut().enumerate() {
                    *px = pixels::working(f(x, y));
                }
                Ok(())
            })
    }
    /// Hooks for a kind that must prepare a scratch frame before painting.
    pub fn hooks(&self) -> &dyn ExecutionHooks {
        self.hooks
    }
}

type Run = fn(&Tx, &mut Out) -> Result<()>;
/// Which sides still contribute pixels at a progress in [0, 1).
type Sides = fn(&Params, f32) -> Result<(bool, bool)>;
struct Kind {
    id: &'static str,
    run: Run,
    sides: Sides,
}
const KINDS: &[Kind] = &[
    Kind {
        id: "cross_dissolve",
        run: cross_dissolve,
        sides: both_sides,
    },
    Kind {
        id: "dip_to_black",
        run: |t, out| dip(t, out, [0., 0., 0., 1.], 0.),
        sides: |_, p| Ok(dip_sides(p, 0.)),
    },
    Kind {
        id: "dip_to_white",
        run: |t, out| dip(t, out, [1.; 4], 0.),
        sides: |_, p| Ok(dip_sides(p, 0.)),
    },
    Kind {
        id: "dip_to_color",
        run: |t, out| dip(t, out, t.params.color("color")?, hold(&t.params)?),
        sides: |params, p| Ok(dip_sides(p, hold(params)?)),
    },
    // Dissolves (261).
    Kind {
        id: "light_fade",
        run: |t, out| dissolve::run("light_fade", t, out),
        sides: both_sides,
    },
    Kind {
        id: "film_fade",
        run: |t, out| dissolve::run("film_fade", t, out),
        sides: both_sides,
    },
    Kind {
        id: "soft_focus_fade",
        run: |t, out| dissolve::run("soft_focus_fade", t, out),
        sides: both_sides,
    },
    Kind {
        id: "burn_through",
        run: |t, out| dissolve::run("burn_through", t, out),
        sides: both_sides,
    },
    Kind {
        id: "brightness_fade",
        run: |t, out| dissolve::run("brightness_fade", t, out),
        sides: both_sides,
    },
    Kind {
        id: "pixel_blocks",
        run: |t, out| dissolve::run("pixel_blocks", t, out),
        sides: both_sides,
    },
    // Wipes (261).
    Kind {
        id: "straight_wipe",
        run: |t, out| wipe::run("straight_wipe", t, out),
        sides: both_sides,
    },
    Kind {
        id: "soft_edge_wipe",
        run: |t, out| wipe::run("soft_edge_wipe", t, out),
        sides: both_sides,
    },
    Kind {
        id: "clock_wipe",
        run: |t, out| wipe::run("clock_wipe", t, out),
        sides: both_sides,
    },
    Kind {
        id: "corner_sweep",
        run: |t, out| wipe::run("corner_sweep", t, out),
        sides: both_sides,
    },
    Kind {
        id: "star_wipe",
        run: |t, out| wipe::run("star_wipe", t, out),
        sides: both_sides,
    },
    Kind {
        id: "panel_wipe",
        run: |t, out| wipe::run("panel_wipe", t, out),
        sides: both_sides,
    },
    Kind {
        id: "stair_wipe",
        run: |t, out| wipe::run("stair_wipe", t, out),
        sides: both_sides,
    },
    Kind {
        id: "stretch_wipe",
        run: |t, out| wipe::run("stretch_wipe", t, out),
        sides: both_sides,
    },
    Kind {
        id: "neon_edge_wipe",
        run: |t, out| wipe::run("neon_edge_wipe", t, out),
        sides: both_sides,
    },
    Kind {
        id: "circle_reveal",
        run: |t, out| wipe::run("circle_reveal", t, out),
        sides: both_sides,
    },
    Kind {
        id: "box_reveal",
        run: |t, out| wipe::run("box_reveal", t, out),
        sides: both_sides,
    },
    Kind {
        id: "diamond_reveal",
        run: |t, out| wipe::run("diamond_reveal", t, out),
        sides: both_sides,
    },
    Kind {
        id: "cross_reveal",
        run: |t, out| wipe::run("cross_reveal", t, out),
        sides: both_sides,
    },
    Kind {
        id: "double_doors",
        run: |t, out| wipe::run("double_doors", t, out),
        sides: both_sides,
    },
    Kind {
        id: "corner_inset",
        run: |t, out| wipe::run("corner_inset", t, out),
        sides: both_sides,
    },
    // Lights and blurs (261).
    Kind {
        id: "zoom_cross",
        run: |t, out| lights::run("zoom_cross", t, out),
        sides: both_sides,
    },
    Kind {
        id: "zoom_smear",
        run: |t, out| lights::run("zoom_smear", t, out),
        sides: both_sides,
    },
    Kind {
        id: "spin_smear",
        run: |t, out| lights::run("spin_smear", t, out),
        sides: both_sides,
    },
    Kind {
        id: "streak_smear",
        run: |t, out| lights::run("streak_smear", t, out),
        sides: both_sides,
    },
    Kind {
        id: "lens_defocus",
        run: |t, out| lights::run("lens_defocus", t, out),
        sides: both_sides,
    },
    Kind {
        id: "bloom",
        run: |t, out| lights::run("bloom", t, out),
        sides: both_sides,
    },
    Kind {
        id: "flash",
        run: |t, out| lights::run("flash", t, out),
        sides: both_sides,
    },
    Kind {
        id: "lens_flare",
        run: |t, out| lights::run("lens_flare", t, out),
        sides: both_sides,
    },
    Kind {
        id: "light_leak",
        run: |t, out| lights::run("light_leak", t, out),
        sides: both_sides,
    },
    Kind {
        id: "colour_leak",
        run: |t, out| lights::run("colour_leak", t, out),
        sides: both_sides,
    },
    Kind {
        id: "colour_burn",
        run: |t, out| lights::run("colour_burn", t, out),
        sides: both_sides,
    },
    Kind {
        id: "light_sweep",
        run: |t, out| lights::run("light_sweep", t, out),
        sides: both_sides,
    },
    Kind {
        id: "light_rays",
        run: |t, out| lights::run("light_rays", t, out),
        sides: both_sides,
    },
    Kind {
        id: "phosphor_trail",
        run: |t, out| lights::run("phosphor_trail", t, out),
        sides: both_sides,
    },
    Kind {
        id: "solarise",
        run: |t, out| lights::run("solarise", t, out),
        sides: both_sides,
    },
    Kind {
        id: "stripes",
        run: |t, out| lights::run("stripes", t, out),
        sides: both_sides,
    },
    // Grunge and distort (261).
    Kind {
        id: "scramble",
        run: |t, out| grunge::run("scramble", t, out),
        sides: both_sides,
    },
    Kind {
        id: "digital_glitch",
        run: |t, out| grunge::run("digital_glitch", t, out),
        sides: both_sides,
    },
    Kind {
        id: "tape_damage",
        run: |t, out| grunge::run("tape_damage", t, out),
        sides: both_sides,
    },
    Kind {
        id: "flicker_cut",
        run: |t, out| grunge::run("flicker_cut", t, out),
        sides: both_sides,
    },
    Kind {
        id: "quake",
        run: |t, out| grunge::run("quake", t, out),
        sides: both_sides,
    },
    Kind {
        id: "glass_shards",
        run: |t, out| grunge::run("glass_shards", t, out),
        sides: both_sides,
    },
    Kind {
        id: "rough_edge",
        run: |t, out| grunge::run("rough_edge", t, out),
        sides: both_sides,
    },
    Kind {
        id: "kaleidoscope",
        run: |t, out| grunge::run("kaleidoscope", t, out),
        sides: both_sides,
    },
    Kind {
        id: "liquid_warp",
        run: |t, out| grunge::run("liquid_warp", t, out),
        sides: both_sides,
    },
    Kind {
        id: "screen_off",
        run: |t, out| grunge::run("screen_off", t, out),
        sides: both_sides,
    },
    // Slides (262).
    Kind {
        id: "push",
        run: |t, out| motion::run("push", t, out),
        sides: both_sides,
    },
    Kind {
        id: "slide",
        run: |t, out| motion::run("slide", t, out),
        sides: both_sides,
    },
    Kind {
        id: "roll",
        run: |t, out| motion::run("roll", t, out),
        sides: both_sides,
    },
    Kind {
        id: "split",
        run: |t, out| motion::run("split", t, out),
        sides: both_sides,
    },
    Kind {
        id: "stretch",
        run: |t, out| motion::run("stretch", t, out),
        sides: both_sides,
    },
    Kind {
        id: "film_roll",
        run: |t, out| motion::run("film_roll", t, out),
        sides: both_sides,
    },
    Kind {
        id: "spring_motion",
        run: |t, out| motion::run("spring_motion", t, out),
        sides: both_sides,
    },
    Kind {
        id: "wave",
        run: |t, out| motion::run("wave", t, out),
        sides: both_sides,
    },
    Kind {
        id: "slice",
        run: |t, out| motion::run("slice", t, out),
        sides: both_sides,
    },
    Kind {
        id: "louver",
        run: |t, out| motion::run("louver", t, out),
        sides: both_sides,
    },
    Kind {
        id: "frame",
        run: |t, out| motion::run("frame", t, out),
        sides: both_sides,
    },
    // 3D (262).
    Kind {
        id: "roll_3d",
        run: |t, out| motion::run("roll_3d", t, out),
        sides: both_sides,
    },
    Kind {
        id: "flip_motion",
        run: |t, out| motion::run("flip_motion", t, out),
        sides: both_sides,
    },
    Kind {
        id: "fold_motion",
        run: |t, out| motion::run("fold_motion", t, out),
        sides: both_sides,
    },
    Kind {
        id: "spin_motion",
        run: |t, out| motion::run("spin_motion", t, out),
        sides: both_sides,
    },
    Kind {
        id: "spin_3d",
        run: |t, out| motion::run("spin_3d", t, out),
        sides: both_sides,
    },
    Kind {
        id: "spinback_3d",
        run: |t, out| motion::run("spinback_3d", t, out),
        sides: both_sides,
    },
    Kind {
        id: "page_peel",
        run: |t, out| motion::run("page_peel", t, out),
        sides: both_sides,
    },
    Kind {
        id: "mirror_transition",
        run: |t, out| motion::run("mirror_transition", t, out),
        sides: both_sides,
    },
    Kind {
        id: "block_motion",
        run: |t, out| motion::run("block_motion", t, out),
        sides: both_sides,
    },
    // Camera (262).
    Kind {
        id: "whip",
        run: |t, out| motion::run("whip", t, out),
        sides: both_sides,
    },
    Kind {
        id: "motion_camera",
        run: |t, out| motion::run("motion_camera", t, out),
        sides: both_sides,
    },
    Kind {
        id: "travel_motion",
        run: |t, out| motion::run("travel_motion", t, out),
        sides: both_sides,
    },
    Kind {
        id: "motion_tween",
        run: |t, out| motion::run("motion_tween", t, out),
        sides: both_sides,
    },
    Kind {
        id: "pull_motion",
        run: |t, out| motion::run("pull_motion", t, out),
        sides: both_sides,
    },
    Kind {
        id: "pop_motion",
        run: |t, out| motion::run("pop_motion", t, out),
        sides: both_sides,
    },
    // Wipes (262).
    Kind {
        id: "shape_dissolve",
        run: |t, out| wipe::run("shape_dissolve", t, out),
        sides: both_sides,
    },
    Kind {
        id: "shape_flow",
        run: |t, out| wipe::run("shape_flow", t, out),
        sides: both_sides,
    },
];

/// Every registered kind id; the catalogue and this table must list the same set.
pub fn registered_kinds() -> impl Iterator<Item = &'static str> {
    KINDS.iter().map(|k| k.id)
}
fn kind(id: &str) -> Result<&'static Kind> {
    KINDS
        .iter()
        .find(|k| k.id == id)
        .ok_or(Error::Invalid("transition kind"))
}

/// Combine `outgoing` and `incoming` at `progress` (clamped to 0..=1) with the named kind.
///
/// Both frames must share one size. Progress 0 returns the outgoing frame and 1 the incoming
/// one exactly. The result is finite, deterministic and continuous towards both ends.
/// Cancellation is checked per row group; a cancelled call returns `Cancelled`.
pub fn apply(
    kind_id: &str,
    params: &TransitionParams,
    outgoing: &Image,
    incoming: &Image,
    progress: f64,
    scale: f32,
    hooks: &dyn ExecutionHooks,
) -> Result<Image> {
    let definition = transition_definition(kind_id)?;
    let kind = kind(kind_id)?;
    if !progress.is_finite() {
        return Err(Error::Invalid("transition progress"));
    }
    if !scale.is_finite() || scale <= 0. {
        return Err(Error::Invalid("transition scale"));
    }
    if (outgoing.width, outgoing.height) != (incoming.width, incoming.height) {
        return Err(Error::Invalid("transition frame size"));
    }
    let params = Params::new(definition.params, params);
    params.validate()?;
    let p = progress.clamp(0., 1.) as f32;
    if p <= 0. {
        return outgoing.copy(hooks);
    }
    if p >= 1. {
        return incoming.copy(hooks);
    }
    let (width, height) = (outgoing.width as usize, outgoing.height as usize);
    let tx = Tx {
        outgoing,
        incoming,
        progress: p,
        width,
        height,
        scale,
        params,
    };
    let mut out = Image::new(outgoing.width, outgoing.height, [0.; 4], hooks)?;
    (kind.run)(
        &tx,
        &mut Out {
            pixels: &mut out.pixels,
            width,
            hooks,
        },
    )?;
    Ok(out)
}

/// Which of the (outgoing, incoming) clips contribute pixels at `progress` in [0, 1); hit
/// testing selects only contributing clips.
pub fn exposed_sides(
    kind_id: &str,
    params: &TransitionParams,
    progress: f64,
) -> Result<(bool, bool)> {
    let definition = transition_definition(kind_id)?;
    let params = Params::new(definition.params, params);
    params.validate()?;
    (kind(kind_id)?.sides)(&params, progress as f32)
}

fn both_sides(_: &Params, p: f32) -> Result<(bool, bool)> {
    Ok((p < 1., p > 0.))
}
fn mix(a: [f32; 4], b: [f32; 4], k: f32) -> [f32; 4] {
    std::array::from_fn(|i| {
        let (l, r) = (
            a.get(i).copied().unwrap_or(0.),
            b.get(i).copied().unwrap_or(0.),
        );
        l * (1. - k) + r * k
    })
}
fn cross_dissolve(t: &Tx, out: &mut Out) -> Result<()> {
    let p = t.progress;
    out.paint(|x, y| mix(t.outgoing_at(x, y), t.incoming_at(x, y), p))
}
fn hold(params: &Params) -> Result<f32> {
    Ok(params.fraction("hold")?.clamp(0., 0.9))
}
/// Fade the outgoing frame to `color`, stay on it for `hold` of the span, fade in the incoming.
fn dip(t: &Tx, out: &mut Out, color: [f32; 4], hold: f32) -> Result<()> {
    let p = t.progress;
    let half = (1. - hold) / 2.;
    out.paint(|x, y| {
        if p < half {
            mix(t.outgoing_at(x, y), color, p / half)
        } else if p > 1. - half {
            mix(color, t.incoming_at(x, y), (p - (1. - half)) / half)
        } else {
            color
        }
    })
}
fn dip_sides(p: f32, hold: f32) -> (bool, bool) {
    let half = (1. - hold) / 2.;
    (p < half, p > 1. - half)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;
    use project::{Color, TransitionParamKind as K};

    const CHOICES: &[&str] = &["a", "b"];
    const fn def(id: &'static str, kind: K) -> TransitionParamDef {
        TransitionParamDef { id, name: id, kind }
    }
    const DEFS: &[TransitionParamDef] = &[
        def(
            "num",
            K::Float {
                min: 0.,
                max: 100.,
                default: 25.,
                unit: "%",
            },
        ),
        def(
            "choice",
            K::Choice {
                options: CHOICES,
                default: 1,
            },
        ),
        def("flag", K::Bool { default: false }),
        def(
            "color",
            K::Color {
                default: Color([255, 0, 0, 7]),
            },
        ),
        def(
            "point",
            K::Point {
                default: [0.5, 0.25],
            },
        ),
        def("angle", K::Angle { default: 180. }),
        def("seed", K::Seed { default: 1 }),
    ];

    #[test]
    fn params_resolve_overrides_and_defaults_and_refuse_wrong_ids_and_types() {
        let none = TransitionParams::new();
        let p = Params::new(DEFS, &none);
        assert_eq!(
            (p.num("num").unwrap(), p.fraction("num").unwrap()),
            (25., 0.25)
        );
        assert_eq!(p.choice("choice").unwrap(), 1);
        assert!(!p.flag("flag").unwrap());
        // The stored alpha is ignored: dips are always opaque.
        assert_eq!(p.color("color").unwrap(), [1., 0., 0., 1.]);
        assert_eq!(p.point("point").unwrap(), [0.5, 0.25]);
        assert!((p.angle("angle").unwrap() - std::f32::consts::PI).abs() < 1e-6);
        assert_eq!(p.seed("seed").unwrap(), 1);
        let set = TransitionParams::from([
            ("num".to_owned(), TransitionValue::Float(50.)),
            ("choice".to_owned(), TransitionValue::Choice(0)),
            ("flag".to_owned(), TransitionValue::Bool(true)),
            (
                "color".to_owned(),
                TransitionValue::Color(Color([0, 0, 255, 255])),
            ),
            ("point".to_owned(), TransitionValue::Point([1., 0.])),
            ("angle".to_owned(), TransitionValue::Angle(90.)),
            ("seed".to_owned(), TransitionValue::Seed(9)),
        ]);
        let p = Params::new(DEFS, &set);
        assert_eq!(p.num("num").unwrap(), 50.);
        assert_eq!(p.choice("choice").unwrap(), 0);
        assert!(p.flag("flag").unwrap());
        assert_eq!(p.color("color").unwrap(), [0., 0., 1., 1.]);
        assert_eq!(p.point("point").unwrap(), [1., 0.]);
        assert!((p.angle("angle").unwrap() - std::f32::consts::FRAC_PI_2).abs() < 1e-6);
        assert_eq!(p.seed("seed").unwrap(), 9);
        let wrong = TransitionParams::from([("num".to_owned(), TransitionValue::Seed(1))]);
        assert!(Params::new(DEFS, &wrong).num("num").is_err());
        for read in [
            p.num("missing").map(drop),
            p.num("choice").map(drop),
            p.choice("num").map(drop),
            p.flag("num").map(drop),
            p.color("num").map(drop),
            p.point("num").map(drop),
            p.angle("num").map(drop),
            p.seed("num").map(drop),
        ] {
            assert!(matches!(read, Err(Error::Invalid(_))));
        }
    }
}
