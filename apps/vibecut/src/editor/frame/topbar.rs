//! Presents editor window actions without mutating project state.

use super::{Action, State};
use crate::editor::Context;
use crate::i18n;
use crate::{
    egui::{self, Align2, FontId, Rect, Vec2},
    i18n::Text,
    kit::{self, Icon},
};

pub(super) fn show(state: &mut State, cx: &Context<'_>, ui: &mut egui::Ui) -> Vec<Action> {
    state.painted.clear();
    let mut actions = Vec::new();
    let tr = |key| i18n::text(cx.locale, key);
    let rect = ui.max_rect();
    let color = kit::text(cx.dark);
    let font = FontId::proportional(kit::type_size::CHROME);
    ui.painter().text(
        egui::pos2(rect.left() + kit::space::LG, rect.center().y),
        Align2::LEFT_CENTER,
        tr(cx.save_status),
        font.clone(),
        kit::secondary(cx.dark),
    );

    // The title remains centred in the window regardless of either action group.
    let title = Rect::from_center_size(
        rect.center(),
        Vec2::new((rect.width() - 480.0).clamp(0.0, 300.0), kit::size::FIELD),
    );
    let mut title_ui = ui.new_child(egui::UiBuilder::new().id_salt("title").max_rect(title));
    title_ui.set_clip_rect(title);
    if let Some(mut name) = state.rename.take() {
        let response = title_ui.add_sized(
            title.size(),
            egui::TextEdit::singleline(&mut name)
                .id_salt("project.rename.input")
                .hint_text(tr(Text::ProjectsRenameTitle))
                .font(font)
                .align(Align2::CENTER_CENTER),
        );
        state
            .painted
            .push(("project.rename.input", None, response.clone()));
        if state.rename_focus {
            response.request_focus();
            state.rename_focus = false;
        }
        let cancel = ui.input(|input| input.key_pressed(egui::Key::Escape));
        let commit = ui.input(|input| input.key_pressed(egui::Key::Enter));
        if !cancel {
            if commit {
                actions.push(Action::Rename(name));
            } else {
                state.rename = Some(name);
            }
        }
    } else {
        let response = title_ui
            .add_sized(
                title.size(),
                egui::Button::new(
                    egui::RichText::new(cx.session.project().data().name.as_str()).font(font),
                )
                .frame(false)
                .truncate(),
            )
            .on_hover_text(tr(Text::ProjectsRenameTitle));
        state.painted.push((
            "project.rename",
            Some(Icon::PencilSimple.name()),
            response.clone(),
        ));
        if response.clicked() {
            actions.push(Action::Activate("project.rename"));
        }
    }

    let export = Rect::from_min_size(
        egui::pos2(
            rect.right() - 11.0 - kit::size::EXPORT_WIDTH,
            rect.top() + 7.0,
        ),
        Vec2::new(kit::size::EXPORT_WIDTH, kit::size::FIELD),
    );
    let mut export_ui = ui.new_child(egui::UiBuilder::new().id_salt("export").max_rect(export));
    let response = kit::action_button(
        &mut export_ui,
        "file.export",
        Icon::Export,
        tr(Text::Export),
        kit::size::EXPORT_WIDTH,
    );
    state
        .painted
        .push(("file.export", Some(Icon::Export.name()), response.clone()));
    if response.clicked() {
        actions.push(Action::Activate("file.export"));
    }

    let menu_rect = Rect::from_min_size(
        egui::pos2(
            export.left() - kit::space::MD - kit::size::CONTROL,
            rect.top() + 6.0,
        ),
        Vec2::splat(kit::size::CONTROL),
    );
    let mut menu_ui = ui.new_child(
        egui::UiBuilder::new()
            .id_salt("window.actions")
            .max_rect(menu_rect),
    );
    let menu = kit::icon_button(
        &mut menu_ui,
        "window.menu",
        Icon::SlidersHorizontal,
        tr(Text::Windows),
        false,
    );
    egui::Popup::menu(&menu).show(|ui| {
        for (id, icon, label) in [
            ("window.home", Icon::SquaresFour, Text::Home),
            ("project.save", Icon::FloppyDisk, Text::Save),
            ("project.save_as", Icon::FloppyDiskBack, Text::SaveAs),
            ("models.open", Icon::File, Text::Models),
            ("layout.reset", Icon::CornersOut, Text::Reset),
            ("appearance.locale", Icon::Translate, Text::Language),
            ("appearance.theme", Icon::CircleHalf, Text::Theme),
            ("shortcuts.open", Icon::GearSix, Text::Shortcuts),
        ] {
            let response = ui
                .push_id(id, |ui| {
                    ui.add(egui::Button::image_and_text(
                        kit::tinted(icon, kit::icon::DENSE, color),
                        tr(label),
                    ))
                })
                .inner;
            state
                .painted
                .push((id, Some(icon.name()), response.clone()));
            if response.clicked() {
                actions.push(Action::Activate(id));
                ui.close();
            }
        }
    });
    state
        .painted
        .push(("window.menu", Some(Icon::SlidersHorizontal.name()), menu));
    actions
}
