#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]
use crate::support::*;
use render::*;
use time::{Tick, TimeRange};
fn scene(texts: &[(&str, &str)]) -> project::Project {
    let mut d = solid_project(2, 360, 640, [25, 50, 90, 255]).into_data();
    let range = TimeRange::new(tick(1, 4), tick(3, 2)).unwrap();
    d.assets.push(
        media::SourceAsset::try_from(media::AssetData {
            ephemeral: false,
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
    d.scene_mut().timeline.insert_track(project::Track {
        id: project::TrackId(2),
        name: "".into(),
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
            animations: Default::default(),
            id: project::ItemId(1),
            source_asset: Some(media::AssetId(1)),
            source_range: TimeRange::new(Tick::ZERO, tick(2, 1)).unwrap(),
            start: Tick::ZERO,
            content: project::ItemParams::Caption(project::CaptionParams {
                emphasis: Default::default(),
                word_highlight: false,
                preset: None,
                overrides: captions::CaptionOverrides::default(),
                cues: captions::CueTrack {
                    styles: Vec::new(),
                    cues: vec![captions::Cue {
                        word_timing: captions::CueWordTiming::Unverified,
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
                        word_ranges: Vec::new(),
                        style: None,
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

fn preset_scene(texts: &[(&str, &str)], boxed: bool) -> project::Project {
    let styled = with_cue_style(
        scene(texts),
        vec![captions::CueStyle {
            font_family: "Inter".into(),
            font_size_ratio: 48.0 / 1080.0,
            primary_color: [255; 4],
            outline_color: [0, 0, 0, 255],
            background_color: if boxed { [0, 0, 0, 255] } else { [0; 4] },
            border_style: if boxed { 3 } else { 1 },
            alignment: captions::CueAlignment::from_numpad(2).unwrap(),
            margin_left_ratio: 0.05,
            margin_right_ratio: 0.05,
            margin_vertical_ratio: 0.05,
            bold: true,
            italic: false,
            underline: false,
            strikeout: false,
        }],
        Some(captions::CueStyleId(0)),
    );
    let mut data = styled.into_data();
    let style = data
        .scene()
        .timeline
        .iter()
        .flat_map(|t| &t.items)
        .find_map(|item| {
            if let project::ItemParams::Caption(p) = &item.content {
                p.cues.styles.first().cloned()
            } else {
                None
            }
        })
        .unwrap();
    data.caption_presets.push(project::SavedCaptionPreset {
        id: project::CaptionPresetId(100),
        name: "Project look".into(),
        style: render::style_from_cue(&style),
    });
    for item in data
        .scene_mut()
        .timeline
        .iter_mut()
        .flat_map(|track| &mut track.items)
    {
        if let project::ItemParams::Caption(params) = &mut item.content {
            params.preset = Some(project::CaptionPresetId(100));
        }
    }
    project::Project::new(data).unwrap()
}

#[test]
fn saved_caption_looks_keep_bilingual_safe_area_diacritics_and_shared_pixels() {
    let texts = [
        ("vi", "Tiếng Việt: Ắ ằ ỡ ự đ"),
        ("en", "Bilingual captions"),
    ];
    let h = Hooks::default();
    for boxed in [false, true] {
        let p = preset_scene(&texts, boxed);
        for (width, height) in [(360, 640), (1080, 1920)] {
            let mut settings = RenderSettings::from_project(&p);
            settings.width = width;
            settings.height = height;
            settings.caption_languages = texts
                .iter()
                .map(|(tag, _)| captions::LanguageTag::parse(tag).unwrap())
                .collect();
            let mut r = Renderer::new(&p, settings.clone()).unwrap();
            let frame = r.preview_frame(tick(1, 2), &mut Empty, &h).unwrap();
            let mut renamed = p.data().clone();
            renamed.caption_presets[0].name = "Renamed".into();
            let renamed = project::Project::new(renamed).unwrap();
            assert_eq!(
                frame.pixels(),
                Renderer::new(&renamed, settings.clone())
                    .unwrap()
                    .render_frame(tick(1, 2), &mut Empty, &h)
                    .unwrap()
                    .pixels(),
                "rename never changes the selected look"
            );
            let mut edited = p.data().clone();
            edited.caption_presets[0].style.size += 12.0;
            let edited = project::Project::new(edited).unwrap();
            assert_ne!(
                frame.pixels(),
                Renderer::new(&edited, settings.clone())
                    .unwrap()
                    .render_frame(tick(1, 2), &mut Empty, &h)
                    .unwrap()
                    .pixels(),
                "the item follows the live preset value"
            );
            let mut removed = p.data().clone();
            removed.caption_presets.clear();
            assert!(
                project::Project::new(removed).is_err(),
                "a live selection never dangles"
            );
            assert_eq!(
                frame.pixels(),
                r.render_frame(tick(1, 2), &mut Empty, &h).unwrap().pixels()
            );
            let gap = r.render_frame(Tick::ZERO, &mut Empty, &h).unwrap();
            assert_eq!(
                gap.pixels(),
                r.render_frame(tick(3, 2), &mut Empty, &h).unwrap().pixels()
            );
            let (x0, y0, x1, y1, count) = ink(&frame, &gap);
            assert!(count > 100 && x0 >= width as usize / 20 && x1 < width as usize * 19 / 20);
            assert!(y0 > height as usize * 4 / 5 && y1 < height as usize * 19 / 20);
            let nfd = preset_scene(
                &[
                    (
                        "vi",
                        "Tie\u{0302}\u{0301}ng Vie\u{0323}\u{0302}t: A\u{0306}\u{0301} a\u{0306}\u{0300} o\u{031b}\u{0303} u\u{031b}\u{0323} đ",
                    ),
                    texts[1],
                ],
                boxed,
            );
            assert_eq!(
                frame.pixels(),
                Renderer::new(&nfd, settings.clone())
                    .unwrap()
                    .render_frame(tick(1, 2), &mut Empty, &h)
                    .unwrap()
                    .pixels()
            );
            let bytes = frame.rgba8(1, &h).unwrap();
            eprintln!(
                "saved preset boxed={boxed} {width}x{height} hash={:#x}",
                hash(&bytes.bytes)
            );
            if width == 360 {
                png(
                    &bytes.bytes,
                    if boxed {
                        "preset-box"
                    } else {
                        "preset-outline"
                    },
                );
            }
            let mut single = p.data().clone();
            single.caption_presets[0].style.max_lines = 1;
            let single = project::Project::new(single).unwrap();
            assert!(matches!(
                Renderer::new(&single, settings.clone())
                    .unwrap()
                    .render_frame(tick(1, 2), &mut Empty, &h),
                Err(render::Error::Unsupported(_))
            ));
            let long = preset_scene(
                &[
                    (
                        "vi",
                        "hôm nay chúng ta cùng tìm hiểu cách tạo phụ đề tiếng Việt cho video dọc thật dễ dàng",
                    ),
                    (
                        "en",
                        "today we learn how to create readable bilingual captions for a portrait video",
                    ),
                ],
                boxed,
            );
            let mut fit = RenderSettings::from_project(&long);
            fit.width = width;
            fit.height = height;
            fit.caption_languages = vec![
                captions::LanguageTag::parse("vi").unwrap(),
                captions::LanguageTag::parse("en").unwrap(),
            ];
            let fitted = Renderer::new(&long, fit)
                .unwrap()
                .render_frame(tick(1, 2), &mut Empty, &h)
                .unwrap();
            assert!(
                ink(&fitted, &gap).4 > 100,
                "both mandatory language lines fit without truncation"
            );
        }
    }
}

#[test]
fn caption_preset_overrides_patch_every_full_style_field() {
    let texts = [
        ("vi", "Tiếng Việt: Ắ ằ ỡ ự đ"),
        ("en", "Bilingual captions"),
    ];
    let h = Hooks::default();
    let at = tick(1, 2);
    let base = preset_scene(&texts, false);
    let overrides = captions::CaptionOverrides {
        font: Some("Noto Serif".into()),
        font_style: Some("Bold".into()),
        size: Some(64.0),
        color: Some([250, 240, 40, 220]),
        outline: Some(Some(captions::CaptionOutline {
            color: [10, 20, 30, 255],
            width: 4.0,
        })),
        background: Some(Some(captions::CaptionBackground {
            color: [0, 0, 90, 180],
            padding: 10.0,
            radius: 6.0,
        })),
        position: Some(captions::CaptionPosition::Top),
        safe_area_percent: Some(12.0),
        max_lines: Some(3),
    };
    let mut patched = base.data().clone();
    for item in patched
        .scene_mut()
        .timeline
        .iter_mut()
        .flat_map(|track| &mut track.items)
    {
        if let project::ItemParams::Caption(params) = &mut item.content {
            params.overrides = overrides.clone();
        }
    }
    patched.caption_presets[0].style = render::style_from_cue(&captions::CueStyle {
        font_family: "Inter".into(),
        font_size_ratio: 48.0 / 1080.0,
        primary_color: [255; 4],
        outline_color: [0, 0, 0, 255],
        background_color: [0; 4],
        border_style: 1,
        alignment: captions::CueAlignment::from_numpad(2).unwrap(),
        margin_left_ratio: 0.05,
        margin_right_ratio: 0.05,
        margin_vertical_ratio: 0.05,
        bold: true,
        italic: false,
        underline: false,
        strikeout: false,
    });
    let selected = project::Project::new(patched).unwrap();
    let mut settings = RenderSettings::from_project(&selected);
    settings.width = 360;
    settings.height = 640;
    settings.caption_languages = texts
        .iter()
        .map(|(tag, _)| captions::LanguageTag::parse(tag).unwrap())
        .collect();
    let mut selection = Renderer::new(&selected, settings.clone()).unwrap();
    let frame = selection.render_frame(at, &mut Empty, &h).unwrap();
    assert_eq!(
        frame.pixels(),
        selection
            .preview_frame(at, &mut Empty, &h)
            .unwrap()
            .pixels(),
        "preview and burn-in share the selected-style path"
    );
    // The same complete values through the transient style path paint identically.
    let mut plain = selected.data().clone();
    plain.caption_presets.clear();
    for item in plain
        .scene_mut()
        .timeline
        .iter_mut()
        .flat_map(|track| &mut track.items)
    {
        if let project::ItemParams::Caption(params) = &mut item.content {
            params.preset = None;
            params.overrides = captions::CaptionOverrides::default();
            params.cues.styles.clear();
            for cue in &mut params.cues.cues {
                cue.style = None;
            }
        }
    }
    let plain = project::Project::new(plain).unwrap();
    let mut effective = render::style_from_cue(&captions::CueStyle {
        font_family: "Inter".into(),
        font_size_ratio: 48.0 / 1080.0,
        primary_color: [255; 4],
        outline_color: [0, 0, 0, 255],
        background_color: [0; 4],
        border_style: 1,
        alignment: captions::CueAlignment::from_numpad(2).unwrap(),
        margin_left_ratio: 0.05,
        margin_right_ratio: 0.05,
        margin_vertical_ratio: 0.05,
        bold: true,
        italic: false,
        underline: false,
        strikeout: false,
    });
    overrides.apply(&mut effective);
    settings.caption_style = effective;
    let expected = Renderer::new(&plain, settings)
        .unwrap()
        .render_frame(at, &mut Empty, &h)
        .unwrap();
    assert_eq!(frame.pixels(), expected.pixels());
    assert_ne!(
        frame.pixels(),
        Renderer::new(&base, RenderSettings::from_project(&base))
            .unwrap()
            .render_frame(at, &mut Empty, &h)
            .unwrap()
            .pixels(),
        "the patch changes pixels"
    );
}

#[test]
fn viewport_caption_visibility_follows_half_open_cue_range() {
    let project = scene(&[("en", "Hello")]);
    let renderer = Renderer::new(&project, RenderSettings::from_project(&project)).unwrap();
    let viewport = render::viewport::Viewport::new(
        Rect {
            x: 0.,
            y: 0.,
            width: 360.,
            height: 640.,
        },
        [360, 640],
        1.,
        [0., 0.],
    )
    .unwrap();
    for (at, visible) in [(Tick::ZERO, false), (tick(1, 4), true), (tick(3, 2), false)] {
        assert_eq!(
            renderer
                .item_geometry(project::ItemId(1), at)
                .unwrap()
                .is_some(),
            visible
        );
        assert_eq!(
            renderer.hit_test(&viewport, [180., 550.], at).unwrap(),
            Some(project::ItemId(u64::from(visible)))
        );
    }
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
            "caption-outline-burn",
            &[
                ("vi", "Tiếng Việt: Ắ ằ ỡ ự đ"),
                ("en", "Bilingual captions"),
            ],
        ),
        (
            "caption-box-burn",
            &[
                ("vi", "Tiếng Việt: Ắ ằ ỡ ự đ"),
                ("en", "Bilingual captions"),
            ],
        ),
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
        let p = if name.contains("outline") || name.contains("box") {
            preset_scene(texts, name.contains("box"))
        } else {
            scene(texts)
        };
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
        let item = &mut data.scene_mut().timeline.overlays[0].items[0];
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
fn imported_cue_style_paints_preview_and_export_like_the_equivalent_caption_style() {
    let h = Hooks::default();
    let at = tick(1, 2);
    let imported = with_cue_style(
        scene(&[("en", "Styled")]),
        vec![imported_style(8)],
        Some(captions::CueStyleId(0)),
    );
    let plain = scene(&[("en", "Styled")]);
    let mut expected = RenderSettings::from_project(&plain);
    expected.caption_style = CaptionStyle {
        font: "Noto Serif".into(),
        font_style: "Bold".into(),
        // 5/64 of 1080 output lines.
        size: 84.375,
        color: [255, 60, 60, 255],
        outline: Some(CaptionOutline {
            color: [0, 0, 255, 255],
            width: 2.0,
        }),
        background: None,
        position: CaptionPosition::Top,
        safe_area_percent: 6.25,
        max_lines: 2,
    };
    let expected = Renderer::new(&plain, expected)
        .unwrap()
        .render_frame(at, &mut Empty, &h)
        .unwrap();
    let mut renderer = Renderer::new(&imported, RenderSettings::from_project(&imported)).unwrap();
    let preview = renderer.preview_frame(at, &mut Empty, &h).unwrap();
    let exported = renderer.render_frame(at, &mut Empty, &h).unwrap();
    assert_eq!(preview.pixels(), exported.pixels(), "one render path");
    assert_eq!(
        preview.pixels(),
        expected.pixels(),
        "imported font, weight, size, colours, outline and top position"
    );
    let project_style = Renderer::new(&plain, RenderSettings::from_project(&plain))
        .unwrap()
        .render_frame(at, &mut Empty, &h)
        .unwrap();
    assert_ne!(preview.pixels(), project_style.pixels());
}

#[test]
fn cues_without_a_style_keep_the_project_caption_style() {
    let h = Hooks::default();
    let at = tick(1, 2);
    let p = scene(&[("en", "Plain")]);
    let baseline = Renderer::new(&p, RenderSettings::from_project(&p))
        .unwrap()
        .render_frame(at, &mut Empty, &h)
        .unwrap();
    let unstyled = with_cue_style(scene(&[("en", "Plain")]), Vec::new(), None);
    let mut renderer = Renderer::new(&unstyled, RenderSettings::from_project(&unstyled)).unwrap();
    assert_eq!(
        renderer.preview_frame(at, &mut Empty, &h).unwrap().pixels(),
        baseline.pixels()
    );
    assert_eq!(
        renderer.render_frame(at, &mut Empty, &h).unwrap().pixels(),
        baseline.pixels()
    );
}

#[test]
fn imported_alignment_underline_and_margins_place_the_cue() {
    let h = Hooks::default();
    let at = tick(1, 2);
    let mut style = imported_style(1);
    style.margin_left_ratio = 0.3;
    style.margin_right_ratio = 0.0;
    style.underline = true;
    let underlined = with_cue_style(
        scene(&[("en", "Left")]),
        vec![style.clone()],
        Some(captions::CueStyleId(0)),
    );
    style.underline = false;
    let plain = with_cue_style(
        scene(&[("en", "Left")]),
        vec![style],
        Some(captions::CueStyleId(0)),
    );
    let background = with_cue_style(scene(&[("en", "Left")]), Vec::new(), None);
    let gap = Renderer::new(&background, RenderSettings::from_project(&background))
        .unwrap()
        .render_frame(Tick::ZERO, &mut Empty, &h)
        .unwrap();
    let mut renderer =
        Renderer::new(&underlined, RenderSettings::from_project(&underlined)).unwrap();
    let preview = renderer.preview_frame(at, &mut Empty, &h).unwrap();
    let exported = renderer.render_frame(at, &mut Empty, &h).unwrap();
    assert_eq!(preview.pixels(), exported.pixels());
    let (min_x, min_y, _, _, count) = ink(&preview, &gap);
    // Left alignment starts the text at the 30% left margin, and bottom
    // alignment keeps it below the middle.
    assert!((104..=125).contains(&min_x), "left margin, got {min_x}");
    assert!(min_y > 320, "bottom position, got {min_y}");
    let without = Renderer::new(&plain, RenderSettings::from_project(&plain))
        .unwrap()
        .render_frame(at, &mut Empty, &h)
        .unwrap();
    let (_, _, _, _, plain_count) = ink(&without, &gap);
    assert!(
        count > plain_count,
        "underline adds ink: {count} vs {plain_count}"
    );
}

#[test]
fn hostile_imported_margins_clamp_and_extreme_font_size_refuses_typed() {
    let h = Hooks::default();
    let at = tick(1, 2);
    let mut style = imported_style(2);
    style.margin_left_ratio = 10.0;
    style.margin_right_ratio = 10.0;
    style.margin_vertical_ratio = 10.0;
    let clamped = with_cue_style(
        scene(&[("en", "Clamped")]),
        vec![style],
        Some(captions::CueStyleId(0)),
    );
    let background = with_cue_style(scene(&[("en", "Clamped")]), Vec::new(), None);
    let gap = Renderer::new(&background, RenderSettings::from_project(&background))
        .unwrap()
        .render_frame(Tick::ZERO, &mut Empty, &h)
        .unwrap();
    let image = Renderer::new(&clamped, RenderSettings::from_project(&clamped))
        .unwrap()
        .render_frame(at, &mut Empty, &h)
        .unwrap();
    let (min_x, _, max_x, _, _) = ink(&image, &gap);
    // Oversized ASS margins never empty the destination: both sides cap at 40%.
    assert!((142..=150).contains(&min_x), "left cap, got {min_x}");
    assert!((210..=218).contains(&max_x), "right cap, got {max_x}");

    let mut huge = imported_style(2);
    huge.font_size_ratio = 10.0;
    let huge = with_cue_style(
        scene(&[("en", "Huge")]),
        vec![huge],
        Some(captions::CueStyleId(0)),
    );
    assert!(matches!(
        Renderer::new(&huge, RenderSettings::from_project(&huge))
            .unwrap()
            .render_frame(at, &mut Empty, &h),
        Err(Error::Invalid("scaled font size"))
    ));
}

fn imported_style(numpad: u8) -> captions::CueStyle {
    captions::CueStyle {
        font_family: "Noto Serif".into(),
        // 5/64 of PlayResY, i.e. exactly 84.375 pixels at 1080 output lines.
        font_size_ratio: 5.0 / 64.0,
        primary_color: [255, 60, 60, 255],
        outline_color: [0, 0, 255, 255],
        background_color: [0, 0, 0, 0],
        border_style: 1,
        alignment: captions::CueAlignment::from_numpad(numpad).unwrap(),
        margin_left_ratio: 0.0625,
        margin_right_ratio: 0.0625,
        margin_vertical_ratio: 0.0625,
        bold: true,
        italic: false,
        underline: false,
        strikeout: false,
    }
}

fn with_cue_style(
    p: project::Project,
    styles: Vec<captions::CueStyle>,
    id: Option<captions::CueStyleId>,
) -> project::Project {
    let mut data = p.into_data();
    let item = &mut data.scene_mut().timeline.overlays[0].items[0];
    let project::ItemParams::Caption(params) = &mut item.content else {
        panic!("expected caption item")
    };
    params.cues.styles = styles;
    params.cues.cues[0].style = id;
    project::Project::new(data).unwrap()
}

/// Bounding box and pixel count of the pixels that differ from `background`.
fn ink(image: &Image, background: &Image) -> (usize, usize, usize, usize, usize) {
    let mut bounds = (usize::MAX, usize::MAX, 0, 0);
    let mut count = 0;
    for (n, (a, b)) in image.pixels().iter().zip(background.pixels()).enumerate() {
        if a != b {
            let (x, y) = (n % image.width() as usize, n / image.width() as usize);
            bounds = (
                bounds.0.min(x),
                bounds.1.min(y),
                bounds.2.max(x),
                bounds.3.max(y),
            );
            count += 1;
        }
    }
    (bounds.0, bounds.1, bounds.2, bounds.3, count)
}

#[test]
fn extreme_font_scaling_is_a_typed_refusal_before_raster_scratch() {
    let mut d = scene(&[("vi", "X")]).into_data();
    d.canvas.width = 1;
    d.scene_mut().timeline.overlays[0].kind = project::TrackKind::Text;
    let item = &mut d.scene_mut().timeline.overlays[0].items[0];
    item.source_asset = None;
    item.content = project::ItemParams::Text(project::TextParams {
        content: "X".into(),
        font_size: 4096.0,
        color: project::Color([255; 4]),
        ..Default::default()
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

#[test]
fn verified_source_words_share_preview_burn_boundaries_and_clear_warnings() {
    use captions::{CueWordTiming, TimingProvenance, VerifiedCueWord, WordHighlightReason};
    let h = Hooks::default();
    for (tag, source, target) in [
        ("vi", "Tiếng Việt", "Clear English"),
        ("en", "office affine", "Tiếng Việt"),
        ("zh", "中文 词语", "Clear English"),
    ] {
        let mut data = scene(&[
            (tag, source),
            (if tag == "en" { "vi" } else { "en" }, target),
        ])
        .into_data();
        let item = &mut data.scene_mut().timeline.overlays[0].items[0];
        let project::ItemParams::Caption(params) = &mut item.content else {
            panic!("caption");
        };
        params.word_highlight = true;
        let cue = &mut params.cues.cues[0];
        let split = source.find(' ').unwrap();
        cue.word_timing = CueWordTiming::Words {
            language: cue.source_lang.clone(),
            text: source.into(),
            records: vec![
                VerifiedCueWord {
                    bytes: 0..split,
                    source_range: TimeRange::new(tick(1, 4), tick(3, 4)).unwrap(),
                    projection: captions::WordProjection::Visible(
                        TimeRange::new(tick(1, 4), tick(3, 4)).unwrap(),
                    ),
                    provenance: TimingProvenance::Native,
                },
                VerifiedCueWord {
                    bytes: split + 1..source.len(),
                    source_range: TimeRange::new(tick(3, 4), tick(1, 1)).unwrap(),
                    projection: captions::WordProjection::Visible(
                        TimeRange::new(tick(3, 4), tick(1, 1)).unwrap(),
                    ),
                    provenance: TimingProvenance::Forced,
                },
            ],
        };
        let p = project::Project::new(data.clone()).unwrap();
        let encoded = project::encode(&p, false).unwrap();
        let decoded = project::decode(&encoded).unwrap().project;
        assert_eq!(
            decoded, p,
            "verified correspondence and opt-in survive persistence"
        );
        let mut s = settings(&p);
        s.caption_languages = [tag, if tag == "en" { "vi" } else { "en" }]
            .iter()
            .map(|tag| captions::LanguageTag::parse(tag).unwrap())
            .collect();
        let mut r = Renderer::new(&p, s.clone()).unwrap();
        assert!(
            r.caption_word_warnings(None, &h)
                .unwrap()
                .warnings
                .is_empty()
        );
        let first = r.render_frame(tick(1, 2), &mut Empty, &h).unwrap();
        let next = r.render_frame(tick(3, 4), &mut Empty, &h).unwrap();
        let rgba = first.rgba8(1, &h).unwrap();
        png(&rgba.bytes, &format!("word-highlight-{tag}"));
        eprintln!("word-highlight-{tag} hash={:#x}", hash(&rgba.bytes));
        let expected = match tag {
            "vi" => 0xb617086f5d85ff1d,
            "en" => 0x4fa62fd0d7be69c0,
            "zh" => 0x4d100236d75f592c,
            _ => panic!("unexpected caption fixture"),
        };
        assert_eq!(
            hash(&rgba.bytes),
            expected,
            "visually inspected portrait source-word golden"
        );
        assert_ne!(
            first.pixels(),
            next.pixels(),
            "touching end belongs only to the next word"
        );
        assert_eq!(
            next.pixels(),
            r.preview_frame(tick(3, 4), &mut Empty, &h)
                .unwrap()
                .pixels()
        );
        assert_eq!(
            first.pixels(),
            r.render_frame(tick(2, 3), &mut Empty, &h).unwrap().pixels()
        );
        // Keyword emphasis rides the same overlay: preview and burn share it, only colour
        // changes, it persists, and a stale selection paints nothing.
        let with_emphasis = |ids: Vec<u32>| {
            let mut data = p.data().clone();
            let project::ItemParams::Caption(params) =
                &mut data.scene_mut().timeline.overlays[0].items[0].content
            else {
                panic!("caption");
            };
            let id = params.cues.cues[0].id.clone();
            params.emphasis.insert(id, ids);
            project::Project::new(data).unwrap()
        };
        let emphasised = with_emphasis(vec![1]);
        assert_eq!(
            project::decode(&project::encode(&emphasised, false).unwrap())
                .unwrap()
                .project,
            emphasised,
            "emphasis survives persistence"
        );
        let mut er = Renderer::new(&emphasised, s.clone()).unwrap();
        assert!(
            er.caption_word_warnings(None, &h)
                .unwrap()
                .warnings
                .is_empty()
        );
        let em_first = er.render_frame(tick(1, 2), &mut Empty, &h).unwrap();
        assert_ne!(
            em_first.pixels(),
            first.pixels(),
            "emphasis recolours: {tag}"
        );
        assert_eq!(
            em_first.pixels(),
            er.preview_frame(tick(1, 2), &mut Empty, &h)
                .unwrap()
                .pixels(),
            "preview and burn share the emphasis path: {tag}"
        );
        let em_rgba = em_first.rgba8(1, &h).unwrap();
        png(&em_rgba.bytes, &format!("word-emphasis-{tag}"));
        eprintln!("word-emphasis-{tag} hash={:#x}", hash(&em_rgba.bytes));
        assert!(
            rgba.bytes
                .chunks_exact(4)
                .zip(em_rgba.bytes.chunks_exact(4))
                .all(|(a, b)| a[3] == b[3]),
            "emphasis never changes alpha or geometry: {tag}"
        );
        let stale = with_emphasis(vec![5]);
        let mut stale_renderer = Renderer::new(&stale, s.clone()).unwrap();
        assert_eq!(
            stale_renderer
                .render_frame(tick(1, 2), &mut Empty, &h)
                .unwrap()
                .pixels(),
            first.pixels(),
            "ids that outlived their words paint nothing: {tag}"
        );
        // Word activity follows the content clock, not the real-local opacity clock.
        let mut held = p.data().clone();
        held.scene_mut().timeline.overlays[0].items[0].effects.push(
            project::effect_definition("hold_time")
                .unwrap()
                .instance("word-hold")
                .with("rate", project::EffectValue::Scalar(2.0))
                .unwrap(),
        );
        let held = project::Project::new(held).unwrap();
        let mut held_renderer = Renderer::new(&held, s.clone()).unwrap();
        let held_early = held_renderer
            .render_frame(tick(1, 2), &mut Empty, &h)
            .unwrap();
        let held_late = held_renderer
            .render_frame(tick(7, 8), &mut Empty, &h)
            .unwrap();
        assert!(
            held_early.pixels() == held_late.pixels(),
            "same content clock and effect pass must give pixel0; differing pixels={}",
            held_early
                .pixels()
                .iter()
                .zip(held_late.pixels())
                .filter(|(a, b)| a != b)
                .count()
        );
        // The clip removes complete words without inventing a time or warning.
        let mut clipped = p.data().clone();
        let item = &mut clipped.scene_mut().timeline.overlays[0].items[0];
        item.source_range = TimeRange::new(tick(5, 4), tick(3, 2)).unwrap();
        let project::ItemParams::Caption(params) = &mut item.content else {
            panic!("unexpected caption fixture")
        };
        params.cues = captions::clip_track(&params.cues, item.source_range).unwrap();
        let clipped = project::Project::new(clipped).unwrap();
        let mut clipped_renderer = Renderer::new(&clipped, s.clone()).unwrap();
        assert!(
            clipped_renderer
                .caption_word_warnings(None, &h)
                .unwrap()
                .warnings
                .is_empty()
        );
        let mut disabled_clip = clipped.data().clone();
        let project::ItemParams::Caption(params) =
            &mut disabled_clip.scene_mut().timeline.overlays[0].items[0].content
        else {
            panic!("unexpected caption fixture")
        };
        params.word_highlight = false;
        let disabled_clip = project::Project::new(disabled_clip).unwrap();
        assert_eq!(
            clipped_renderer
                .render_frame(tick(1, 10), &mut Empty, &h)
                .unwrap()
                .pixels(),
            Renderer::new(&disabled_clip, s.clone())
                .unwrap()
                .render_frame(tick(1, 10), &mut Empty, &h)
                .unwrap()
                .pixels()
        );
        for duplicate_cue in [false, true] {
            let mut ambiguous = p.data().clone();
            let project::ItemParams::Caption(params) =
                &mut ambiguous.scene_mut().timeline.overlays[0].items[0].content
            else {
                panic!("unexpected caption fixture")
            };
            if duplicate_cue {
                let mut cue = params.cues.cues[0].clone();
                cue.id = "overlapping-source".into();
                params.cues.cues.push(cue);
            } else {
                let CueWordTiming::Words { records, .. } = &mut params.cues.cues[0].word_timing
                else {
                    panic!("unexpected caption fixture")
                };
                records[1].projection = captions::WordProjection::Visible(
                    TimeRange::new(tick(1, 2), tick(3, 4)).unwrap(),
                );
            }
            let enabled = project::Project::new(ambiguous.clone()).unwrap();
            let mut renderer = Renderer::new(&enabled, s.clone()).unwrap();
            let warnings = renderer.caption_word_warnings(None, &h).unwrap();
            assert_eq!(
                warnings.warnings.len(),
                1,
                "one summary for the ambiguous state"
            );
            assert_eq!(
                warnings.warnings[0].reason,
                WordHighlightReason::AmbiguousTiming
            );
            let project::ItemParams::Caption(params) =
                &mut ambiguous.scene_mut().timeline.overlays[0].items[0].content
            else {
                panic!("unexpected caption fixture")
            };
            params.word_highlight = false;
            let ordinary = project::Project::new(ambiguous).unwrap();
            assert_eq!(
                renderer
                    .render_frame(tick(1, 2), &mut Empty, &h)
                    .unwrap()
                    .pixels(),
                Renderer::new(&ordinary, s.clone())
                    .unwrap()
                    .render_frame(tick(1, 2), &mut Empty, &h)
                    .unwrap()
                    .pixels()
            );
        }
        let project::ItemParams::Caption(params) =
            &mut data.scene_mut().timeline.overlays[0].items[0].content
        else {
            panic!("unexpected caption fixture")
        };
        params.word_highlight = false;
        let disabled = project::Project::new(data.clone()).unwrap();
        let mut base = Renderer::new(&disabled, s.clone()).unwrap();
        assert_eq!(
            r.render_frame(tick(5, 4), &mut Empty, &h).unwrap().pixels(),
            base.render_frame(tick(5, 4), &mut Empty, &h)
                .unwrap()
                .pixels(),
            "no active word after the last word"
        );
        let project::ItemParams::Caption(params) =
            &mut data.scene_mut().timeline.overlays[0].items[0].content
        else {
            panic!("unexpected caption fixture")
        };
        params.word_highlight = true;
        params.cues.cues[0]
            .texts
            .get_mut(&captions::LanguageTag::parse(tag).unwrap())
            .unwrap()
            .edited = true;
        let stale = project::Project::new(data.clone()).unwrap();
        let mut stale_renderer = Renderer::new(&stale, s.clone()).unwrap();
        assert_eq!(
            stale_renderer
                .caption_word_warnings(None, &h)
                .unwrap()
                .warnings[0]
                .reason,
            WordHighlightReason::SourceChanged
        );
        assert_eq!(
            stale_renderer
                .render_frame(tick(1, 2), &mut Empty, &h)
                .unwrap()
                .pixels(),
            base.render_frame(tick(1, 2), &mut Empty, &h)
                .unwrap()
                .pixels()
        );
        let mut only_translation = s;
        only_translation.caption_languages.remove(0);
        assert_eq!(
            Renderer::new(&p, only_translation)
                .unwrap()
                .caption_word_warnings(None, &h)
                .unwrap()
                .warnings[0]
                .reason,
            WordHighlightReason::SourceNotDisplayed
        );
        assert!(
            base.caption_word_warnings(None, &h)
                .unwrap()
                .warnings
                .is_empty()
        );
    }
}
