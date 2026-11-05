//! Bounded, labelled previews use the production compositor and engine scheduler.
use crate::control::{ControlResult, MAX_RESPONSE_BYTES, PreviewFrame, Revision, bounded_json};
use crate::jobs::{JobBytes, JobKind, JobRunner, JobSpec, ResourceClass};
use crate::{EditError, Result, Session};
use std::io::Write;
use time::{SourceRange, Tick};

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
pub struct PreviewResult {
    pub revision: Revision,
    pub frames: Vec<PreviewFrame>,
}
pub(crate) fn preview(
    session: &Session,
    frames: Vec<i64>,
    runner: &mut JobRunner,
) -> Result<ControlResult> {
    if session.transaction_active() {
        return Err(EditError::TransactionActive);
    }
    if frames.is_empty() || frames.len() > crate::control::MAX_PREVIEW_FRAMES {
        return Err(EditError::Limit);
    }
    let project = session.project().clone();
    let fps = project.data().canvas.fps;
    for frame in &frames {
        let at = fps.tick_of(*frame)?;
        if *frame < 0
            || at.0 > crate::control::MAX_SAFE_INTEGER
            || (at >= project.duration() && at != Tick::ZERO)
        {
            return Err(EditError::InvalidParams);
        }
    }
    let revision = session.revision();
    let key = super::control::control_key(blake3::hash(&bounded_json(
        &(revision.clone(), &frames),
        1024,
    )?))?;
    let accepted_revision = revision.clone();
    let (job, _) = runner.submit_for_session(
        session,
        JobSpec {
            kind: JobKind::Preview,
            class: ResourceClass::Cpu,
            input: key,
            bytes: JobBytes {
                result: (MAX_RESPONSE_BYTES / 2) as u64,
                ..JobBytes::default()
            },
        },
        move |context| {
            let mut output = Vec::new();
            for frame in &frames {
                context.check_cancelled()?;
                let at = fps.tick_of(*frame)?;
                let clips: Vec<_> = project
                    .data()
                    .scene()
                    .timeline
                    .iter()
                    .filter(|t| !t.hidden)
                    .flat_map(|t| &t.items)
                    .filter(|i| i.start <= at && i.end().is_ok_and(|end| at < end))
                    .map(|i| i.id)
                    .collect();
                if clips
                    .iter()
                    .any(|id| id.0 > crate::control::MAX_SAFE_INTEGER as u64)
                {
                    return Err(EditError::InvalidParams);
                }
                if clips.len() > crate::control::MAX_PREVIEW_CLIPS {
                    return Err(EditError::Limit);
                }
                let canvas = &project.data().canvas;
                let long = canvas.width.max(canvas.height);
                let mut settings = render::RenderSettings::from_project(&project);
                settings.width = (u64::from(canvas.width)
                    * u64::from(crate::control::MAX_PREVIEW_EDGE)
                    / u64::from(long))
                .max(1) as u32;
                settings.height = (u64::from(canvas.height)
                    * u64::from(crate::control::MAX_PREVIEW_EDGE)
                    / u64::from(long))
                .max(1) as u32;
                let mut data = project.data().clone();
                let next = data
                    .scenes
                    .iter()
                    .flat_map(|scene| {
                        scene.timeline.iter().flat_map(|track| {
                            std::iter::once(track.id.0)
                                .chain(track.items.iter().map(|item| item.id.0))
                        })
                    })
                    .max()
                    .unwrap_or(0)
                    .checked_add(1)
                    .ok_or(EditError::IdExhausted)?;
                let background_id =
                    project::ItemId(next.checked_add(1).ok_or(EditError::IdExhausted)?);
                let label_track =
                    project::TrackId(next.checked_add(2).ok_or(EditError::IdExhausted)?);
                let label_id = project::ItemId(next.checked_add(3).ok_or(EditError::IdExhausted)?);
                let label = format!(
                    "Frame {} | clips {}",
                    frame,
                    clips
                        .iter()
                        .map(|id| id.0.to_string())
                        .collect::<Vec<_>>()
                        .join(",")
                );
                let label_size = 14.0 * canvas.width as f32 / settings.width as f32;
                let columns = (f64::from(settings.width) * 0.96 / 14.0).floor() as usize;
                let band = label.len().div_ceil(columns.max(1)) * 22 + 12;
                if !(1.0..=4096.0).contains(&label_size)
                    || columns < 8
                    || band > settings.height as usize / 2
                {
                    return Err(EditError::Limit);
                }
                data.scene_mut().timeline.insert_track(project::Track {
                    id: project::TrackId(next),
                    name: "Preview label background".into(),
                    kind: project::TrackKind::Video,
                    muted: false,
                    hidden: false,
                    locked: false,
                    items: vec![project::Item {
                        blend_mode: Default::default(),
                        masks: Vec::new(),
                        effects: Vec::new(),
                        animations: Default::default(),
                        id: background_id,
                        source_asset: None,
                        source_range: SourceRange::new(Tick::ZERO, fps.tick_of(1)?)?,
                        start: at,
                        content: project::ItemParams::Overlay(project::OverlayParams {
                            color: project::Color([0, 0, 0, 224]),
                        }),
                    }],
                });
                settings.placements.insert(
                    background_id,
                    render::Placement {
                        destination: render::Rect {
                            x: 0.01,
                            y: 0.01,
                            width: 0.98,
                            height: (band as f64 + 6.0) / f64::from(settings.height),
                        },
                        ..Default::default()
                    },
                );
                data.scene_mut().timeline.insert_track(project::Track {
                    id: label_track,
                    name: "Preview evidence".into(),
                    kind: project::TrackKind::Text,
                    muted: false,
                    hidden: false,
                    locked: false,
                    items: vec![project::Item {
                        blend_mode: Default::default(),
                        masks: Vec::new(),
                        effects: Vec::new(),
                        animations: Default::default(),
                        id: label_id,
                        source_asset: None,
                        source_range: SourceRange::new(Tick::ZERO, fps.tick_of(1)?)?,
                        start: at,
                        content: project::ItemParams::Text(project::TextParams {
                            content: label.into(),
                            font_size: label_size,
                            color: project::Color([255; 4]),
                        }),
                    }],
                });
                let labelled = project::Project::new(data)?;
                settings.placements.insert(
                    label_id,
                    render::Placement {
                        destination: render::Rect {
                            x: 0.02,
                            y: 0.02,
                            width: 0.96,
                            height: band as f64 / f64::from(settings.height),
                        },
                        fit: render::Fit::Contain,
                        ..Default::default()
                    },
                );
                let width = settings.width;
                let height = settings.height;
                let mut renderer = render::Renderer::new(&labelled, settings)?;
                let mut sources = render::FileSources::default();
                let image = renderer.render_frame(at, &mut sources, context)?;
                let rgba = image.video_rgba8(13, context)?;
                let _encoding = context.memory_budget().reserve(
                    crate::ByteOwner::Export,
                    u64::from(width) * u64::from(height) * 8,
                )?;
                let rgb: Vec<u8> = rgba
                    .bytes
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .flat_map(|p| p.iter().take(3).copied())
                    .collect();
                let mut jpeg = JpegWriter(Vec::new());
                image::codecs::jpeg::JpegEncoder::new_with_quality(
                    &mut jpeg,
                    crate::control::PREVIEW_QUALITY,
                )
                .encode(&rgb, width, height, image::ExtendedColorType::Rgb8)
                .map_err(|_| EditError::Invalid("preview encoding"))?;
                output.push(PreviewFrame {
                    frame: *frame,
                    output_tick: at,
                    clips: clips.clone(),
                    width,
                    height,
                    jpeg: jpeg.0,
                });
            }
            bounded_json(
                &PreviewResult {
                    revision: revision.clone(),
                    frames: output,
                },
                MAX_RESPONSE_BYTES / 2,
            )
        },
    )?;
    Ok(ControlResult::JobAccepted {
        job,
        revision: accepted_revision,
        paths: Vec::new(),
    })
}
struct JpegWriter(Vec<u8>);
impl Write for JpegWriter {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if self.0.len().saturating_add(bytes.len()) > 1 << 20 {
            return Err(std::io::Error::other("preview JPEG limit"));
        }
        self.0.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
