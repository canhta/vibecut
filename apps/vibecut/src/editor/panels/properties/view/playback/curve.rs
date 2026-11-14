//! The Speed tab's Curve view: the preset tiles and the speed plot.
use super::super::Panel;
use crate::editor::properties::{Action, ItemId, Text, egui, read, theme};
use project::{SPEED_SPAN_TOTAL, SpeedPoint, SpeedPreset};
use time::Speed;

/// Preset tiles (capture 100057, logical px): 62 px squares 16 px apart, six to a row, the
/// name beneath in a 23 px band (the 85 px row pitch).
const TILE: f32 = 62.0;
const TILE_GAP: f32 = 16.0;
const NAME_BAND: f32 = 23.0;
const MAX_COLUMNS: usize = 6;
/// Gap from the last tile row to the plot box (capture 100104).
const PLOT_GAP: f32 = 24.0;
/// The plot box: a 35 px Duration header, the 146 px plot, a 34 px footer with the buttons.
const HEADER: f32 = 35.0;
const PLOT_HEIGHT: f32 = 146.0;
const FOOTER: f32 = 34.0;
const BOX_PAD: f32 = 10.0;
/// Reset is 72 px wide; the point button 20 px (their height is not measured, 24 used).
const RESET_WIDTH: f32 = 72.0;
const POINT_BUTTON_WIDTH: f32 = 20.0;
const BUTTON_HEIGHT: f32 = 24.0;
/// The Duration line: the source length starts 88 px in, its dashed run 116 px in and stops
/// 40 px before the right edge.
const SOURCE_X: f32 = 88.0;
const DASH_START_X: f32 = 116.0;
const DASH_END_INSET: f32 = 40.0;
/// Point handles: a 10 x 14 pill, a 5.5 px disc when selected, a 18 x 22 hit box.
const PILL: egui::Vec2 = egui::vec2(10.0, 14.0);
const DISC_RADIUS: f32 = 5.5;
const HIT: egui::Vec2 = egui::vec2(18.0, 22.0);
/// The plot's log axis spans 0.1x to 10x; its dashed guides sit at 2x and 0.5x. A tile plots
/// 55 px a decade (capture 100057).
const ICON_DECADE: f32 = 55.0;
const GUIDES: [f64; 2] = [2.0, 0.5];
const MIN_SHOWN: f64 = 0.1;
const MAX_SHOWN: f64 = 10.0;
/// Closest two points may sit, in thousandths of the output span.
const MIN_GAP: u16 = 10;
const SAMPLES: usize = 96;
const ICON_SAMPLES: usize = 32;

/// The drag in flight: the whole shape the moved point produces.
#[derive(Clone)]
struct Drag {
    /// From the pointer to the handle's centre where the drag took hold.
    grab: egui::Vec2,
    points: Vec<SpeedPoint>,
}

/// What one frame of the plot gathered for the footer.
struct Edit<'a> {
    points: &'a [SpeedPoint],
    selected: Option<usize>,
    commit: Option<Vec<SpeedPoint>>,
}

/// Automation word and label of a preset tile; `None` is the tile that removes the curve.
fn tile_entry(preset: Option<SpeedPreset>) -> (&'static str, Text) {
    match preset {
        None => ("none", Text::SpeedPresetNone),
        Some(SpeedPreset::Custom) => ("custom", Text::SpeedPresetCustom),
        Some(SpeedPreset::Montage) => ("montage", Text::SpeedPresetMontage),
        Some(SpeedPreset::Hero) => ("hero", Text::SpeedPresetHero),
        Some(SpeedPreset::Bullet) => ("bullet", Text::SpeedPresetBullet),
        Some(SpeedPreset::JumpCut) => ("jump_cut", Text::SpeedPresetJumpCut),
        Some(SpeedPreset::FlashIn) => ("flash_in", Text::SpeedPresetFlashIn),
        Some(SpeedPreset::FlashOut) => ("flash_out", Text::SpeedPresetFlashOut),
    }
}

/// The preset a shape shows as: the named shape it equals, else Custom; `None` without a curve.
fn held_preset(shape: Option<&[SpeedPoint]>) -> Option<SpeedPreset> {
    let shape = shape?;
    Some(
        SpeedPreset::ALL
            .into_iter()
            .filter(|preset| *preset != SpeedPreset::Custom)
            .find(|preset| preset.points() == shape)
            .unwrap_or(SpeedPreset::Custom),
    )
}

/// How many tiles fit a row of `width`: the measured six, fewer in a narrow body.
fn columns(width: f32) -> usize {
    let fit = ((width + TILE_GAP) / (TILE + TILE_GAP)).floor();
    if fit.is_finite() && fit >= 1.0 {
        (fit as usize).min(MAX_COLUMNS)
    } else {
        1
    }
}

fn plot_x(rect: egui::Rect, at: u16) -> f32 {
    rect.left() + rect.width() * f32::from(at) / f32::from(SPEED_SPAN_TOTAL)
}

/// Speed to height on the log axis, 1x in the middle, 10x at the top and 0.1x at the bottom.
fn plot_y(rect: egui::Rect, speed: f64) -> f32 {
    let log = (speed.log10() as f32).clamp(-1.0, 1.0);
    rect.center().y - log * rect.height() / 2.0
}

fn speed_at_y(rect: egui::Rect, y: f32) -> f64 {
    let log = ((rect.center().y - y) / (rect.height() / 2.0)).clamp(-1.0, 1.0);
    10f64.powf(f64::from(log))
}

fn position_at_x(rect: egui::Rect, x: f32) -> u16 {
    let fraction = ((x - rect.left()) / rect.width()).clamp(0.0, 1.0);
    (fraction * f32::from(SPEED_SPAN_TOTAL)).round() as u16
}

/// The shape with point `index` moved: its ends keep their position, an interior point stays
/// [`MIN_GAP`] clear of both neighbours, and the speed stays on the plotted 0.1x-10x axis.
fn moved(points: &[SpeedPoint], index: usize, at: u16, speed: f64) -> Option<Vec<SpeedPoint>> {
    let last = points.len().checked_sub(1)?;
    let speed = Speed::from_f64(speed.clamp(MIN_SHOWN, MAX_SHOWN)).ok()?;
    let at = if index == 0 {
        0
    } else if index == last {
        SPEED_SPAN_TOTAL
    } else {
        let low = points.get(index.checked_sub(1)?)?.at.checked_add(MIN_GAP)?;
        let high = points.get(index + 1)?.at.checked_sub(MIN_GAP)?;
        if low > high {
            return None;
        }
        at.clamp(low, high)
    };
    let mut next = points.to_vec();
    *next.get_mut(index)? = SpeedPoint { at, speed };
    Some(next)
}

/// The shape with a new point at `at`, on the curve; `None` when full or too close to a point.
fn with_point(points: &[SpeedPoint], at: u16) -> Option<(Vec<SpeedPoint>, usize)> {
    if at == 0
        || at >= SPEED_SPAN_TOTAL
        || points.len() >= project::MAX_SPEED_POINTS
        || points.iter().any(|point| point.at.abs_diff(at) < MIN_GAP)
    {
        return None;
    }
    let speed = Speed::from_f64(SpeedPoint::interpolate(
        points,
        f64::from(at) / f64::from(SPEED_SPAN_TOTAL),
    ))
    .ok()?;
    let index = points.iter().take_while(|point| point.at < at).count();
    let mut next = points.to_vec();
    next.insert(index, SpeedPoint { at, speed });
    Some((next, index))
}

/// The shape without interior point `index`.
fn without_point(points: &[SpeedPoint], index: usize) -> Option<Vec<SpeedPoint>> {
    if index == 0 || index + 1 >= points.len() {
        return None;
    }
    let mut next = points.to_vec();
    next.remove(index);
    Some(next)
}

/// Seconds as the Duration line shows them: `10s`, `7.5s`.
fn seconds_text(seconds: f64) -> String {
    let text = format!("{seconds:.1}");
    format!("{}s", text.strip_suffix(".0").unwrap_or(&text))
}

fn pairs(points: &[SpeedPoint]) -> Vec<(u16, f64)> {
    points
        .iter()
        .map(|point| (point.at, point.speed.as_f64()))
        .collect()
}

impl Panel<'_, '_> {
    /// The Curve view: the preset tiles, then, once the clip carries a curve, the speed plot
    /// of the shape the clip plays (a split piece's slice of it)
    /// with its Duration line, Reset and point button, then the pitch policy.
    pub(super) fn speed_curve_controls(
        &mut self,
        ui: &mut egui::Ui,
        item: ItemId,
        shape: Option<Vec<SpeedPoint>>,
        maintain_pitch: bool,
        has_audio: bool,
    ) {
        self.speed_preset_tiles(ui, item, shape.as_deref(), maintain_pitch);
        let Some(shape) = shape else {
            return;
        };
        ui.add_space(PLOT_GAP);
        self.speed_plot(ui, item, &shape, maintain_pitch);
        if has_audio {
            ui.add_space(theme::space::SM);
            let (row, mut value_ui) = self.row_value_ui(ui, theme::size::PROPERTY_ROW);
            self.row_label(ui, row, self.tr(Text::ChangePitch));
            let mut follows = !maintain_pitch;
            let response = value_ui.add(egui::Checkbox::without_text(&mut follows));
            let name = self.tr(Text::ChangePitch);
            response
                .ctx
                .accesskit_node_builder(response.id, |node| node.set_label(name));
            self.remember("inspector.speed.pitch", &response);
            if response.changed() {
                self.actions.push(Action::SetSpeedCurve {
                    item,
                    points: pairs(&shape),
                    maintain_pitch: !follows,
                });
            }
        }
    }

    /// None, Custom and the six named shapes, six to a row.
    fn speed_preset_tiles(
        &mut self,
        ui: &mut egui::Ui,
        item: ItemId,
        shape: Option<&[SpeedPoint]>,
        maintain_pitch: bool,
    ) {
        let held = held_preset(shape);
        let tiles: Vec<Option<SpeedPreset>> = std::iter::once(None)
            .chain(SpeedPreset::ALL.into_iter().map(Some))
            .collect();
        for row in tiles.chunks(columns(ui.available_width())) {
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = TILE_GAP;
                for entry in row {
                    self.speed_preset_tile(ui, item, *entry, *entry == held, shape, maintain_pitch);
                }
            });
        }
    }

    fn speed_preset_tile(
        &mut self,
        ui: &mut egui::Ui,
        item: ItemId,
        entry: Option<SpeedPreset>,
        selected: bool,
        shape: Option<&[SpeedPoint]>,
        maintain_pitch: bool,
    ) {
        let dark = self.cx.dark;
        let (word, text) = tile_entry(entry);
        let label = self.tr(text);
        let mut clicked = false;
        ui.vertical(|ui| {
            ui.set_width(TILE);
            ui.spacing_mut().item_spacing.y = 0.0;
            let (rect, response) =
                ui.allocate_exact_size(egui::Vec2::splat(TILE), egui::Sense::click());
            let radius = theme::radius::UPLOAD;
            ui.painter().rect_filled(rect, radius, theme::field(dark));
            self.paint_preset_icon(ui, rect, entry);
            if selected || response.hovered() || response.has_focus() {
                ui.painter().rect_stroke(
                    rect,
                    radius,
                    egui::Stroke::new(1.0, theme::accent(dark)),
                    egui::StrokeKind::Inside,
                );
            }
            response.widget_info(|| {
                egui::WidgetInfo::selected(egui::Role::Button, true, selected, label)
            });
            self.remember(format!("inspector.speed.preset.{word}"), &response);
            clicked = response.clicked() && !selected;
            let (band, _) =
                ui.allocate_exact_size(egui::vec2(TILE, NAME_BAND), egui::Sense::hover());
            ui.painter().text(
                egui::pos2(band.center().x, band.top() + 10.5),
                egui::Align2::CENTER_CENTER,
                label,
                theme::label_font(),
                self.text_color(),
            );
        });
        if !clicked {
            return;
        }
        match entry {
            None => {
                self.state.speed_curve_view = true;
                if shape.is_some() {
                    self.set_speed(item, 1.0, maintain_pitch);
                }
            }
            Some(preset) => self.actions.push(Action::ApplySpeedPreset {
                item,
                preset,
                maintain_pitch,
            }),
        }
    }

    /// The tile picture: the 2x and 0.5x guides with the preset's shape over them; None is a
    /// struck circle and Custom three sliders.
    fn paint_preset_icon(&self, ui: &egui::Ui, rect: egui::Rect, entry: Option<SpeedPreset>) {
        let dark = self.cx.dark;
        let painter = ui.painter();
        let ink = theme::accent(dark);
        match entry {
            None => {
                let stroke = egui::Stroke::new(2.0, self.secondary());
                painter.circle_stroke(rect.center(), 11.0, stroke);
                let reach = 11.0 * std::f32::consts::FRAC_1_SQRT_2;
                painter.line_segment(
                    [
                        rect.center() - egui::vec2(reach, reach),
                        rect.center() + egui::vec2(reach, reach),
                    ],
                    stroke,
                );
            }
            Some(SpeedPreset::Custom) => {
                let stroke = egui::Stroke::new(1.5, ink);
                for (row, knob) in [(-14.0, -6.0), (0.0, 4.0), (14.0, 8.0)] {
                    let y = rect.center().y + row;
                    painter.line_segment(
                        [
                            egui::pos2(rect.left() + 14.0, y),
                            egui::pos2(rect.right() - 14.0, y),
                        ],
                        stroke,
                    );
                    painter.circle_stroke(egui::pos2(rect.center().x + knob, y), 3.0, stroke);
                }
            }
            Some(preset) => {
                let inner = rect.shrink2(egui::vec2(2.0, 4.0));
                let middle = rect.center().y;
                let height = |speed: f64| middle - speed.log10() as f32 * ICON_DECADE;
                let guide = egui::Stroke::new(1.0, theme::border(dark));
                painter.line_segment(
                    [
                        egui::pos2(inner.left(), middle),
                        egui::pos2(inner.right(), middle),
                    ],
                    guide,
                );
                for speed in GUIDES {
                    let y = height(speed);
                    painter.extend(egui::Shape::dashed_line(
                        &[egui::pos2(inner.left(), y), egui::pos2(inner.right(), y)],
                        guide,
                        3.0,
                        3.0,
                    ));
                }
                let points = preset.points();
                let line: Vec<egui::Pos2> = (0..=ICON_SAMPLES)
                    .map(|n| {
                        let fraction = n as f32 / ICON_SAMPLES as f32;
                        let speed = SpeedPoint::interpolate(&points, f64::from(fraction));
                        egui::pos2(
                            inner.left() + inner.width() * fraction,
                            height(speed).clamp(inner.top(), inner.bottom()),
                        )
                    })
                    .collect();
                painter.add(egui::Shape::line(line, egui::Stroke::new(1.5, ink)));
            }
        }
    }

    /// The plot box: Duration line, log speed plot with draggable points, Reset and the
    /// add/remove point button. A drag commits one `timeline.set_speed_curve` on release.
    fn speed_plot(
        &mut self,
        ui: &mut egui::Ui,
        item: ItemId,
        shape: &[SpeedPoint],
        maintain_pitch: bool,
    ) {
        let dark = self.cx.dark;
        let width = ui.available_width().max(1.0);
        let (outer, _) = ui.allocate_exact_size(
            egui::vec2(width, HEADER + PLOT_HEIGHT + FOOTER),
            egui::Sense::hover(),
        );
        let header = egui::Rect::from_min_size(outer.min, egui::vec2(width, HEADER));
        let plot = egui::Rect::from_min_size(header.left_bottom(), egui::vec2(width, PLOT_HEIGHT));
        let footer = egui::Rect::from_min_max(plot.left_bottom(), outer.right_bottom());
        let painter = ui.painter().clone();
        painter.rect_filled(outer, theme::radius::FIELD, theme::window(dark));

        let drag_id = egui::Id::unique(("inspector.speed.drag", item.0));
        let select_id = egui::Id::unique(("inspector.speed.selected", item.0));
        let drag: Option<Drag> = ui.data(|data| data.get_temp(drag_id));
        let points: Vec<SpeedPoint> = drag
            .as_ref()
            .map_or_else(|| shape.to_vec(), |drag| drag.points.clone());
        let mut selected: Option<usize> = ui
            .data(|data| data.get_temp::<usize>(select_id))
            .filter(|index| *index < points.len());

        self.paint_duration_line(&painter, header, item);
        self.paint_plot_grid(&painter, plot);
        if let Some(at) = read::speed_playhead(self.cx.session.project(), item, self.cx.playhead) {
            let x = plot_x(plot, at);
            painter.line_segment(
                [egui::pos2(x, plot.top()), egui::pos2(x, plot.bottom())],
                egui::Stroke::new(1.0, self.text_color()),
            );
        }
        let line: Vec<egui::Pos2> = (0..=SAMPLES)
            .map(|n| {
                let fraction = n as f32 / SAMPLES as f32;
                let speed = SpeedPoint::interpolate(&points, f64::from(fraction));
                egui::pos2(plot.left() + plot.width() * fraction, plot_y(plot, speed))
            })
            .collect();
        painter.add(egui::Shape::line(
            line,
            egui::Stroke::new(1.5, self.text_color()),
        ));

        let area = ui.interact(
            plot,
            egui::Id::unique(("inspector.speed.plot", item.0)),
            egui::Sense::hover(),
        );
        let plot_label = self.tr(Text::SpeedCurvePlot);
        area.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Group, true, plot_label));
        self.remember("inspector.speed.plot", &area);

        let mut commit: Option<Vec<SpeedPoint>> = None;
        for (index, point) in points.iter().enumerate() {
            let center = egui::pos2(plot_x(plot, point.at), plot_y(plot, point.speed.as_f64()));
            let handle = ui.interact(
                egui::Rect::from_center_size(center, HIT),
                egui::Id::unique(("inspector.speed.point", item.0, index)),
                egui::Sense::click_and_drag(),
            );
            self.remember(format!("inspector.speed.point.{index}"), &handle);
            if handle.clicked() || handle.drag_started() {
                selected = Some(index);
            }
            let position = handle
                .interact_pointer_pos()
                .or_else(|| ui.input(|input| input.pointer.interact_pos()));
            if handle.drag_started() || handle.dragged() || handle.drag_stopped() {
                let origin = ui.input(|input| input.pointer.press_origin());
                let grab = drag.as_ref().map_or_else(
                    || {
                        origin
                            .or(position)
                            .map_or(egui::Vec2::ZERO, |at| center - at)
                    },
                    |held| held.grab,
                );
                let next = position
                    .and_then(|at| {
                        let target = at + grab;
                        moved(
                            &points,
                            index,
                            position_at_x(plot, target.x),
                            speed_at_y(plot, target.y),
                        )
                    })
                    .unwrap_or_else(|| points.clone());
                if handle.drag_stopped() {
                    ui.data_mut(|data| data.remove::<Drag>(drag_id));
                    if next != shape {
                        commit = Some(next);
                    }
                } else {
                    ui.data_mut(|data| data.insert_temp(drag_id, Drag { grab, points: next }));
                }
            }
            let chosen = selected == Some(index);
            if chosen {
                painter.circle_filled(center, DISC_RADIUS, self.text_color());
            } else {
                let pill = egui::Rect::from_center_size(center, PILL);
                painter.rect_filled(pill, 3.0, theme::border(dark));
                for dx in [-1.5, 1.5] {
                    painter.line_segment(
                        [
                            egui::pos2(center.x + dx, pill.top() + 3.0),
                            egui::pos2(center.x + dx, pill.bottom() - 3.0),
                        ],
                        egui::Stroke::new(1.0, self.secondary()),
                    );
                }
            }
        }
        match selected {
            Some(index) => {
                ui.data_mut(|data| data.insert_temp(select_id, index));
            }
            None => {
                ui.data_mut(|data| data.remove::<usize>(select_id));
            }
        }

        let edit = Edit {
            points: &points,
            selected,
            commit,
        };
        self.speed_plot_footer(ui, item, footer, shape, edit, maintain_pitch);
    }

    /// Reset and the add/remove point button; also publishes the frame's commit.
    fn speed_plot_footer(
        &mut self,
        ui: &mut egui::Ui,
        item: ItemId,
        footer: egui::Rect,
        current: &[SpeedPoint],
        edit: Edit<'_>,
        maintain_pitch: bool,
    ) {
        let Edit {
            points,
            selected,
            commit,
        } = edit;
        let mut next = commit;
        let select_id = egui::Id::unique(("inspector.speed.selected", item.0));
        let right = footer.right() - BOX_PAD - 2.0;
        let point_rect = egui::Rect::from_min_size(
            egui::pos2(
                right - POINT_BUTTON_WIDTH,
                footer.center().y - BUTTON_HEIGHT / 2.0,
            ),
            egui::vec2(POINT_BUTTON_WIDTH, BUTTON_HEIGHT),
        );
        let reset_rect = egui::Rect::from_min_size(
            egui::pos2(point_rect.left() - 8.0 - RESET_WIDTH, point_rect.top()),
            egui::vec2(RESET_WIDTH, BUTTON_HEIGHT),
        );
        let flat = SpeedPreset::Custom.points();
        let reset_label = self.tr(Text::SpeedReset);
        let reset = ui
            .add_enabled_ui(points != flat.as_slice(), |ui| {
                ui.put(
                    reset_rect,
                    egui::Button::new(reset_label).min_size(reset_rect.size()),
                )
            })
            .inner;
        self.remember("inspector.speed.curve.reset", &reset);
        if reset.clicked() {
            self.actions.push(Action::ApplySpeedPreset {
                item,
                preset: SpeedPreset::Custom,
                maintain_pitch,
            });
        }

        // "+" adds a point at the playhead; with an interior point selected it reads "-" and
        // removes that point.
        let removable = selected.filter(|index| without_point(points, *index).is_some());
        let at = read::speed_playhead(self.cx.session.project(), item, self.cx.playhead);
        let addition = at.and_then(|at| with_point(points, at));
        let (glyph, label, id) = if removable.is_some() {
            (
                "\u{2212}",
                Text::SpeedCurveRemovePoint,
                "inspector.speed.curve.remove",
            )
        } else {
            ("+", Text::SpeedCurveAddPoint, "inspector.speed.curve.add")
        };
        let enabled = removable.is_some() || addition.is_some();
        let name = self.tr(label);
        let button = ui
            .add_enabled_ui(enabled, |ui| {
                ui.put(
                    point_rect,
                    egui::Button::new(glyph).min_size(point_rect.size()),
                )
            })
            .inner
            .on_hover_text(name);
        button.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, enabled, name));
        self.remember(id, &button);
        if button.clicked() && enabled {
            if let Some(index) = removable {
                next = without_point(points, index);
                ui.data_mut(|data| data.remove::<usize>(select_id));
            } else if let Some((shape, index)) = addition {
                next = Some(shape);
                ui.data_mut(|data| data.insert_temp(select_id, index));
            }
        }
        if let Some(shape) = next
            && shape != current
        {
            self.actions.push(Action::SetSpeedCurve {
                item,
                points: pairs(&shape),
                maintain_pitch,
            });
        }
    }

    /// `Duration  10s ----> 7s`: the clip's source length, a dashed run, the retimed length.
    fn paint_duration_line(&self, painter: &egui::Painter, header: egui::Rect, item: ItemId) {
        let y = header.center().y;
        painter.text(
            egui::pos2(header.left() + BOX_PAD, y),
            egui::Align2::LEFT_CENTER,
            self.tr(Text::SpeedDuration),
            theme::label_font(),
            self.text_color(),
        );
        let Some((source, output)) = read::retime_seconds(self.cx.session.project(), item) else {
            return;
        };
        painter.text(
            egui::pos2(header.left() + SOURCE_X, y),
            egui::Align2::LEFT_CENTER,
            seconds_text(source),
            theme::label_font(),
            self.secondary(),
        );
        let dash = egui::Stroke::new(1.0, self.secondary());
        painter.extend(egui::Shape::dashed_line(
            &[
                egui::pos2(header.left() + DASH_START_X, y),
                egui::pos2(header.right() - DASH_END_INSET, y),
            ],
            dash,
            3.0,
            3.0,
        ));
        painter.text(
            egui::pos2(header.right() - BOX_PAD, y),
            egui::Align2::RIGHT_CENTER,
            seconds_text(output),
            theme::label_font(),
            self.text_color(),
        );
    }

    /// The 10x, 1x and 0.1x axis labels with the 1x line and the 2x and 0.5x dashed guides.
    fn paint_plot_grid(&self, painter: &egui::Painter, plot: egui::Rect) {
        let dark = self.cx.dark;
        let guide = egui::Stroke::new(1.0, theme::border(dark));
        painter.line_segment(
            [
                egui::pos2(plot.left(), plot.center().y),
                egui::pos2(plot.right(), plot.center().y),
            ],
            guide,
        );
        for speed in GUIDES {
            let y = plot_y(plot, speed);
            painter.extend(egui::Shape::dashed_line(
                &[egui::pos2(plot.left(), y), egui::pos2(plot.right(), y)],
                guide,
                4.0,
                3.0,
            ));
        }
        for (label, y) in [
            ("10x", plot.top() + 9.0),
            ("1x", plot.center().y - 9.0),
            ("0.1x", plot.bottom() - 9.0),
        ] {
            painter.text(
                egui::pos2(plot.left() + BOX_PAD, y),
                egui::Align2::LEFT_CENTER,
                label,
                theme::label_font(),
                theme::disabled(dark),
            );
        }
    }
}
