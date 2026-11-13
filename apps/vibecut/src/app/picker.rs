//! Native file pickers: requested by a window, polled without blocking, delivered to its editor.
use super::window::EditorWindow;
use crate::{
    Error, Result, egui,
    i18n::{self, Text},
    services::lifecycle::CaptionOutput,
};
use project::ItemId;
use std::{path::PathBuf, sync::Arc};
use time::Tick;

#[derive(Clone, Copy)]
pub(super) enum PickerAction {
    Open,
    Save {
        leave: bool,
        close: bool,
    },
    ImportMedia,
    ImportLut,
    ExportClips,
    /// Replace one clip's source with a file the project already holds.
    ReplaceMedia {
        item: ItemId,
    },
    ExportSave,
    /// A caption file or the burned MP4, saved from the export dialog's caption section.
    ExportCaption(CaptionOutput),
    SnapshotSave,
    /// Destination of one timeline clip's download.
    ClipDownload {
        item: ItemId,
        mode: engine::ClipExportMode,
    },
    KeybindingsImport {
        replace: bool,
    },
    KeybindingsExport,
    /// A caption file the Add captions page imports, parsed at the playhead it was asked from.
    ImportSubtitles {
        at: Tick,
    },
    /// A custom font file for the font picker's Import action.
    FontUpload,
}
pub(super) struct Picker {
    pub(super) future:
        std::pin::Pin<Box<dyn std::future::Future<Output = Option<Vec<rfd::FileHandle>>>>>,
    pub(super) action: PickerAction,
    pub(super) waker: std::task::Waker,
}
struct Repaint(egui::Context);
impl std::task::Wake for Repaint {
    fn wake(self: Arc<Self>) {
        self.0.request_repaint();
    }
}

impl EditorWindow {
    pub(super) fn pick(&mut self, ctx: &egui::Context, action: PickerAction) -> Result<()> {
        if self.busy() {
            return Err(Error::Invalid("project operation in progress"));
        }
        let future: std::pin::Pin<
            Box<dyn std::future::Future<Output = Option<Vec<rfd::FileHandle>>>>,
        > = match action {
            PickerAction::Open => {
                let dialog = rfd::AsyncFileDialog::new()
                    .add_filter(self.tr(Text::AppName), &["vibecut", "json"]);
                Box::pin(async move { dialog.pick_file().await.map(|file| vec![file]) })
            }
            PickerAction::Save { .. } => {
                let dialog = rfd::AsyncFileDialog::new()
                    .add_filter(self.tr(Text::AppName), &["vibecut", "json"]);
                let file_name = self.tr(Text::DefaultProjectFileName);
                Box::pin(async move {
                    dialog
                        .set_file_name(file_name)
                        .save_file()
                        .await
                        .map(|file| vec![file])
                })
            }
            PickerAction::ExportSave => {
                let editor = self.editor.as_ref().ok_or(Error::Invalid("no project"))?;
                let format = editor.export_save_format();
                let (file_name, directory) = editor.export_save_defaults();
                let mut dialog = rfd::AsyncFileDialog::new()
                    .set_title(i18n::text(editor.preferences.locale, Text::ExportTitle))
                    .add_filter(
                        i18n::text(editor.preferences.locale, Text::ExportFormat),
                        &[format.extension()],
                    )
                    .set_file_name(file_name);
                if let Some(directory) = directory {
                    dialog = dialog.set_directory(directory);
                }
                Box::pin(async move { dialog.save_file().await.map(|file| vec![file]) })
            }
            PickerAction::ExportCaption(output) => {
                let file_name = format!("{}.{}", self.tr(Text::SubFileBase), output.extension());
                let dialog = rfd::AsyncFileDialog::new().set_file_name(file_name);
                Box::pin(async move { dialog.save_file().await.map(|file| vec![file]) })
            }
            PickerAction::ExportClips => {
                let dialog = rfd::AsyncFileDialog::new().set_title(self.tr(Text::ClipExportFolder));
                Box::pin(async move { dialog.pick_folder().await.map(|folder| vec![folder]) })
            }
            PickerAction::KeybindingsImport { .. } => {
                let dialog = rfd::AsyncFileDialog::new()
                    .add_filter(self.tr(Text::KeybindingsFormat), &["json"]);
                Box::pin(async move { dialog.pick_file().await.map(|file| vec![file]) })
            }
            PickerAction::KeybindingsExport => {
                let dialog = rfd::AsyncFileDialog::new()
                    .add_filter(self.tr(Text::KeybindingsFormat), &["json"])
                    .set_file_name(self.tr(Text::KeybindingsFileName));
                Box::pin(async move { dialog.save_file().await.map(|file| vec![file]) })
            }
            PickerAction::ReplaceMedia { .. } => {
                let dialog = rfd::AsyncFileDialog::new().add_filter(
                    self.tr(Text::Media),
                    crate::editor::assets::import::MEDIA_EXTENSIONS,
                );
                Box::pin(async move { dialog.pick_file().await.map(|file| vec![file]) })
            }
            PickerAction::ImportLut => {
                let dialog =
                    rfd::AsyncFileDialog::new().add_filter(self.tr(Text::Lut), &["cube", "3dl"]);
                Box::pin(async move { dialog.pick_file().await.map(|file| vec![file]) })
            }
            PickerAction::ImportSubtitles { .. } => {
                let dialog = rfd::AsyncFileDialog::new()
                    .add_filter(self.tr(Text::SubImport), &["srt", "vtt"]);
                Box::pin(async move { dialog.pick_file().await.map(|file| vec![file]) })
            }
            PickerAction::FontUpload => {
                let title = self.tr(Text::FontUpload);
                let dialog = rfd::AsyncFileDialog::new()
                    .set_title(title)
                    .add_filter(title, &["ttf", "otf", "woff2"]);
                Box::pin(async move { dialog.pick_file().await.map(|file| vec![file]) })
            }
            PickerAction::ImportMedia => {
                let dialog = rfd::AsyncFileDialog::new().add_filter(
                    self.tr(Text::Media),
                    crate::editor::assets::import::MEDIA_EXTENSIONS,
                );
                Box::pin(dialog.pick_files())
            }
            PickerAction::SnapshotSave => {
                let editor = self.editor.as_ref().ok_or(Error::Invalid("no project"))?;
                let (file_name, directory) = editor.snapshot_save_defaults();
                let mut dialog = rfd::AsyncFileDialog::new()
                    .set_title(i18n::text(editor.preferences.locale, Text::SnapshotSave))
                    .add_filter(
                        i18n::text(editor.preferences.locale, Text::SnapshotFormat),
                        &["png"],
                    )
                    .set_file_name(file_name);
                if let Some(directory) = directory {
                    dialog = dialog.set_directory(directory);
                }
                Box::pin(async move { dialog.save_file().await.map(|file| vec![file]) })
            }
            PickerAction::ClipDownload { .. } => {
                let editor = self.editor.as_ref().ok_or(Error::Invalid("no project"))?;
                let dialog = rfd::AsyncFileDialog::new()
                    .set_title(i18n::text(editor.preferences.locale, Text::DownloadClip))
                    .add_filter(
                        i18n::text(editor.preferences.locale, Text::ExportFormat),
                        &["mp4", "mov", "webm"],
                    )
                    .set_file_name(format!(
                        "{}.mp4",
                        i18n::text(editor.preferences.locale, Text::ClipDownloadFileName)
                    ));
                Box::pin(async move { dialog.save_file().await.map(|file| vec![file]) })
            }
        };
        self.picker = Some(Picker {
            future,
            action,
            waker: std::task::Waker::from(Arc::new(Repaint(ctx.clone()))),
        });
        Ok(())
    }
    pub(super) fn save_from_ui(
        &mut self,
        ctx: &egui::Context,
        leave: bool,
        close: bool,
        force_as: bool,
    ) -> Result<()> {
        let editor = self.editor.as_mut().ok_or(Error::Invalid("no project"))?;
        let needs_path = editor
            .session
            .autosave()
            .is_some_and(|s| s.saved_path().is_none());
        if force_as || needs_path {
            self.pick(ctx, PickerAction::Save { leave, close })
        } else {
            self.persist(None, leave, close, false)
        }
    }
    /// Complete the real LUT picker without opening a native window in tests.
    pub fn lut_picked_file(&mut self, path: Option<PathBuf>) -> Result<()> {
        self.editor
            .as_mut()
            .ok_or(Error::Invalid("no project"))?
            .lut_picked_file(path)
    }
    /// Apply native picker result paths through the same import path as drops.
    /// This OS boundary accepts injected results without opening a native dialog.
    pub fn import_picked_files(&mut self, paths: Vec<PathBuf>) -> Result<()> {
        self.editor
            .as_mut()
            .ok_or(Error::Invalid("no project"))?
            .start_import(paths)
    }
    /// Start `export.clips` in the folder the native picker chose.
    pub fn export_clips_picked(&mut self, directory: PathBuf) -> Result<()> {
        self.editor
            .as_mut()
            .ok_or(Error::Invalid("no project"))?
            .export_clips_picked(directory)
    }
    /// A dismissed folder picker leaves the queued clips unexported.
    pub fn export_clips_cancelled(&mut self) {
        if let Some(editor) = &mut self.editor {
            editor.export_clips_cancelled();
        }
    }
    /// Apply the replace picker's file to the clip it was requested for. Tests inject a
    /// path here without opening a native dialog.
    pub fn replace_picked_file(&mut self, item: ItemId, path: PathBuf) -> Result<()> {
        self.editor
            .as_mut()
            .ok_or(Error::Invalid("no project"))?
            .replace_picked_file(item, path)
    }
    /// Apply a save-picker destination to the existing export dialog.
    pub fn export_picked_file(&mut self, path: PathBuf) -> Result<()> {
        self.editor
            .as_mut()
            .ok_or(Error::Invalid("no project"))?
            .export_picked_file(path)
    }
    pub fn export_save_cancelled(&mut self) {
        if let Some(editor) = &mut self.editor {
            editor.export_save_cancelled();
        }
    }
    /// Apply the native snapshot picker's destination through `preview.snapshot`.
    pub fn snapshot_picked_file(&mut self, path: PathBuf) -> Result<()> {
        self.editor
            .as_mut()
            .ok_or(Error::Invalid("no project"))?
            .snapshot_picked_file(path)
    }
    /// Save the chosen caption output to the picked destination through the captions panel.
    pub fn caption_output_picked(&mut self, path: PathBuf, output: CaptionOutput) -> Result<()> {
        self.editor
            .as_mut()
            .ok_or(Error::Invalid("no project"))?
            .caption_output_picked(path, output)
    }
    /// Cancelling the native snapshot dialog issues no command.
    pub fn snapshot_save_cancelled(&mut self) {
        self.picker = self
            .picker
            .take()
            .filter(|picker| !matches!(picker.action, PickerAction::SnapshotSave));
        if let Some(editor) = &mut self.editor {
            editor.snapshot_save_cancelled();
        }
    }
    /// Start the `export.clip` download the native save picker chose.
    pub fn clip_download_picked(
        &mut self,
        item: ItemId,
        mode: engine::ClipExportMode,
        path: PathBuf,
    ) -> Result<()> {
        self.editor
            .as_mut()
            .ok_or(Error::Invalid("no project"))?
            .clip_download_picked(item, mode, path)
    }
    /// Apply a shortcut file chosen in the native picker, merging or replacing it.
    pub fn keybindings_import_picked(&mut self, path: PathBuf, replace: bool) -> Result<()> {
        self.editor
            .as_mut()
            .ok_or(Error::Invalid("no project"))?
            .keybindings_import_picked(path, replace)
    }
    /// Write the canonical shortcut profile to the picked destination.
    pub fn keybindings_export_picked(&mut self, path: PathBuf) -> Result<()> {
        self.editor
            .as_mut()
            .ok_or(Error::Invalid("no project"))?
            .keybindings_export_picked(path)
    }
    /// Parse the SRT/VTT file the picker chose through the captions panel.
    pub fn subtitles_import_picked(&mut self, path: PathBuf, at: Tick) -> Result<()> {
        self.editor
            .as_mut()
            .ok_or(Error::Invalid("no project"))?
            .subtitles_import_picked(path, at)
    }
    /// Complete the font picker's file; `None` is a dismissed or refused pick.
    pub fn font_import_picked(&mut self, path: Option<PathBuf>) -> Result<()> {
        self.editor
            .as_mut()
            .ok_or(Error::Invalid("no project"))?
            .font_import_picked(path)
    }
    pub(super) fn poll_picker(&mut self) {
        let Some(mut picker) = self.picker.take() else {
            return;
        };
        match picker
            .future
            .as_mut()
            .poll(&mut std::task::Context::from_waker(&picker.waker))
        {
            std::task::Poll::Pending => self.picker = Some(picker),
            std::task::Poll::Ready(None) => {
                if matches!(picker.action, PickerAction::ImportLut)
                    && let Err(error) = self.lut_picked_file(None)
                {
                    self.error = Some(error);
                }
                if matches!(picker.action, PickerAction::FontUpload)
                    && let Err(error) = self.font_import_picked(None)
                {
                    self.error = Some(error);
                }
                if matches!(picker.action, PickerAction::ExportClips) {
                    self.export_clips_cancelled();
                }
            }
            std::task::Poll::Ready(Some(mut files)) => {
                let result = match picker.action {
                    PickerAction::Open => match files.pop() {
                        Some(file) => {
                            self.requested_open = Some(file.path().to_owned());
                            Ok(())
                        }
                        None => Ok(()),
                    },
                    PickerAction::Save { leave, close } => match files.pop() {
                        Some(file) => {
                            self.persist(Some(file.path().to_owned()), leave, close, false)
                        }
                        None => Ok(()),
                    },
                    PickerAction::ExportSave => match files.pop() {
                        Some(file) => self.export_picked_file(file.path().to_owned()),
                        None => {
                            self.export_save_cancelled();
                            Ok(())
                        }
                    },
                    PickerAction::SnapshotSave => match files.pop() {
                        Some(file) => self.snapshot_picked_file(file.path().to_owned()),
                        None => {
                            self.snapshot_save_cancelled();
                            Ok(())
                        }
                    },
                    PickerAction::ExportCaption(output) => match files.pop() {
                        Some(file) => self.caption_output_picked(file.path().to_owned(), output),
                        None => Ok(()),
                    },
                    PickerAction::ClipDownload { item, mode } => match files.pop() {
                        Some(file) => self.clip_download_picked(item, mode, file.path().to_owned()),
                        None => Ok(()),
                    },
                    PickerAction::ExportClips => match files.pop() {
                        Some(folder) => self.export_clips_picked(folder.path().to_owned()),
                        None => {
                            self.export_clips_cancelled();
                            Ok(())
                        }
                    },
                    PickerAction::KeybindingsImport { replace } => match files.pop() {
                        Some(file) => {
                            self.keybindings_import_picked(file.path().to_owned(), replace)
                        }
                        None => Ok(()),
                    },
                    PickerAction::KeybindingsExport => match files.pop() {
                        Some(file) => self.keybindings_export_picked(file.path().to_owned()),
                        None => Ok(()),
                    },
                    PickerAction::ImportLut => {
                        self.lut_picked_file(files.pop().map(|file| file.path().to_owned()))
                    }
                    PickerAction::ImportSubtitles { at } => match files.pop() {
                        Some(file) => self.subtitles_import_picked(file.path().to_owned(), at),
                        None => Ok(()),
                    },
                    PickerAction::FontUpload => {
                        self.font_import_picked(files.pop().map(|file| file.path().to_owned()))
                    }
                    PickerAction::ReplaceMedia { item } => match files.pop() {
                        Some(file) => self.replace_picked_file(item, file.path().to_owned()),
                        None => Ok(()),
                    },
                    PickerAction::ImportMedia => {
                        let paths = files
                            .into_iter()
                            .map(|file| file.path().to_owned())
                            .collect();
                        self.import_picked_files(paths)
                    }
                };
                if let Err(e) = result {
                    self.error = Some(e);
                }
            }
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::{
        Editor,
        app::Workspace,
        preferences::Preferences,
        services::{
            lifecycle::EditorAction,
            playback::{Files, Playback},
        },
    };
    use engine::ByteBudget;
    /// Source check: `rfd` is named only by this module, so every native dialog is a
    /// `PickerAction` the window owns.
    #[test]
    fn native_dialogs_are_requested_only_through_this_picker() {
        fn visit(directory: &std::path::Path, root: &std::path::Path, found: &mut Vec<String>) {
            for entry in std::fs::read_dir(directory).unwrap() {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    visit(&path, root, found);
                } else if path.extension().and_then(|ext| ext.to_str()) == Some("rs")
                    && std::fs::read_to_string(&path).unwrap().contains("rfd::")
                {
                    let relative = path.strip_prefix(root).unwrap();
                    found.push(relative.to_string_lossy().replace('\\', "/"));
                }
            }
        }
        // shorts/state.rs keeps its panel-local dialog until ticket 226's rewrite lands;
        // drop that entry in the same change as the rewrite.
        const ALLOWED: [&str; 2] = ["app/picker.rs", "shorts/state.rs"];
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut found = Vec::new();
        visit(&root, &root, &mut found);
        assert!(
            found.iter().all(|path| ALLOWED.contains(&path.as_str())),
            "rfd outside app/picker.rs: {found:?}"
        );
    }
    #[test]
    fn cancelled_lut_picker_releases_capture_without_project_edit() {
        use egui_kittest::kittest::{By, NodeT, Queryable};
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("source.png");
        image::RgbaImage::from_pixel(8, 8, image::Rgba([80, 120, 200, 255]))
            .save(&path)
            .unwrap();
        let mut data = project::Project::empty(project::ProjectId(410), "LUT cancel".into())
            .unwrap()
            .into_data();
        data.assets
            .push(media::SourceAsset::import(media::AssetId(1), path).unwrap());
        data.scene_mut().timeline.main.items.push(
            testkit::item::video(project::ItemId(2))
                .asset(media::AssetId(1))
                .source(
                    time::SourceRange::new(time::Tick::ZERO, time::Tick(time::TICKS_PER_SECOND))
                        .unwrap(),
                )
                .build(),
        );
        let session = engine::Session::new(project::Project::new(data).unwrap(), 20).unwrap();
        let playback = Playback::new(
            &session,
            ByteBudget::new(768 << 20).unwrap(),
            Arc::new(Files),
        )
        .unwrap();
        let mut editor = Editor::new(session, playback, Preferences::default());
        editor
            .preferences
            .properties_tabs
            .insert("image".into(), "adjust".into());
        editor.select([project::ItemId(2)].into()).unwrap();
        let mut workspace =
            Workspace::new(root.path().join("recovery"), Preferences::default(), None).unwrap();
        let id = workspace.register_editor(Box::new(editor));
        let window = workspace.windows.remove(&id).unwrap();
        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::vec2(1440., 900.))
            .build_ui_state(
                |ui, window: &mut EditorWindow| window.editor.as_mut().unwrap().ui(ui),
                window,
            );
        harness.run_steps(3);
        harness
            .get(By::new().predicate(|node| node.author_id() == Some("inspector.tab.adjust")))
            .click();
        harness.run_steps(2);
        let before = harness
            .state()
            .editor
            .as_ref()
            .unwrap()
            .session
            .project()
            .clone();
        let revision = harness.state().editor.as_ref().unwrap().session.revision();
        let history = harness
            .state()
            .editor
            .as_ref()
            .unwrap()
            .session
            .history_len();
        let import =
            || By::new().predicate(|node| node.author_id() == Some("inspector.lut.import"));
        // The grade rows above the LUT push its import button below the
        // inspector's visible area at this window size; wheel-scroll it in.
        for _ in 0..16 {
            let bounds = harness.get(import()).accesskit_node().raw_bounds().unwrap();
            if bounds.y1 < 480.0 {
                break;
            }
            harness.hover_at(egui::pos2(1200.0, 300.0));
            harness.event(egui::Event::MouseWheel {
                unit: egui::MouseWheelUnit::Point,
                delta: egui::vec2(0.0, -120.0),
                phase: egui::TouchPhase::Move,
                source: egui::MouseWheelSource::Unknown,
                modifiers: egui::Modifiers::NONE,
            });
            harness.run_steps(8);
        }
        harness.get(import()).click();
        harness.run_steps(2);
        assert_eq!(
            harness
                .state_mut()
                .editor
                .as_mut()
                .unwrap()
                .take_lifecycle_action(),
            Some(EditorAction::ImportLut)
        );
        assert!(harness.get(import()).accesskit_node().is_disabled());
        harness.state_mut().picker = Some(Picker {
            future: Box::pin(std::future::ready(None)),
            action: PickerAction::ImportLut,
            waker: std::task::Waker::noop().clone(),
        });
        harness.state_mut().poll_picker();
        harness.run_steps(2);
        assert!(!harness.get(import()).accesskit_node().is_disabled());
        let editor = harness.state().editor.as_ref().unwrap();
        assert_eq!(editor.session.project(), &before);
        assert_eq!(editor.session.revision(), revision);
        assert_eq!(editor.session.history_len(), history);
    }
}
