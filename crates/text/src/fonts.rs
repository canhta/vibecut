//! The bundled font database and deterministic family/style resolution.
//!
//! Faces are described by [`BUNDLED`] data and embedded with `include_bytes!`. Their order is the
//! generic fallback order, so a missing glyph always resolves to the same face for a given
//! request. Language profiles supply their own ordered fallback chain to [`fallback_for`].

use std::sync::OnceLock;

use skrifa::Font;

pub const INTER_REGULAR: &[u8] = include_bytes!("../../../assets/fonts/Inter-Regular.ttf");
pub const INTER_MEDIUM: &[u8] = include_bytes!("../../../assets/fonts/Inter-Medium.ttf");
pub const INTER_SEMIBOLD: &[u8] = include_bytes!("../../../assets/fonts/Inter-SemiBold.ttf");
pub const INTER_BOLD: &[u8] = include_bytes!("../../../assets/fonts/Inter-Bold.ttf");
pub const INTER_ITALIC: &[u8] = include_bytes!("../../../assets/fonts/Inter-Italic.ttf");
pub const NOTO_SERIF_REGULAR: &[u8] = include_bytes!("../../../assets/fonts/NotoSerif-Regular.ttf");
pub const VIBECUT_SANS_CJK_REGULAR: &[u8] =
    include_bytes!("../../../assets/fonts/VibeCutSansCJK-Regular.otf");

/// The default family for new text.
pub const DEFAULT_FAMILY: &str = "Inter";

/// Index of a bundled face; stable for the life of the process and across platforms.
pub type FaceId = usize;

/// One face in the bundled font data table.
struct BundledFace {
    family: &'static str,
    style: &'static str,
    weight: u16,
    italic: bool,
    bytes: &'static [u8],
}

/// Bundled faces in generic fallback order. Adding a script adds rows and assets, not logic.
const BUNDLED: &[BundledFace] = &[
    BundledFace {
        family: "Inter",
        style: "Regular",
        weight: 400,
        italic: false,
        bytes: INTER_REGULAR,
    },
    BundledFace {
        family: "Inter",
        style: "Medium",
        weight: 500,
        italic: false,
        bytes: INTER_MEDIUM,
    },
    BundledFace {
        family: "Inter",
        style: "SemiBold",
        weight: 600,
        italic: false,
        bytes: INTER_SEMIBOLD,
    },
    BundledFace {
        family: "Inter",
        style: "Bold",
        weight: 700,
        italic: false,
        bytes: INTER_BOLD,
    },
    BundledFace {
        family: "Inter",
        style: "Italic",
        weight: 400,
        italic: true,
        bytes: INTER_ITALIC,
    },
    BundledFace {
        family: "Noto Serif",
        style: "Regular",
        weight: 400,
        italic: false,
        bytes: NOTO_SERIF_REGULAR,
    },
    BundledFace {
        family: "VibeCut Sans CJK",
        style: "Regular",
        weight: 400,
        italic: false,
        bytes: VIBECUT_SANS_CJK_REGULAR,
    },
];

/// One loaded font face.
#[derive(Debug)]
pub struct Face {
    pub id: FaceId,
    pub family: &'static str,
    pub style: &'static str,
    pub weight: u16,
    pub italic: bool,
    bytes: &'static [u8],
}

impl Face {
    fn font(&self) -> Option<Font> {
        Font::new(self.bytes, 0)
    }

    fn units_per_em(&self) -> f32 {
        self.font()
            .map(|f| f32::from(f.metrics().units_per_em))
            .filter(|u| *u > 0.0)
            .unwrap_or(1000.0)
    }

    /// Glyph identifier for a character, or `None` when the face lacks it.
    pub fn glyph(&self, c: char) -> Option<u32> {
        let font = self.font()?;
        font.charmap()
            .map_unicode(c as u32)
            .map(|g| g.to_u32())
            .filter(|g| *g != 0)
    }

    /// Whether the face covers a character.
    pub fn has_char(&self, c: char) -> bool {
        self.glyph(c).is_some()
    }

    /// Vertical metrics in pixels for a font size.
    pub fn metrics(&self, px: f32) -> Metrics {
        let Some(font) = self.font() else {
            return Metrics {
                ascent: px * 0.8,
                descent: px * 0.2,
                line_gap: 0.0,
            };
        };
        let metrics = font.metrics();
        let scale = px / f32::from(metrics.units_per_em.max(1));
        let line = if metrics.use_typo_metrics {
            metrics.typo_line.or(metrics.hhea_line)
        } else {
            metrics.hhea_line.or(metrics.typo_line)
        };
        line.map_or(
            Metrics {
                ascent: px * 0.8,
                descent: px * 0.2,
                line_gap: 0.0,
            },
            |line| Metrics {
                ascent: (line.ascender.to_f32() * scale).max(0.0),
                descent: (-line.descender.to_f32() * scale).max(0.0),
                line_gap: (line.line_gap.to_f32() * scale).max(0.0),
            },
        )
    }

    pub(crate) fn font_ref(&self) -> Option<Font> {
        self.font()
    }

    pub(crate) fn scale(&self, px: f32) -> f32 {
        px / self.units_per_em()
    }
}

/// Vertical metrics of a face at a pixel size.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Metrics {
    pub ascent: f32,
    pub descent: f32,
    pub line_gap: f32,
}

fn faces() -> &'static [Face] {
    static FACES: OnceLock<Vec<Face>> = OnceLock::new();
    FACES.get_or_init(|| {
        BUNDLED
            .iter()
            .enumerate()
            .map(|(id, face)| Face {
                id,
                family: face.family,
                style: face.style,
                weight: face.weight,
                italic: face.italic,
                bytes: face.bytes,
            })
            .collect()
    })
}

static EMPTY_FACE: Face = Face {
    id: 0,
    family: "Inter",
    style: "Regular",
    weight: 400,
    italic: false,
    bytes: INTER_REGULAR,
};

/// A face by identifier; unknown identifiers resolve to the default face.
pub fn face(id: FaceId) -> &'static Face {
    match faces().get(id) {
        Some(face) => face,
        None => match faces().first() {
            Some(face) => face,
            None => &EMPTY_FACE,
        },
    }
}

type FamilyStyles = Vec<(String, Vec<(u16, bool, String)>)>;

/// The families and style names of the bundled faces, sorted by family then weight.
pub fn families() -> Vec<(String, Vec<String>)> {
    let mut out: FamilyStyles = Vec::new();
    for face in faces() {
        if let Some(entry) = out.iter_mut().find(|(family, _)| family == face.family) {
            if !entry.1.iter().any(|(_, _, style)| style == face.style) {
                entry
                    .1
                    .push((face.weight, face.italic, face.style.to_owned()));
            }
        } else {
            out.push((
                face.family.to_owned(),
                vec![(face.weight, face.italic, face.style.to_owned())],
            ));
        }
    }
    out.into_iter()
        .map(|(family, mut styles)| {
            styles.sort();
            (
                family,
                styles.into_iter().map(|(_, _, style)| style).collect(),
            )
        })
        .collect()
}

/// The face chosen for a family/style request.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Resolved {
    pub face: FaceId,
    /// The family lacks a real bold face, so the outline is grown.
    pub synth_bold: bool,
    /// The family lacks a real italic face, so the outline is slanted.
    pub synth_italic: bool,
    /// The requested family was not found and Inter was substituted.
    pub missing: bool,
}

fn style_wants(style: &str) -> (u16, bool) {
    let s = style.to_ascii_lowercase().replace([' ', '-', '_'], "");
    let italic = s.contains("italic") || s.contains("oblique");
    let weight = if s.contains("thin") || s.contains("hairline") {
        100
    } else if s.contains("extralight") || s.contains("ultralight") {
        200
    } else if s.contains("light") {
        300
    } else if s.contains("medium") {
        500
    } else if s.contains("semibold") || s.contains("demibold") {
        600
    } else if s.contains("extrabold") || s.contains("ultrabold") {
        800
    } else if s.contains("black") || s.contains("heavy") {
        900
    } else if s.contains("bold") {
        700
    } else {
        400
    };
    (weight, italic)
}

/// Resolve a family and style name. The match is case-insensitive; an unavailable style picks the
/// nearest weight and synthesizes bold or italic. Unknown families substitute Inter.
pub fn resolve(family: &str, style: &str) -> Resolved {
    if let Some(resolved) = resolve_in(family, style) {
        return resolved;
    }
    let mut resolved = resolve_in(DEFAULT_FAMILY, style).unwrap_or(Resolved {
        face: 0,
        synth_bold: false,
        synth_italic: false,
        missing: true,
    });
    resolved.missing = !family.is_empty() && !family.eq_ignore_ascii_case(DEFAULT_FAMILY);
    resolved
}

fn resolve_in(family: &str, style: &str) -> Option<Resolved> {
    let members: Vec<&Face> = faces()
        .iter()
        .filter(|face| face.family.eq_ignore_ascii_case(family))
        .collect();
    if members.is_empty() {
        return None;
    }
    if let Some(exact) = members
        .iter()
        .find(|face| face.style.eq_ignore_ascii_case(style))
    {
        return Some(Resolved {
            face: exact.id,
            synth_bold: false,
            synth_italic: false,
            missing: false,
        });
    }
    let (weight, italic) = style_wants(style);
    let best = members.iter().min_by_key(|face| {
        let italic_penalty = if face.italic == italic { 0 } else { 1000 };
        italic_penalty + i32::from(face.weight).abs_diff(i32::from(weight))
    })?;
    Some(Resolved {
        face: best.id,
        synth_bold: weight >= 600 && best.weight <= 500,
        synth_italic: italic && !best.italic,
        missing: false,
    })
}

/// Face identifiers for a language profile's fallback families, in order. Unknown families are
/// skipped so profiles stay data.
pub fn fallback_chain(families: &[&str], style: &str) -> Vec<FaceId> {
    let mut chain = Vec::new();
    for family in families {
        if let Some(resolved) = resolve_in(family, style)
            && !chain.contains(&resolved.face)
        {
            chain.push(resolved.face);
        }
    }
    chain
}

/// The first face in `chain`, then in the generic bundled order, that covers `c`. The preferred
/// face wins when it covers the character, is whitespace or is a control character.
pub fn fallback_for(c: char, prefer: FaceId, chain: &[FaceId]) -> FaceId {
    let preferred = face(prefer);
    if c.is_control() || c.is_whitespace() || preferred.has_char(c) {
        return preferred.id;
    }
    for id in chain {
        let candidate = face(*id);
        if candidate.has_char(c) {
            return candidate.id;
        }
    }
    faces()
        .iter()
        .find(|face| face.has_char(c))
        .map_or(preferred.id, |face| face.id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundled_families_resolve_exactly_and_nearest() {
        let families = families();
        let inter = families
            .iter()
            .find(|(family, _)| family == "Inter")
            .map(|(_, styles)| styles)
            .expect("Inter is bundled");
        for style in ["Regular", "Medium", "SemiBold", "Bold", "Italic"] {
            assert!(inter.iter().any(|s| s == style), "{style} in {inter:?}");
        }
        assert!(families.iter().any(|(family, _)| family == "Noto Serif"));
        assert!(
            families
                .iter()
                .any(|(family, _)| family == "VibeCut Sans CJK")
        );

        let bold = resolve("inter", "bold");
        assert_eq!(face(bold.face).style, "Bold");
        assert!(!bold.synth_bold && !bold.synth_italic && !bold.missing);

        let serif_bold = resolve("Noto Serif", "Bold");
        assert_eq!(face(serif_bold.face).family, "Noto Serif");
        assert!(serif_bold.synth_bold && !serif_bold.synth_italic);
    }

    #[test]
    fn unknown_family_substitutes_inter_and_reports_missing() {
        let resolved = resolve("No Such Family", "Regular");
        assert_eq!(face(resolved.face).family, "Inter");
        assert!(resolved.missing);
    }

    #[test]
    fn fallback_chain_is_data_driven_and_stable() {
        let inter = resolve("Inter", "Regular").face;
        let serif = resolve("Noto Serif", "Regular").face;
        let cjk = resolve("VibeCut Sans CJK", "Regular").face;
        assert_eq!(fallback_for('A', inter, &[]), inter);
        assert_eq!(fallback_for('A', serif, &[]), serif);
        // No bundled Latin face covers Hebrew: the preferred face stays selected every time.
        for _ in 0..8 {
            assert_eq!(fallback_for('\u{5d0}', inter, &[]), inter);
        }
        // The chain decides which face receives an uncovered character.
        assert_eq!(fallback_for('中', inter, &[cjk]), cjk);
        assert_eq!(fallback_for('中', inter, &[serif, cjk]), cjk);
        assert_eq!(fallback_chain(&["VibeCut Sans CJK"], "Regular"), vec![cjk]);
        assert!(fallback_chain(&["No Such Family"], "Regular").is_empty());
    }

    #[test]
    fn metrics_are_positive_and_scale_linearly() {
        let f = face(resolve("Inter", "Regular").face);
        let m = f.metrics(100.0);
        assert!(m.ascent > 80.0 && m.ascent < 120.0, "{m:?}");
        assert!(m.descent > 10.0 && m.descent < 50.0, "{m:?}");
        assert!(f.has_char('A') && f.has_char('ữ'));
        assert!(!f.has_char('\u{5d0}'));
        assert!(face(resolve("VibeCut Sans CJK", "Regular").face).has_char('中'));
    }
}
