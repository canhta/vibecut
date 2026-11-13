//! Inserts chosen assets into the timeline.
//!
//! The Media-panel "+", a Media drag released over the lanes and a paste of OS files all
//! resolve to `insert_assets`: one `timeline.insert` per asset in one transaction.

use super::super::{
    AssetId, AssetState, BTreeSet, Command, CreatedId, ItemParams, NewItem, PathBuf, Result,
    SourceAsset, SourceRange, TICKS_PER_SECOND, Text, Tick, TrackKind, asset_kind, egui,
    error_detail,
};
use crate::editor::context::MediaDrag;
use super::State;
use crate::editor::Port;
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
    /// Insert assets in library order, one `timeline.insert` each, all in one transaction so
    /// one undo step reverts the batch. The engine owns track allocation and overlap policy.
    /// A secondary placement lands the first asset on the track it creates and stacks the
    /// rest on that track.
    pub(in crate::editor) fn insert_assets(
        &mut self,
        port: &mut Port<'_>,
        assets: &[SourceAsset],
        at: Tick,
        placement: engine::Placement,
    ) -> std::result::Result<Vec<project::ItemId>, engine::control::Error> {
        let unsupported: BTreeSet<AssetId> = assets
            .iter()
            .filter(|asset| self.asset_states.get(&asset.id()) != Some(&AssetState::Ready))
            .map(SourceAsset::id)
            .collect();
        port.session.transaction(|session| {
            let mut created = Vec::new();
            let mut video_at = at;
            let mut audio_at = at;
            let mut target = placement;
            for asset in assets {
                if unsupported.contains(&asset.id()) {
                    return Err(engine::control::Error::Edit(
                        engine::EditError::Unsupported("media codec is unsupported in this build"),
                    ));
                }
                let kind = asset_kind(asset)?;
                if let engine::Placement::Track { track } = target {
                    let owner = session
                        .project()
                        .data()
                        .scene()
                        .timeline
                        .iter()
                        .find(|candidate| candidate.id == track)
                        .map(|candidate| candidate.kind);
                    if owner != Some(kind) {
                        return Err(engine::control::Error::Edit(
                            engine::EditError::Unsupported("track does not accept this media"),
                        ));
                    }
                }
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
                            source_audio_enabled: true,
                            ..project::VideoParams::plain()
                        }),
                        _ => ItemParams::Audio(project::AudioParams::default()),
                    },
                };
                let outcome = session.apply(Command::InsertItem {
                    placement: target,
                    item,
                })?;
                let placed = outcome
                    .placements
                    .first()
                    .ok_or(engine::control::Error::Edit(engine::EditError::Invalid(
                        "item placement",
                    )))?;
                if matches!(
                    target,
                    engine::Placement::Above { .. } | engine::Placement::Below { .. }
                ) {
                    target = engine::Placement::Track {
                        track: placed.track,
                    };
                }
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
        })
    }

    /// Report an insert result: select what it created, or say it failed.
    fn finish_insert(
        &mut self,
        port: &mut Port<'_>,
        result: std::result::Result<Vec<project::ItemId>, engine::control::Error>,
    ) {
        match result {
            Ok(created) => {
                port.select(created.into_iter().collect());
                self.import_notice = None;
            }
            Err(error) => {
                tracing::warn!(category = "asset_insert", "insertion failed");
                self.import_notice = Some(error_detail(&error));
                let text = i18n::text(port.locale(), Text::InsertFailed);
                port.toast(text, None);
            }
        }
    }

    /// The Media "+" (`media.insert`): the selected assets at the playhead, auto-placed.
    pub(in crate::editor) fn insert_selected_assets(&mut self, port: &mut Port<'_>) {
        let selected: Vec<SourceAsset> = port
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
        let at = match port.playhead() {
            Ok(at) => at,
            Err(_error) => {
                tracing::warn!(category = "asset_position", "insertion unavailable");
                let text = i18n::text(port.locale(), Text::InsertFailed);
                port.toast(text, None);
                return;
            }
        };
        let result = self.insert_assets(port, &selected, at, engine::Placement::Auto);
        self.finish_insert(port, result);
    }

    /// Record the Media drag that a pointer press on an asset card starts.
    pub(in crate::editor) fn begin_media_drag(&mut self, port: &mut Port<'_>, ids: Vec<AssetId>) {
        let kind = ids.first().and_then(|first| {
            port.session
                .project()
                .data()
                .assets
                .iter()
                .find(|asset| asset.id() == *first)
                .and_then(|asset| asset_kind(asset).ok())
        });
        self.media_drag = kind.map(|kind| MediaDrag { assets: ids, kind });
    }

    /// The pointer let go with no timeline landing: the drag ends without an insert.
    pub(in crate::editor) fn finish_media_drag(&mut self, ctx: &egui::Context) {
        if self.media_drag.is_some() && ctx.input(|input| !input.pointer.primary_down()) {
            self.media_drag = None;
        }
    }

    /// A Media drag released over the lanes: insert the dragged assets with the placement the
    /// timeline resolved for the drop, at the drop time.
    pub(in crate::editor) fn insert_dropped(
        &mut self,
        port: &mut Port<'_>,
        ids: &[AssetId],
        at: Tick,
        placement: engine::Placement,
    ) {
        let assets: Vec<SourceAsset> = port
            .session
            .project()
            .data()
            .assets
            .iter()
            .filter(|asset| !asset.ephemeral() && ids.contains(&asset.id()))
            .cloned()
            .collect();
        if assets.is_empty() {
            return;
        }
        let result = self.insert_assets(port, &assets, at, placement);
        self.finish_insert(port, result);
    }

    /// Paste with files on the OS clipboard: import them; the probed ones insert at the
    /// playhead when the batch finishes (see `finish_import`).
    pub(in crate::editor) fn paste_files(
        &mut self,
        port: &mut Port<'_>,
        paths: Vec<PathBuf>,
        at: Tick,
    ) -> Result<()> {
        self.start_import(port, paths)?;
        self.paste_at = Some(at);
        Ok(())
    }

    /// The batch a paste started has finished probing: insert what is ready at its playhead.
    pub(in crate::editor) fn insert_pasted(&mut self, port: &mut Port<'_>, at: Tick) {
        let ids = std::mem::take(&mut self.import_added);
        let assets: Vec<SourceAsset> = port
            .session
            .project()
            .data()
            .assets
            .iter()
            .filter(|asset| {
                !asset.ephemeral()
                    && ids.contains(&asset.id())
                    && self.asset_states.get(&asset.id()) == Some(&AssetState::Ready)
            })
            .cloned()
            .collect();
        if assets.is_empty() {
            return;
        }
        let result = self.insert_assets(port, &assets, at, engine::Placement::Auto);
        self.finish_insert(port, result);
    }

    pub(in crate::editor) fn insert_graphic(
        &mut self,
        port: &mut Port<'_>,
        shape: crate::actions::GraphicShape,
    ) {
        let result = (|| -> crate::Result<project::ItemId> {
            let start = port.playhead()?;
            let content = match shape {
                crate::actions::GraphicShape::Rectangle => project::GraphicParams::rectangle(),
                crate::actions::GraphicShape::Ellipse => project::GraphicParams::ellipse(),
                crate::actions::GraphicShape::Polygon => project::GraphicParams::polygon(),
                crate::actions::GraphicShape::Star => project::GraphicParams::star(),
            };
            let outcome = port.session.apply(Command::InsertItem {
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
                port.select(std::collections::BTreeSet::from([item]));
                port.reset_selection(true);
            }
            Err(error) => {
                self.import_notice = Some(error_detail(&error));
                let text = i18n::text(port.locale(), Text::InsertFailed);
                port.toast(text, None);
            }
        }
    }
    pub(in crate::editor) fn insert_text(&mut self, port: &mut Port<'_>) {
        let at = match port.playhead() {
            Ok(at) => at,
            Err(_error) => {
                tracing::warn!(category = "asset_position", "insertion unavailable");
                let text = i18n::text(port.locale(), Text::TextInsertFailed);
                port.toast(text, None);
                return;
            }
        };
        let content = i18n::text(port.locale(), Text::TextItem).to_owned();
        let canvas_height = port.session.project().data().canvas.height;
        let Ok(animations) = lower_third(canvas_height) else {
            tracing::warn!(category = "asset_insert", "text placement unavailable");
            let text = i18n::text(port.locale(), Text::TextInsertFailed);
            port.toast(text, None);
            return;
        };
        let result = port.session.transaction(|session| {
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
                port.select(std::collections::BTreeSet::from([item]));
                port.reset_selection(true);
            }
            Err(error) => {
                tracing::warn!(category = "asset_insert", "insertion failed");
                let detail = error_detail(&error);
                self.import_notice = Some(detail.clone());
                let text = i18n::text(port.locale(), Text::TextInsertFailed);
                port.toast(text, None);
            }
        }
    }
}
