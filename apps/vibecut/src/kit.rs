//! Supplies reusable visual primitives without editor state.
mod appearance;
mod dialog;
mod feedback;
mod icons;
mod style;
mod theme;
mod widgets;

pub use appearance::Look;
pub(crate) use dialog::{dialog_button, dialog_frame, dialog_title};
pub(crate) use feedback::{Tone, panel_state, progress_bar, toast, toast_top};
pub(crate) use icons::{Icon, icon, install, tinted};
pub(crate) use style::{install_editor_density, install_fonts, install_theme};
pub(crate) use theme::{
    ADJUSTMENT_CLIP_DARK, AUDIO_CLIP_DARK, EFFECT_CLIP_DARK, FILTER_CLIP_DARK, STICKER_CLIP_DARK,
    TEXT_CLIP_DARK, VIDEO_FILL_DARK, accent, audio_waveform, border, bright, disabled,
    disabled_icon, error_text, field, guide_line, hover, icon, image_tint, label_font, light_clip,
    missing_band, missing_body, missing_border, missing_canvas, modal, on_accent, on_missing,
    on_scrim, panel, preview_backdrop, radius, scrim, secondary, segment_active, segment_outline,
    separator, size, space, strip, success, text, toggle_knob, track_rest, type_size, video_border,
    video_fill, warning, waveform, window,
};
pub(crate) use widgets::{
    KeyframeControl, action_button, badge_button, checkbox, dashed_outline, icon_button, icon_tab,
    keyframe_cluster, rail_slider, rail_stops_slider, section_header_with, segmented, stepper,
    text_tabs, toggle, transport_button, value_field, zoom_rail,
};
