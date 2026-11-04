//! One catalog and preference owner, extended by later frontend tickets.
use crate::{Error, Result};
use serde::{Deserialize, Serialize};
use std::io::Read;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Locale {
    #[default]
    En,
    Vi,
}
#[derive(Clone, Copy)]
pub enum Text {
    Media,
    Preview,
    Inspector,
    Timeline,
    EmptyMedia,
    EmptyPreview,
    EmptyInspector,
    EmptyTimeline,
    Import,
    Export,
    Play,
    Pause,
    Back,
    Forward,
    Undo,
    Redo,
    Theme,
    Language,
    Reset,
    Unavailable,
    Loading,
    ReadOnly,
    Project,
    Details,
    Retry,
    Offline,
}
pub fn text(locale: Locale, key: Text) -> &'static str {
    use Text::*;
    match (locale, key) {
        (Locale::En, Media) => "Media",
        (Locale::Vi, Media) => "Tư liệu",
        (Locale::En, Preview) => "Player",
        (Locale::Vi, Preview) => "Trình phát",
        (Locale::En, Inspector) => "Properties",
        (Locale::Vi, Inspector) => "Thuộc tính",
        (Locale::En, Timeline) => "Timeline",
        (Locale::Vi, Timeline) => "Dòng thời gian",
        (Locale::En, EmptyMedia) => "Your media will appear here",
        (Locale::Vi, EmptyMedia) => "Tư liệu của bạn sẽ xuất hiện ở đây",
        (Locale::En, EmptyPreview) => "Add media to the timeline to begin",
        (Locale::Vi, EmptyPreview) => "Thêm tư liệu vào dòng thời gian để bắt đầu",
        (Locale::En, EmptyInspector) => "Select a timeline item to see its properties",
        (Locale::Vi, EmptyInspector) => "Chọn một mục trên dòng thời gian để xem thuộc tính",
        (Locale::En, EmptyTimeline) => "Your story starts here",
        (Locale::Vi, EmptyTimeline) => "Câu chuyện của bạn bắt đầu ở đây",
        (Locale::En, Import) => "Import",
        (Locale::Vi, Import) => "Nhập",
        (Locale::En, Export) => "Export",
        (Locale::Vi, Export) => "Xuất",
        (Locale::En, Play) => "Play",
        (Locale::Vi, Play) => "Phát",
        (Locale::En, Pause) => "Pause",
        (Locale::Vi, Pause) => "Tạm dừng",
        (Locale::En, Back) => "Back 1 second",
        (Locale::Vi, Back) => "Lùi 1 giây",
        (Locale::En, Forward) => "Forward 1 second",
        (Locale::Vi, Forward) => "Tiến 1 giây",
        (Locale::En, Undo) => "Undo",
        (Locale::Vi, Undo) => "Hoàn tác",
        (Locale::En, Redo) => "Redo",
        (Locale::Vi, Redo) => "Làm lại",
        (Locale::En, Theme) => "Switch theme",
        (Locale::Vi, Theme) => "Đổi giao diện",
        (Locale::En, Language) => "Tiếng Việt",
        (Locale::Vi, Language) => "English",
        (Locale::En, Reset) => "Reset layout",
        (Locale::Vi, Reset) => "Đặt lại bố cục",
        (Locale::En, Unavailable) => "Preview unavailable. Pause and retry.",
        (Locale::Vi, Unavailable) => "Không thể xem trước. Hãy tạm dừng và thử lại.",
        (Locale::En, Loading) => "Preparing preview…",
        (Locale::Vi, Loading) => "Đang chuẩn bị xem trước…",
        (Locale::En, ReadOnly) => "Timeline editing arrives in the next step",
        (Locale::Vi, ReadOnly) => "Chỉnh sửa dòng thời gian sẽ có ở bước tiếp theo",
        (Locale::En, Project) => "Project",
        (Locale::Vi, Project) => "Dự án",
        (Locale::En, Details) => "Details",
        (Locale::Vi, Details) => "Chi tiết",
        (Locale::En, Retry) => "Retry preview",
        (Locale::Vi, Retry) => "Thử xem trước lại",
        (Locale::En, Offline) => "Audio output unavailable",
        (Locale::Vi, Offline) => "Đầu ra âm thanh không khả dụng",
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Preferences {
    pub locale: Locale,
    pub dark: bool,
    /// Ratios of the full content rectangle; validated when loaded.
    pub panels: [f32; 3],
}
impl Default for Preferences {
    fn default() -> Self {
        Self {
            locale: Locale::En,
            dark: true,
            panels: [0.25, 0.25, 0.4],
        }
    }
}
pub struct PreferenceStore {
    path: PathBuf,
}
impl PreferenceStore {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }
    pub fn path(&self) -> &Path {
        &self.path
    }
    pub fn load(&self) -> Result<Preferences> {
        let file = match std::fs::File::open(&self.path) {
            Ok(file) => file,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Preferences::default()),
            Err(e) => return Err(e.into()),
        };
        if file.metadata()?.len() > 4096 {
            return Err(Error::Invalid("preference size"));
        }
        let mut bytes = Vec::new();
        file.take(4097).read_to_end(&mut bytes)?;
        if bytes.len() > 4096 {
            return Err(Error::Invalid("preference size"));
        }
        let value: Preferences = serde_json::from_slice(&bytes)?;
        if value.panels.iter().any(|v| !v.is_finite())
            || !(0.15..=0.35).contains(&value.panels[0])
            || !(0.15..=0.35).contains(&value.panels[1])
            || !(0.15..=0.7).contains(&value.panels[2])
        {
            return Err(Error::Invalid("panel ratios"));
        }
        Ok(value)
    }
    pub fn save(&self, value: &Preferences) -> Result<()> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        project::atomic_write(&self.path, &serde_json::to_vec(value)?)?;
        Ok(())
    }
}
