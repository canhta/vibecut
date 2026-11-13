//! Aggregates the field values a multi-item selection shows in one row.
use project::{AnimationPath, BuiltinPath, ItemId, ItemParams, Project};
use std::collections::BTreeSet;
use time::Tick;

#[derive(Clone, Debug, PartialEq)]
pub struct SharedField {
    pub id: &'static str,
    /// None means mixed values; the UI shows a dash, never a representative value.
    pub value: Option<String>,
    pub compatible: usize,
}
/// An RGBA colour as the `#RRGGBBAA` value a shared field carries.
fn hex(color: project::Color) -> String {
    let [r, g, b, a] = color.0;
    format!("#{r:02X}{g:02X}{b:02X}{a:02X}")
}

/// Aggregate only compatible items. An edit never copies unrelated fields.
pub fn shared_fields(project: &Project, selection: &BTreeSet<ItemId>) -> Vec<SharedField> {
    let items: Vec<_> = project
        .data()
        .scene()
        .timeline
        .iter()
        .flat_map(|t| &t.items)
        .filter(|i| selection.contains(&i.id))
        .collect();
    [
        "blendMode",
        "opacity",
        "audio_gain",
        "audio_muted",
        "text_content",
        "text_font",
        "text_size",
        "text_background",
        "text_stroke",
        "text_shadow",
        "text_glow",
    ]
    .into_iter()
    .filter_map(|id| {
        let values: Vec<_> = items
            .iter()
            .filter_map(|i| match (id, &i.content) {
                ("blendMode", content) if !matches!(content, ItemParams::Audio(_)) => {
                    Some(i.blend_mode.id().into())
                }
                ("opacity", content) if !matches!(content, ItemParams::Audio(_)) => {
                    let path = AnimationPath::from(BuiltinPath::Opacity);
                    let value = i.animations.value_at(&path, Tick::ZERO).ok().flatten();
                    Some(match value {
                        Some(project::ParamValue::Scalar(v)) => v.clamp(0.0, 1.0).to_string(),
                        _ => "1".into(),
                    })
                }
                ("audio_gain", ItemParams::Audio(p)) => Some(if p.volume == 0.0 {
                    "0".to_owned()
                } else {
                    p.volume.to_string()
                }),
                ("audio_gain", ItemParams::Video(p)) => Some(p.volume.to_string()),
                ("audio_muted", ItemParams::Audio(p)) => Some(p.muted.to_string()),
                ("text_content", ItemParams::Text(p)) => Some(p.content.as_str().to_owned()),
                ("text_font", ItemParams::Text(p)) => Some(p.font.as_str().to_owned()),
                ("audio_muted", ItemParams::Video(p)) => Some(p.muted.to_string()),
                ("text_size", ItemParams::Text(p)) => Some(p.font_size.to_string()),
                ("text_background", ItemParams::Text(p)) => Some(
                    p.background
                        .as_ref()
                        .map_or_else(|| "none".to_owned(), |background| hex(background.color)),
                ),
                ("text_stroke", ItemParams::Text(p)) => Some(
                    p.stroke
                        .as_ref()
                        .map_or_else(|| "none".to_owned(), |stroke| hex(stroke.color)),
                ),
                ("text_shadow", ItemParams::Text(p)) => Some(
                    p.shadow
                        .as_ref()
                        .map_or_else(|| "none".to_owned(), |shadow| hex(shadow.color)),
                ),
                ("text_glow", ItemParams::Text(p)) => Some(p.glow.to_string()),
                _ => None,
            })
            .collect();
        let first = values.first()?;
        Some(SharedField {
            id,
            value: values.iter().all(|v| v == first).then(|| first.clone()),
            compatible: values.len(),
        })
    })
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[allow(clippy::unwrap_used, reason = "validated fixture assertions")]
    fn shared_fields_distinguish_mixed_values_from_equal_values() {
        use project::{Color, ProjectId, TextParams, Track, TrackId, TrackKind};
        let mut data = Project::empty(ProjectId(1), "".into()).unwrap().into_data();
        data.scene_mut().timeline.insert_track(Track {
            id: TrackId(1),
            name: "Text".into(),
            kind: TrackKind::Text,
            muted: false,
            hidden: false,
            locked: false,
            items: (0..2)
                .map(|n| {
                    testkit::item::item(
                        ItemId(n + 10),
                        ItemParams::Text(TextParams {
                            content: "Keep".into(),
                            font: "Inter".into(),
                            font_size: 24.0 + n as f32,
                            color: Color([10, 20, 30, 255]),
                            background: None,
                            ..TextParams::default()
                        }),
                    )
                    .source(time::SourceRange::new(Tick::ZERO, Tick(100)).unwrap())
                    .start(Tick(n as i64 * 100))
                    .build()
                })
                .collect(),
        });
        let project = Project::new(data).unwrap();
        assert!(shared_fields(&project, &BTreeSet::new()).is_empty());
        for (ids, expected_size, count) in [
            (vec![ItemId(10)], Some("24"), 1),
            (vec![ItemId(10), ItemId(11)], None, 2),
        ] {
            let fields = shared_fields(&project, &ids.into_iter().collect());
            assert_eq!(
                fields,
                vec![
                    SharedField {
                        id: "blendMode",
                        value: Some("normal".into()),
                        compatible: count
                    },
                    SharedField {
                        id: "opacity",
                        value: Some("1".into()),
                        compatible: count
                    },
                    SharedField {
                        id: "text_content",
                        value: Some("Keep".into()),
                        compatible: count
                    },
                    SharedField {
                        id: "text_font",
                        value: Some("Inter".into()),
                        compatible: count
                    },
                    SharedField {
                        id: "text_size",
                        value: expected_size.map(str::to_owned),
                        compatible: count
                    },
                    SharedField {
                        id: "text_background",
                        value: Some("none".into()),
                        compatible: count
                    },
                    SharedField {
                        id: "text_stroke",
                        value: Some("none".into()),
                        compatible: count
                    },
                    SharedField {
                        id: "text_shadow",
                        value: Some("none".into()),
                        compatible: count
                    },
                    SharedField {
                        id: "text_glow",
                        value: Some("false".into()),
                        compatible: count
                    }
                ]
            );
        }
    }
}
