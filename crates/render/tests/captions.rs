#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]
mod common;
use common::*;
use render::*;
use time::{Tick, TimeRange};
fn scene(texts: &[(&str, &str)]) -> project::Project {
    let mut d = solid_project(2, 360, 640, [25, 50, 90, 255]).into_data();
    let range = TimeRange::new(tick(1, 4), tick(3, 2)).unwrap();
    d.assets.push(
        media::SourceAsset::try_from(media::AssetData {
            id: media::AssetId(1),
            path: std::env::temp_dir().join("caption-source-identity"),
            content_hash: media::ContentHash([0; 32]),
            probe: media::SourceProbe {
                format: "caption identity".into(),
                duration: Some(tick(2, 1)),
                streams: vec![],
            },
        })
        .unwrap(),
    );
    d.timeline.insert_track(project::Track {
        id: project::TrackId(2),
        name: "".into(),
        kind: project::TrackKind::Text,
        muted: false,
        hidden: false,
        locked: false,
        items: vec![project::Item {
            masks: Vec::new(),
            effects: Vec::new(),
            animations: Default::default(),
            id: project::ItemId(1),
            source_asset: Some(media::AssetId(1)),
            source_range: TimeRange::new(Tick::ZERO, tick(2, 1)).unwrap(),
            start: Tick::ZERO,
            content: project::ItemParams::Caption(project::CaptionParams {
                cues: captions::CueTrack {
                    cues: vec![captions::Cue {
                        id: "stress".into(),
                        range,
                        source_lang: captions::LanguageTag::parse(texts[0].0).unwrap(),
                        texts: texts
                            .iter()
                            .map(|(lang, value)| {
                                (
                                    captions::LanguageTag::parse(lang).unwrap(),
                                    captions::CueText {
                                        text: (*value).into(),
                                        edited: false,
                                        translation_ref: None,
                                    },
                                )
                            })
                            .collect(),
                    }],
                },
            }),
        }],
    });
    project::Project::new(d).unwrap()
}
fn settings(p: &project::Project) -> RenderSettings {
    let mut settings = RenderSettings::from_project(p);
    settings.caption_style.size = 36.0;
    settings.caption_style.safe_area_percent = 10.0;
    settings.caption_style.color = [255, 240, 160, 128];
    settings.caption_style.background = Some(CaptionBackground {
        color: [10, 10, 20, 96],
        padding: 8.0,
        radius: 5.0,
    });
    settings
}
fn hash(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf29ce484222325u64, |h, b| {
        (h ^ u64::from(*b)).wrapping_mul(0x100000001b3)
    })
}
fn png(bytes: &[u8], name: &str) {
    let raw = root().join(format!("{name}.rgba"));
    std::fs::write(&raw, bytes).unwrap();
    ffmpeg(&[
        "-f".as_ref(),
        "rawvideo".as_ref(),
        "-pixel_format".as_ref(),
        "rgba".as_ref(),
        "-video_size".as_ref(),
        "360x640".as_ref(),
        "-i".as_ref(),
        raw.as_os_str(),
        "-frames:v".as_ref(),
        "1".as_ref(),
        root().join(format!("{name}.png")).as_os_str(),
    ]);
}
#[test]
fn preview_export_bilingual_diacritics_cjk_gaps_and_style_goldens() {
    let cases: &[(&str, &[(&str, &str)])] = &[
        (
            "caption-vi-en",
            &[
                ("vi", "Tiếng Việt: Ắ ằ ỡ ự đ"),
                ("en", "Bilingual captions"),
            ],
        ),
        (
            "caption-long-vi",
            &[(
                "vi",
                "hôm nay chúng ta cùng tìm hiểu cách tạo phụ đề tiếng Việt cho video dọc thật dễ dàng",
            )],
        ),
        (
            "caption-long-bilingual",
            &[
                (
                    "vi",
                    "hôm nay chúng ta cùng tìm hiểu cách tạo phụ đề tiếng Việt cho video dọc thật dễ dàng",
                ),
                (
                    "en",
                    "today we learn how to create readable captions for a portrait video",
                ),
            ],
        ),
        (
            "caption-hans-hant",
            &[
                ("zh-Hans", "这是简体中文字幕测试，我们一起学习。"),
                ("zh-Hant", "這是繁體中文字幕測試，我們一起學習。"),
            ],
        ),
    ];
    for (name, texts) in cases {
        let p = scene(texts);
        let h = Hooks::default();
        let mut s = settings(&p);
        s.caption_languages = texts
            .iter()
            .map(|(lang, _)| captions::LanguageTag::parse(lang).unwrap())
            .collect();
        if name.contains("hans") {
            s.caption_style.max_lines = 4;
        }
        let mut renderer = Renderer::new(&p, s.clone()).unwrap();
        let preview = renderer.preview_frame(tick(1, 2), &mut Empty, &h).unwrap();
        let exported = renderer.render_frame(tick(1, 2), &mut Empty, &h).unwrap();
        assert_eq!(
            preview.pixels(),
            exported.pixels(),
            "preview/export pixel diff must be zero"
        );
        let packed = preview.rgba8(1, &h).unwrap();
        assert_eq!(
            hash(&packed.bytes),
            match *name {
                "caption-vi-en" => 0x22d92066556dd4c5,
                "caption-hans-hant" => 0x442017b175aa5544,
                "caption-long-vi" => 0x26e5c2c4450a1b31,
                "caption-long-bilingual" => 0x61c1b0ec35120af3,
                _ => panic!("unknown golden case"),
            },
            "visually inspected caption golden"
        );
        if tools() {
            png(&packed.bytes, name);
        }
        let mut no_cache = s.clone();
        no_cache.text_cache_bytes = 0;
        assert_eq!(
            preview.pixels(),
            Renderer::new(&p, no_cache)
                .unwrap()
                .render_frame(tick(1, 2), &mut Empty, &h)
                .unwrap()
                .pixels()
        );
        let before = renderer.preview_frame(Tick::ZERO, &mut Empty, &h).unwrap();
        let after = renderer.render_frame(tick(3, 2), &mut Empty, &h).unwrap();
        assert_eq!(before.pixels(), after.pixels(), "half-open end and gaps");
        assert_ne!(preview.pixels(), before.pixels());
        // Every changed pixel lies inside the 10% safe rectangle, including paint.
        for y in 0..640 {
            for x in 0..360 {
                if !(36..324).contains(&x) || !(64..576).contains(&y) {
                    assert_eq!(preview.pixels()[y * 360 + x], before.pixels()[y * 360 + x]);
                }
            }
        }
        if *name == "caption-vi-en" {
            let nfd = scene(&[
                (
                    "vi",
                    "Tie\u{0302}\u{0301}ng Vie\u{0323}\u{0302}t: A\u{0306}\u{0301} a\u{0306}\u{0300} o\u{031b}\u{0303} u\u{031b}\u{0323} đ",
                ),
                ("en", "Bilingual captions"),
            ]);
            let mut ns = s.clone();
            ns.caption_languages = s.caption_languages.clone();
            assert_eq!(
                preview.pixels(),
                Renderer::new(&nfd, ns)
                    .unwrap()
                    .render_frame(tick(1, 2), &mut Empty, &h)
                    .unwrap()
                    .pixels()
            );
        }
    }
}
#[test]
fn styled_captions_survive_final_encoded_decode() {
    if !tools() {
        return;
    }
    let cases: &[(&str, &[(&str, &str)])] = &[
        (
            "caption-burn",
            &[
                ("vi", "Tiếng Việt: Ắ ằ ỡ ự đ"),
                ("en", "Bilingual captions"),
            ],
        ),
        (
            "caption-long-burn",
            &[
                (
                    "vi",
                    "hôm nay chúng ta cùng tìm hiểu cách tạo phụ đề tiếng Việt cho video dọc thật dễ dàng",
                ),
                (
                    "en",
                    "today we learn how to create readable captions for a portrait video",
                ),
            ],
        ),
    ];
    for (name, texts) in cases {
        let p = scene(texts);
        let h = Hooks::default();
        let mut s = ExportSettings::from_project(&p);
        s.render = settings(&p);
        s.render.caption_languages = vec![
            captions::LanguageTag::parse("vi").unwrap(),
            captions::LanguageTag::parse("en").unwrap(),
        ];
        s.include_audio = false;
        s.mode = ExportMode::Encode;
        s.quality = 100;
        s.overwrite = project::atomic::Overwrite::Replace;
        let at = tick(1, 2);
        let mut renderer = Renderer::new(&p, s.render.clone()).unwrap();
        let preview = renderer.preview_frame(at, &mut Empty, &h).unwrap();
        let boundary = preview.video_rgba8(1, &h).unwrap();
        let path = root().join(format!("{name}.mp4"));
        export_with_sources(
            &p,
            TimeRange::new(at, tick(3, 5)).unwrap(),
            &s,
            &path,
            &mut Empty,
            &h,
        )
        .unwrap();
        let actual = rgba(&path);
        assert_eq!(actual.len(), boundary.bytes.len());
        let gap = renderer
            .preview_frame(Tick::ZERO, &mut Empty, &h)
            .unwrap()
            .video_rgba8(1, &h)
            .unwrap();
        let mut changed_error = 0.0;
        let mut changed = 0;
        for ((pixel, expected), background) in actual
            .as_chunks::<4>()
            .0
            .iter()
            .zip(boundary.bytes.as_chunks::<4>().0.iter())
            .zip(gap.bytes.as_chunks::<4>().0.iter())
        {
            if expected != background {
                changed_error += pixel
                    .iter()
                    .zip(expected)
                    .map(|(a, b)| (f64::from(*a) - f64::from(*b)).powi(2))
                    .sum::<f64>();
                changed += 4;
            }
        }
        assert!(
            changed > 1000,
            "golden must contain visible subtitle pixels"
        );
        let caption_mse = changed_error / changed as f64;
        assert!(
            caption_mse < 150.0,
            "{name} caption-region encoded MSE {caption_mse}"
        );
        eprintln!("{name} caption-region encoded MSE {caption_mse}");
        let mse = actual
            .iter()
            .zip(&boundary.bytes)
            .map(|(a, b)| (f64::from(*a) - f64::from(*b)).powi(2))
            .sum::<f64>()
            / actual.len() as f64;
        assert!(mse < 150.0, "{name} encoded subtitle MSE {mse}");
        eprintln!("{name} encoded subtitle MSE {mse}");
        ffmpeg(&[
            "-i".as_ref(),
            path.as_os_str(),
            "-frames:v".as_ref(),
            "1".as_ref(),
            root().join(format!("{name}-decoded.png")).as_os_str(),
        ]);
    }
}
#[test]
fn invalid_styles_and_line_overflow_refuse_without_silently_dropping_language() {
    let p = scene(&[("vi", "Tiếng Việt"), ("en", "English")]);
    let h = Hooks::default();
    let mut s = settings(&p);
    s.caption_style.size = f32::NAN;
    assert!(matches!(Renderer::new(&p, s), Err(Error::Invalid(_))));
    let mut s = settings(&p);
    s.caption_style.safe_area_percent = 41.0;
    assert!(matches!(Renderer::new(&p, s), Err(Error::Invalid(_))));
    let mut s = settings(&p);
    s.caption_style.max_lines = 1;
    s.caption_languages = vec![
        captions::LanguageTag::parse("vi").unwrap(),
        captions::LanguageTag::parse("en").unwrap(),
    ];
    assert!(matches!(
        Renderer::new(&p, s)
            .unwrap()
            .render_frame(tick(1, 2), &mut Empty, &h),
        Err(Error::Unsupported(_))
    ));
    let hostile = scene(&[("vi", &"x".repeat(16_000))]);
    assert!(matches!(
        Renderer::new(&hostile, settings(&hostile))
            .unwrap()
            .render_frame(tick(1, 2), &mut Empty, &h),
        Err(Error::Unsupported(_))
    ));
}

#[test]
#[ignore = "manual fixed-scene timing and peak RSS measurement"]
fn caption_frame_benchmark() {
    for (name, vi, en) in [
        ("short", "Tiếng Việt: Ắ ằ ỡ ự đ", "Bilingual captions"),
        (
            "fitted",
            "hôm nay chúng ta cùng tìm hiểu cách tạo phụ đề tiếng Việt cho video dọc thật dễ dàng",
            "today we learn how to create readable captions for a portrait video",
        ),
    ] {
        let p = scene(&[("vi", vi), ("en", en)]);
        let h = Hooks::default();
        let mut s = settings(&p);
        s.width = 1080;
        s.height = 1920;
        s.caption_languages = vec![
            captions::LanguageTag::parse("vi").unwrap(),
            captions::LanguageTag::parse("en").unwrap(),
        ];
        let mut reference = None;
        for cached in [false, true] {
            let mut config = s.clone();
            if !cached {
                config.text_cache_bytes = 0;
            }
            let mut renderer = Renderer::new(&p, config).unwrap();
            let start = std::time::Instant::now();
            let first = renderer.render_frame(tick(1, 2), &mut Empty, &h).unwrap();
            let cold = start.elapsed();
            if let Some(previous) = &reference {
                assert_eq!(first.pixels(), previous);
            } else {
                reference = Some(first.pixels().to_vec());
            }
            let start = std::time::Instant::now();
            for _ in 0..20 {
                let frame = renderer.render_frame(tick(1, 2), &mut Empty, &h).unwrap();
                assert_eq!(first.pixels(), frame.pixels());
            }
            eprintln!(
                "caption benchmark case={name} cached={cached} canvas=1080x1920 cold={cold:?} warm20={:?} stats={:?} pixel_diff=0",
                start.elapsed(),
                renderer.text_cache_stats()
            );
        }
    }
}

#[test]
fn empty_captions_paint_nothing_and_position_font_and_color_are_applied() {
    let blank = scene(&[("vi", "  \n ")]);
    let p = scene(&[("vi", "Tiếng Việt")]);
    let h = Hooks::default();
    let gap = Renderer::new(&p, settings(&p))
        .unwrap()
        .render_frame(Tick::ZERO, &mut Empty, &h)
        .unwrap();
    assert_eq!(
        gap.pixels(),
        Renderer::new(&blank, settings(&blank))
            .unwrap()
            .render_frame(tick(1, 2), &mut Empty, &h)
            .unwrap()
            .pixels()
    );
    let mut centers = Vec::new();
    let mut images = Vec::new();
    for position in [
        CaptionPosition::Top,
        CaptionPosition::Middle,
        CaptionPosition::Bottom,
    ] {
        let mut s = settings(&p);
        s.caption_style.position = position;
        s.caption_style.font = "Noto Serif".into();
        s.caption_style.color = [180, 255, 180, 255];
        let image = Renderer::new(&p, s)
            .unwrap()
            .render_frame(tick(1, 2), &mut Empty, &h)
            .unwrap();
        let ys: Vec<_> = image
            .pixels()
            .iter()
            .zip(gap.pixels())
            .enumerate()
            .filter_map(|(n, (a, b))| (a != b).then_some(n / 360))
            .collect();
        centers.push((ys.iter().min().unwrap() + ys.iter().max().unwrap()) / 2);
        images.push(image);
    }
    assert!(centers[0] < 150 && (300..340).contains(&centers[1]) && centers[2] > 500);
    let standard = Renderer::new(&p, settings(&p))
        .unwrap()
        .render_frame(tick(1, 2), &mut Empty, &h)
        .unwrap();
    assert_ne!(images[2].pixels(), standard.pixels());
}

#[test]
fn long_cues_fit_portrait_landscape_and_partial_source_items() {
    let p = scene(&[(
        "vi",
        "hôm nay chúng ta cùng tìm hiểu cách tạo phụ đề tiếng Việt cho video dọc thật dễ dàng",
    )]);
    let h = Hooks::default();
    for (width, height, destination) in [
        (360, 640, None),
        (640, 360, None),
        (
            360,
            640,
            Some(Rect {
                x: 0.2,
                y: 0.6,
                width: 0.6,
                height: 0.1,
            }),
        ),
    ] {
        let mut s = RenderSettings::from_project(&p);
        s.width = width;
        s.height = height;
        if let Some(destination) = destination {
            s.placements.insert(
                project::ItemId(1),
                Placement {
                    destination,
                    ..Default::default()
                },
            );
        }
        let full = Renderer::new(&p, s.clone())
            .unwrap()
            .render_frame(tick(3, 4), &mut Empty, &h)
            .unwrap();
        let mut data = p.clone().into_data();
        let item = &mut data.timeline.overlays[0].items[0];
        item.source_range = TimeRange::new(tick(1, 2), tick(1, 1)).unwrap();
        let project::ItemParams::Caption(params) = &mut item.content else {
            panic!("expected caption item")
        };
        params.cues =
            captions::EditMapping::new(vec![(item.source_range, item.source_range.start())])
                .unwrap()
                .map_cues(&params.cues)
                .unwrap();
        let partial = project::Project::new(data).unwrap();
        let mut renderer = Renderer::new(&partial, s).unwrap();
        let inside = renderer.render_frame(tick(1, 4), &mut Empty, &h).unwrap();
        assert_eq!(
            inside.pixels(),
            full.pixels(),
            "partial cues retain full text"
        );
        let outside = renderer.render_frame(tick(1, 2), &mut Empty, &h).unwrap();
        assert_ne!(
            inside.pixels(),
            outside.pixels(),
            "half-open clip end hides the caption"
        );
    }
}

#[test]
fn extreme_font_scaling_is_a_typed_refusal_before_raster_scratch() {
    let mut d = scene(&[("vi", "X")]).into_data();
    d.canvas.width = 1;
    d.timeline.overlays[0].kind = project::TrackKind::Text;
    let item = &mut d.timeline.overlays[0].items[0];
    item.source_asset = None;
    item.content = project::ItemParams::Text(project::TextParams {
        content: "X".into(),
        font_size: 4096.0,
        color: project::Color([255; 4]),
    });
    let p = project::Project::new(d).unwrap();
    let h = Hooks::default();
    let mut s = RenderSettings::from_project(&p);
    s.width = 8192;
    s.height = 2;
    assert!(matches!(
        Renderer::new(&p, s)
            .unwrap()
            .render_frame(tick(1, 2), &mut Empty, &h),
        Err(Error::Invalid("scaled font size"))
    ));
}
