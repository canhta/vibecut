use crate::{Error, Result, egui};
fn svg(name: &str) -> &'static [u8] {
    match name {
        "film-strip" => include_bytes!("../../../assets/icons/film-strip.svg"),
        "music-notes" => include_bytes!("../../../assets/icons/music-notes.svg"),
        "text-t" => include_bytes!("../../../assets/icons/text-t.svg"),
        "sticker" => include_bytes!("../../../assets/icons/sticker.svg"),
        "sparkle" => include_bytes!("../../../assets/icons/sparkle.svg"),
        "funnel" => include_bytes!("../../../assets/icons/funnel.svg"),
        "sliders-horizontal" => include_bytes!("../../../assets/icons/sliders-horizontal.svg"),
        "speaker-high" => include_bytes!("../../../assets/icons/speaker-high.svg"),
        "speaker-slash" => include_bytes!("../../../assets/icons/speaker-slash.svg"),
        "eye" => include_bytes!("../../../assets/icons/eye.svg"),
        "eye-slash" => include_bytes!("../../../assets/icons/eye-slash.svg"),
        "magnet" => include_bytes!("../../../assets/icons/magnet.svg"),
        "trash" => include_bytes!("../../../assets/icons/trash.svg"),
        _ => include_bytes!("../../../assets/icons/film-strip.svg"),
    }
}
fn raster(name: &str, pixels: u32) -> Result<egui::ColorImage> {
    let tree = resvg::usvg::Tree::from_data(svg(name), &resvg::usvg::Options::default())
        .map_err(|_| Error::Unsupported("invalid embedded icon"))?;
    let mut pixmap = resvg::tiny_skia::Pixmap::new(pixels, pixels)
        .ok_or(Error::Unsupported("icon dimensions"))?;
    let transform = resvg::tiny_skia::Transform::from_scale(
        pixels as f32 / tree.size().width(),
        pixels as f32 / tree.size().height(),
    );
    resvg::render(&tree, transform, &mut pixmap.as_mut());
    Ok(egui::ColorImage::from_rgba_premultiplied(
        [pixels as usize, pixels as usize],
        pixmap.data(),
    ))
}
/// Cache embedded vectors at the current display density; retain handles with the context.
pub(crate) fn image(ui: &egui::Ui, name: &str) -> egui::ImageSource<'static> {
    let pixels = (16.0 * ui.ctx().pixels_per_point())
        .ceil()
        .clamp(16.0, 128.0) as u32;
    let id = egui::Id::unique(("track.icon", name, pixels));
    let texture = ui
        .ctx()
        .data_mut(|data| data.get_temp::<egui::TextureHandle>(id));
    let texture = texture.unwrap_or_else(|| {
        let image = raster(name, pixels).unwrap_or_else(|error| {
            tracing::error!(stage="embedded-icon",category=%error);
            egui::ColorImage::filled([pixels as usize, pixels as usize], egui::Color32::MAGENTA)
        });
        let texture = ui.ctx().load_texture(
            format!("track.icon.{name}.{pixels}"),
            image,
            egui::TextureOptions::LINEAR,
        );
        ui.ctx()
            .data_mut(|data| data.insert_temp(id, texture.clone()));
        texture
    });
    (texture.id(), egui::Vec2::splat(16.0)).into()
}
