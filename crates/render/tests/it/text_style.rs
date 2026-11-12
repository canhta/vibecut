#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]
//! Text item style: weight/slant/decoration, alignment, spacing and the background box render
//! through the one text raster, identically for preview and export.
use crate::support::*;
use project::{Color, TextAlign, TextBackground, TextDecoration, TextParams};
use render::*;
use time::Tick;

const W: u32 = 640;
const H: u32 = 360;

fn hash(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf29ce484222325u64, |h, b| {
        (h ^ u64::from(*b)).wrapping_mul(0x100000001b3)
    })
}

fn project_with(
    text: TextParams,
    animations: project::Animations,
    duration_seconds: i64,
) -> project::Project {
    let mut data = solid_project(duration_seconds, W, H, [0, 0, 0, 255]).into_data();
    data.scene_mut().timeline.insert_track(project::Track {
        id: project::TrackId(2),
        name: "Text".into(),
        kind: project::TrackKind::Text,
        muted: false,
        hidden: false,
        locked: false,
        items: vec![project::Item {
            hidden: false,
            blend_mode: Default::default(),
            retime: Default::default(),
            masks: Vec::new(),
            effects: Vec::new(),
            animations,
            id: project::ItemId(1),
            source_asset: None,
            source_range: time::SourceRange::new(Tick::ZERO, tick(duration_seconds, 1)).unwrap(),
            start: Tick::ZERO,
            content: project::ItemParams::Text(text),
        }],
    });
    project::Project::new(data).unwrap()
}

/// Straight-alpha RGBA8 of the frame at `at`, rendered as export.
fn export(project: &project::Project, at: Tick) -> Vec<u8> {
    let hooks = Hooks::default();
    let mut renderer = Renderer::new(project, RenderSettings::from_project(project)).unwrap();
    renderer
        .render_frame(at, &mut Empty, &hooks)
        .unwrap()
        .rgba8(1, &hooks)
        .unwrap()
        .bytes
}

fn base() -> TextParams {
    let mut text = TextParams::new("VibeCut Studio\nXin chào");
    text.font_size = 28.0;
    text
}

fn bright(frame: &[u8]) -> Vec<(usize, usize)> {
    frame
        .as_chunks::<4>()
        .0
        .iter()
        .enumerate()
        .filter(|(_, p)| p[0] > 128 && p[1] > 128 && p[2] > 128)
        .map(|(i, _)| (i % W as usize, i / W as usize))
        .collect()
}

fn coloured(frame: &[u8], keep: impl Fn(&[u8]) -> bool) -> Vec<(usize, usize)> {
    frame
        .as_chunks::<4>()
        .0
        .iter()
        .enumerate()
        .filter(|(_, p)| keep(*p))
        .map(|(i, _)| (i % W as usize, i / W as usize))
        .collect()
}

fn red(frame: &[u8]) -> Vec<(usize, usize)> {
    coloured(frame, |p| p[0] > 200 && p[1] < 60 && p[2] < 60)
}

fn blue(frame: &[u8]) -> Vec<(usize, usize)> {
    coloured(frame, |p| p[2] > 200 && p[0] < 60 && p[1] < 60)
}

fn centroid_x(points: &[(usize, usize)]) -> f64 {
    points.iter().map(|p| p.0 as f64).sum::<f64>() / points.len() as f64
}

/// `(x0, y0, x1, y1)` inclusive.
fn bounds(points: &[(usize, usize)]) -> (usize, usize, usize, usize) {
    let x0 = points.iter().map(|p| p.0).min().unwrap();
    let x1 = points.iter().map(|p| p.0).max().unwrap();
    let y0 = points.iter().map(|p| p.1).min().unwrap();
    let y1 = points.iter().map(|p| p.1).max().unwrap();
    (x0, y0, x1, y1)
}

fn boxed(color: [u8; 4]) -> TextBackground {
    TextBackground {
        color: Color(color),
        padding_x: 10.0,
        padding_y: 6.0,
        offset_x: 0.0,
        offset_y: 0.0,
        radius: 0.0,
    }
}

type Edit = Box<dyn Fn(&mut TextParams)>;

#[test]
fn text_style_params_render_per_spec_and_preview_equals_export() {
    // Every row changes the picture against the plain title; the pinned hash is the golden
    // (visually inspected, re-bless only after looking).
    let mut cases: Vec<(String, Edit)> = Vec::new();
    for weight in [400, 700] {
        for italic in [false, true] {
            for decoration in [
                TextDecoration::None,
                TextDecoration::Underline,
                TextDecoration::LineThrough,
            ] {
                let name = format!(
                    "w{weight}-{}-{decoration:?}",
                    if italic { "italic" } else { "upright" }
                );
                cases.push((
                    name,
                    Box::new(move |t| {
                        t.weight = weight;
                        t.italic = italic;
                        t.decoration = decoration;
                    }),
                ));
            }
        }
    }
    let extra: Vec<(&str, Edit)> = vec![
        ("align-left", Box::new(|t| t.align = TextAlign::Left)),
        ("align-right", Box::new(|t| t.align = TextAlign::Right)),
        ("spacing-wide", Box::new(|t| t.letter_spacing = 6.0)),
        ("spacing-tight", Box::new(|t| t.letter_spacing = -2.0)),
        ("line-height", Box::new(|t| t.line_height = 40.0)),
        (
            "box-square",
            Box::new(|t| t.background = Some(boxed([200, 0, 0, 255]))),
        ),
        (
            "box-round",
            Box::new(|t| {
                t.background = Some(TextBackground {
                    radius: 24.0,
                    ..boxed([200, 0, 0, 255])
                });
            }),
        ),
        (
            "box-padding",
            Box::new(|t| {
                t.background = Some(TextBackground {
                    padding_x: 40.0,
                    padding_y: 2.0,
                    ..boxed([200, 0, 0, 255])
                });
            }),
        ),
        (
            "box-offset",
            Box::new(|t| {
                t.background = Some(TextBackground {
                    offset_x: 18.0,
                    offset_y: -8.0,
                    ..boxed([200, 0, 0, 255])
                });
            }),
        ),
        (
            "vietnamese",
            Box::new(|t| {
                t.content = "Tiếng Việt: Ắ ằ ỡ ự đ ẩ ữ ọ\nNgười Hà Nội".into();
                t.weight = 700;
                t.italic = true;
                t.decoration = TextDecoration::Underline;
            }),
        ),
    ];
    cases.extend(extra.into_iter().map(|(n, e)| (n.to_owned(), e)));

    let plain = export(&project_with(base(), Default::default(), 1), Tick::ZERO);
    let mut seen = std::collections::BTreeMap::new();
    let mut goldens = Vec::new();
    for (name, edit) in &cases {
        let mut text = base();
        edit(&mut text);
        let project = project_with(text, Default::default(), 1);
        let hooks = Hooks::default();
        let settings = RenderSettings::from_project(&project);
        let mut renderer = Renderer::new(&project, settings.clone()).unwrap();
        let preview = renderer
            .preview_frame(Tick::ZERO, &mut Empty, &hooks)
            .unwrap();
        let exported = renderer
            .render_frame(Tick::ZERO, &mut Empty, &hooks)
            .unwrap();
        assert_eq!(
            preview.pixels(),
            exported.pixels(),
            "{name}: preview/export"
        );
        let mut cold = settings;
        cold.text_cache_bytes = 0;
        assert_eq!(
            preview.pixels(),
            Renderer::new(&project, cold)
                .unwrap()
                .render_frame(Tick::ZERO, &mut Empty, &hooks)
                .unwrap()
                .pixels(),
            "{name}: cached and uncached raster"
        );
        let bytes = exported.rgba8(1, &hooks).unwrap().bytes;
        if name != "w400-upright-None" {
            assert_ne!(bytes, plain, "{name} changes the picture");
        }
        let digest = hash(&bytes);
        if let Some(other) = seen.insert(digest, name.clone()) {
            panic!("{name} duplicates {other}");
        }
        goldens.push((name.clone(), digest));
    }
    if std::env::var_os("VIBECUT_PRINT_TEXT_GOLDENS").is_some() {
        for (name, value) in &goldens {
            println!("    (\"{name}\", {value:#018x}),");
        }
    }
    assert_eq!(
        goldens,
        GOLDENS
            .iter()
            .map(|(n, h)| ((*n).to_owned(), *h))
            .collect::<Vec<_>>()
    );

    // Decoration: a thin bar per line, under the baseline or through the x-height.
    let diff_rows = |decoration| {
        let mut text = base();
        text.decoration = decoration;
        let frame = export(&project_with(text, Default::default(), 1), Tick::ZERO);
        let rows: Vec<usize> = frame
            .as_chunks::<4>()
            .0
            .iter()
            .zip(plain.as_chunks::<4>().0)
            .enumerate()
            .filter(|(_, (a, b))| a != b)
            .map(|(i, _)| i / W as usize)
            .collect();
        (*rows.iter().min().unwrap(), *rows.iter().max().unwrap())
    };
    let (under_top, under_bottom) = diff_rows(TextDecoration::Underline);
    let (strike_top, strike_bottom) = diff_rows(TextDecoration::LineThrough);
    assert!(
        strike_top < under_top && strike_bottom < under_bottom,
        "line-through sits above the underline on both lines"
    );
    // Two lines at 28 px, one bar each: the changed rows span the line pitch, not the block.
    assert!(
        (20..60).contains(&(under_bottom - under_top)),
        "{under_top}..{under_bottom}"
    );

    // Alignment moves the whole block.
    let centroid = |align| {
        let mut text = base();
        text.align = align;
        centroid_x(&bright(&export(
            &project_with(text, Default::default(), 1),
            Tick::ZERO,
        )))
    };
    let (left, centre, right) = (
        centroid(TextAlign::Left),
        centroid(TextAlign::Center),
        centroid(TextAlign::Right),
    );
    assert!(
        left < centre - 20.0 && centre < right - 20.0,
        "{left} {centre} {right}"
    );

    // The box follows the text bounds; offset and padding move it, never the text.
    let framed = |offset: (f32, f32), padding: (f32, f32)| {
        let mut text = base();
        text.background = Some(TextBackground {
            offset_x: offset.0,
            offset_y: offset.1,
            padding_x: padding.0,
            padding_y: padding.1,
            ..boxed([255, 0, 0, 255])
        });
        let frame = export(&project_with(text, Default::default(), 1), Tick::ZERO);
        (bounds(&red(&frame)), bounds(&bright(&frame)))
    };
    let (home, text_home) = framed((0.0, 0.0), (10.0, 6.0));
    let (moved, text_moved) = framed((20.0, -9.0), (10.0, 6.0));
    assert_eq!(text_home, text_moved, "box offset never moves the text");
    assert_eq!((moved.0, moved.2), (home.0 + 20, home.2 + 20));
    assert_eq!((moved.1 + 9, moved.3 + 9), (home.1, home.3));
    let (wide, text_wide) = framed((0.0, 0.0), (30.0, 6.0));
    assert_eq!(text_home, text_wide, "padding never moves the text");
    assert_eq!((wide.0 + 20, wide.2), (home.0, home.2 + 20));
    assert_eq!((wide.1, wide.3), (home.1, home.3));
}

#[test]
fn stroke_shadow_and_glow_paint_behind_the_fill_and_stroke_colour_keys_animate() {
    use project::{
        AnimationPath, BuiltinPath, Channel, Extrapolation, Keyframe, Param, ParamValue,
        TextShadow, TextStroke,
    };
    let plain = export(&project_with(base(), Default::default(), 1), Tick::ZERO);
    let plain_text = bounds(&bright(&plain));
    let cases: Vec<(&str, Edit)> = vec![
        (
            "stroke",
            Box::new(|t| {
                t.stroke = Some(TextStroke {
                    color: Color([255, 0, 0, 255]),
                    width: 6.0,
                });
            }),
        ),
        (
            "shadow",
            Box::new(|t| {
                t.shadow = Some(TextShadow {
                    color: Color([255, 0, 0, 255]),
                    offset_x: 10.0,
                    offset_y: 8.0,
                    blur: 0.0,
                });
            }),
        ),
        ("glow", Box::new(|t| t.glow = true)),
    ];
    for (name, edit) in cases {
        let mut text = base();
        edit(&mut text);
        let project = project_with(text, Default::default(), 1);
        let hooks = Hooks::default();
        let settings = RenderSettings::from_project(&project);
        let mut renderer = Renderer::new(&project, settings.clone()).unwrap();
        let preview = renderer
            .preview_frame(Tick::ZERO, &mut Empty, &hooks)
            .unwrap();
        let exported = renderer
            .render_frame(Tick::ZERO, &mut Empty, &hooks)
            .unwrap();
        assert_eq!(
            preview.pixels(),
            exported.pixels(),
            "{name}: preview/export"
        );
        let mut cold = settings;
        cold.text_cache_bytes = 0;
        assert_eq!(
            preview.pixels(),
            Renderer::new(&project, cold)
                .unwrap()
                .render_frame(Tick::ZERO, &mut Empty, &hooks)
                .unwrap()
                .pixels(),
            "{name}: cached and uncached raster"
        );
        let frame = exported.rgba8(1, &hooks).unwrap().bytes;
        assert_ne!(frame, plain, "{name} changes the picture");
        // The solid project background is opaque black, so an uncovered pixel is
        // black on the plain title and picks up decoration colour here.
        let added = frame
            .as_chunks::<4>()
            .0
            .iter()
            .zip(plain.as_chunks::<4>().0)
            .filter(|(decorated, plain)| {
                plain[0] == 0
                    && plain[1] == 0
                    && plain[2] == 0
                    && (decorated[0] > 0 || decorated[1] > 0 || decorated[2] > 0)
            })
            .count();
        assert!(added > 50, "{name}: coverage outside the fill {added}");
        // Stroke and shadow stay behind the fill, so the bright text block is the
        // plain one except that decoration colour under an antialiased glyph edge
        // can drop one threshold pixel (the exact form fails by 1 px on the
        // stroke). The glow is the text colour spread over `GLOW_SIZE *
        // font_size` (28 * 0.15 = 4.2 px), so its bright halo may widen the block
        // by at most that radius rounded up.
        let lit = bounds(&bright(&frame));
        let tolerance = if name == "glow" { 5 } else { 1 };
        assert!(
            lit.0.abs_diff(plain_text.0) <= tolerance
                && lit.1.abs_diff(plain_text.1) <= tolerance
                && lit.2.abs_diff(plain_text.2) <= tolerance
                && lit.3.abs_diff(plain_text.3) <= tolerance,
            "{name}: {lit:?} {plain_text:?}"
        );
        match name {
            "stroke" => assert!(!red(&frame).is_empty(), "red stroke"),
            "shadow" => {
                let shadow = bounds(&red(&frame));
                assert!(
                    shadow.2 > plain_text.2 && shadow.3 > plain_text.3,
                    "{shadow:?} {plain_text:?}"
                );
            }
            "glow" => {
                let grey = |pixels: &[u8]| {
                    coloured(pixels, |p| {
                        p[0] > 0 && p[0] < 200 && p[0] == p[1] && p[1] == p[2]
                    })
                    .len()
                };
                assert!(grey(&frame) > grey(&plain) + 50, "glow spreads a grey halo");
            }
            _ => unreachable!(),
        }
    }

    let mut text = base();
    text.stroke = Some(TextStroke {
        color: Color([255, 0, 0, 255]),
        width: 8.0,
    });
    let from = [1.0, 0.0, 0.0, 1.0];
    let to = [0.0, 0.0, 1.0, 1.0];
    let channels: [Channel; 4] = std::array::from_fn(|i| {
        Channel::new(
            ParamValue::Scalar(from[i]),
            vec![
                Keyframe::new(Tick::ZERO, ParamValue::Scalar(from[i])),
                Keyframe::new(tick(1, 1), ParamValue::Scalar(to[i])),
            ],
            Extrapolation::Hold,
            Extrapolation::Hold,
        )
        .unwrap()
    });
    let mut animations = project::Animations::default();
    animations
        .set(
            AnimationPath::from(BuiltinPath::StrokeColor),
            Param::Color(Box::new(channels)),
        )
        .unwrap();
    let project = project_with(text, animations, 2);
    let start = export(&project, Tick::ZERO);
    let end = export(&project, tick(1, 1));
    assert!(
        !red(&start).is_empty() && blue(&start).is_empty(),
        "stroke starts red"
    );
    assert!(
        !blue(&end).is_empty() && red(&end).is_empty(),
        "stroke ends blue"
    );
    assert_eq!(export(&project, tick(3, 2)), end, "keys hold after the end");
}

#[test]
fn background_box_and_colour_animate_and_unkeyed_text_keeps_stored_values() {
    use project::{AnimationPath, Channel, Extrapolation, Keyframe, Param, ParamValue};
    let channel = |from: f64, to: f64| {
        Channel::new(
            ParamValue::Scalar(from),
            vec![
                Keyframe::new(Tick::ZERO, ParamValue::Scalar(from)),
                Keyframe::new(tick(1, 1), ParamValue::Scalar(to)),
            ],
            Extrapolation::Hold,
            Extrapolation::Hold,
        )
        .unwrap()
    };
    let mut text = base();
    text.background = Some(boxed([255, 0, 0, 255]));
    let mut animations = project::Animations::default();
    for (path, from, to) in [
        ("background.offsetX", 0.0, 40.0),
        ("background.offsetY", 0.0, -10.0),
        ("background.paddingX", 10.0, 30.0),
    ] {
        animations
            .set(
                AnimationPath::new(path).unwrap(),
                Param::Value(channel(from, to)),
            )
            .unwrap();
    }
    // Linear-light red to blue.
    let from = [1.0, 0.0, 0.0, 1.0];
    let to = [0.0, 0.0, 1.0, 1.0];
    let colour: [Channel; 4] = std::array::from_fn(|i| channel(from[i], to[i]));
    animations
        .set(
            AnimationPath::new("background.color").unwrap(),
            Param::Color(Box::new(colour)),
        )
        .unwrap();
    let project = project_with(text.clone(), animations, 2);

    let start = export(&project, Tick::ZERO);
    let end = export(&project, tick(1, 1));
    let after = export(&project, tick(3, 2));
    assert_eq!(after, end, "channels hold after the last key");
    assert!(
        blue(&start).is_empty() && red(&end).is_empty(),
        "colour runs red to blue"
    );
    let start_box = bounds(&red(&start));
    let end_box = bounds(&blue(&end));
    // Offset +40 and padding +20 per side: left edge +40-20, right edge +40+20; up 10.
    assert_eq!(end_box.0, start_box.0 + 20);
    assert_eq!(end_box.2, start_box.2 + 60);
    assert_eq!(end_box.1 + 10, start_box.1);

    // The middle frame is identical in preview and export.
    let hooks = Hooks::default();
    let mut renderer = Renderer::new(&project, RenderSettings::from_project(&project)).unwrap();
    let preview = renderer
        .preview_frame(tick(1, 2), &mut Empty, &hooks)
        .unwrap();
    let exported = renderer
        .render_frame(tick(1, 2), &mut Empty, &hooks)
        .unwrap();
    assert_eq!(preview.pixels(), exported.pixels());
    assert_ne!(preview.pixels(), export_pixels(&project, Tick::ZERO));

    // Without keys the stored values stay authoritative at every time.
    let still = project_with(text, Default::default(), 2);
    assert_eq!(export(&still, Tick::ZERO), export(&still, tick(3, 2)));
}

fn export_pixels(project: &project::Project, at: Tick) -> Vec<[f32; 4]> {
    let hooks = Hooks::default();
    Renderer::new(project, RenderSettings::from_project(project))
        .unwrap()
        .render_frame(at, &mut Empty, &hooks)
        .unwrap()
        .pixels()
        .to_vec()
}

#[test]
fn bilingual_fades_scale_glyphs_and_background_once_in_real_time() {
    use project::{
        AnimationPath, Channel, Extrapolation::Hold, Keyframe, Param, ParamValue::Scalar,
    };
    let mut nfc = None;
    for content in [
        "Tiếng Việt\nEnglish",
        "Tie\u{0302}\u{0301}ng Vie\u{0323}\u{0302}t\nEnglish",
    ] {
        let mut text = TextParams::new(content);
        text.font_size = 28.0;
        text.color = Color([220, 120, 40, 160]);
        text.background = Some(boxed([30, 80, 180, 128]));
        let mut data = project_with(text, Default::default(), 2).into_data();
        data.canvas.background = Color([0; 4]);
        for track in data.scene_mut().timeline.iter_mut() {
            track
                .items
                .retain(|item| matches!(item.content, project::ItemParams::Text(_)));
        }
        let target = data
            .scene_mut()
            .timeline
            .iter_mut()
            .flat_map(|t| &mut t.items)
            .next()
            .unwrap();
        target.start = tick(1, 1);
        target.source_range = time::SourceRange::new(tick(2, 1), tick(3, 1)).unwrap();
        let still = project::Project::new(data.clone()).unwrap();
        let coverage = export_pixels(&still, tick(1, 1));
        if let Some(expected) = &nfc {
            assert_eq!(&coverage, expected);
        } else {
            nfc = Some(coverage.clone());
        }
        assert!(
            coverage.iter().any(|p| p[0] > p[2] && p[3] > 0.5),
            "Vietnamese and English glyphs"
        );
        assert!(
            coverage.iter().any(|p| p[2] > p[0] && p[3] > 0.4),
            "real box"
        );
        let target = data
            .scene_mut()
            .timeline
            .iter_mut()
            .flat_map(|t| &mut t.items)
            .next()
            .unwrap();
        target
            .animations
            .set(
                AnimationPath::new("opacity").unwrap(),
                Param::Value(
                    Channel::new(
                        Scalar(0.6),
                        [
                            (0, 0.0),
                            (76_204_800_000, 0.6),
                            (177_811_200_000, 0.6),
                            (254_016_000_000, 0.0),
                        ]
                        .into_iter()
                        .map(|(t, v)| Keyframe::new(Tick(t), Scalar(v)))
                        .collect(),
                        Hold,
                        Hold,
                    )
                    .unwrap(),
                ),
            )
            .unwrap();
        let plain = project::Project::new(data.clone()).unwrap();
        data.scene_mut()
            .timeline
            .iter_mut()
            .flat_map(|t| &mut t.items)
            .next()
            .unwrap()
            .effects
            .push(
                project::effect_definition("hold_time")
                    .unwrap()
                    .instance("hold"),
            );
        let fade = project::Project::new(data.clone()).unwrap();
        for (local, opacity) in [
            (0, 0.0),
            (38_102_400_000, 0.3),
            (76_204_800_000, 0.6),
            (177_811_200_000, 0.6),
            (215_913_600_000, 0.3),
            (254_016_000_000, 0.0),
        ] {
            let at = tick(1, 1).checked_add(Tick(local)).unwrap();
            let pixels = export_pixels(&fade, at);
            let h = Hooks::default();
            let preview = Renderer::new(&plain, RenderSettings::from_project(&plain))
                .unwrap()
                .preview_frame(at, &mut Empty, &h)
                .unwrap();
            let exact = export_pixels(&plain, at);
            let mismatch = preview
                .pixels()
                .iter()
                .zip(&exact)
                .position(|(a, b)| a != b);
            assert!(
                mismatch.is_none(),
                "preview/render local {local}: {:?}",
                mismatch.map(|i| (i, preview.pixels()[i], pixels[i]))
            );
            for (actual, source) in pixels.iter().zip(&coverage) {
                for c in 0..4 {
                    assert!(
                        (f64::from(actual[c]) - f64::from(source[c]) * opacity).abs() < 3e-6,
                        "local {local}, c {c}, actual {actual:?}, source {source:?}"
                    );
                }
                assert!(actual[..3].iter().all(|v| *v <= actual[3]));
            }
        }
        assert!(
            export_pixels(&fade, tick(1, 1).checked_sub(Tick(1)).unwrap())
                .iter()
                .all(|p| *p == [0.0; 4])
        );
        // Independently compose over a partial-alpha coloured backdrop in linear light.
        data.canvas.background = Color([64, 128, 192, 153]);
        let over = project::Project::new(data).unwrap();
        let linear = |v: f64| {
            if v <= 0.04045 {
                v / 12.92
            } else {
                ((v + 0.055) / 1.055).powf(2.4)
            }
        };
        let back = [
            linear(64.0 / 255.0),
            linear(128.0 / 255.0),
            linear(192.0 / 255.0),
            1.0,
        ];
        for (actual, source) in export_pixels(&over, tick(23, 20)).iter().zip(&coverage) {
            let alpha = f64::from(source[3]) * 0.3;
            for c in 0..4 {
                let expected =
                    f64::from(source[c]) * 0.3 + back[c] * (153.0 / 255.0) * (1.0 - alpha);
                assert!(
                    (f64::from(actual[c]) - expected).abs() < 3e-6,
                    "channel {c}"
                );
            }
        }
    }
}

const GOLDENS: &[(&str, u64)] = &[
    ("w400-upright-None", 0xf042f3edaf91e427),
    ("w400-upright-Underline", 0x9cd839895551819f),
    ("w400-upright-LineThrough", 0x5afd5563d594f763),
    ("w400-italic-None", 0x52eb3d064359ee5c),
    ("w400-italic-Underline", 0x6e1e68ace7c178f4),
    ("w400-italic-LineThrough", 0x62e3ae5e2b3501ec),
    ("w700-upright-None", 0xe70faa500eceb1d8),
    ("w700-upright-Underline", 0xe3e1aff5682439e0),
    ("w700-upright-LineThrough", 0x44a978116f81422a),
    ("w700-italic-None", 0xa3f7cd95f30635aa),
    ("w700-italic-Underline", 0xf2fc36a0e9713591),
    ("w700-italic-LineThrough", 0x40f1db9f56b27e8f),
    ("align-left", 0xc501e0e6b781b81a),
    ("align-right", 0x96f0462aab0dd232),
    ("spacing-wide", 0x845dd3263ac8a307),
    ("spacing-tight", 0x561432fcef08b8fa),
    ("line-height", 0x5374f95d46ec14bd),
    ("box-square", 0xe9300fe48842e3f6),
    ("box-round", 0xf6faac6d02a002aa),
    ("box-padding", 0xd5a55011cc423d26),
    ("box-offset", 0x293e340269473861),
    ("vietnamese", 0x61b64cea71d7feed),
];
