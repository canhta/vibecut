//! One catalog and preference owner, extended by later frontend tickets.
//!
//! The [`Text`] table is the stable key set. Both locales must fill every key,
//! so a missing Vietnamese row is a compile error; `cargo xtask i18n` also
//! rejects empty rows, duplicate keys and keys no frontend code uses.
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
impl Locale {
    pub const fn code(self) -> &'static str {
        match self {
            Self::En => "en",
            Self::Vi => "vi",
        }
    }
    /// The language's own name, for a language picker.
    pub const fn name(self) -> &'static str {
        match self {
            Self::En => "English",
            Self::Vi => "Tiếng Việt",
        }
    }
    /// Match a BCP 47 or POSIX tag (`vi`, `vi-VN`, `vi_VN.UTF-8`) to an
    /// offered locale. Anything not Vietnamese falls back to English.
    pub fn from_tag(tag: &str) -> Self {
        match tag
            .trim()
            .split(['-', '_', '.', '@'])
            .next()
            .unwrap_or_default()
        {
            language if language.eq_ignore_ascii_case("vi") => Self::Vi,
            _ => Self::En,
        }
    }
    /// The operating system's preferred language, English when unavailable.
    pub fn from_system() -> Self {
        sys_locale::get_locale()
            .as_deref()
            .map_or(Self::En, Self::from_tag)
    }
}

macro_rules! catalog {
    ($(
        $variant:ident {
            key: $key:literal,
            en: $en:literal,
            vi: $vi:literal $(,)?
        }
    ),* $(,)?) => {
        /// Stable message keys. The enum keeps every call site compiler-checked.
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
        pub enum Text {
            $($variant),*
        }
        impl Text {
            pub const ALL: &'static [Self] = &[$(Self::$variant),*];
            pub const fn key(self) -> &'static str {
                match self { $(Self::$variant => $key),* }
            }
            pub const fn text(self, locale: Locale) -> &'static str {
                match locale {
                    Locale::En => match self { $(Self::$variant => $en),* },
                    Locale::Vi => match self { $(Self::$variant => $vi),* },
                }
            }
        }
    };
}

#[rustfmt::skip]
catalog! {
    // Shell and lifecycle.
    AppName { key: "app_name", en: "VibeCut", vi: "VibeCut" },
    ChooseProject { key: "choose_project", en: "Choose a project file…", vi: "Chọn tệp dự án…" },
    Projects { key: "projects", en: "Projects", vi: "Dự án" },
    NewProject { key: "new_project", en: "New project", vi: "Dự án mới" },
    UntitledProject { key: "untitled_project", en: "Untitled Project", vi: "Dự án chưa đặt tên" },
    OpenProject { key: "open_project", en: "Open project…", vi: "Mở dự án…" },
    Save { key: "save", en: "Save", vi: "Lưu" },
    SaveAs { key: "save_as", en: "Save as…", vi: "Lưu thành…" },
    Cancel { key: "cancel", en: "Cancel", vi: "Hủy" },
    Close { key: "close_recovery", en: "Keep recovery copy and close", vi: "Giữ bản khôi phục và đóng" },
    SaveClose { key: "save_close", en: "Save and close", vi: "Lưu và đóng" },
    LoadingProject { key: "loading_project", en: "Loading project…", vi: "Đang mở dự án…" },
    ExitingProject { key: "exiting_project", en: "Exiting project…", vi: "Đang thoát dự án…" },
    SavingProject { key: "saving_project", en: "Saving project…", vi: "Đang lưu dự án…" },
    Discovering { key: "discovering", en: "Checking recovery copies…", vi: "Đang kiểm tra bản khôi phục…" },
    Recovery { key: "recovery", en: "Recover your work", vi: "Khôi phục công việc" },
    Recover { key: "recover", en: "Recover a copy", vi: "Khôi phục bản sao" },
    Discard { key: "discard", en: "Discard recovery copy", vi: "Bỏ bản khôi phục" },
    ClosePrompt { key: "close_prompt", en: "Save your project before closing?", vi: "Lưu dự án trước khi đóng?" },
    BackProjects { key: "back_projects", en: "Back to projects", vi: "Trở về dự án" },
    RetryProject { key: "retry_project", en: "Retry", vi: "Thử lại" },
    ProjectError { key: "project_error", en: "Project operation failed. Your work is retained.", vi: "Thao tác dự án thất bại. Công việc của bạn được giữ lại." },
    RecoveryDiagnostic { key: "recovery_diagnostic", en: "A damaged recovery copy was skipped.", vi: "Đã bỏ qua một bản khôi phục bị hỏng." },
    TooNew { key: "too_new", en: "This project needs a newer version of VibeCut. The file was not changed.", vi: "Dự án này cần phiên bản VibeCut mới hơn. Tệp chưa bị thay đổi." },
    TooOld { key: "too_old", en: "This project uses an older format. Export or back it up with the version that created it.", vi: "Dự án dùng định dạng cũ. Hãy xuất hoặc sao lưu bằng phiên bản đã tạo dự án." },
    Busy { key: "busy", en: "Another editor is using this project. Retry to wait, or cancel.", vi: "Một trình chỉnh sửa khác đang dùng dự án. Thử lại để chờ hoặc hủy." },
    Conflict { key: "conflict", en: "The file changed outside this session. Save a copy to a different path.", vi: "Tệp đã thay đổi bên ngoài phiên này. Hãy lưu bản sao vào đường dẫn khác." },
    SaveAsNeeded { key: "save_as_needed", en: "Recovered and new projects need Save as.", vi: "Dự án mới và bản khôi phục cần Lưu thành." },
    DefaultProjectFileName { key: "default_project_file_name", en: "project.vibecut", vi: "project.vibecut" },

    // Header and appearance.
    Theme { key: "theme", en: "Switch theme", vi: "Đổi giao diện" },
    Language { key: "language", en: "Tiếng Việt", vi: "English" },
    Reset { key: "reset_layout", en: "Reset layout", vi: "Đặt lại bố cục" },
    Export { key: "export", en: "Export", vi: "Xuất" },

    // Panels and empty states.
    Media { key: "media", en: "Media", vi: "Tư liệu" },
    Preview { key: "preview", en: "Player", vi: "Trình phát" },
    Timeline { key: "timeline", en: "Timeline", vi: "Dòng thời gian" },
    Project { key: "project", en: "Project", vi: "Dự án" },
    Details { key: "details", en: "Details", vi: "Chi tiết" },
    EmptyMedia { key: "empty_media", en: "Your media will appear here", vi: "Tư liệu của bạn sẽ xuất hiện ở đây" },
    EmptyPreview { key: "empty_preview", en: "Add media to the timeline to begin", vi: "Thêm tư liệu vào dòng thời gian để bắt đầu" },
    EmptyInspector { key: "empty_inspector", en: "Select a timeline item to see its properties", vi: "Chọn một mục trên dòng thời gian để xem thuộc tính" },
    EmptyTimeline { key: "empty_timeline", en: "Your story starts here", vi: "Câu chuyện của bạn bắt đầu ở đây" },

    // Import.
    Import { key: "import", en: "Import", vi: "Nhập" },
    AddToTimeline { key: "add_to_timeline", en: "Add to timeline", vi: "Thêm vào dòng thời gian" },
    DropMedia { key: "drop_media", en: "Drop media here to import", vi: "Thả tư liệu vào đây để nhập" },
    SelectAssets { key: "select_assets", en: "Select media to add", vi: "Chọn tư liệu để thêm" },
    Importing { key: "importing", en: "Importing media…", vi: "Đang nhập tư liệu…" },
    ImportBusy { key: "import_busy", en: "An import is already running", vi: "Đang có một lần nhập khác" },
    Processing { key: "processing", en: "Processing", vi: "Đang xử lý" },
    ProcessingPercent { key: "processing_percent", en: "{status} {percent}%", vi: "{status} {percent}%" },
    ImportComplete { key: "import_complete", en: "Import complete", vi: "Nhập hoàn tất" },
    ImportPartial { key: "import_partial", en: "Some files were not imported", vi: "Một số tệp chưa được nhập" },
    ImportFailed { key: "import_failed", en: "Import failed", vi: "Nhập thất bại" },
    ImportCancelled { key: "import_cancelled", en: "Import cancelled", vi: "Đã hủy nhập" },
    ImportIssues { key: "import_issues", en: "Import issues", vi: "Lỗi khi nhập" },
    InsertFailed { key: "insert_failed", en: "Could not add media to the timeline", vi: "Không thể thêm tư liệu vào dòng thời gian" },
    ImportSummary { key: "import_summary", en: "Imported: {imported} · Failed: {failed} · Unfinished: {unfinished}", vi: "Đã nhập: {imported} · Thất bại: {failed} · Chưa xử lý: {unfinished}" },
    ImportProgress { key: "import_progress", en: "{done} / {total}", vi: "{done} / {total}" },
    FailureDetail { key: "failure_detail", en: "{name}: {reason}", vi: "{name}: {reason}" },
    DurationUnknown { key: "duration_unknown", en: "duration unknown", vi: "chưa rõ thời lượng" },
    SelectedCountOne { key: "selected_count_one", en: "{n} item selected", vi: "{n} mục đã chọn" },
    SelectedCountOther { key: "selected_count_other", en: "{n} items selected", vi: "{n} mục đã chọn" },

    // Scenes.
    CloseScenes { key: "close_scenes", en: "Close", vi: "Đóng" },
    Scenes { key: "scenes", en: "Scenes", vi: "Cảnh" },
    SelectScenes { key: "select_scenes", en: "Select scenes", vi: "Chọn cảnh" },
    SelectScenesHint { key: "select_scenes_hint", en: "Select scenes to delete", vi: "Chọn các cảnh để xóa" },
    SwitchScenesHint { key: "switch_scenes_hint", en: "Switch between scenes in your project", vi: "Chuyển cảnh trong dự án" },
    Select { key: "select", en: "Select", vi: "Chọn" },
    CancelSelection { key: "cancel_selection", en: "Cancel selection", vi: "Hủy chọn" },
    MainScene { key: "main_scene", en: "Main", vi: "Chính" },
    DeleteScenesPrompt { key: "delete_scenes_prompt", en: "Delete selected scenes? You can undo this action.", vi: "Xóa các cảnh đã chọn? Bạn có thể hoàn tác thao tác này." },

    // Transport.
    Play { key: "play", en: "Play", vi: "Phát" },
    Pause { key: "pause", en: "Pause", vi: "Tạm dừng" },
    Back { key: "back", en: "Back 1 second", vi: "Lùi 1 giây" },
    Forward { key: "forward", en: "Forward 1 second", vi: "Tiến 1 giây" },
    Unavailable { key: "unavailable", en: "Preview unavailable. Pause and retry.", vi: "Không thể xem trước. Hãy tạm dừng và thử lại." },
    Loading { key: "loading", en: "Preparing preview…", vi: "Đang chuẩn bị xem trước…" },
    Retry { key: "retry", en: "Retry preview", vi: "Thử xem trước lại" },
    Offline { key: "offline", en: "Audio output unavailable", vi: "Đầu ra âm thanh không khả dụng" },
    TimePosition { key: "time_position", en: "{current} / {duration}", vi: "{current} / {duration}" },

    // Inspector.
    AudioGain { key: "audio_gain", en: "Volume (dB)", vi: "Âm lượng (dB)" },
    TextSize { key: "text_size", en: "Font size (px)", vi: "Cỡ chữ (px)" },
    TextColor { key: "text_color", en: "Text colour (RGBA)", vi: "Màu chữ (RGBA)" },
    CompatibleCount { key: "compatible_count", en: "Compatible: {compatible} / {total}", vi: "Tương thích: {compatible} / {total}" },
    NoSharedFields { key: "no_shared_fields", en: "No editable shared fields for this selection.", vi: "Không có thuộc tính chung có thể sửa cho lựa chọn này." },
    MaskPoints { key: "mask_points", en: "Mask points selected", vi: "Đã chọn điểm mặt nạ" },
    Keyframes { key: "keyframes", en: "Keyframes selected", vi: "Đã chọn khung hình chính" },
    EmptyValue { key: "empty_value", en: "—", vi: "—" },
    CanvasSize { key: "canvas_size", en: "{width} × {height}", vi: "{width} × {height}" },
    FrameRate { key: "frame_rate", en: "{num} / {den} fps", vi: "{num} / {den} fps" },
    SampleRate { key: "sample_rate", en: "{rate} Hz", vi: "{rate} Hz" },
    CreatedDate { key: "created_date", en: "Created {date}", vi: "Đã tạo {date}" },
    ModifiedDate { key: "modified_date", en: "Modified {date}", vi: "Đã sửa {date}" },

    // Tracks and items.
    Video { key: "video", en: "Video", vi: "Video" },
    Audio { key: "audio", en: "Audio", vi: "Âm thanh" },
    TextItem { key: "text_item", en: "Text", vi: "Văn bản" },
    Caption { key: "caption", en: "Captions", vi: "Phụ đề" },
    Overlay { key: "overlay", en: "Overlay", vi: "Lớp phủ" },
    VideoTrackName { key: "video_track_name", en: "Video {n}", vi: "Video {n}" },
    AudioTrackName { key: "audio_track_name", en: "Audio {n}", vi: "Âm thanh {n}" },
    MuteTrack { key: "mute_track", en: "Mute track", vi: "Tắt tiếng rãnh" },
    HideTrack { key: "hide_track", en: "Hide track", vi: "Ẩn rãnh" },
    DeleteTrack { key: "delete_track", en: "Delete track", vi: "Xóa rãnh" },
    TrackLocked { key: "track_locked", en: "Track is locked", vi: "Rãnh đang khóa" },
    Magnet { key: "magnet", en: "Main track magnet", vi: "Nam châm rãnh chính" },
    TrackToggle { key: "track_toggle", en: "{action}: {track}", vi: "{action}: {track}" },

    // Timeline actions.
    Undo { key: "undo", en: "Undo", vi: "Hoàn tác" },
    Redo { key: "redo", en: "Redo", vi: "Làm lại" },
    Split { key: "split", en: "Split", vi: "Tách" },
    Delete { key: "delete", en: "Delete", vi: "Xóa" },
    Snap { key: "snap", en: "Snap", vi: "Bám dính" },
    Ripple { key: "ripple", en: "Ripple", vi: "Cuốn theo" },
    ZoomIn { key: "zoom_in", en: "Zoom +", vi: "Phóng to" },
    ZoomOut { key: "zoom_out", en: "Zoom −", vi: "Thu nhỏ" },
    ActionShortcut { key: "action_shortcut", en: "{action} ({shortcut})", vi: "{action} ({shortcut})" },

    // Shortcut notation. Key names stay literal; the catalog owns their display.
    ShortcutUndo { key: "shortcut_undo", en: "Cmd/Ctrl+Z", vi: "Cmd/Ctrl+Z" },
    ShortcutRedo { key: "shortcut_redo", en: "Shift+Cmd/Ctrl+Z", vi: "Shift+Cmd/Ctrl+Z" },
    ShortcutSplit { key: "shortcut_split", en: "Cmd/Ctrl+B", vi: "Cmd/Ctrl+B" },
    ShortcutDelete { key: "shortcut_delete", en: "Delete", vi: "Delete" },
    ShortcutSnap { key: "shortcut_snap", en: "N", vi: "N" },
    ShortcutRippleDelete { key: "shortcut_ripple_delete", en: "Shift+Delete", vi: "Shift+Delete" },
    ShortcutBack { key: "shortcut_back", en: "J", vi: "J" },
    ShortcutPause { key: "shortcut_pause", en: "K", vi: "K" },
    ShortcutPlay { key: "shortcut_play", en: "Space", vi: "Space" },
    ShortcutForward { key: "shortcut_forward", en: "L", vi: "L" },

    // Export.
    ExportMemory { key: "export_memory", en: "Not enough memory for export. Pause playback or reduce the project canvas, then retry.", vi: "Không đủ bộ nhớ để xuất. Tạm dừng phát hoặc giảm kích thước khung hình rồi thử lại." },
    ExportCapability { key: "export_capability", en: "The selected format or source cannot be encoded on this system. Try another format or restore the source.", vi: "Hệ thống không mã hóa được định dạng hoặc nguồn đã chọn. Thử định dạng khác hoặc khôi phục nguồn." },
    ExportOverload { key: "export_overload", en: "Audio is too loud to export safely. Reduce clip volume and retry.", vi: "Âm thanh quá lớn để xuất an toàn. Giảm âm lượng của đoạn rồi thử lại." },
    ExportStorage { key: "export_storage", en: "Cannot write the destination. Check free space and folder permissions, or choose another path.", vi: "Không thể ghi tệp đích. Kiểm tra dung lượng trống và quyền thư mục hoặc chọn đường dẫn khác." },
    ExportTextFailure { key: "export_text_failure", en: "Text cannot fit or render with these settings. Check caption layout and retry.", vi: "Không thể hiển thị văn bản với thiết lập này. Kiểm tra bố cục phụ đề rồi thử lại." },
    ExportTitle { key: "export_title", en: "Export project", vi: "Xuất dự án" },
    ExportFormat { key: "export_format", en: "Format", vi: "Định dạng" },
    ExportQuality { key: "export_quality", en: "Quality", vi: "Chất lượng" },
    ExportMp4 { key: "export_mp4", en: "MP4 · H.264 / AAC", vi: "MP4 · H.264 / AAC" },
    ExportMov { key: "export_mov", en: "MOV · H.264 / AAC", vi: "MOV · H.264 / AAC" },
    ExportWebm { key: "export_webm", en: "WebM · VP9 / Opus", vi: "WebM · VP9 / Opus" },
    QualityLow { key: "quality_low", en: "Low", vi: "Thấp" },
    QualityMedium { key: "quality_medium", en: "Medium", vi: "Trung bình" },
    QualityHigh { key: "quality_high", en: "High", vi: "Cao" },
    QualityVeryHigh { key: "quality_very_high", en: "Very high", vi: "Rất cao" },
    IncludeAudio { key: "include_audio", en: "Include audio in export", vi: "Xuất kèm âm thanh" },
    ExportDestination { key: "export_destination", en: "Destination", vi: "Tệp đích" },
    ChooseDestination { key: "choose_destination", en: "Choose destination…", vi: "Chọn tệp đích…" },
    ExportReplace { key: "export_replace", en: "Replace the existing destination file", vi: "Thay thế tệp đích hiện có" },
    ExportDegraded { key: "export_degraded", en: "I accept black frames / silence for the listed source ranges", vi: "Tôi chấp nhận khung hình đen / im lặng cho các đoạn nguồn đã liệt kê" },
    ExportBlocked { key: "export_blocked", en: "Export blocked: restore these sources or explicitly accept degraded output.", vi: "Không thể xuất: khôi phục các nguồn này hoặc chấp nhận xuất suy giảm." },
    ExportEmpty { key: "export_empty", en: "The timeline is empty. Add media before exporting.", vi: "Dòng thời gian trống. Thêm tư liệu trước khi xuất." },
    ExportProgress { key: "export_progress", en: "Exporting…", vi: "Đang xuất…" },
    ExportComplete { key: "export_complete", en: "Export complete", vi: "Xuất hoàn tất" },
    ExportCancelled { key: "export_cancelled", en: "Export cancelled. Your previous file is preserved.", vi: "Đã hủy xuất. Tệp cũ của bạn được giữ nguyên." },
    ExportFailure { key: "export_failure", en: "Export failed. Your previous file is preserved. Choose another destination or retry.", vi: "Xuất thất bại. Tệp cũ được giữ nguyên. Chọn tệp đích khác hoặc thử lại." },
    ExportCancelling { key: "export_cancelling", en: "Cancelling…", vi: "Đang hủy…" },
    ExportDone { key: "export_done", en: "Done", vi: "Xong" },
    SourceMissing { key: "source_missing", en: "Missing source", vi: "Thiếu nguồn" },
    SourceChanged { key: "source_changed", en: "Source changed", vi: "Nguồn đã thay đổi" },
    SourceMetadata { key: "source_metadata", en: "Unsupported source metadata", vi: "Siêu dữ liệu nguồn không được hỗ trợ" },
    SourceVideo { key: "source_video", en: "Video unavailable", vi: "Video không khả dụng" },
    SourceAudio { key: "source_audio", en: "Audio unavailable", vi: "Âm thanh không khả dụng" },
    SourceAsset { key: "source_asset", en: "Asset", vi: "Tư liệu" },
    SourceItem { key: "source_item", en: "Item", vi: "Mục" },
    SourceRange { key: "source_range", en: "Source range (seconds)", vi: "Đoạn nguồn (giây)" },
    ExportQueued { key: "export_queued", en: "Queued", vi: "Đang chờ" },
    ExportAdmit { key: "export_admit", en: "Checking sources", vi: "Đang kiểm tra nguồn" },
    ExportRender { key: "export_render", en: "Rendering video", vi: "Đang dựng video" },
    ExportMix { key: "export_mix", en: "Mixing audio", vi: "Đang trộn âm thanh" },
    ExportEncode { key: "export_encode", en: "Encoding", vi: "Đang mã hóa" },
    ExportFinalize { key: "export_finalize", en: "Finalizing", vi: "Đang hoàn tất" },
    ExportPublish { key: "export_publish", en: "Publishing file", vi: "Đang ghi tệp" },
    ExportDuration { key: "export_duration", en: "Duration", vi: "Thời lượng" },
    ExportSettingsHint { key: "export_settings_hint", en: "Canvas size and frame rate follow the project.", vi: "Kích thước khung hình và tốc độ khung hình theo dự án." },
    ExportChoosePath { key: "export_choose_path", en: "Choose a destination before exporting.", vi: "Chọn tệp đích trước khi xuất." },

    // Asset rows.
    AssetDimensions { key: "asset_dimensions", en: "{width}×{height}", vi: "{width}×{height}" },
    AssetFps { key: "asset_fps", en: "{rate} fps", vi: "{rate} fps" },
    AssetAudio { key: "asset_audio", en: "{rate} Hz · {channels} ch", vi: "{rate} Hz · {channels} kênh" },
}

/// The message for `key` in `locale`.
pub const fn text(locale: Locale, key: Text) -> &'static str {
    key.text(locale)
}

/// Fill `{name}` placeholders in a catalog message in one pass, so text a
/// replacement inserts is never read as another placeholder.
pub fn format(locale: Locale, key: Text, args: &[(&str, &str)]) -> String {
    let mut out = String::new();
    let mut rest = key.text(locale);
    while let Some(open) = rest.find('{') {
        out.push_str(rest.get(..open).unwrap_or(""));
        let Some(relative_close) = rest.get(open..).and_then(|tail| tail.find('}')) else {
            out.push_str(rest.get(open..).unwrap_or(""));
            return out;
        };
        let close = open + relative_close;
        let name = rest.get(open + 1..close).unwrap_or("");
        match args.iter().find(|(argument, _)| *argument == name) {
            Some((_, value)) => out.push_str(value),
            None => out.push_str(rest.get(open..close + 1).unwrap_or("")),
        }
        rest = rest.get(close + 1..).unwrap_or("");
    }
    out.push_str(rest);
    out
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Plural {
    One,
    Other,
}

/// English distinguishes one from other; Vietnamese CLDR has one `other`
/// category, so it always takes the `other` row.
pub const fn plural(locale: Locale, count: u64) -> Plural {
    match locale {
        Locale::En if count == 1 => Plural::One,
        _ => Plural::Other,
    }
}

/// A count message with the locale's plural row and group-formatted number.
pub fn format_count(locale: Locale, count: u64, one: Text, other: Text) -> String {
    let key = match plural(locale, count) {
        Plural::One => one,
        Plural::Other => other,
    };
    format(locale, key, &[("n", &number(locale, count))])
}

fn separators(locale: Locale) -> (char, char) {
    match locale {
        Locale::En => (',', '.'),
        Locale::Vi => ('.', ','),
    }
}
fn group(digits: &str, separator: char) -> String {
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    let len = digits.len();
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (len - index).is_multiple_of(3) {
            out.push(separator);
        }
        out.push(digit);
    }
    out
}

/// An unsigned integer with the locale's grouping separator.
pub fn number(locale: Locale, value: u64) -> String {
    group(&value.to_string(), separators(locale).0)
}

/// A signed integer with the locale's grouping separator.
pub fn signed(locale: Locale, value: i64) -> String {
    let (sign, magnitude) = if value < 0 {
        ("-", value.unsigned_abs())
    } else {
        ("", value as u64)
    };
    std::format!("{sign}{}", number(locale, magnitude))
}

/// A finite number with `decimals` digits after the locale's decimal mark.
pub fn decimal(locale: Locale, value: f64, decimals: usize) -> String {
    if !value.is_finite() {
        return value.to_string();
    }
    let (grouping, decimal) = separators(locale);
    let text = std::format!("{value:.decimals$}");
    let (sign, text) = text
        .strip_prefix('-')
        .map_or(("", text.as_str()), |rest| ("-", rest));
    let (integer, fraction) = text.split_once('.').unwrap_or((text, ""));
    let mut out = String::with_capacity(text.len() + 4);
    out.push_str(sign);
    out.push_str(&group(integer, grouping));
    if !fraction.is_empty() {
        out.push(decimal);
        out.push_str(fraction);
    }
    out
}

/// A short date in the locale's convention, from Unix milliseconds. The app
/// stores timestamps in UTC; this is display only, never edit or timing math.
pub fn date(locale: Locale, unix_ms: u64) -> String {
    let days = i64::try_from(unix_ms / 86_400_000).unwrap_or(i64::MAX);
    let (year, month, day) = civil_from_days(days);
    match locale {
        Locale::En => {
            let names = [
                "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
            ];
            let name = names
                .get(month.saturating_sub(1) as usize)
                .copied()
                .unwrap_or("");
            std::format!("{name} {day}, {year}")
        }
        Locale::Vi => std::format!("{day} thg {month}, {year}"),
    }
}

fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    (
        if month <= 2 { year + 1 } else { year },
        month as u32,
        day as u32,
    )
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
impl Preferences {
    /// First launch follows the operating system language.
    pub fn system() -> Self {
        Self {
            locale: Locale::from_system(),
            ..Self::default()
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
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Preferences::system());
            }
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

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::panic)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    #[test]
    fn every_key_has_two_nonempty_rows_and_a_unique_id() {
        let mut keys = BTreeSet::new();
        for key in Text::ALL {
            assert!(keys.insert(key.key()), "duplicate key {}", key.key());
            assert!(!key.text(Locale::En).is_empty(), "{:?} en", key.key());
            assert!(!key.text(Locale::Vi).is_empty(), "{:?} vi", key.key());
        }
    }

    #[test]
    fn named_arguments_replace_without_leaving_placeholders() {
        let args = [("imported", "2"), ("failed", "1"), ("unfinished", "1")];
        assert_eq!(
            format(Locale::En, Text::ImportSummary, &args),
            "Imported: 2 · Failed: 1 · Unfinished: 1"
        );
        assert_eq!(
            format(Locale::Vi, Text::ImportSummary, &args),
            "Đã nhập: 2 · Thất bại: 1 · Chưa xử lý: 1"
        );
    }

    #[test]
    fn numbers_dates_and_plurals_follow_the_locale() {
        assert_eq!(number(Locale::En, 1_234_567), "1,234,567");
        assert_eq!(number(Locale::Vi, 1_234_567), "1.234.567");
        assert_eq!(decimal(Locale::En, 23.976, 3), "23.976");
        assert_eq!(decimal(Locale::Vi, 23.976, 3), "23,976");
        assert_eq!(signed(Locale::Vi, -12_345), "-12.345");
        assert_eq!(date(Locale::En, 1_780_000_000_000), "May 28, 2026");
        assert_eq!(date(Locale::Vi, 1_780_000_000_000), "28 thg 5, 2026");
        assert_eq!(plural(Locale::En, 1), Plural::One);
        assert_eq!(plural(Locale::En, 0), Plural::Other);
        assert_eq!(plural(Locale::Vi, 1), Plural::Other);
        assert_eq!(
            format_count(
                Locale::En,
                1,
                Text::SelectedCountOne,
                Text::SelectedCountOther
            ),
            "1 item selected"
        );
        assert_eq!(
            format_count(
                Locale::En,
                1_234,
                Text::SelectedCountOne,
                Text::SelectedCountOther
            ),
            "1,234 items selected"
        );
        assert_eq!(
            format_count(
                Locale::Vi,
                1_234,
                Text::SelectedCountOne,
                Text::SelectedCountOther
            ),
            "1.234 mục đã chọn"
        );
    }

    #[test]
    fn default_is_deterministic_and_system_preferences_follow_the_os() {
        assert_eq!(Preferences::default().locale, Locale::En);
        assert_eq!(Preferences::system().locale, Locale::from_system());
    }

    #[test]
    fn tags_map_to_an_offered_locale() {
        assert_eq!(Locale::En.code(), "en");
        assert_eq!(Locale::Vi.name(), "Tiếng Việt");
        for tag in ["vi", "vi-VN", "vi_VN.UTF-8", "VI-vn"] {
            assert_eq!(Locale::from_tag(tag), Locale::Vi, "{tag}");
        }
        for tag in ["en", "en-US", "fr-FR", "", "vibecut"] {
            assert_eq!(Locale::from_tag(tag), Locale::En, "{tag}");
        }
    }

    #[test]
    fn bundled_ui_font_covers_every_vietnamese_catalog_character() {
        use skrifa::MetadataProvider;
        let bytes = include_bytes!("../../../assets/fonts/Inter-Regular.ttf");
        let font = skrifa::FontRef::new(bytes).expect("bundled Inter font");
        let charmap = font.charmap();
        let mut missing = BTreeSet::new();
        for key in Text::ALL {
            for character in key.text(Locale::Vi).chars() {
                if !character.is_whitespace() && charmap.map(character).is_none() {
                    missing.insert(character);
                }
            }
        }
        for character in "0123456789.,·/×—%()…−".chars() {
            if charmap.map(character).is_none() {
                missing.insert(character);
            }
        }
        assert!(missing.is_empty(), "missing glyphs: {missing:?}");
    }
}
