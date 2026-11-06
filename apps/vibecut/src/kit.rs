//! Supplies reusable visual primitives without editor state.
mod icons;
mod style;
mod theme;
mod widgets;

pub(crate) use icons::{Icon, icon, install, tinted};
pub(crate) use style::{install_editor_density, install_fonts, install_theme};
pub(crate) use theme::{
    accent, border, bright, disabled, disabled_icon, error_text, field, hover, icon, image_tint,
    label_font, modal, on_accent, on_scrim, panel, preview_backdrop, radius, scrim, secondary,
    separator, size, space, strip, success, text, track_rest, type_size, video_border, video_fill,
    warning, window,
};
#[allow(unused_imports)] // Shared contract consumed by the parallel area tickets.
pub(crate) use widgets::{
    NumberSpec, SectionResponse, SliderResponse, action_button, icon_button, property_row,
    rail_slider, section_header, section_header_with, segmented, slider_stepper, stepper, tab_strip,
    text_tabs, toggle, value_field,
};
