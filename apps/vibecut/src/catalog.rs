//! One catalog and preference owner, extended by later frontend tickets.
//!
//! The [`Text`] table is the stable key set. Both locales must fill every key,
//! so a missing Vietnamese row is a compile error; `cargo xtask i18n` also
//! rejects empty rows, duplicate keys and keys no frontend code uses.
use crate::{
    Error, Result,
    actions::KeybindingPreferences,
    media_panel::{MediaSort, MediaView},
    panels::{AssetsTab, ElementKind, PropertiesTab},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    io::Read,
    path::{Path, PathBuf},
};

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
    SubFileBase { key: "sub_file_base", en: "captions", vi: "phu-de" },
    SubDraftRecovery { key: "sub_draft_recovery", en: "Your retained draft", vi: "Bản nháp được giữ lại" },
    SubDraftConflict { key: "sub_draft_conflict", en: "This caption changed elsewhere. Copy your draft, then Dismiss to reload the current text.", vi: "Phụ đề này đã thay đổi ở nơi khác. Sao chép bản nháp, rồi Đóng thông báo để nạp văn bản hiện tại." },
    SubMp4 { key: "sub_mp4", en: "MP4", vi: "MP4" },
    SubProviderLocal { key: "sub_provider_local", en: "{provider} (local)", vi: "{provider} (cục bộ)" },
    SubExportScope { key: "sub_export_scope", en: "SRT/VTT exports the selected caption track. MP4 burns visible captions.", vi: "SRT/VTT xuất rãnh phụ đề đã chọn. MP4 ghi các phụ đề đang hiện." },
    SubAttempts { key: "sub_attempts", en: "At most {count} attempts for this translation request.", vi: "Tối đa {count} lần thử cho yêu cầu dịch này." },
    SubRunnerMissing { key: "sub_runner_missing", en: "The local ASR helper is missing. Install a complete app distribution or choose whisper.cpp.", vi: "Thiếu trình nhận dạng cục bộ. Cài bản ứng dụng đầy đủ hoặc chọn whisper.cpp." },
    SubOverwrite { key: "sub_overwrite", en: "Replace the existing file?", vi: "Thay thế tệp hiện có?" },
    SubPage { key: "sub_page", en: "Page {page} / {pages}", vi: "Trang {page} / {pages}" },
    SubVietnamese { key: "sub_vietnamese", en: "Vietnamese", vi: "Tiếng Việt" },
    SubEnglish { key: "sub_english", en: "English", vi: "Tiếng Anh" },
    SubPrevious { key: "sub_previous", en: "Previous page", vi: "Trang trước" },
    SubNext { key: "sub_next", en: "Next page", vi: "Trang sau" },
    SubGenerate { key: "sub_generate", en: "Generate captions", vi: "Tạo phụ đề" },
    SubImport { key: "sub_import", en: "Import SRT / VTT", vi: "Nhập SRT / VTT" },
    SubSrt { key: "sub_srt", en: "SRT", vi: "SRT" },
    SubVtt { key: "sub_vtt", en: "VTT", vi: "VTT" },
    SubBurn { key: "sub_burn", en: "Burn into MP4", vi: "Ghi vào MP4" },
    SubTranslate { key: "sub_translate", en: "Translate", vi: "Dịch" },
    SubTarget { key: "sub_target", en: "Target / export language", vi: "Ngôn ngữ đích / xuất" },
    SubTranslator { key: "sub_translator", en: "Translation provider / model", vi: "Nhà cung cấp / mô hình dịch" },
    SubBilingual { key: "sub_bilingual", en: "Bilingual export", vi: "Xuất song ngữ" },
    SubReplaceEdited { key: "sub_replace_edited", en: "Replace manually edited translations", vi: "Thay bản dịch đã sửa thủ công" },
    SubTranslateSelected { key: "sub_translate_selected", en: "Select cues below to translate. Unselected cues stay unchanged.", vi: "Chọn các câu bên dưới để dịch. Các câu khác được giữ nguyên." },
    SubSelectMedia { key: "sub_select_media", en: "Select a video or audio clip on the timeline.", vi: "Chọn đoạn video hoặc âm thanh trên dòng thời gian." },
    SubChooseProvider { key: "sub_choose_provider", en: "Choose a supported provider", vi: "Chọn nhà cung cấp phù hợp" },
    SubEmpty { key: "sub_empty", en: "Generate from a selected clip or import an SRT / VTT file to start editing.", vi: "Tạo từ đoạn đã chọn hoặc nhập tệp SRT / VTT để bắt đầu sửa." },
    SubHans { key: "sub_hans", en: "Chinese (Simplified)", vi: "Tiếng Trung (Giản thể)" },
    SubHant { key: "sub_hant", en: "Chinese (Traditional)", vi: "Tiếng Trung (Phồn thể)" },
    SubChinese { key: "sub_chinese", en: "Chinese", vi: "Tiếng Trung" },
    SubItem { key: "sub_item", en: "Captions {id}", vi: "Phụ đề {id}" },
    SubSelectAll { key: "sub_select_all", en: "Select all cues", vi: "Chọn tất cả câu" },
    SubMerge { key: "sub_merge", en: "Merge", vi: "Gộp" },
    SubEarlier { key: "sub_earlier", en: "Earlier 1 frame", vi: "Sớm 1 khung hình" },
    SubLater { key: "sub_later", en: "Later 1 frame", vi: "Muộn 1 khung hình" },
    SubSelectionHelp { key: "sub_selection_help", en: "Click a time to seek. Shift selects a range; Cmd/Ctrl toggles. Split at the playhead. Text saves when focus leaves the field.", vi: "Bấm thời gian để tua. Shift chọn khoảng; Cmd/Ctrl bật tắt chọn. Tách tại con trỏ. Văn bản lưu khi rời ô." },
    SubGlossary { key: "sub_glossary", en: "Glossary", vi: "Bảng thuật ngữ" },
    SubGlossaryHelp { key: "sub_glossary_help", en: "Terms apply to this translation session. Leave the translation blank to preserve a name.", vi: "Thuật ngữ áp dụng trong phiên dịch này. Để trống bản dịch để giữ nguyên tên." },
    SubTerm { key: "sub_term", en: "Source term", vi: "Thuật ngữ nguồn" },
    SubTermTranslation { key: "sub_term_translation", en: "Translation (blank: keep original)", vi: "Bản dịch (trống: giữ nguyên)" },
    SubAddTerm { key: "sub_add_term", en: "Add term", vi: "Thêm thuật ngữ" },
    SubEdited { key: "sub_edited", en: "Edited", vi: "Đã sửa" },
    SubCps { key: "sub_cps", en: "Reading speed too high", vi: "Tốc độ đọc quá cao" },
    SubDone { key: "sub_done", en: "Done · {count} cues", vi: "Hoàn tất · {count} câu" },
    SubStale { key: "sub_stale", en: "The project changed during processing. Select the current cues and retry.", vi: "Dự án đã đổi trong khi xử lý. Chọn các câu hiện tại và thử lại." },
    SubFailed { key: "sub_failed", en: "Subtitle processing failed. Check the source, provider setup or destination and retry.", vi: "Xử lý phụ đề thất bại. Kiểm tra nguồn, thiết lập nhà cung cấp hoặc nơi lưu rồi thử lại." },
    SubEditFailed { key: "sub_edit_failed", en: "Cannot apply this edit. Check selection, timing, languages and unlocked tracks, then retry.", vi: "Không thể sửa. Kiểm tra vùng chọn, thời gian, ngôn ngữ và khóa rãnh rồi thử lại." },
    SubApprove { key: "sub_approve", en: "Approve this upload", vi: "Cho phép lần tải lên này" },
    SubDisclosure { key: "sub_disclosure", en: "{bytes} bytes · {seconds} seconds of audio · cost ceiling per attempt: {cost}", vi: "{bytes} byte · {seconds} giây âm thanh · chi phí tối đa mỗi lần thử: {cost}" },
    SubProviderPolicy { key: "sub_provider_policy", en: "Provider policy / request details", vi: "Chính sách / chi tiết yêu cầu" },
    SubTranscribing { key: "sub_transcribing", en: "Transcribing…", vi: "Đang nhận dạng…" },
    SubAligning { key: "sub_aligning", en: "Aligning speech timing…", vi: "Đang căn thời gian lời nói…" },
    SubTranslating { key: "sub_translating", en: "Translating…", vi: "Đang dịch…" },
    SubAssembling { key: "sub_assembling", en: "Preparing cues…", vi: "Đang tạo câu phụ đề…" },
    BlendMode { key: "blend_mode", en: "Blend mode", vi: "Chế độ hòa trộn" },
    Opacity { key: "opacity", en: "Opacity", vi: "Độ mờ" },
    BlendNormal { key: "blend_normal", en: "Normal", vi: "Bình thường" },
    BlendDarken { key: "blend_darken", en: "Darken", vi: "Làm tối" },
    BlendMultiply { key: "blend_multiply", en: "Multiply", vi: "Nhân" },
    BlendColorBurn { key: "blend_colorburn", en: "Color burn", vi: "Đốt màu" },
    BlendLighten { key: "blend_lighten", en: "Lighten", vi: "Làm sáng" },
    BlendScreen { key: "blend_screen", en: "Screen", vi: "Màn hình" },
    BlendPlusLighter { key: "blend_pluslighter", en: "Plus lighter", vi: "Cộng sáng" },
    BlendColorDodge { key: "blend_colordodge", en: "Color dodge", vi: "Né màu" },
    BlendOverlay { key: "blend_overlay", en: "Overlay", vi: "Phủ" },
    BlendSoftLight { key: "blend_softlight", en: "Soft light", vi: "Ánh sáng mềm" },
    BlendHardLight { key: "blend_hardlight", en: "Hard light", vi: "Ánh sáng mạnh" },
    BlendDifference { key: "blend_difference", en: "Difference", vi: "Khác biệt" },
    BlendExclusion { key: "blend_exclusion", en: "Exclusion", vi: "Loại trừ" },
    BlendHue { key: "blend_hue", en: "Hue", vi: "Sắc độ" },
    BlendSaturation { key: "blend_saturation", en: "Saturation", vi: "Độ bão hòa" },
    BlendColor { key: "blend_color", en: "Color", vi: "Màu sắc" },
    BlendLuminosity { key: "blend_luminosity", en: "Luminosity", vi: "Độ sáng" },
    ShortsJobFailed { key: "shorts_job_failed", en: "Failed", vi: "Thất bại" },
    ShortsDismiss { key: "shorts_dismiss", en: "Dismiss", vi: "Đóng thông báo" },
    ShortsProviderModel { key: "shorts_provider_model", en: "{provider} / {model}", vi: "{provider} / {model}" },
    ShortsUploadIdentity { key: "shorts_upload_identity", en: "{provider} / {model} · {languages}", vi: "{provider} / {model} · {languages}" },
    ShortsValue { key: "shorts_value", en: "{label}: {value}", vi: "{label}: {value}" },
    ShortsMoney { key: "shorts_money", en: "{currency} {amount}", vi: "{currency} {amount}" },

    Windows { key: "windows", en: "Window", vi: "Cửa sổ" },
    Home { key: "home", en: "Home", vi: "Trang chủ" },

    // Long-video Shorts workflow.
    ShortsBack { key: "shorts_back", en: "Back", vi: "Quay lại" },
    ShortsCancelling { key: "shorts_cancelling", en: "Cancelling…", vi: "Đang hủy…" },
    ShortsDrop { key: "shorts_drop", en: "Choose or drop a video", vi: "Chọn hoặc thả video vào đây" },
    ShortsSourceUnavailable { key: "shorts_source_unavailable", en: "Source video is missing or changed. Choose the video again.", vi: "Video nguồn bị mất hoặc đã thay đổi. Hãy chọn lại video." },
    ShortsMemory { key: "shorts_memory", en: "Memory budget is full. Finish or cancel other jobs, then retry.", vi: "Đã hết bộ nhớ cho phép. Hãy chờ hoặc hủy tác vụ khác rồi thử lại." },
    ShortsOutputExists { key: "shorts_output_exists", en: "Output already exists. Choose another folder.", vi: "Đã có tệp kết quả. Hãy chọn thư mục khác." },
    ShortsPolicy { key: "shorts_policy", en: "Privacy policy", vi: "Chính sách riêng tư" },
    ShortsUploadAudio { key: "shorts_upload_audio", en: "Your audio is sent to turn speech into text.", vi: "Âm thanh của bạn được gửi đi để chuyển thành chữ." },
    ShortsUploadTranscript { key: "shorts_upload_transcript", en: "Your transcript and timings are sent, not your audio.", vi: "Bản chép lời và thời gian được gửi đi, không gồm âm thanh." },
    ShortsJobCancelled { key: "shorts_job_cancelled", en: "Cancelled", vi: "Đã hủy" },
    ShortsModelMissing { key: "shorts_model_missing", en: "Speech model missing. Install it or pick another.", vi: "Thiếu mô hình giọng nói. Hãy cài hoặc chọn mô hình khác." },
    ShortsCredentialsMissing { key: "shorts_credentials_missing", en: "No key for this service. Add it or pick a local one.", vi: "Chưa có khóa cho dịch vụ này. Hãy thêm khóa hoặc chọn dịch vụ trên máy." },
    ShortsConsentRequired { key: "shorts_consent_required", en: "Upload not allowed. Pick a local option or try again.", vi: "Chưa cho phép tải lên. Hãy chọn tùy chọn trên máy hoặc thử lại." },
    Shorts { key: "shorts", en: "Long video to shorts", vi: "Video dài thành video ngắn" },
    ShortsPick { key: "shorts_pick", en: "Choose video", vi: "Chọn video" },
    ShortsReplace { key: "shorts_replace", en: "Replace video", vi: "Đổi video" },
    ShortsGenerate { key: "shorts_generate", en: "Create shorts", vi: "Tạo video ngắn" },
    ShortsTranscriber { key: "shorts_transcriber", en: "Speech to text", vi: "Chuyển giọng nói thành chữ" },
    ShortsSuggester { key: "shorts_suggester", en: "Pick clips with", vi: "Chọn đoạn bằng" },
    ShortsLocal { key: "shorts_local", en: "Local heuristic · no transcript upload", vi: "Luật cục bộ · không tải bản chép lời lên" },
    ShortsCount { key: "shorts_count", en: "How many", vi: "Số lượng" },
    ShortsMin { key: "shorts_min", en: "Shortest (s)", vi: "Ngắn nhất (giây)" },
    ShortsMax { key: "shorts_max", en: "Longest (s)", vi: "Dài nhất (giây)" },
    ShortsMaxSegments { key: "shorts_max_segments", en: "Parts per short", vi: "Số đoạn mỗi video" },
    ShortsSourceLanguage { key: "shorts_source_language", en: "Video language", vi: "Ngôn ngữ video" },
    ShortsCreating { key: "shorts_creating", en: "Creating shorts…", vi: "Đang tạo video ngắn…" },
    ShortsNoResults { key: "shorts_no_results", en: "No safe shorts found. Try a longer video or different durations.", vi: "Không tìm được đoạn phù hợp. Hãy thử video dài hơn hoặc đổi thời lượng." },
    ShortsReview { key: "shorts_review", en: "Review shorts", vi: "Duyệt video ngắn" },
    ShortsAccept { key: "shorts_accept", en: "Accept", vi: "Chấp nhận" },
    ShortsReject { key: "shorts_reject", en: "Reject", vi: "Loại bỏ" },
    ShortsEdit { key: "shorts_edit", en: "Edit more", vi: "Chỉnh sửa thêm" },
    ShortsExport { key: "shorts_export", en: "Export accepted shorts", vi: "Xuất các video đã chọn" },
    ShortsIn { key: "shorts_in", en: "In", vi: "Vào" },
    ShortsOut { key: "shorts_out", en: "Out", vi: "Ra" },
    ShortsApplyTrim { key: "shorts_apply_trim", en: "Trim", vi: "Cắt" },
    ShortsCrop { key: "shorts_crop", en: "9:16 crop", vi: "Vùng cắt 9:16" },
    ShortsCenter { key: "shorts_center", en: "Center crop", vi: "Căn giữa vùng cắt" },
    ShortsHook { key: "shorts_hook", en: "Hook", vi: "Mở đầu" },
    ShortsContext { key: "shorts_context", en: "Context", vi: "Nội dung" },
    ShortsPayoff { key: "shorts_payoff", en: "Payoff", vi: "Kết luận" },
    ShortsConsent { key: "shorts_consent", en: "Allow upload", vi: "Cho phép tải lên" },
    ShortsDeny { key: "shorts_deny", en: "Keep on this computer", vi: "Giữ trên máy này" },
    ShortsDisclosure { key: "shorts_disclosure", en: "Sends data off this computer. Policy:", vi: "Dữ liệu rời khỏi máy này. Chính sách:" },
    ShortsExporting { key: "shorts_exporting", en: "Exporting", vi: "Đang xuất" },
    ShortsError { key: "shorts_error", en: "Could not complete this action. Adjust the input or retry.", vi: "Không thể hoàn tất. Hãy điều chỉnh dữ liệu hoặc thử lại." },
    ShortsTrimError { key: "shorts_trim_error", en: "No clean cut there. Your last edit is kept.", vi: "Không có điểm cắt gọn ở đó. Giữ nguyên chỉnh sửa trước." },
    ShortsCancelled { key: "shorts_cancelled", en: "Cancelled. Finished exports are kept.", vi: "Đã hủy. Các video đã xuất được giữ lại." },
    ShortsBytes { key: "shorts_bytes", en: "Size", vi: "Dung lượng" },
    ShortsSeconds { key: "shorts_seconds", en: "Audio length (s)", vi: "Độ dài âm thanh (giây)" },
    ShortsCost { key: "shorts_cost", en: "Highest cost (USD)", vi: "Chi phí cao nhất (USD)" },
    ShortsFree { key: "shorts_free", en: "Free", vi: "Miễn phí" },
    ShortsUnknownCost { key: "shorts_unknown_cost", en: "Unknown cost", vi: "Chi phí chưa xác định" },
    ShortsCaptions { key: "shorts_captions", en: "Burn in captions + SRT", vi: "Ghi phụ đề vào video + SRT" },
    ShortsDone { key: "shorts_done", en: "Completed", vi: "Hoàn tất" },
    ShortsHero { key: "shorts_hero", en: "Turn a long video into", vi: "Biến video dài thành" },
    ShortsHeroAccent { key: "shorts_hero_accent", en: "engaging shorts", vi: "video ngắn hấp dẫn" },
    ShortsCountLabel { key: "shorts_count_label", en: "{count} shorts", vi: "{count} video ngắn" },
    ShortsCreated { key: "shorts_created", en: "created from your video", vi: "được tạo từ video của bạn" },
    ShortsPercent { key: "shorts_percent", en: "{value}%", vi: "{value}%" },
    ShortsOptions { key: "shorts_options", en: "Options", vi: "Tùy chọn" },
    ShortsList { key: "shorts_list", en: "Shorts", vi: "Video ngắn" },
    ShortsTrim { key: "shorts_trim", en: "Trim", vi: "Cắt" },
    ShortsLayout { key: "shorts_layout", en: "Layout", vi: "Bố cục" },
    ShortsApproval { key: "shorts_approval", en: "Waiting for your approval", vi: "Đang chờ bạn cho phép" },
    ShortsSeek { key: "shorts_seek", en: "Seek", vi: "Tua" },

    // Shell and lifecycle.
    AppName { key: "app_name", en: "VibeCut", vi: "VibeCut" },
    ChooseProject { key: "choose_project", en: "Choose a project file…", vi: "Chọn tệp dự án…" },
    Projects { key: "projects", en: "Projects", vi: "Dự án" },
    HomeShortsHint { key: "home_shorts_hint", en: "Turn a long video into shorts", vi: "Biến video dài thành video ngắn" },
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

    // Projects page.
    ProjectsSearch { key: "projects_search", en: "Search projects", vi: "Tìm dự án" },
    ProjectsGrid { key: "projects_grid", en: "Grid view", vi: "Dạng lưới" },
    ProjectsList { key: "projects_list", en: "List view", vi: "Dạng danh sách" },
    ProjectsSort { key: "projects_sort", en: "Sort: {sort}", vi: "Sắp xếp: {sort}" },
    ProjectsSortValue { key: "projects_sort_value", en: "{key} · {order}", vi: "{key} · {order}" },
    ProjectsSortCreated { key: "projects_sort_created", en: "Created", vi: "Ngày tạo" },
    ProjectsSortModified { key: "projects_sort_modified", en: "Modified", vi: "Ngày sửa" },
    ProjectsSortName { key: "projects_sort_name", en: "Name", vi: "Tên" },
    ProjectsSortDuration { key: "projects_sort_duration", en: "Duration", vi: "Thời lượng" },
    ProjectsSortAscending { key: "projects_sort_ascending", en: "Ascending", vi: "Tăng dần" },
    ProjectsSortDescending { key: "projects_sort_descending", en: "Descending", vi: "Giảm dần" },
    ProjectsEmpty { key: "projects_empty", en: "No projects yet", vi: "Chưa có dự án" },
    ProjectsNoResults { key: "projects_no_results", en: "No matches", vi: "Không có kết quả" },
    ProjectsOpen { key: "projects_open", en: "Open", vi: "Mở" },
    ProjectsCardMenu { key: "projects_card_menu", en: "Project actions", vi: "Thao tác dự án" },
    ProjectsDuplicate { key: "projects_duplicate", en: "Duplicate", vi: "Nhân bản" },
    ProjectsDuplicating { key: "projects_duplicating", en: "Duplicating {done} of {total}…", vi: "Đang nhân bản {done}/{total}…" },
    ProjectsDuplicateFailed { key: "projects_duplicate_failed", en: "Some projects could not be duplicated. Successful copies are kept.", vi: "Một số dự án không thể nhân bản. Các bản sao thành công được giữ lại." },
    ProjectsDeleteTitle { key: "projects_delete_title", en: "Delete projects?", vi: "Xóa dự án?" },
    ProjectsDeletePrompt { key: "projects_delete_prompt", en: "This permanently deletes the selected projects. This cannot be undone.", vi: "Thao tác này xóa vĩnh viễn các dự án đã chọn. Không thể hoàn tác." },
    ProjectsDeleteConfirm { key: "projects_delete_confirm", en: "Delete permanently", vi: "Xóa vĩnh viễn" },
    ProjectsDeleteFailed { key: "projects_delete_failed", en: "Some projects could not be deleted. They remain in the library.", vi: "Một số dự án không thể xóa. Các dự án này vẫn còn trong thư viện." },
    ProjectsRenameTitle { key: "projects_rename_title", en: "Rename project", vi: "Đổi tên dự án" },
    ProjectsRename { key: "projects_rename", en: "Rename", vi: "Đổi tên" },
    ProjectsRenameSave { key: "projects_rename_save", en: "Save name", vi: "Lưu tên" },
    ProjectsRenameInvalid { key: "projects_rename_invalid", en: "Enter a unique, non-empty project name.", vi: "Hãy nhập tên dự án không trống và chưa được dùng." },
    ProjectsInfoTitle { key: "projects_info_title", en: "Project info", vi: "Thông tin dự án" },
    ProjectsInfoClose { key: "projects_info_close", en: "Close", vi: "Đóng" },
    ProjectsInfoId { key: "projects_info_id", en: "Project ID", vi: "Mã dự án" },
    ProjectsDuration { key: "projects_duration", en: "Duration {duration}", vi: "Thời lượng {duration}" },
    ProjectsRefreshThumbnail { key: "projects_refresh_thumbnail", en: "Refresh thumbnail", vi: "Làm mới ảnh thu nhỏ" },
    ProjectsThumbnailMissing { key: "projects_thumbnail_missing", en: "No thumbnail", vi: "Chưa có ảnh thu nhỏ" },
    ProjectsThumbnailFailed { key: "projects_thumbnail_failed", en: "Could not refresh the thumbnail.", vi: "Không thể làm mới ảnh thu nhỏ." },
    ProjectsRetry { key: "projects_retry", en: "Retry project list", vi: "Tải lại danh sách dự án" },
    ProjectsSelected { key: "projects_selected", en: "{n} selected", vi: "{n} đã chọn" },
    ProjectsListError { key: "projects_list_error", en: "Project list unavailable.", vi: "Không thể tải danh sách dự án." },

    // Header and appearance.
    AutoSaved { key: "auto_saved", en: "Auto saved", vi: "Đã tự động lưu" },
    NotSaved { key: "not_saved", en: "Not saved", vi: "Chưa lưu" },
    SaveFailed { key: "save_failed", en: "Save failed", vi: "Không thể lưu" },
    Theme { key: "theme", en: "Switch theme", vi: "Đổi giao diện" },
    Language { key: "language", en: "Tiếng Việt", vi: "English" },
    Reset { key: "reset_layout", en: "Reset layout", vi: "Đặt lại bố cục" },
    Export { key: "export", en: "Export", vi: "Xuất" },
    PreferencesWarning { key: "preferences_warning", en: "Preferences", vi: "Tùy chọn" },
    PreferencesReset { key: "preferences_reset", en: "Saved preferences could not be read; defaults were restored.", vi: "Không thể đọc tùy chọn đã lưu; đã khôi phục mặc định." },
    Ok { key: "ok", en: "OK", vi: "OK" },

    // Panels and empty states.
    Media { key: "media", en: "Media", vi: "Tư liệu" },
    Sounds { key: "sounds", en: "Sounds", vi: "Âm thanh" },
    Stickers { key: "stickers", en: "Stickers", vi: "Nhãn dán" },
    Effects { key: "effects", en: "Effects", vi: "Hiệu ứng" },
    Transitions { key: "transitions", en: "Transitions", vi: "Chuyển cảnh" },
    Adjustment { key: "adjustment", en: "Adjustment", vi: "Điều chỉnh" },
    Settings { key: "settings", en: "Settings", vi: "Cài đặt" },
    Transform { key: "transform", en: "Transform", vi: "Biến đổi" },
    ResetValue { key: "reset_value", en: "Reset value", vi: "Đặt lại giá trị" },
    Scale { key: "scale", en: "Scale", vi: "Tỷ lệ" },
    UniformScale { key: "uniform_scale", en: "Uniform scale", vi: "Co giãn đều" },
    Basic { key: "basic", en: "Basic", vi: "Cơ bản" },
    Position { key: "position", en: "Position", vi: "Vị trí" },
    PositionX { key: "position_x", en: "Position X", vi: "Vị trí X" },
    PositionY { key: "position_y", en: "Position Y", vi: "Vị trí Y" },
    ScaleX { key: "scale_x", en: "Scale X", vi: "Tỷ lệ X" },
    ScaleY { key: "scale_y", en: "Scale Y", vi: "Tỷ lệ Y" },
    Rotate { key: "rotate", en: "Rotate", vi: "Xoay" },
    ColorPicker { key: "color_picker", en: "Colour", vi: "Màu" },
    ColorHex { key: "color_hex", en: "Hex", vi: "Mã hex" },
    Hue { key: "color_hue", en: "Hue", vi: "Sắc độ" },
    Saturation { key: "color_saturation", en: "Saturation", vi: "Độ bão hòa" },
    Value { key: "color_value", en: "Value", vi: "Độ sáng" },
    Alpha { key: "color_alpha", en: "Alpha", vi: "Độ trong suốt" },
    Swatches { key: "color_swatches", en: "Swatches", vi: "Bảng màu" },
    RecentColors { key: "recent_colors", en: "Recent colours", vi: "Màu gần đây" },
    Eyedropper { key: "eyedropper", en: "Pick screen colour", vi: "Lấy màu màn hình" },
    EyedropperUnavailable { key: "eyedropper_unavailable", en: "Screen colour picker is unavailable on this platform.", vi: "Bộ lấy màu màn hình không khả dụng trên nền tảng này." },
    EyedropperPending { key: "eyedropper_pending", en: "Choose a screen pixel; Escape cancels.", vi: "Chọn một điểm ảnh; nhấn Esc để hủy." },
    EyedropperFailed { key: "eyedropper_failed", en: "Screen colour picker failed. Try again.", vi: "Lấy màu màn hình thất bại. Hãy thử lại." },
    InvalidColor { key: "invalid_color", en: "Enter a valid hex or finite HSV value.", vi: "Nhập mã hex hợp lệ hoặc giá trị HSV hữu hạn." },
    GraphicFill { key: "graphic_fill", en: "Fill", vi: "Màu tô" },
    GraphicStroke { key: "graphic_stroke", en: "Stroke", vi: "Màu viền" },
    CanvasBackground { key: "canvas_background", en: "Canvas background", vi: "Nền khung hình" },
    Speed { key: "speed", en: "Speed", vi: "Tốc độ" },
    Blending { key: "blending", en: "Blending", vi: "Hòa trộn" },
    Masks { key: "masks", en: "Masks", vi: "Mặt nạ" },
    Graphic { key: "graphic", en: "Graphic", vi: "Đồ họa" },

    // Effects panel and per-clip stack.
    EffectBlur { key: "effect_blur", en: "Blur", vi: "Làm mờ" },
    EffectIntensity { key: "effect_intensity", en: "Intensity", vi: "Cường độ" },
    EffectEmptyStack { key: "effect_empty_stack", en: "No effects on this clip yet", vi: "Clip này chưa có hiệu ứng" },
    EffectAdd { key: "effect_add", en: "Add effect", vi: "Thêm hiệu ứng" },
    EffectRemove { key: "effect_remove", en: "Remove effect", vi: "Xóa hiệu ứng" },
    EffectEnable { key: "effect_enable", en: "Enable effect", vi: "Bật hiệu ứng" },
    EffectDisable { key: "effect_disable", en: "Disable effect", vi: "Tắt hiệu ứng" },
    EffectDisabled { key: "effect_disabled", en: "Disabled", vi: "Đã tắt" },
    EffectReorder { key: "effect_reorder", en: "Reorder effect", vi: "Đổi thứ tự hiệu ứng" },
    EffectPreviewPending { key: "effect_preview_pending", en: "Rendering preview…", vi: "Đang dựng bản xem trước…" },
    EffectPreviewFailed { key: "effect_preview_failed", en: "Preview unavailable", vi: "Không có bản xem trước" },
    EffectPreviewRetry { key: "effect_preview_retry", en: "Retry preview", vi: "Thử lại bản xem trước" },
    EffectAudioUnsupported { key: "effect_audio_unsupported", en: "Audio clips cannot use visual effects", vi: "Clip âm thanh không dùng được hiệu ứng hình ảnh" },
    EffectKindUnsupported { key: "effect_kind_unsupported", en: "This clip type cannot use effects", vi: "Loại clip này không dùng được hiệu ứng" },
    EffectLocked { key: "effect_locked", en: "Unlock the track to change effects", vi: "Mở khóa track để thay đổi hiệu ứng" },
    EffectDropMiss { key: "effect_drop_miss", en: "Drop the effect on a video, image, sticker or graphic clip", vi: "Thả hiệu ứng vào clip video, hình ảnh, nhãn dán hoặc đồ họa" },
    EffectSelectClip { key: "effect_select_clip", en: "Select a video, image, sticker or graphic clip first", vi: "Hãy chọn clip video, hình ảnh, nhãn dán hoặc đồ họa trước" },
    EffectLimit { key: "effect_limit", en: "This clip already has the maximum number of effects", vi: "Clip này đã có số hiệu ứng tối đa" },
    EffectAdded { key: "effect_added", en: "Effect added", vi: "Đã thêm hiệu ứng" },
    EffectMissing { key: "effect_missing", en: "The effect is no longer on this clip", vi: "Hiệu ứng không còn trên clip này" },
    PlayerFull { key: "player_full", en: "Full", vi: "Đầy đủ" },
    PlayerRatio { key: "player_ratio", en: "Ratio", vi: "Tỷ lệ" },
    Preview { key: "preview", en: "Player", vi: "Trình phát" },
    Project { key: "project", en: "Project", vi: "Dự án" },
    Details { key: "details", en: "Details", vi: "Chi tiết" },
    AssetsAddText { key: "assets_add_text", en: "Add text", vi: "Thêm văn bản" },
    AssetsNoStyles { key: "assets_no_styles", en: "No styles", vi: "Chưa có kiểu" },
    AssetsAll { key: "assets_all", en: "All", vi: "Tất cả" },
    AssetsSearch { key: "assets_search", en: "Search media", vi: "Tìm tư liệu" },
    AssetsNoMedia { key: "assets_no_media", en: "No media", vi: "Chưa có tư liệu" },
    AssetsNoMatches { key: "assets_no_matches", en: "No matches", vi: "Không có kết quả" },
    AssetsUnavailable { key: "assets_unavailable", en: "No items", vi: "Chưa có mục nào" },
    AssetsImportError { key: "assets_import_error", en: "Couldn't use this file; try another.", vi: "Không dùng được tệp; hãy thử tệp khác." },
    EmptyPreview { key: "empty_preview", en: "Add media to the timeline to begin", vi: "Thêm tư liệu vào dòng thời gian để bắt đầu" },
    EmptyTimeline { key: "empty_timeline", en: "Your story starts here", vi: "Câu chuyện của bạn bắt đầu ở đây" },

    // Import.
    Import { key: "import", en: "Import", vi: "Nhập" },
    AddToTimeline { key: "add_to_timeline", en: "Add to timeline", vi: "Thêm vào dòng thời gian" },
    DropMedia { key: "drop_media", en: "Drop media here to import", vi: "Thả tư liệu vào đây để nhập" },
    Importing { key: "importing", en: "Importing media…", vi: "Đang nhập tư liệu…" },
    ImportBusy { key: "import_busy", en: "An import is already running", vi: "Đang có một lần nhập khác" },
    Processing { key: "processing", en: "Processing", vi: "Đang xử lý" },
    ProcessingPercent { key: "processing_percent", en: "{status} {percent}%", vi: "{status} {percent}%" },
    ImportComplete { key: "import_complete", en: "Import complete", vi: "Nhập hoàn tất" },
    ImportPartial { key: "import_partial", en: "Some files were not imported; try another.", vi: "Có tệp không nhập được; hãy thử tệp khác." },
    ImportFailed { key: "import_failed", en: "Import failed; try again.", vi: "Không nhập được; hãy thử lại." },
    ImportCancelled { key: "import_cancelled", en: "Import cancelled", vi: "Đã hủy nhập" },
    InsertFailed { key: "insert_failed", en: "Couldn't add this clip; try another.", vi: "Không thêm được clip; hãy thử clip khác." },
    TextInsertFailed { key: "text_insert_failed", en: "Couldn't add text; try again.", vi: "Không thêm được văn bản; hãy thử lại." },
    ImportSummary { key: "import_summary", en: "Imported: {imported} · Failed: {failed} · Unfinished: {unfinished}", vi: "Đã nhập: {imported} · Thất bại: {failed} · Chưa xử lý: {unfinished}" },
    ImportProgress { key: "import_progress", en: "{done} / {total}", vi: "{done} / {total}" },
    FailureDetail { key: "failure_detail", en: "{name}: {reason}", vi: "{name}: {reason}" },
    DurationUnknown { key: "duration_unknown", en: "duration unknown", vi: "chưa rõ thời lượng" },
    SelectedCountOne { key: "selected_count_one", en: "{n} item selected", vi: "{n} mục đã chọn" },
    SelectedCountOther { key: "selected_count_other", en: "{n} items selected", vi: "{n} mục đã chọn" },

    // Media library views, sorting and removal.
    Image { key: "image", en: "Image", vi: "Hình ảnh" },
    SwitchToGrid { key: "switch_to_grid", en: "Switch to grid view", vi: "Chuyển sang dạng lưới" },
    SwitchToList { key: "switch_to_list", en: "Switch to list view", vi: "Chuyển sang dạng danh sách" },
    SortName { key: "sort_name", en: "Name", vi: "Tên" },
    SortType { key: "sort_type", en: "Type", vi: "Loại" },
    SortDuration { key: "sort_duration", en: "Duration", vi: "Thời lượng" },
    SortSize { key: "sort_size", en: "File size", vi: "Kích thước tệp" },
    SortAscending { key: "sort_ascending", en: "ascending", vi: "tăng dần" },
    SortDescending { key: "sort_descending", en: "descending", vi: "giảm dần" },
    SortTooltip { key: "sort_tooltip", en: "Sort by {key} ({order})", vi: "Sắp xếp theo {key} ({order})" },
    MediaSize { key: "media_size", en: "{size} {unit}", vi: "{size} {unit}" },
    DeleteItemOne { key: "delete_item_one", en: "Delete item", vi: "Xóa mục" },
    DeleteItemsOther { key: "delete_items_other", en: "Delete {n} items", vi: "Xóa {n} mục" },
    DeleteMediaPromptOne { key: "delete_media_prompt_one", en: "Delete this media asset? You can undo this action.", vi: "Xóa tư liệu này? Bạn có thể hoàn tác thao tác này." },
    DeleteMediaPromptOther { key: "delete_media_prompt_other", en: "Delete {n} media assets? You can undo this action.", vi: "Xóa {n} tư liệu? Bạn có thể hoàn tác thao tác này." },
    DeleteMediaFailed { key: "delete_media_failed", en: "Could not delete media", vi: "Không thể xóa tư liệu" },
    MediaDeletedOne { key: "media_deleted_one", en: "Media deleted", vi: "Đã xóa tư liệu" },
    MediaDeletedOther { key: "media_deleted_other", en: "Media deleted", vi: "Đã xóa tư liệu" },
    RevealMedia { key: "reveal_media", en: "Reveal media", vi: "Tìm tư liệu" },

    // Scenes.
    NewScene { key: "new_scene", en: "New scene", vi: "Cảnh mới" },
    RenameScene { key: "rename_scene", en: "Rename", vi: "Đổi tên" },
    SceneDefaultName { key: "scene_default_name", en: "Scene {n}", vi: "Cảnh {n}" },
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
    Unavailable { key: "unavailable", en: "Couldn't play this video; retry.", vi: "Không thể phát video; hãy thử lại." },
    Loading { key: "loading", en: "Preparing preview…", vi: "Đang chuẩn bị xem trước…" },
    Retry { key: "retry", en: "Retry preview", vi: "Thử xem trước lại" },
    Offline { key: "offline", en: "Audio output unavailable", vi: "Đầu ra âm thanh không khả dụng" },
    TimeTotal { key: "time_total", en: "/ {duration}", vi: "/ {duration}" },
    ProjectEmpty { key: "project_empty", en: "Project is empty", vi: "Dự án trống" },
    TimecodeInvalid { key: "timecode_invalid", en: "Invalid timecode", vi: "Mã thời gian không hợp lệ" },
    ZoomFit { key: "zoom_fit", en: "Fit", vi: "Vừa khung" },
    SnapshotSave { key: "snapshot_save", en: "Save frame", vi: "Lưu khung hình" },
    SnapshotCopy { key: "snapshot_copy", en: "Copy frame", vi: "Sao chép khung hình" },
    SnapshotSaved { key: "snapshot_saved", en: "Frame saved", vi: "Đã lưu khung hình" },
    SnapshotFailed { key: "snapshot_failed", en: "Couldn't save this frame; retry.", vi: "Không thể lưu khung hình; hãy thử lại." },
    SnapshotFormat { key: "snapshot_format", en: "PNG image", vi: "Ảnh PNG" },
    Fullscreen { key: "fullscreen", en: "Fullscreen preview", vi: "Xem toàn màn hình" },
    ExitFullscreen { key: "exit_fullscreen", en: "Exit fullscreen", vi: "Thoát toàn màn hình" },

    // Inspector.
    AudioGain { key: "audio_gain", en: "Volume (dB)", vi: "Âm lượng (dB)" },
    AudioMuted { key: "audio_muted", en: "Mute", vi: "Tắt tiếng" },
    ChangePitch { key: "change_pitch", en: "Change pitch", vi: "Đổi cao độ" },
    SpeedReset { key: "speed_reset", en: "Reset speed", vi: "Đặt lại tốc độ" },
    SpeedDuration { key: "speed_duration", en: "Duration", vi: "Thời lượng" },
    TextContent { key: "text_content", en: "Content", vi: "Nội dung" },
    TextFont { key: "text_font", en: "Font", vi: "Phông chữ" },
    TextSize { key: "text_size", en: "Font size", vi: "Cỡ chữ" },
    TextColor { key: "text_color", en: "Text colour", vi: "Màu chữ" },
    DragToAdjust { key: "drag_to_adjust", en: "Drag to adjust", vi: "Kéo để điều chỉnh" },
    TextBackground { key: "text_background", en: "Background", vi: "Nền" },
    BackgroundColor { key: "background_color", en: "Background colour", vi: "Màu nền" },
    TextPositionX { key: "text_position_x", en: "Horizontal position", vi: "Vị trí ngang" },
    TextPositionY { key: "text_position_y", en: "Vertical position", vi: "Vị trí dọc" },
    CompatibleCount { key: "compatible_count", en: "Compatible: {compatible} / {total}", vi: "Tương thích: {compatible} / {total}" },
    NoSharedFields { key: "no_shared_fields", en: "No editable shared fields for this selection.", vi: "Không có thuộc tính chung có thể sửa cho lựa chọn này." },
    MaskPoints { key: "mask_points", en: "Mask points selected", vi: "Đã chọn điểm mặt nạ" },
    Keyframes { key: "keyframes", en: "Keyframes selected", vi: "Đã chọn khung hình chính" },
    EmptyValue { key: "empty_value", en: "—", vi: "—" },
    Canvas { key: "canvas", en: "Canvas", vi: "Khung hình" },
    Metadata { key: "metadata", en: "Metadata", vi: "Siêu dữ liệu" },
    Selection { key: "selection", en: "Selection", vi: "Lựa chọn" },
    CanvasSize { key: "canvas_size", en: "{width} × {height}", vi: "{width} × {height}" },
    FrameRate { key: "frame_rate", en: "{num} / {den} fps", vi: "{num} / {den} fps" },
    SampleRate { key: "sample_rate", en: "{rate} Hz", vi: "{rate} Hz" },
    CreatedDate { key: "created_date", en: "Created {date}", vi: "Đã tạo {date}" },
    ModifiedDate { key: "modified_date", en: "Modified {date}", vi: "Đã sửa {date}" },

    // Tracks and items.
    Video { key: "video", en: "Video", vi: "Video" },
    Audio { key: "audio", en: "Audio", vi: "Âm thanh" },
    TextItem { key: "text_item", en: "Text", vi: "Văn bản" },
    VideoTrackName { key: "video_track_name", en: "Video {n}", vi: "Video {n}" },
    AudioTrackName { key: "audio_track_name", en: "Audio {n}", vi: "Âm thanh {n}" },
    Caption { key: "caption", en: "Captions", vi: "Phụ đề" },
    MuteTrack { key: "mute_track", en: "Mute track", vi: "Tắt tiếng rãnh" },
    HideTrack { key: "hide_track", en: "Hide track", vi: "Ẩn rãnh" },
    DeleteTrack { key: "delete_track", en: "Delete track", vi: "Xóa rãnh" },
    RenameTrack { key: "rename_track", en: "Rename track", vi: "Đổi tên rãnh" },
    LockTrack { key: "lock_track", en: "Lock track", vi: "Khóa rãnh" },
    UnlockTrack { key: "unlock_track", en: "Unlock track", vi: "Mở khóa rãnh" },
    AddTrack { key: "add_track", en: "Add track", vi: "Thêm rãnh" },
    TrackLocked { key: "track_locked", en: "Track is locked", vi: "Rãnh đang khóa" },
    Magnet { key: "magnet", en: "Main track magnet", vi: "Nam châm rãnh chính" },
    TrackMore { key: "track_more", en: "More", vi: "Thêm" },
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
    ZoomLevel { key: "zoom_level", en: "Timeline zoom", vi: "Mức thu phóng dòng thời gian" },
    ActionShortcut { key: "action_shortcut", en: "{action} ({shortcut})", vi: "{action} ({shortcut})" },

    // Shortcuts dialog.
    ShortcutsTitle { key: "shortcuts_title", en: "Shortcuts", vi: "Phím tắt" },
    Shortcuts { key: "shortcuts", en: "Shortcuts", vi: "Phím tắt" },
    ShortcutsPressKey { key: "shortcuts_press_key", en: "Press keys…", vi: "Nhấn phím…" },
    ShortcutsUnbound { key: "shortcuts_unbound", en: "Unbound", vi: "Chưa gán phím" },
    ShortcutsReset { key: "shortcuts_reset", en: "Reset", vi: "Đặt lại" },
    ShortcutsResetAll { key: "shortcuts_reset_all", en: "Reset all", vi: "Đặt lại tất cả" },
    ShortcutsImport { key: "shortcuts_import", en: "Import JSON", vi: "Nhập JSON" },
    ShortcutsExport { key: "shortcuts_export", en: "Export JSON", vi: "Xuất JSON" },
    ShortcutsIoPending { key: "shortcuts_io_pending", en: "Import and export arrive with settings import/export (p1-parity/75).", vi: "Nhập và xuất sẽ có cùng tính năng nhập/xuất cài đặt (p1-parity/75)." },
    ShortcutsErrorConflict { key: "shortcuts_error_conflict", en: "`{chord}` is already used by {action}.", vi: "`{chord}` đã được dùng bởi {action}." },
    ShortcutsErrorInvalid { key: "shortcuts_error_invalid", en: "Invalid shortcut: {reason}", vi: "Phím tắt không hợp lệ: {reason}" },
    ShortcutsClose { key: "shortcuts_close", en: "Close", vi: "Đóng" },
    ShortcutsProfile { key: "shortcuts_profile", en: "Shortcut 1", vi: "Phím tắt 1" },
    ShortcutsSave { key: "shortcuts_save", en: "Save", vi: "Lưu" },
    ShortcutsCancel { key: "shortcuts_cancel", en: "Cancel", vi: "Hủy" },
    ShortcutsSaveHint { key: "shortcuts_save_hint", en: "Changes apply immediately and can be undone; no explicit save is needed.", vi: "Thay đổi được áp dụng ngay và có thể hoàn tác; không cần lưu riêng." },
    ShortcutsTabTimeline { key: "shortcuts_tab_timeline", en: "Timeline", vi: "Dòng thời gian" },
    ShortcutsTabPlayer { key: "shortcuts_tab_player", en: "Player", vi: "Trình phát" },
    ShortcutsTabBasic { key: "shortcuts_tab_basic", en: "Basic", vi: "Cơ bản" },
    ShortcutsTabOther { key: "shortcuts_tab_other", en: "Other", vi: "Khác" },

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
    ExportDegraded { key: "export_degraded", en: "I accept black frames / silence for the listed source ranges", vi: "Tôi chấp nhận khung hình đen / im lặng cho các đoạn nguồn đã liệt kê" },
    ExportBlocked { key: "export_blocked", en: "Export blocked: restore these sources or explicitly accept degraded output.", vi: "Không thể xuất: khôi phục các nguồn này hoặc chấp nhận xuất suy giảm." },
    ExportEmpty { key: "export_empty", en: "The timeline is empty. Add media before exporting.", vi: "Dòng thời gian trống. Thêm tư liệu trước khi xuất." },
    ExportProgress { key: "export_progress", en: "Exporting…", vi: "Đang xuất…" },
    ExportFailure { key: "export_failure", en: "Export failed. Your previous file is preserved. Retry or check the listed sources.", vi: "Xuất thất bại. Tệp cũ được giữ nguyên. Thử lại hoặc kiểm tra các nguồn đã liệt kê." },
    ExportRetry { key: "export_retry", en: "Retry", vi: "Thử lại" },
    ExportCopy { key: "export_copy", en: "Copy", vi: "Sao chép" },
    ExportSaved { key: "export_saved", en: "Export saved", vi: "Đã lưu bản xuất" },
    ExportCancelling { key: "export_cancelling", en: "Cancelling…", vi: "Đang hủy…" },
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

    // Local models.
    Models { key: "models", en: "Models", vi: "Mô hình" },
    ModelsHint { key: "models_hint", en: "Downloads start only from the button here and never when the app opens or a provider is chosen.", vi: "Tệp chỉ được tải khi bạn bấm nút tại đây, không tự tải khi mở ứng dụng hoặc chọn nhà cung cấp." },
    ModelDefaultPolicy { key: "model_default_policy", en: "Suggested defaults are limited to MIT, Apache-2.0 and BSD licences.", vi: "Mặc định gợi ý chỉ dùng giấy phép MIT, Apache-2.0 và BSD." },
    ModelChoose { key: "model_choose", en: "Choose a model to see its licence, source and files.", vi: "Chọn một mô hình để xem giấy phép, nguồn và các tệp." },
    ModelProvider { key: "model_provider", en: "Provider", vi: "Nhà cung cấp" },
    ModelOnDevice { key: "model_on_device", en: "On-device", vi: "Trên thiết bị" },
    ModelLanguages { key: "model_languages", en: "Languages", vi: "Ngôn ngữ" },
    ModelLanguageNeutral { key: "model_language_neutral", en: "Language-neutral weights", vi: "Trọng số không phụ thuộc ngôn ngữ" },
    ModelSize { key: "model_size", en: "Size", vi: "Kích thước" },
    ModelSizeValue { key: "model_size_value", en: "{size} ({bytes} bytes)", vi: "{size} ({bytes} byte)" },
    ModelLicence { key: "model_licence", en: "Licence", vi: "Giấy phép" },
    ModelSource { key: "model_source", en: "Source", vi: "Nguồn" },
    ModelFiles { key: "model_files", en: "Files and SHA-256", vi: "Tệp và SHA-256" },
    ModelFileLine { key: "model_file_line", en: "{name} · {size}", vi: "{name} · {size}" },
    ModelSuggested { key: "model_suggested", en: "Suggested default", vi: "Mặc định gợi ý" },
    ModelPermissive { key: "model_permissive", en: "Permissive licence eligible as a suggested default. Download still requires your click.", vi: "Giấy phép cho phép dùng làm mặc định gợi ý. Bạn vẫn phải bấm tải xuống." },
    ModelRestrictions { key: "model_restrictions", en: "This licence is not eligible as a suggested default. Review the exact licence and source before opting in.", vi: "Giấy phép này không được dùng làm mặc định gợi ý. Hãy xem đúng giấy phép và nguồn trước khi đồng ý." },
    ModelAccept { key: "model_accept", en: "I have reviewed this licence and want to download these weights", vi: "Tôi đã xem giấy phép này và muốn tải các trọng số này" },
    ModelProgress { key: "model_progress", en: "{model} · {stage}", vi: "{model} · {stage}" },
    ModelDownload { key: "model_download", en: "Download", vi: "Tải xuống" },
    ModelResume { key: "model_resume", en: "Resume", vi: "Tiếp tục" },
    ModelCancel { key: "model_cancel", en: "Cancel download", vi: "Hủy tải xuống" },
    ModelClose { key: "model_close", en: "Close", vi: "Đóng" },
    ModelChecking { key: "model_checking", en: "Checking installed models…", vi: "Đang kiểm tra mô hình đã cài…" },
    ModelNotInstalled { key: "model_not_installed", en: "Not installed", vi: "Chưa cài" },
    ModelInstalled { key: "model_installed", en: "Installed", vi: "Đã cài" },
    ModelQueued { key: "model_queued", en: "Queued", vi: "Đang chờ" },
    ModelDownloading { key: "model_downloading", en: "Downloading…", vi: "Đang tải…" },
    ModelVerifying { key: "model_verifying", en: "Verifying checksum…", vi: "Đang kiểm tra mã băm…" },
    ModelPublishing { key: "model_publishing", en: "Saving model…", vi: "Đang lưu mô hình…" },
    ModelCancelling { key: "model_cancelling", en: "Cancelling…", vi: "Đang hủy…" },
    ModelCancelled { key: "model_cancelled", en: "Download cancelled. Resume to reuse the saved part.", vi: "Đã hủy tải. Tiếp tục để dùng phần đã lưu." },
    ModelFailed { key: "model_failed", en: "Download failed", vi: "Tải xuống thất bại" },
    ModelStorageFull { key: "model_storage_full", en: "Not enough disk space for these weights. Free space and retry.", vi: "Không đủ dung lượng cho các trọng số này. Hãy giải phóng dung lượng và thử lại." },
    ModelIntegrity { key: "model_integrity", en: "The downloaded file failed its checksum and was discarded. Retry.", vi: "Tệp tải về sai mã băm và đã bị bỏ. Hãy thử lại." },
    ModelNetwork { key: "model_network", en: "The download was interrupted. Resume to continue.", vi: "Quá trình tải bị gián đoạn. Tiếp tục để tải tiếp." },
    ModelBusy { key: "model_busy", en: "Another model download is using the cache.", vi: "Một lần tải mô hình khác đang dùng bộ nhớ đệm." },
    ModelCorrupt { key: "model_corrupt", en: "The cached model is damaged. Remove it outside VibeCut or retry after clearing the cache.", vi: "Mô hình trong bộ nhớ đệm bị hỏng. Hãy xóa nó bên ngoài VibeCut hoặc xóa bộ nhớ đệm rồi thử lại." },

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

/// Parse a finite decimal in the same notation as [`decimal`]. Grouping,
/// when present, must use complete groups of three; exponent notation and
/// malformed/mixed separators refuse. Input is bounded to 128 bytes.
pub fn parse_decimal(locale: Locale, input: &str) -> Result<f64> {
    let invalid = || Error::Invalid("localized decimal");
    if input.len() > 128 {
        return Err(invalid());
    }
    let value = input.trim();
    if value.is_empty() {
        return Err(invalid());
    }
    let (grouping, decimal) = separators(locale);
    let (sign, value) = value
        .strip_prefix('-')
        .map_or(("", value), |rest| ("-", rest));
    let mut parts = value.split(decimal);
    let integer = parts.next().ok_or_else(invalid)?;
    let fraction = parts.next();
    if parts.next().is_some()
        || fraction.is_some_and(|s| s.is_empty() || !s.bytes().all(|b| b.is_ascii_digit()))
    {
        return Err(invalid());
    }
    let digits = if integer.contains(grouping) {
        let mut groups = integer.split(grouping);
        let first = groups.next().ok_or_else(invalid)?;
        if first.is_empty()
            || first.len() > 3
            || !first.bytes().all(|b| b.is_ascii_digit())
            || groups.any(|g| g.len() != 3 || !g.bytes().all(|b| b.is_ascii_digit()))
        {
            return Err(invalid());
        }
        integer.replace(grouping, "")
    } else {
        if !integer.bytes().all(|b| b.is_ascii_digit())
            || (integer.is_empty() && fraction.is_none())
        {
            return Err(invalid());
        }
        if integer.is_empty() {
            "0".into()
        } else {
            integer.into()
        }
    };
    let value = std::format!("{sign}{digits}.{}", fraction.unwrap_or("0"))
        .parse::<f64>()
        .map_err(|_| invalid())?;
    if !value.is_finite() {
        return Err(invalid());
    }
    Ok(value)
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

/// Bumped when the local preference document changes incompatibly.
pub const PREFERENCES_VERSION: u32 = 4;
const MAX_PREFERENCES_BYTES: u64 = 16384;

/// Additive preference keys start at their product default when an older
/// same-version document omits them.
const fn default_snapping() -> bool {
    true
}
const MAX_SECTIONS: usize = 128;

/// Splitter bounds are fractions of the full content rectangle. The two side
/// panels plus the preview always fill the upper row, so their limits imply the
/// preview's 30–85% range (side panels keep at least 15% each).
pub const PANEL_MIN: f32 = 0.15;
pub const PANEL_MAX: f32 = 0.40;
pub const PREVIEW_MIN: f32 = 0.30;
pub const PREVIEW_MAX: f32 = 0.85;
pub const TIMELINE_MIN: f32 = 0.15;
pub const TIMELINE_MAX: f32 = 0.70;
// Widths exclude both gutters; row fractions include the area-owned strips.
pub const DEFAULT_PANELS: [f32; 3] = [582.0 / 1696.0, 486.0 / 1696.0, 447.0 / 979.0];

/// Projects page presentation state, persisted with the workspace shell.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ProjectsView {
    #[default]
    Grid,
    List,
}

/// Projects page sort keys. `Modified` is part of the sort menu and info dialog,
/// never displayed on the card.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ProjectsSortKey {
    #[default]
    Created,
    Modified,
    Name,
    Duration,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectsPreferences {
    pub view: ProjectsView,
    pub sort: ProjectsSortKey,
    pub ascending: bool,
}
impl Default for ProjectsPreferences {
    fn default() -> Self {
        Self {
            view: ProjectsView::Grid,
            sort: ProjectsSortKey::Created,
            ascending: false,
        }
    }
}

/// Every user-visible preference key in the one versioned document.
pub const PREFERENCE_KEYS: &[&str] = &[
    "locale",
    "dark",
    "snapping",
    "panels",
    "sections",
    "export",
    "assets_tab",
    "properties_tabs",
    "projects",
    "media_view",
    "media_sort",
    "preview_fit",
    "preview_zoom",
    "keybindings",
    "recent_colors",
];

/// A splitter drag that keeps every region non-empty and inside its limits.
pub fn drag_panel(panels: [f32; 3], axis: usize, delta: f32) -> [f32; 3] {
    let Ok(mut next) = validate_panels(panels) else {
        return DEFAULT_PANELS;
    };
    if !delta.is_finite() {
        return next;
    }
    match axis {
        0 => {
            let max = PANEL_MAX.min(1.0 - PREVIEW_MIN - next[1]);
            next[0] = (next[0] + delta).clamp(PANEL_MIN, max);
        }
        1 => {
            let max = PANEL_MAX.min(1.0 - PREVIEW_MIN - next[0]);
            next[1] = (next[1] - delta).clamp(PANEL_MIN, max);
        }
        2 => next[2] = (next[2] - delta).clamp(TIMELINE_MIN, TIMELINE_MAX),
        _ => return next,
    }
    next
}

pub fn validate_panels(panels: [f32; 3]) -> Result<[f32; 3]> {
    if panels.iter().any(|value| !value.is_finite()) {
        return Err(Error::Invalid("panel ratios"));
    }
    let [assets, properties, timeline] = panels;
    let preview = 1.0 - assets - properties;
    if !(PANEL_MIN..=PANEL_MAX).contains(&assets)
        || !(PANEL_MIN..=PANEL_MAX).contains(&properties)
        // Subtracting both side ratios can round one ULP below the limit.
        || !((PREVIEW_MIN - f32::EPSILON)..=PREVIEW_MAX).contains(&preview)
        || !(TIMELINE_MIN..=TIMELINE_MAX).contains(&timeline)
    {
        return Err(Error::Invalid("panel ratios"));
    }
    Ok(panels)
}

/// Native logical-pixel geometry, outside project content and undo.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WindowGeometry {
    pub size: [f32; 2],
    pub position: Option<[f32; 2]>,
    pub maximized: bool,
}
impl Default for WindowGeometry {
    fn default() -> Self {
        Self {
            size: [1440.0, 900.0],
            position: None,
            maximized: false,
        }
    }
}
impl WindowGeometry {
    fn valid(&self) -> bool {
        self.size
            .iter()
            .all(|v| v.is_finite() && (320.0..=16384.0).contains(v))
            && self
                .position
                .is_none_or(|p| p.iter().all(|v| v.is_finite() && v.abs() <= 65536.0))
    }
    pub fn builder(&self) -> crate::egui::ViewportBuilder {
        let mut builder = crate::egui::ViewportBuilder::default()
            .with_inner_size(self.size)
            .with_min_inner_size([1280.0, 800.0])
            .with_maximized(self.maximized);
        if let Some(position) = self.position {
            builder = builder.with_position(position);
        }
        builder
    }
    pub fn read(ui: &crate::egui::Ui) -> Option<Self> {
        ui.input(|i| {
            let viewport = i.viewport();
            let inner = viewport.inner_rect?;
            let geometry = Self {
                size: [inner.width(), inner.height()],
                position: viewport.outer_rect.map(|r| [r.min.x, r.min.y]),
                maximized: viewport.maximized.unwrap_or(false),
            };
            geometry.valid().then_some(geometry)
        })
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Preferences {
    /// Missing versions are refused as version zero.
    #[serde(default)]
    pub version: u32,
    #[serde(default)]
    pub windows: BTreeMap<String, WindowGeometry>,
    #[serde(default)]
    pub export: crate::export::ExportOptions,
    #[serde(default)]
    pub export_directory: Option<PathBuf>,
    #[serde(default)]
    pub subtitles: crate::subtitles::Options,
    pub locale: Locale,
    pub dark: bool,
    /// Timeline snapping; on by default and persisted across runs.
    #[serde(default = "default_snapping")]
    pub snapping: bool,
    /// Ratios of the full content rectangle; validated when loaded and set.
    pub panels: [f32; 3],
    /// Open/closed state by property-section key.
    #[serde(default)]
    pub sections: BTreeMap<String, bool>,
    /// Last assets-panel tab. Presentation state, never a project edit.
    #[serde(default)]
    pub assets_tab: AssetsTab,
    /// Last properties tab by persisted element-kind id. Applying it to a kind
    /// that does not offer that tab falls back to the kind's default.
    #[serde(default)]
    pub properties_tabs: BTreeMap<String, String>,
    /// Projects page view mode and sort order; presentation only.
    #[serde(default)]
    pub projects: ProjectsPreferences,
    /// Media library grid/list presentation.
    #[serde(default)]
    pub media_view: MediaView,
    /// Media library sort key and direction.
    #[serde(default)]
    pub media_sort: MediaSort,
    /// Preview follows the panel size when true; otherwise `preview_zoom` applies.
    #[serde(default = "default_preview_fit")]
    pub preview_fit: bool,
    /// Explicit preview scale where 1.0 is 100%; clamped to 0.25–16.
    #[serde(default = "default_preview_zoom")]
    pub preview_zoom: f64,
    /// Last directory used by the snapshot save dialog.
    #[serde(default)]
    pub snapshot_directory: Option<PathBuf>,
    /// App-local action overrides. Defaults and action metadata live in
    /// `actions`; this is the only persisted binding map.
    #[serde(default)]
    pub keybindings: KeybindingPreferences,
    #[serde(default)]
    pub recent_colors: Vec<project::Color>,
}
pub(crate) const fn default_preview_fit() -> bool {
    true
}
pub(crate) const fn default_preview_zoom() -> f64 {
    1.0
}
impl Default for Preferences {
    fn default() -> Self {
        Self {
            version: PREFERENCES_VERSION,
            windows: BTreeMap::new(),
            export: crate::export::ExportOptions::default(),
            export_directory: None,
            subtitles: crate::subtitles::Options::default(),
            locale: Locale::En,
            dark: true,
            snapping: true,
            panels: DEFAULT_PANELS,
            sections: BTreeMap::new(),
            assets_tab: AssetsTab::default(),
            properties_tabs: BTreeMap::new(),
            projects: ProjectsPreferences::default(),
            media_view: MediaView::default(),
            media_sort: MediaSort::default(),
            preview_fit: default_preview_fit(),
            preview_zoom: default_preview_zoom(),
            snapshot_directory: None,
            keybindings: KeybindingPreferences::default(),
            recent_colors: Vec::new(),
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
    pub fn validate(&self) -> Result<()> {
        if self.version != PREFERENCES_VERSION {
            return Err(Error::Unsupported("preferences version"));
        }
        if self.windows.len() > 64 || self.windows.iter().any(|(k, v)| k.len() > 64 || !v.valid()) {
            return Err(Error::Invalid("window geometry"));
        }
        validate_panels(self.panels)?;
        self.subtitles.validate()?;
        self.validate_sections()?;
        self.validate_properties_tabs()?;
        if !self.preview_zoom.is_finite()
            || !(f64::from(crate::preview::MIN_ZOOM)..=f64::from(crate::preview::MAX_ZOOM))
                .contains(&self.preview_zoom)
        {
            return Err(Error::Invalid("preview zoom"));
        }
        if self
            .snapshot_directory
            .as_ref()
            .is_some_and(|path| path.as_os_str().len() > crate::preview::MAX_PATH_BYTES)
        {
            return Err(Error::Invalid("snapshot directory"));
        }
        self.keybindings.validate()?;
        if self.recent_colors.len() > 16 {
            return Err(Error::Invalid("recent colours"));
        }
        if serde_json::to_vec(self)?.len() as u64 > MAX_PREFERENCES_BYTES {
            return Err(Error::Invalid("preference size"));
        }
        Ok(())
    }
    fn validate_properties_tabs(&self) -> Result<()> {
        if self.properties_tabs.len() > ElementKind::ALL.len() {
            return Err(Error::Invalid("preference tabs"));
        }
        for (kind, tab) in &self.properties_tabs {
            let kind = ElementKind::from_id(kind).ok_or(Error::Invalid("preference tabs"))?;
            let tab = PropertiesTab::from_id(tab).ok_or(Error::Invalid("preference tabs"))?;
            if !kind.tabs().contains(&tab) {
                return Err(Error::Invalid("preference tabs"));
            }
        }
        Ok(())
    }
    fn validate_sections(&self) -> Result<()> {
        if self.sections.len() > MAX_SECTIONS
            || self
                .sections
                .keys()
                .any(|key| key.is_empty() || key.len() > 64)
        {
            return Err(Error::Invalid("preference sections"));
        }
        Ok(())
    }
    pub fn section_open(&self, key: &str, default: bool) -> bool {
        self.sections.get(key).copied().unwrap_or(default)
    }
    /// Typed `ui.prefs.set` payload access used by the app history.
    pub fn get(&self, key: &str) -> Result<serde_json::Value> {
        match key {
            "locale" => Ok(serde_json::to_value(self.locale)?),
            "dark" => Ok(serde_json::Value::Bool(self.dark)),
            "snapping" => Ok(serde_json::Value::Bool(self.snapping)),
            "panels" => Ok(serde_json::to_value(self.panels)?),
            "sections" => Ok(serde_json::to_value(&self.sections)?),
            "export" => Ok(serde_json::to_value(self.export)?),
            "assets_tab" => Ok(serde_json::to_value(self.assets_tab)?),
            "properties_tabs" => Ok(serde_json::to_value(&self.properties_tabs)?),
            "subtitles" => Ok(serde_json::to_value(&self.subtitles)?),
            "projects" => Ok(serde_json::to_value(self.projects)?),
            "media_view" => Ok(serde_json::to_value(self.media_view)?),
            "media_sort" => Ok(serde_json::to_value(self.media_sort)?),
            "preview_fit" => Ok(serde_json::Value::Bool(self.preview_fit)),
            "preview_zoom" => Ok(serde_json::json!(self.preview_zoom)),
            "snapshot_directory" => Ok(serde_json::to_value(&self.snapshot_directory)?),
            "keybindings" => Ok(serde_json::to_value(&self.keybindings)?),
            "recent_colors" => Ok(serde_json::to_value(&self.recent_colors)?),
            _ => Err(Error::Unsupported("preference key")),
        }
    }
    pub fn set(&mut self, key: &str, value: serde_json::Value) -> Result<()> {
        let mut next = self.clone();
        next.set_value(key, value)?;
        next.validate()?;
        *self = next;
        Ok(())
    }
    fn set_value(&mut self, key: &str, value: serde_json::Value) -> Result<()> {
        match key {
            "locale" => self.locale = serde_json::from_value(value)?,
            "dark" => self.dark = serde_json::from_value(value)?,
            "snapping" => self.snapping = serde_json::from_value(value)?,
            "panels" => self.panels = validate_panels(serde_json::from_value(value)?)?,
            "sections" => {
                self.sections = serde_json::from_value(value)?;
            }
            "export" => self.export = serde_json::from_value(value)?,
            "assets_tab" => self.assets_tab = serde_json::from_value(value)?,
            "properties_tabs" => self.properties_tabs = serde_json::from_value(value)?,
            "subtitles" => self.subtitles = serde_json::from_value(value)?,
            "projects" => self.projects = serde_json::from_value(value)?,
            "media_view" => self.media_view = serde_json::from_value(value)?,
            "media_sort" => self.media_sort = serde_json::from_value(value)?,
            "preview_fit" => self.preview_fit = serde_json::from_value(value)?,
            "preview_zoom" => self.preview_zoom = serde_json::from_value(value)?,
            "snapshot_directory" => self.snapshot_directory = serde_json::from_value(value)?,
            "keybindings" => self.keybindings = serde_json::from_value(value)?,
            "recent_colors" => self.recent_colors = serde_json::from_value(value)?,
            _ => return Err(Error::Unsupported("preference key")),
        }
        Ok(())
    }
}

/// Which part of a damaged preference document was replaced by defaults.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PreferenceIssue {
    Read,
    Version,
    Layout,
    Keybindings,
}
impl std::fmt::Display for PreferenceIssue {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::Read => "preferences could not be read",
            Self::Version => "preferences use an unsupported version",
            Self::Layout => "layout preferences were reset",
            Self::Keybindings => "invalid keybindings were removed",
        })
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
    /// Strict read used by tests and callers that must refuse a damaged file.
    pub fn load(&self) -> Result<Preferences> {
        let value = self.read()?;
        value.validate()?;
        Ok(value)
    }
    /// Startup read: a damaged document is replaced wholesale, while a damaged
    /// layout keeps unrelated preferences and is reported through the warning.
    pub fn load_with_warning(&self) -> (Preferences, Option<PreferenceIssue>) {
        let mut value = match self.read() {
            Ok(value) => value,
            Err(_) => return (Preferences::system(), Some(PreferenceIssue::Read)),
        };
        if value.version != PREFERENCES_VERSION {
            return (Preferences::system(), Some(PreferenceIssue::Version));
        }
        let mut issue = None;
        if value.windows.len() > 64
            || value
                .windows
                .iter()
                .any(|(key, geometry)| key.len() > 64 || !geometry.valid())
        {
            value.windows.clear();
            issue = Some(PreferenceIssue::Layout);
        }
        if validate_panels(value.panels).is_err() {
            value.panels = DEFAULT_PANELS;
            issue = Some(PreferenceIssue::Layout);
        }
        if value.validate_sections().is_err() {
            value.sections.clear();
            issue = Some(PreferenceIssue::Layout);
        }
        if value.validate_properties_tabs().is_err() {
            value.properties_tabs.clear();
            issue = Some(PreferenceIssue::Layout);
        }
        if value.keybindings.validate().is_err() {
            let (sanitized, _) = std::mem::take(&mut value.keybindings).sanitize();
            value.keybindings = sanitized;
            issue = Some(PreferenceIssue::Keybindings);
        }
        if value.recent_colors.len() > 16 {
            value.recent_colors.clear();
            issue = Some(PreferenceIssue::Layout);
        }
        (value, issue)
    }
    fn read(&self) -> Result<Preferences> {
        let file = match std::fs::File::open(&self.path) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Preferences::system());
            }
            Err(error) => return Err(error.into()),
        };
        if file.metadata()?.len() > MAX_PREFERENCES_BYTES {
            return Err(Error::Invalid("preference size"));
        }
        let mut bytes = Vec::new();
        file.take(MAX_PREFERENCES_BYTES + 1)
            .read_to_end(&mut bytes)?;
        if bytes.len() as u64 > MAX_PREFERENCES_BYTES {
            return Err(Error::Invalid("preference size"));
        }
        Ok(serde_json::from_slice(&bytes)?)
    }
    pub fn save(&self, value: &Preferences) -> Result<()> {
        value.validate()?;
        if let Ok(previous) = self.read()
            && previous.version != PREFERENCES_VERSION
        {
            return Err(Error::Unsupported("preferences version"));
        }
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        project::atomic_write(&self.path, &serde_json::to_vec(value)?)?;
        Ok(())
    }
}

/// Export measurements stay in the catalog alongside their unit and locale.
pub fn export_duration(locale: Locale, seconds: f64) -> String {
    let label = text(locale, Text::ExportDuration);
    let number = export_decimal(locale, seconds, 2);
    match locale {
        Locale::En => format!("{label}: {number} s"),
        Locale::Vi => format!("{label}: {number} giây"),
    }
}
pub fn export_range(locale: Locale, start: f64, end: f64) -> String {
    let label = text(locale, Text::SourceRange);
    format!(
        "{label}: {}–{}",
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

/// Localized label; persisted blend IDs are locale-independent.
pub fn blend_label(mode: project::BlendMode) -> Text {
    match mode {
        project::BlendMode::Normal => Text::BlendNormal,
        project::BlendMode::Darken => Text::BlendDarken,
        project::BlendMode::Multiply => Text::BlendMultiply,
        project::BlendMode::ColorBurn => Text::BlendColorBurn,
        project::BlendMode::Lighten => Text::BlendLighten,
        project::BlendMode::Screen => Text::BlendScreen,
        project::BlendMode::PlusLighter => Text::BlendPlusLighter,
        project::BlendMode::ColorDodge => Text::BlendColorDodge,
        project::BlendMode::Overlay => Text::BlendOverlay,
        project::BlendMode::SoftLight => Text::BlendSoftLight,
        project::BlendMode::HardLight => Text::BlendHardLight,
        project::BlendMode::Difference => Text::BlendDifference,
        project::BlendMode::Exclusion => Text::BlendExclusion,
        project::BlendMode::Hue => Text::BlendHue,
        project::BlendMode::Saturation => Text::BlendSaturation,
        project::BlendMode::Color => Text::BlendColor,
        project::BlendMode::Luminosity => Text::BlendLuminosity,
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::panic)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    fn valid_preview(panels: [f32; 3]) -> bool {
        (PREVIEW_MIN..=PREVIEW_MAX).contains(&(1.0 - panels[0] - panels[1]))
    }

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
        for (locale, input, expected) in [
            (Locale::En, "1,014.500", 1014.5),
            (Locale::Vi, "1.014,500", 1014.5),
            (Locale::En, ".5", 0.5),
            (Locale::Vi, ",5", 0.5),
            (Locale::En, "-1,014.500", -1014.5),
            (Locale::Vi, "-1.014,500", -1014.5),
        ] {
            assert_eq!(parse_decimal(locale, input).unwrap(), expected);
        }
        for locale in [Locale::En, Locale::Vi] {
            for input in ["", "NaN", "inf", "1e3", "1.01,4", "1,,000", "1..000", "--1"] {
                assert!(parse_decimal(locale, input).is_err(), "{locale:?} {input}");
            }
            assert!(parse_decimal(locale, &"1".repeat(129)).is_err());
        }
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
        assert_eq!(Preferences::default().panels, DEFAULT_PANELS);
        assert!(Preferences::default().snapping);
        assert_eq!(Preferences::system().locale, Locale::from_system());
    }

    #[test]
    fn splitter_drags_clamp_each_region_to_its_documented_limits() {
        let start = DEFAULT_PANELS;
        let far_right = drag_panel(start, 0, 10.0);
        assert_eq!(far_right[0], PANEL_MAX);
        assert!(valid_preview(far_right));
        let far_left = drag_panel(start, 0, -10.0);
        assert_eq!(far_left[0], PANEL_MIN);
        assert!(valid_preview(far_left));
        let inspector_narrow = drag_panel(start, 1, 10.0);
        assert_eq!(inspector_narrow[1], PANEL_MIN);
        let inspector_wide = drag_panel(start, 1, -10.0);
        assert_eq!(
            inspector_wide[1],
            PANEL_MAX.min(1.0 - PREVIEW_MIN - start[0])
        );
        assert!(valid_preview(inspector_wide));
        let timeline_high = drag_panel(start, 2, -10.0);
        assert_eq!(timeline_high[2], TIMELINE_MAX);
        let timeline_low = drag_panel(start, 2, 10.0);
        assert_eq!(timeline_low[2], TIMELINE_MIN);
        assert_eq!(drag_panel(start, 7, 0.1), start);
        assert_eq!(drag_panel(start, 0, f32::NAN), start);
        assert!(validate_panels([0.40, 0.40, 0.50]).is_err());
        assert!(validate_panels([f32::NAN, 0.25, 0.50]).is_err());
        assert!(validate_panels([0.25, 0.25, 0.80]).is_err());
    }

    #[test]
    fn every_registered_preference_key_roundtrips_through_the_store() {
        let dir = tempfile::tempdir().expect("temporary preferences");
        let store = PreferenceStore::new(dir.path().join("preferences.json"));
        let mut preferences = Preferences::default();
        for (key, value) in [
            ("locale", serde_json::json!("vi")),
            ("dark", serde_json::json!(false)),
            ("snapping", serde_json::json!(false)),
            ("panels", serde_json::json!([0.30, 0.20, 0.35])),
            ("sections", serde_json::json!({"inspector.canvas": false})),
            ("assets_tab", serde_json::json!("captions")),
            ("properties_tabs", serde_json::json!({"text": "transform"})),
            ("media_view", serde_json::json!("list")),
            (
                "media_sort",
                serde_json::json!({"key": "size", "order": "desc"}),
            ),
            (
                "projects",
                serde_json::json!({"view": "list", "sort": "duration", "ascending": true}),
            ),
            (
                "keybindings",
                serde_json::json!({
                    "bindings": {"timeline.snap": ["Ctrl+Shift+N"]},
                    "isCustomized": ["timeline.snap"]
                }),
            ),
            (
                "recent_colors",
                serde_json::json!([[10, 20, 30, 0], [255, 0, 100, 128]]),
            ),
            (
                "export",
                serde_json::json!({
                    "format": "webm",
                    "quality": "very_high",
                    "include_audio": false
                }),
            ),
        ] {
            assert!(PREFERENCE_KEYS.contains(&key), "unregistered key {key}");
            preferences.set(key, value).expect("set preference");
        }
        store.save(&preferences).expect("save preferences");
        let loaded = store.load().expect("load preferences");
        assert_eq!(loaded.version, PREFERENCES_VERSION);
        for key in PREFERENCE_KEYS {
            assert_eq!(
                loaded.get(key).expect("get preference"),
                preferences.get(key).expect("get preference"),
                "{key} must round-trip"
            );
        }
        assert!(Preferences::default().get("unknown").is_err());
    }

    #[test]
    fn corrupt_layout_falls_back_to_defaults_with_a_warning() {
        let dir = tempfile::tempdir().expect("temporary preferences");
        let store = PreferenceStore::new(dir.path().join("preferences.json"));
        std::fs::write(
            store.path(),
            format!(r#"{{"version":{PREFERENCES_VERSION},"locale":"vi","dark":false,"panels":[0.9,0.9,0.9]}}"#),
        )
        .expect("write preferences");
        let (preferences, issue) = store.load_with_warning();
        assert_eq!(issue, Some(PreferenceIssue::Layout));
        assert_eq!(preferences.panels, DEFAULT_PANELS);
        assert_eq!(preferences.locale, Locale::Vi);
        assert!(!preferences.dark);
        std::fs::write(store.path(), b"{").expect("write corrupt preferences");
        let (preferences, issue) = store.load_with_warning();
        assert_eq!(issue, Some(PreferenceIssue::Read));
        assert_eq!(preferences, Preferences::system());
        std::fs::write(
            store.path(),
            br#"{"version":9,"locale":"en","dark":true,"panels":[0.25,0.25,0.5]}"#,
        )
        .expect("write newer preferences");
        assert_eq!(store.load_with_warning().1, Some(PreferenceIssue::Version));
    }

    #[test]
    fn same_version_document_without_snapping_keeps_the_default_on() {
        let dir = tempfile::tempdir().expect("temporary preferences");
        let store = PreferenceStore::new(dir.path().join("preferences.json"));
        let original = format!(
            r#"{{"version":{PREFERENCES_VERSION},"locale":"en","dark":true,"panels":[0.25,0.25,0.5]}}"#
        ).into_bytes();
        std::fs::write(store.path(), &original).expect("write preferences");
        let loaded = store.load().expect("load preferences");
        assert!(loaded.snapping);
        let bytes = std::fs::read(store.path()).expect("read preferences");
        assert_eq!(
            bytes, original,
            "loading an additive default must not rewrite the document"
        );
    }

    #[test]
    fn older_preference_version_is_refused_and_bytes_are_retained() {
        let dir = tempfile::tempdir().expect("temporary preferences");
        let store = PreferenceStore::new(dir.path().join("preferences.json"));
        std::fs::write(
            store.path(),
            br#"{"version":1,"locale":"vi","dark":false,"panels":[0.25,0.25,0.5]}"#,
        )
        .expect("write old preferences");
        let bytes = std::fs::read(store.path()).expect("read old preferences");
        assert!(matches!(
            store.load(),
            Err(Error::Unsupported("preferences version"))
        ));
        assert_eq!(store.load_with_warning().1, Some(PreferenceIssue::Version));
        assert!(store.save(&Preferences::default()).is_err());
        assert_eq!(std::fs::read(store.path()).expect("retained bytes"), bytes);
    }

    #[test]
    fn media_preferences_refuse_unknown_values_atomically() {
        let mut preferences = Preferences::default();
        let before = preferences.clone();
        for (key, value) in [
            ("media_view", serde_json::json!("columns")),
            (
                "media_sort",
                serde_json::json!({"key": "bogus", "order": "asc"}),
            ),
            (
                "media_sort",
                serde_json::json!({"key": "name", "order": "bogus"}),
            ),
        ] {
            assert!(preferences.set(key, value).is_err(), "{key}");
            assert_eq!(preferences, before, "{key} must fail atomically");
        }
    }

    #[test]
    fn tab_preferences_validate_kind_and_tab_ids_atomically() {
        let mut preferences = Preferences::default();
        preferences
            .set(
                "properties_tabs",
                serde_json::json!({
                    "text": "transform",
                    "video": "audio",
                    "graphic": "graphic"
                }),
            )
            .expect("valid tab memory");
        assert_eq!(
            preferences.properties_tabs.get("text").map(String::as_str),
            Some("transform")
        );
        for value in [
            serde_json::json!({"unknown": "transform"}),
            serde_json::json!({"text": "masks"}),
            serde_json::json!({"text": "unknown"}),
        ] {
            let before = preferences.clone();
            assert!(preferences.set("properties_tabs", value).is_err());
            assert_eq!(preferences, before);
        }
        let mut unknown = Preferences::default();
        unknown
            .properties_tabs
            .insert("text".into(), "masks".into());
        let dir = tempfile::tempdir().expect("temporary preferences");
        let store = PreferenceStore::new(dir.path().join("preferences.json"));
        std::fs::write(
            store.path(),
            serde_json::to_vec(&Preferences {
                properties_tabs: unknown.properties_tabs,
                ..Default::default()
            })
            .expect("encode preferences"),
        )
        .expect("write preferences");
        let (loaded, issue) = store.load_with_warning();
        assert_eq!(issue, Some(PreferenceIssue::Layout));
        assert!(loaded.properties_tabs.is_empty());
        assert_eq!(loaded.assets_tab, AssetsTab::Media);
    }

    #[test]
    fn repeated_splitter_drags_always_produce_valid_preferences() {
        // Unequal side panels expose subtraction round-off at the preview minimum.
        let boundary = drag_panel([0.37875625, 0.25, 0.5], 1, -10.0);
        assert!(validate_panels(boundary).is_ok(), "{boundary:?}");
        for first in [0, 1] {
            let mut panels =
                drag_panel(DEFAULT_PANELS, first, if first == 0 { 10.0 } else { -10.0 });
            for axis in [1 - first, first, 2, 0, 1] {
                for delta in [-10.0, 0.00001, 10.0, -0.00001] {
                    panels = drag_panel(panels, axis, delta);
                    assert!(validate_panels(panels).is_ok(), "{panels:?}");
                }
            }
        }
    }

    #[test]
    fn oversized_preference_save_preserves_previous_document() {
        let dir = tempfile::tempdir().unwrap();
        let store = PreferenceStore::new(dir.path().join("preferences.json"));
        let mut preferences = Preferences::default();
        store.save(&preferences).unwrap();
        for index in 0..128 {
            preferences.sections.insert(format!("{index:064}"), false);
        }
        for index in 0..64 {
            preferences
                .windows
                .insert(format!("{index:064}"), WindowGeometry::default());
        }
        assert!(store.save(&preferences).is_err());
        assert_eq!(store.load().unwrap(), Preferences::default());
    }

    #[test]
    fn recent_colours_refuse_over_capacity_without_changing_preferences() {
        let mut preferences = Preferences::default();
        let recent = (0..16)
            .map(|r| project::Color([r, 0, 0, r]))
            .collect::<Vec<_>>();
        preferences
            .set("recent_colors", serde_json::to_value(&recent).unwrap())
            .unwrap();
        let before = preferences.clone();
        let mut oversized = recent;
        oversized.push(project::Color([255; 4]));
        assert!(
            preferences
                .set("recent_colors", serde_json::to_value(oversized).unwrap())
                .is_err()
        );
        assert_eq!(preferences, before);
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
