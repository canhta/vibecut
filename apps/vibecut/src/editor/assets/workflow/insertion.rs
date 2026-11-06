//! Inserts chosen assets into the timeline.

use crate::editor::assets::{
    AssetId, AssetState, BTreeSet, Command, CreatedId, Editor, ItemParams, NewItem,
    SelectionScopes, SourceAsset, SourceRange, TICKS_PER_SECOND, Text, Tick, TrackKind, asset_kind,
    error_detail,
};

impl Editor {
    /// Insert selected assets in library order; engine owns track and overlap policy.
    pub(in crate::editor) fn insert_selected_assets(&mut self, command: engine::CommandKind) {
        let selected: Vec<SourceAsset> = self
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
        let at = match self.playback.position() {
            Ok(at) => at,
            Err(error) => {
                self.set_toast(self.tr(Text::InsertFailed), Some(error_detail(&error)));
                return;
            }
        };
        let unsupported: BTreeSet<AssetId> = selected
            .iter()
            .filter(|asset| self.asset_states.get(&asset.id()) != Some(&AssetState::Ready))
            .map(SourceAsset::id)
            .collect();
        let result = self.session.transaction(|session| {
            let mut created = Vec::new();
            let mut video_at = at;
            let mut audio_at = at;
            for asset in &selected {
                if unsupported.contains(&asset.id()) {
                    return Err(engine::EditError::Unsupported(
                        "media codec is unsupported in this build",
                    ));
                }
                let kind = asset_kind(asset)?;
                let duration = asset
                    .probe()
                    .duration
                    .filter(|duration| *duration > Tick::ZERO)
                    .ok_or(engine::EditError::Unsupported(
                        "media duration is unavailable",
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
                        }),
                        _ => ItemParams::Audio(project::AudioParams {
                            fades: Default::default(),
                            volume: 0.0,
                            muted: false,
                            source: Default::default(),
                        }),
                    },
                };
                let outcome = session.execute(
                    command.id(),
                    serde_json::json!({
                        "placement": engine::Placement::Auto, "item": item,
                    }),
                )?;
                let placed = outcome
                    .placements
                    .first()
                    .ok_or(engine::EditError::Invalid("item placement"))?;
                let end = session
                    .project()
                    .data()
                    .scene()
                    .timeline
                    .iter()
                    .find(|track| track.id == placed.track)
                    .and_then(|track| track.items.iter().find(|item| item.id == placed.item))
                    .ok_or(engine::EditError::NoItem(placed.item))?
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
                self.selection = created.into_iter().collect();
                self.import_notice = None;
                self.session.end_merge();
            }
            Err(error) => {
                let detail = error_detail(&error);
                self.import_notice = Some(detail.clone());
                self.set_toast(self.tr(Text::InsertFailed), Some(detail));
            }
        }
    }
    pub(in crate::editor) fn insert_text(&mut self) {
        let at = match self.playback.position() {
            Ok(at) => at,
            Err(error) => {
                self.set_toast(self.tr(Text::TextInsertFailed), Some(error_detail(&error)));
                return;
            }
        };
        let content = self.tr(Text::TextItem).to_owned();
        let result = self.session.transaction(|session| {
            let duration = Tick(5 * TICKS_PER_SECOND);
            let outcome = session.apply(Command::InsertItem {
                placement: engine::Placement::Auto,
                item: NewItem {
                    retime: Default::default(),
                    animations: Default::default(),
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
                .ok_or(engine::EditError::Unsupported("text item allocation"))
        });
        match result {
            Ok(item) => {
                self.selection = [item].into_iter().collect();
                self.selection_scopes = SelectionScopes::default();
                self.properties.clear_drafts();
            }
            Err(error) => {
                let detail = error_detail(&error);
                self.import_notice = Some(detail.clone());
                self.set_toast(self.tr(Text::TextInsertFailed), Some(detail));
            }
        }
    }
}
