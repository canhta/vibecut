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
    ChooseProject,
    Projects,
    NewProject,
    OpenProject,
    Save,
    SaveAs,
    Cancel,
    Close,
    SaveClose,
    LoadingProject,
    ExitingProject,
    SavingProject,
    Discovering,
    Recovery,
    Recover,
    Discard,
    ClosePrompt,
    BackProjects,
    RetryProject,
    ProjectError,
    RecoveryDiagnostic,
    TooNew,
    TooOld,
    Busy,
    Conflict,
    SaveAsNeeded,
    Media,
    Preview,
    Playhead,
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
        (Locale::En, ChooseProject) => "Choose a project file…",
        (Locale::Vi, ChooseProject) => "Chọn tệp dự án…",
        (Locale::En, Projects) => "Projects",
        (Locale::Vi, Projects) => "Dự án",
        (Locale::En, NewProject) => "New project",
        (Locale::Vi, NewProject) => "Dự án mới",
        (Locale::En, OpenProject) => "Open project…",
        (Locale::Vi, OpenProject) => "Mở dự án…",
        (Locale::En, Save) => "Save",
        (Locale::Vi, Save) => "Lưu",
        (Locale::En, SaveAs) => "Save as…",
        (Locale::Vi, SaveAs) => "Lưu thành…",
        (Locale::En, Cancel) => "Cancel",
        (Locale::Vi, Cancel) => "Hủy",
        (Locale::En, Close) => "Keep recovery copy and close",
        (Locale::Vi, Close) => "Giữ bản khôi phục và đóng",
        (Locale::En, SaveClose) => "Save and close",
        (Locale::Vi, SaveClose) => "Lưu và đóng",
        (Locale::En, LoadingProject) => "Loading project…",
        (Locale::Vi, LoadingProject) => "Đang mở dự án…",
        (Locale::En, ExitingProject) => "Exiting project…",
        (Locale::Vi, ExitingProject) => "Đang thoát dự án…",
        (Locale::En, SavingProject) => "Saving project…",
        (Locale::Vi, SavingProject) => "Đang lưu dự án…",
        (Locale::En, Discovering) => "Checking recovery copies…",
        (Locale::Vi, Discovering) => "Đang kiểm tra bản khôi phục…",
        (Locale::En, Recovery) => "Recover your work",
        (Locale::Vi, Recovery) => "Khôi phục công việc",
        (Locale::En, Recover) => "Recover a copy",
        (Locale::Vi, Recover) => "Khôi phục bản sao",
        (Locale::En, Discard) => "Discard recovery copy",
        (Locale::Vi, Discard) => "Bỏ bản khôi phục",
        (Locale::En, ClosePrompt) => "Save your project before closing?",
        (Locale::Vi, ClosePrompt) => "Lưu dự án trước khi đóng?",
        (Locale::En, BackProjects) => "Back to projects",
        (Locale::Vi, BackProjects) => "Trở về dự án",
        (Locale::En, RetryProject) => "Retry",
        (Locale::Vi, RetryProject) => "Thử lại",
        (Locale::En, ProjectError) => "Project operation failed. Your work is retained.",
        (Locale::Vi, ProjectError) => "Thao tác dự án thất bại. Công việc của bạn được giữ lại.",
        (Locale::En, RecoveryDiagnostic) => "A damaged recovery copy was skipped.",
        (Locale::Vi, RecoveryDiagnostic) => "Đã bỏ qua một bản khôi phục bị hỏng.",
        (Locale::En, TooNew) => {
            "This project needs a newer version of VibeCut. The file was not changed."
        }
        (Locale::Vi, TooNew) => "Dự án này cần phiên bản VibeCut mới hơn. Tệp chưa bị thay đổi.",
        (Locale::En, TooOld) => {
            "This project uses an older format. Export or back it up with the version that created it."
        }
        (Locale::Vi, TooOld) => {
            "Dự án dùng định dạng cũ. Hãy xuất hoặc sao lưu bằng phiên bản đã tạo dự án."
        }
        (Locale::En, Busy) => "Another editor is using this project. Retry to wait, or cancel.",
        (Locale::Vi, Busy) => "Một trình chỉnh sửa khác đang dùng dự án. Thử lại để chờ hoặc hủy.",
        (Locale::En, Conflict) => {
            "The file changed outside this session. Save a copy to a different path."
        }
        (Locale::Vi, Conflict) => {
            "Tệp đã thay đổi bên ngoài phiên này. Hãy lưu bản sao vào đường dẫn khác."
        }
        (Locale::En, SaveAsNeeded) => "Recovered and new projects need Save as.",
        (Locale::Vi, SaveAsNeeded) => "Dự án mới và bản khôi phục cần Lưu thành.",

        (Locale::En, Media) => "Media",
        (Locale::Vi, Media) => "Tư liệu",
        (Locale::En, Playhead) => "Playhead",
        (Locale::Vi, Playhead) => "Đầu phát",
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
