//! Inserts chosen assets into the timeline.

use super::super::{
    AssetId, AssetState, BTreeSet, Command, CreatedId, ItemParams, NewItem, SelectionScopes,
    SourceAsset, SourceRange, TICKS_PER_SECOND, Text, Tick, TrackKind, asset_kind, error_detail,
};
use super::State;
use crate::editor::Desk;
use crate::i18n;

/// New text sits in the lower third: its box centre is this fraction of the canvas height
/// below the centre (the old 84.5 % lower-third box against a 50 % centre).
const TEXT_LOWER_THIRD_SHIFT: f64 = 0.345;

/// The Transform of new text: the Y offset to the lower third, in canvas pixels. Transform
/// (0, 0) is the canvas centre for every kind, text included.
fn lower_third(canvas_height: u32) -> project::Result<project::Animations> {
    let mut animations = project::Animations::default();
    animations.set(
        project::AnimationPath::from(project::BuiltinPath::TransformPositionY),
        project::Param::new(project::ParamValue::Scalar(
            f64::from(canvas_height) * TEXT_LOWER_THIRD_SHIFT,
        ))?,
    )?;
    Ok(animations)
}

impl State {
    /// Insert selected assets in library order; engine owns track and overlap policy.
    pub(in crate::editor) fn insert_selected_assets(&mut self, desk: &mut Desk<'_>) {
        let selected: Vec<SourceAsset> = desk
            .editor()
            .session
            .project()
            .data()
            .assets
            .iter()
            .filter(|asset| !asset.ephemeral())
            .filter(|asset| self.media_selection.contains(&asset.id()))
            .cloned()
            .collect();
        if selected.is_empty() {
            return;
        }
        let at = match desk.editor().playback.position() {
            Ok(at) => at,
            Err(_error) => {
                tracing::warn!(category = "asset_position", "insertion unavailable");
                let text = i18n::text(desk.editor().preferences.locale, Text::InsertFailed);
                desk.toast(text, None);
                return;
            }
        };
        let unsupported: BTreeSet<AssetId> = selected
            .iter()
            .filter(|asset| self.asset_states.get(&asset.id()) != Some(&AssetState::Ready))
            .map(SourceAsset::id)
            .collect();
        let result = desk.editor().session.transaction(|session| {
            let mut created = Vec::new();
            let mut video_at = at;
            let mut audio_at = at;
            for asset in &selected {
                if unsupported.contains(&asset.id()) {
                    return Err(engine::control::Error::Edit(
                        engine::EditError::Unsupported("media codec is unsupported in this build"),
                    ));
                }
                let kind = asset_kind(asset)?;
                let duration = asset
                    .probe()
                    .duration
                    .filter(|duration| *duration > Tick::ZERO)
                    .ok_or(engine::control::Error::Edit(
                        engine::EditError::Unsupported("media duration is unavailable"),
                    ))?;
                let item = NewItem {
                    retime: Default::default(),
                    animations: Default::default(),
                    source_asset: Some(asset.id()),
                    source_range: SourceRange::new(Tick::ZERO, duration)?,
                    start: if kind == TrackKind::Video {
                        video_at
                    } else {
                        audio_at
                    },
                    content: match kind {
                        TrackKind::Video => ItemParams::Video(project::VideoParams {
                            fades: Default::default(),
                            source_audio_enabled: true,
                            volume: 0.0,
                            muted: false,
                            crop: None,
                            cleanup: Default::default(),
                            background: project::ClipBackground::None,
                            reduce_noise: false,
                        }),
                        _ => ItemParams::Audio(project::AudioParams {
                            fades: Default::default(),
                            volume: 0.0,
                            muted: false,
                            source: Default::default(),
                            cleanup: Default::default(),
                        }),
                    },
                };
                let outcome = session.apply(Command::InsertItem {
                    placement: engine::Placement::Auto,
                    item,
                })?;
                let placed = outcome
                    .placements
                    .first()
                    .ok_or(engine::control::Error::Edit(engine::EditError::Invalid(
                        "item placement",
                    )))?;
                let end = session
                    .project()
                    .data()
                    .scene()
                    .timeline
                    .iter()
                    .find(|track| track.id == placed.track)
                    .and_then(|track| track.items.iter().find(|item| item.id == placed.item))
                    .ok_or(engine::control::Error::Edit(engine::EditError::NoItem(
                        placed.item,
                    )))?
                    .end()?;
                if kind == TrackKind::Video {
                    video_at = end;
                } else {
                    audio_at = end;
                }
                created.extend(outcome.created.into_iter().filter_map(|id| match id {
                    CreatedId::Item(item) => Some(item),
                    _ => None,
                }));
            }
            Ok(created)
        });
        match result {
            Ok(created) => {
                desk.select(created.into_iter().collect());
                self.import_notice = None;
            }
            Err(error) => {
                tracing::warn!(category = "asset_insert", "insertion failed");
                let detail = error_detail(&error);
                self.import_notice = Some(detail.clone());
                let text = i18n::text(desk.editor().preferences.locale, Text::InsertFailed);
                desk.toast(text, None);
            }
        }
    }
    pub(in crate::editor) fn insert_graphic(
        &mut self,
        desk: &mut Desk<'_>,
        shape: crate::actions::GraphicShape,
    ) {
        let result = (|| -> crate::Result<project::ItemId> {
            let start = desk.editor().playback.position()?;
            let content = match shape {
                crate::actions::GraphicShape::Rectangle => project::GraphicParams::rectangle(),
                crate::actions::GraphicShape::Ellipse => project::GraphicParams::ellipse(),
                crate::actions::GraphicShape::Polygon => project::GraphicParams::polygon(),
                crate::actions::GraphicShape::Star => project::GraphicParams::star(),
            };
            let outcome = desk.editor().session.apply(Command::InsertItem {
                placement: engine::Placement::Auto,
                item: NewItem {
                    retime: Default::default(),
                    animations: Default::default(),
                    source_asset: None,
                    source_range: SourceRange::new(Tick::ZERO, Tick(5 * TICKS_PER_SECOND))?,
                    start,
                    content: ItemParams::Graphic(content),
                },
            })?;
            outcome
                .created
                .into_iter()
                .find_map(|id| match id {
                    CreatedId::Item(item) => Some(item),
                    _ => None,
                })
                .ok_or(crate::Error::Invalid("graphic allocation"))
        })();
        match result {
            Ok(item) => {
                desk.select(std::collections::BTreeSet::from([item]));
                desk.editor().selection_scopes = SelectionScopes::default();
                desk.editor().properties.clear_drafts();
            }
            Err(error) => {
                self.import_notice = Some(error_detail(&error));
                let text = i18n::text(desk.editor().preferences.locale, Text::InsertFailed);
                desk.toast(text, None);
            }
        }
    }
    pub(in crate::editor) fn insert_text(&mut self, desk: &mut Desk<'_>) {
        let at = match desk.editor().playback.position() {
            Ok(at) => at,
            Err(_error) => {
                tracing::warn!(category = "asset_position", "insertion unavailable");
                let text = i18n::text(desk.editor().preferences.locale, Text::TextInsertFailed);
                desk.toast(text, None);
                return;
            }
        };
        let content = i18n::text(desk.editor().preferences.locale, Text::TextItem).to_owned();
        let canvas_height = desk.editor().session.project().data().canvas.height;
        let Ok(animations) = lower_third(canvas_height) else {
            tracing::warn!(category = "asset_insert", "text placement unavailable");
            let text = i18n::text(desk.editor().preferences.locale, Text::TextInsertFailed);
            desk.toast(text, None);
            return;
        };
        let result = desk.editor().session.transaction(|session| {
            let duration = Tick(5 * TICKS_PER_SECOND);
            let outcome = session.apply(Command::InsertItem {
                placement: engine::Placement::Auto,
                item: NewItem {
                    retime: Default::default(),
                    animations,
                    source_asset: None,
                    source_range: SourceRange::new(Tick::ZERO, duration)?,
                    start: at,
                    content: ItemParams::Text(project::TextParams::new(content)),
                },
            })?;
            outcome
                .created
                .into_iter()
                .find_map(|id| match id {
                    CreatedId::Item(item) => Some(item),
                    _ => None,
                })
                .ok_or(engine::control::Error::Edit(
                    engine::EditError::Unsupported("text item allocation"),
                ))
        });
        match result {
            Ok(item) => {
                desk.select(std::collections::BTreeSet::from([item]));
                desk.editor().selection_scopes = SelectionScopes::default();
                desk.editor().properties.clear_drafts();
            }
            Err(error) => {
                tracing::warn!(category = "asset_insert", "insertion failed");
                let detail = error_detail(&error);
                self.import_notice = Some(detail.clone());
                let text = i18n::text(desk.editor().preferences.locale, Text::TextInsertFailed);
                desk.toast(text, None);
            }
        }
    }
}
