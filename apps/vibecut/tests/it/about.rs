#![allow(clippy::unwrap_used, clippy::indexing_slicing, clippy::panic)]
use egui_kittest::{
    Harness,
    kittest::{By, NodeT, Queryable},
};
use engine::{ByteBudget, Session};
use project::{Project, ProjectId};
use std::sync::Arc;
use vibecut::{
    Editor, egui,
    i18n::{self, Locale},
    preferences::Preferences,
    services::lifecycle::EditorAction,
    services::playback::{Files, Playback},
};

fn editor(locale: Locale) -> Editor {
    let session = Session::new(Project::empty(ProjectId(1), "Test".into()).unwrap(), 10).unwrap();
    let mut playback = Playback::new(
        &session,
        ByteBudget::new(64 << 20).unwrap(),
        Arc::new(Files),
    )
    .unwrap();
    playback.set_device_available(true);
    let preferences = Preferences {
        locale,
        ..Preferences::default()
    };
    Editor::new(session, playback, preferences)
}

fn build(locale: Locale) -> Harness<'static, Editor> {
    let mut harness = crate::gpu::builder()
        .with_size(egui::vec2(1440.0, 900.0))
        .build_ui_state(|ui, editor: &mut Editor| editor.ui(ui), editor(locale));
    harness.run_steps(3);
    harness
}

fn by_id(id: &'static str) -> By<'static> {
    By::new().predicate(move |node| node.author_id() == Some(id))
}

fn present(harness: &Harness<Editor>, id: &'static str) -> bool {
    harness.query_all(by_id(id)).count() > 0
}

fn click(harness: &mut Harness<Editor>, id: &'static str) {
    harness.get(by_id(id)).click();
    harness.run_steps(3);
}

fn open_from_header_menu(harness: &mut Harness<Editor>) {
    click(harness, "window.menu");
    click(harness, "app.about");
}

fn version() -> String {
    format!("v{}", env!("CARGO_PKG_VERSION"))
}

#[test]
fn header_menu_opens_about_with_version_name_and_copyright() {
    let mut harness = build(Locale::En);
    assert!(!present(&harness, "app.about.close"));
    open_from_header_menu(&mut harness);
    assert!(present(&harness, "app.about.close"));
    assert!(harness.query_by_label(&version()).is_some());
    for text in [
        i18n::text(Locale::En, i18n::Text::AppName),
        i18n::text(Locale::En, i18n::Text::AboutCopyright),
        i18n::text(Locale::En, i18n::Text::ActionAppAbout),
        i18n::text(Locale::En, i18n::Text::ActionAppTerms),
        i18n::text(Locale::En, i18n::Text::ActionAppPrivacy),
        i18n::text(Locale::En, i18n::Text::AboutLicencesLink),
    ] {
        assert!(harness.query_by_label(text).is_some(), "{text}");
    }
}

#[test]
fn about_closes_with_its_button_and_with_escape() {
    let mut harness = build(Locale::En);
    open_from_header_menu(&mut harness);
    click(&mut harness, "app.about.close");
    assert!(!present(&harness, "app.about.close"));

    open_from_header_menu(&mut harness);
    harness.key_press(egui::Key::Escape);
    harness.run_steps(3);
    assert!(!present(&harness, "app.about.close"));
}

#[test]
fn licence_page_lists_ffmpeg_and_bundled_asset_authors_and_goes_back() {
    let mut harness = build(Locale::En);
    open_from_header_menu(&mut harness);
    click(&mut harness, "app.licences");
    assert!(present(&harness, "app.about"), "the back button");
    assert!(harness.query_by_label("FFmpeg developers").is_some());
    for author in ["Panayiotis Lipiridis", "Phosphor Icons"] {
        assert!(harness.query_by_label(author).is_some(), "{author}");
    }
    assert!(harness.query_by_label(&version()).is_none());
    click(&mut harness, "app.about");
    assert!(harness.query_by_label(&version()).is_some());
}

#[test]
fn terms_and_privacy_open_their_pages_through_the_window() {
    let mut harness = build(Locale::En);
    open_from_header_menu(&mut harness);
    for (id, expected) in [
        ("app.terms", EditorAction::OpenTerms),
        ("app.privacy", EditorAction::OpenPrivacy),
    ] {
        click(&mut harness, id);
        assert_eq!(
            harness.state_mut().take_lifecycle_action(),
            Some(expected),
            "{id}"
        );
    }
}

#[test]
fn about_uses_the_vietnamese_catalog() {
    let mut harness = build(Locale::Vi);
    harness.state_mut().activate("app.about").unwrap();
    harness.run_steps(3);
    for text in [
        i18n::text(Locale::Vi, i18n::Text::ActionAppAbout),
        i18n::text(Locale::Vi, i18n::Text::AboutCopyright),
        i18n::text(Locale::Vi, i18n::Text::ActionAppTerms),
        i18n::text(Locale::Vi, i18n::Text::ActionAppPrivacy),
        i18n::text(Locale::Vi, i18n::Text::AboutLicencesLink),
    ] {
        assert!(harness.query_by_label(text).is_some(), "{text}");
    }
    assert_ne!(
        i18n::text(Locale::Vi, i18n::Text::ActionAppAbout),
        i18n::text(Locale::En, i18n::Text::ActionAppAbout)
    );
}
