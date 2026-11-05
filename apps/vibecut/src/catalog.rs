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
    ExportMemory,
    ExportCapability,
    ExportOverload,
    ExportStorage,
    ExportTextFailure,

    ExportTitle,
    ExportFormat,
    ExportQuality,
    ExportMp4,
    ExportMov,
    ExportWebm,
    QualityLow,
    QualityMedium,
    QualityHigh,
    QualityVeryHigh,
    IncludeAudio,
    ExportDestination,
    ChooseDestination,
    ExportReplace,
    ExportDegraded,
    ExportBlocked,
    ExportEmpty,
    ExportProgress,
    ExportComplete,
    ExportCancelled,
    ExportFailure,
    ExportCancelling,
    ExportDone,
    SourceMissing,
    SourceChanged,
    SourceMetadata,
    SourceVideo,
    SourceAudio,
    SourceAsset,
    SourceItem,
    SourceRange,
    ExportQueued,
    ExportAdmit,
    ExportRender,
    ExportMix,
    ExportEncode,
    ExportFinalize,
    ExportPublish,
    ExportDuration,
    ExportSettingsHint,
    ExportChoosePath,

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
    AddToTimeline,
    DropMedia,
    SelectAssets,
    Selected,
    Importing,
    Processing,
    Imported,
    Failed,
    ImportComplete,
    ImportPartial,
    ImportFailed,
    ImportCancelled,
    Unfinished,
    ImportIssues,
    InsertFailed,
    DurationUnknown,
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
    Project,
    Details,
    Retry,
    Offline,
    Video,
    Audio,
    TextItem,
    Caption,
    Overlay,
    Sticker,
    Effect,
    Filter,
    Adjustment,
    MuteTrack,
    HideTrack,
    DeleteTrack,
    TrackLocked,
    Magnet,
    Split,
    Delete,
    Snap,
    Ripple,
    ZoomIn,
    ZoomOut,
    AudioGain,
    TextSize,
    TextColor,
    Compatible,
    NoSharedFields,
    MaskPoints,
    Keyframes,
    Scenes,
    SelectScenes,
    SelectScenesHint,
    SwitchScenesHint,
    Select,
    CancelSelection,
    CloseScenes,
    MainScene,
    DeleteScenesPrompt,
}
pub fn text(locale: Locale, key: Text) -> &'static str {
    use Text::*;
    match (locale, key) {
        (Locale::En, CloseScenes) => "Close",
        (Locale::Vi, CloseScenes) => "Đóng",
        (Locale::En, Scenes) => "Scenes",
        (Locale::Vi, Scenes) => "Cảnh",
        (Locale::En, SelectScenes) => "Select scenes",
        (Locale::Vi, SelectScenes) => "Chọn cảnh",
        (Locale::En, SelectScenesHint) => "Select scenes to delete",
        (Locale::Vi, SelectScenesHint) => "Chọn các cảnh để xóa",
        (Locale::En, SwitchScenesHint) => "Switch between scenes in your project",
        (Locale::Vi, SwitchScenesHint) => "Chuyển cảnh trong dự án",
        (Locale::En, Select) => "Select",
        (Locale::Vi, Select) => "Chọn",
        (Locale::En, CancelSelection) => "Cancel selection",
        (Locale::Vi, CancelSelection) => "Hủy chọn",
        (Locale::En, MainScene) => "Main",
        (Locale::Vi, MainScene) => "Chính",
        (Locale::En, DeleteScenesPrompt) => "Delete selected scenes? You can undo this action.",
        (Locale::Vi, DeleteScenesPrompt) => {
            "Xóa các cảnh đã chọn? Bạn có thể hoàn tác thao tác này."
        }

        (Locale::En, ExportMemory) => {
            "Not enough memory for export. Pause playback or reduce the project canvas, then retry."
        }
        (Locale::Vi, ExportMemory) => {
            "Không đủ bộ nhớ để xuất. Tạm dừng phát hoặc giảm kích thước khung hình rồi thử lại."
        }
        (Locale::En, ExportCapability) => {
            "The selected format or source cannot be encoded on this system. Try another format or restore the source."
        }
        (Locale::Vi, ExportCapability) => {
            "Hệ thống không mã hóa được định dạng hoặc nguồn đã chọn. Thử định dạng khác hoặc khôi phục nguồn."
        }
        (Locale::En, ExportOverload) => {
            "Audio is too loud to export safely. Reduce clip volume and retry."
        }
        (Locale::Vi, ExportOverload) => {
            "Âm thanh quá lớn để xuất an toàn. Giảm âm lượng của đoạn rồi thử lại."
        }
        (Locale::En, ExportStorage) => {
            "Cannot write the destination. Check free space and folder permissions, or choose another path."
        }
        (Locale::Vi, ExportStorage) => {
            "Không thể ghi tệp đích. Kiểm tra dung lượng trống và quyền thư mục hoặc chọn đường dẫn khác."
        }
        (Locale::En, ExportTextFailure) => {
            "Text cannot fit or render with these settings. Check caption layout and retry."
        }
        (Locale::Vi, ExportTextFailure) => {
            "Không thể hiển thị văn bản với thiết lập này. Kiểm tra bố cục phụ đề rồi thử lại."
        }

        (Locale::En, ExportTitle) => "Export project",
        (Locale::Vi, ExportTitle) => "Xuất dự án",
        (Locale::En, ExportFormat) => "Format",
        (Locale::Vi, ExportFormat) => "Định dạng",
        (Locale::En, ExportQuality) => "Quality",
        (Locale::Vi, ExportQuality) => "Chất lượng",
        (Locale::En, ExportMp4) => "MP4 · H.264 / AAC",
        (Locale::Vi, ExportMp4) => "MP4 · H.264 / AAC",
        (Locale::En, ExportMov) => "MOV · H.264 / AAC",
        (Locale::Vi, ExportMov) => "MOV · H.264 / AAC",
        (Locale::En, ExportWebm) => "WebM · VP9 / Opus",
        (Locale::Vi, ExportWebm) => "WebM · VP9 / Opus",
        (Locale::En, QualityLow) => "Low",
        (Locale::Vi, QualityLow) => "Thấp",
        (Locale::En, QualityMedium) => "Medium",
        (Locale::Vi, QualityMedium) => "Trung bình",
        (Locale::En, QualityHigh) => "High",
        (Locale::Vi, QualityHigh) => "Cao",
        (Locale::En, QualityVeryHigh) => "Very high",
        (Locale::Vi, QualityVeryHigh) => "Rất cao",
        (Locale::En, IncludeAudio) => "Include audio in export",
        (Locale::Vi, IncludeAudio) => "Xuất kèm âm thanh",
        (Locale::En, ExportDestination) => "Destination",
        (Locale::Vi, ExportDestination) => "Tệp đích",
        (Locale::En, ChooseDestination) => "Choose destination…",
        (Locale::Vi, ChooseDestination) => "Chọn tệp đích…",
        (Locale::En, ExportReplace) => "Replace the existing destination file",
        (Locale::Vi, ExportReplace) => "Thay thế tệp đích hiện có",
        (Locale::En, ExportDegraded) => {
            "I accept black frames / silence for the listed source ranges"
        }
        (Locale::Vi, ExportDegraded) => {
            "Tôi chấp nhận khung hình đen / im lặng cho các đoạn nguồn đã liệt kê"
        }
        (Locale::En, ExportBlocked) => {
            "Export blocked: restore these sources or explicitly accept degraded output."
        }
        (Locale::Vi, ExportBlocked) => {
            "Không thể xuất: khôi phục các nguồn này hoặc chấp nhận xuất suy giảm."
        }
        (Locale::En, ExportEmpty) => "The timeline is empty. Add media before exporting.",
        (Locale::Vi, ExportEmpty) => "Dòng thời gian trống. Thêm tư liệu trước khi xuất.",
        (Locale::En, ExportProgress) => "Exporting…",
        (Locale::Vi, ExportProgress) => "Đang xuất…",
        (Locale::En, ExportComplete) => "Export complete",
        (Locale::Vi, ExportComplete) => "Xuất hoàn tất",
        (Locale::En, ExportCancelled) => "Export cancelled. Your previous file is preserved.",
        (Locale::Vi, ExportCancelled) => "Đã hủy xuất. Tệp cũ của bạn được giữ nguyên.",
        (Locale::En, ExportFailure) => {
            "Export failed. Your previous file is preserved. Choose another destination or retry."
        }
        (Locale::Vi, ExportFailure) => {
            "Xuất thất bại. Tệp cũ được giữ nguyên. Chọn tệp đích khác hoặc thử lại."
        }
        (Locale::En, ExportCancelling) => "Cancelling…",
        (Locale::Vi, ExportCancelling) => "Đang hủy…",
        (Locale::En, ExportDone) => "Done",
        (Locale::Vi, ExportDone) => "Xong",
        (Locale::En, SourceMissing) => "Missing source",
        (Locale::Vi, SourceMissing) => "Thiếu nguồn",
        (Locale::En, SourceChanged) => "Source changed",
        (Locale::Vi, SourceChanged) => "Nguồn đã thay đổi",
        (Locale::En, SourceMetadata) => "Unsupported source metadata",
        (Locale::Vi, SourceMetadata) => "Siêu dữ liệu nguồn không được hỗ trợ",
        (Locale::En, SourceVideo) => "Video unavailable",
        (Locale::Vi, SourceVideo) => "Video không khả dụng",
        (Locale::En, SourceAudio) => "Audio unavailable",
        (Locale::Vi, SourceAudio) => "Âm thanh không khả dụng",
        (Locale::En, SourceAsset) => "Asset",
        (Locale::Vi, SourceAsset) => "Tư liệu",
        (Locale::En, SourceItem) => "Item",
        (Locale::Vi, SourceItem) => "Mục",
        (Locale::En, SourceRange) => "Source range (seconds)",
        (Locale::Vi, SourceRange) => "Đoạn nguồn (giây)",
        (Locale::En, ExportQueued) => "Queued",
        (Locale::Vi, ExportQueued) => "Đang chờ",
        (Locale::En, ExportAdmit) => "Checking sources",
        (Locale::Vi, ExportAdmit) => "Đang kiểm tra nguồn",
        (Locale::En, ExportRender) => "Rendering video",
        (Locale::Vi, ExportRender) => "Đang dựng video",
        (Locale::En, ExportMix) => "Mixing audio",
        (Locale::Vi, ExportMix) => "Đang trộn âm thanh",
        (Locale::En, ExportEncode) => "Encoding",
        (Locale::Vi, ExportEncode) => "Đang mã hóa",
        (Locale::En, ExportFinalize) => "Finalizing",
        (Locale::Vi, ExportFinalize) => "Đang hoàn tất",
        (Locale::En, ExportPublish) => "Publishing file",
        (Locale::Vi, ExportPublish) => "Đang ghi tệp",
        (Locale::En, ExportDuration) => "Duration",
        (Locale::Vi, ExportDuration) => "Thời lượng",
        (Locale::En, ExportSettingsHint) => "Canvas size and frame rate follow the project.",
        (Locale::Vi, ExportSettingsHint) => {
            "Kích thước khung hình và tốc độ khung hình theo dự án."
        }
        (Locale::En, ExportChoosePath) => "Choose a destination before exporting.",
        (Locale::Vi, ExportChoosePath) => "Chọn tệp đích trước khi xuất.",

        (Locale::En, AudioGain) => "Volume (dB)",
        (Locale::Vi, AudioGain) => "Âm lượng (dB)",
        (Locale::En, TextSize) => "Font size (px)",
        (Locale::Vi, TextSize) => "Cỡ chữ (px)",
        (Locale::En, TextColor) => "Text colour (RGBA)",
        (Locale::Vi, TextColor) => "Màu chữ (RGBA)",
        (Locale::En, Compatible) => "Compatible items",
        (Locale::Vi, Compatible) => "Mục tương thích",
        (Locale::En, NoSharedFields) => "No editable shared fields for this selection.",
        (Locale::Vi, NoSharedFields) => "Không có thuộc tính chung có thể sửa cho lựa chọn này.",
        (Locale::En, MaskPoints) => "Mask points selected",
        (Locale::Vi, MaskPoints) => "Đã chọn điểm mặt nạ",
        (Locale::En, Keyframes) => "Keyframes selected",
        (Locale::Vi, Keyframes) => "Đã chọn khung hình chính",

        (Locale::En, Sticker) => "Sticker",
        (Locale::Vi, Sticker) => "Nhãn dán",
        (Locale::En, Effect) => "Effect",
        (Locale::Vi, Effect) => "Hiệu ứng",
        (Locale::En, Filter) => "Filter",
        (Locale::Vi, Filter) => "Bộ lọc",
        (Locale::En, Adjustment) => "Adjustment",
        (Locale::Vi, Adjustment) => "Điều chỉnh",
        (Locale::En, MuteTrack) => "Mute track",
        (Locale::Vi, MuteTrack) => "Tắt tiếng rãnh",
        (Locale::En, HideTrack) => "Hide track",
        (Locale::Vi, HideTrack) => "Ẩn rãnh",
        (Locale::En, TrackLocked) => "Track is locked",
        (Locale::Vi, TrackLocked) => "Rãnh đang khóa",
        (Locale::En, DeleteTrack) => "Delete track",
        (Locale::Vi, DeleteTrack) => "Xóa rãnh",
        (Locale::En, Magnet) => "Main track magnet",
        (Locale::Vi, Magnet) => "Nam châm rãnh chính",
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
        (Locale::En, AddToTimeline) => "Add to timeline",
        (Locale::Vi, AddToTimeline) => "Thêm vào dòng thời gian",
        (Locale::En, DropMedia) => "Drop media here to import",
        (Locale::Vi, DropMedia) => "Thả tư liệu vào đây để nhập",
        (Locale::En, SelectAssets) => "Select media to add",
        (Locale::Vi, SelectAssets) => "Chọn tư liệu để thêm",
        (Locale::En, Selected) => "Selected",
        (Locale::Vi, Selected) => "Đã chọn",
        (Locale::En, Importing) => "Importing media…",
        (Locale::Vi, Importing) => "Đang nhập tư liệu…",
        (Locale::En, Processing) => "Processing",
        (Locale::Vi, Processing) => "Đang xử lý",
        (Locale::En, Imported) => "Imported",
        (Locale::Vi, Imported) => "Đã nhập",
        (Locale::En, Failed) => "Failed",
        (Locale::Vi, Failed) => "Thất bại",
        (Locale::En, ImportComplete) => "Import complete",
        (Locale::Vi, ImportComplete) => "Nhập hoàn tất",
        (Locale::En, ImportPartial) => "Some files were not imported",
        (Locale::Vi, ImportPartial) => "Một số tệp chưa được nhập",
        (Locale::En, ImportFailed) => "Import failed",
        (Locale::Vi, ImportFailed) => "Nhập thất bại",
        (Locale::En, ImportCancelled) => "Import cancelled",
        (Locale::Vi, ImportCancelled) => "Đã hủy nhập",
        (Locale::En, Unfinished) => "Unfinished",
        (Locale::Vi, Unfinished) => "Chưa xử lý",
        (Locale::En, ImportIssues) => "Import issues",
        (Locale::Vi, ImportIssues) => "Lỗi khi nhập",
        (Locale::En, InsertFailed) => "Could not add media to the timeline",
        (Locale::Vi, InsertFailed) => "Không thể thêm tư liệu vào dòng thời gian",
        (Locale::En, DurationUnknown) => "duration unknown",
        (Locale::Vi, DurationUnknown) => "chưa rõ thời lượng",
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
        (Locale::En, Project) => "Project",
        (Locale::Vi, Project) => "Dự án",
        (Locale::En, Details) => "Details",
        (Locale::Vi, Details) => "Chi tiết",
        (Locale::En, Retry) => "Retry preview",
        (Locale::Vi, Retry) => "Thử xem trước lại",
        (Locale::En, Offline) => "Audio output unavailable",
        (Locale::Vi, Offline) => "Đầu ra âm thanh không khả dụng",
        (Locale::En, Video) => "Video",
        (Locale::Vi, Video) => "Video",
        (Locale::En, Audio) => "Audio",
        (Locale::Vi, Audio) => "Âm thanh",
        (Locale::En, TextItem) => "Text",
        (Locale::Vi, TextItem) => "Văn bản",
        (Locale::En, Caption) => "Captions",
        (Locale::Vi, Caption) => "Phụ đề",
        (Locale::En, Overlay) => "Overlay",
        (Locale::Vi, Overlay) => "Lớp phủ",
        (Locale::En, Split) => "Split",
        (Locale::Vi, Split) => "Tách",
        (Locale::En, Delete) => "Delete",
        (Locale::Vi, Delete) => "Xóa",
        (Locale::En, Snap) => "Snap",
        (Locale::Vi, Snap) => "Bám dính",
        (Locale::En, Ripple) => "Ripple",
        (Locale::Vi, Ripple) => "Cuốn theo",
        (Locale::En, ZoomIn) => "Zoom +",
        (Locale::Vi, ZoomIn) => "Phóng to",
        (Locale::En, ZoomOut) => "Zoom −",
        (Locale::Vi, ZoomOut) => "Thu nhỏ",
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Preferences {
    #[serde(default)]
    pub export: crate::export::ExportOptions,
    pub locale: Locale,
    pub dark: bool,
    /// Ratios of the full content rectangle; validated when loaded.
    pub panels: [f32; 3],
}
impl Default for Preferences {
    fn default() -> Self {
        Self {
            export: crate::export::ExportOptions::default(),
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

/// Export measurements stay in the catalog alongside their unit and locale.
pub fn export_duration(locale: Locale, seconds: f64) -> String {
    let number = export_decimal(locale, seconds, 2);
    match locale {
        Locale::En => format!("{}: {number} s", text(locale, Text::ExportDuration)),
        Locale::Vi => format!("{}: {number} giây", text(locale, Text::ExportDuration)),
    }
}
pub fn export_range(locale: Locale, start: f64, end: f64) -> String {
    format!(
        "{}: {}–{}",
        text(locale, Text::SourceRange),
        export_decimal(locale, start, 3),
        export_decimal(locale, end, 3)
    )
}
fn export_decimal(locale: Locale, value: f64, precision: usize) -> String {
    let number = format!("{value:.precision$}");
    match locale {
        Locale::En => number,
        Locale::Vi => number.replace('.', ","),
    }
}
