//! Auto captions page layout: the footer stays fixed and the page scrolls above it.
#![allow(clippy::unwrap_used, clippy::panic)]
use super::Subtitles;
use crate::editor::egui;
use crate::editor::panels::assets::CaptionsSection;
use crate::i18n::Locale;
use crate::preferences::SubtitleOptions;
use egui_kittest::{
    Harness,
    kittest::{By, Queryable},
};
use engine::{ByteBudget, Command, NewItem, Placement, Session};
use media::AssetId;
use project::{CaptionParams, ItemParams, Project, ProjectId};
use time::{SourceRange, TICKS_PER_SECOND, Tick};

/// The assets column width the assets tests use. The short height makes the page scroll.
const PANEL_WIDTH: f32 = 420.0;
const PANEL_HEIGHT: f32 = 300.0;
/// Find and replace controls in document order; each must reach the area above the footer.
const FIND: [&str; 6] = [
    "subtitles.find.query",
    "subtitles.find.replacement",
    "subtitles.find.case",
    "subtitles.find.whole",
    "subtitles.find.preview",
    "subtitles.find.replace",
];

struct Rig {
    captions: Subtitles,
    session: Session,
    options: SubtitleOptions,
    /// The top of the page area, which is the top of the scroll viewport.
    top: f32,
    /// The app's fonts are installed on the first frame, as the window does.
    fonts: bool,
}

impl Rig {
    fn paint(&mut self, ui: &mut egui::Ui) {
        if !self.fonts {
            crate::kit::install_fonts(ui.ctx());
            self.fonts = true;
        }
        self.top = ui.max_rect().top();
        self.captions.ui(
            ui,
            &mut self.session,
            &mut self.options,
            Tick::ZERO,
            Locale::En,
            false,
            false,
            CaptionsSection::Auto,
        );
    }
}

/// A project with one caption item, so the Find section is drawn.
fn session() -> Session {
    let mut session =
        Session::new(Project::empty(ProjectId(1), "Captions".into()).unwrap(), 10).unwrap();
    session
        .apply(Command::AddAsset {
            asset: media::SourceAsset::try_from(media::AssetData {
                ephemeral: true,
                id: AssetId(1),
                path: std::env::temp_dir().join("caption-footer.srt"),
                content_hash: media::ContentHash::from_bytes(b"footer"),
                probe: media::SourceProbe {
                    format: "srt".into(),
                    duration: None,
                    streams: Vec::new(),
                },
            })
            .unwrap(),
        })
        .unwrap();
    session
        .apply(Command::InsertItem {
            placement: Placement::Auto,
            item: NewItem {
                retime: Default::default(),
                animations: Default::default(),
                source_asset: Some(AssetId(1)),
                source_range: SourceRange::new(Tick::ZERO, Tick(TICKS_PER_SECOND * 5)).unwrap(),
                start: Tick::ZERO,
                content: ItemParams::Caption(CaptionParams {
                    emphasis: Default::default(),
                    word_highlight: false,
                    preset: None,
                    overrides: Default::default(),
                    cues: Default::default(),
                }),
            },
        })
        .unwrap();
    session
}

fn rig() -> Rig {
    Rig {
        captions: Subtitles::new(ByteBudget::new(64 << 20).unwrap()).unwrap(),
        session: session(),
        options: SubtitleOptions::default(),
        top: 0.0,
        fonts: false,
    }
}

fn rect(harness: &Harness<'_, Rig>, id: &str) -> egui::Rect {
    harness
        .get(By::new().predicate(|node| node.author_id() == Some(id)))
        .rect()
}

/// The footer row's top: the higher of its checkbox and its primary button.
fn footer_top(harness: &Harness<'_, Rig>) -> f32 {
    rect(harness, "subtitles.delete_current")
        .top()
        .min(rect(harness, "subtitles.generate").top())
}

#[test]
fn find_and_replace_scroll_above_the_auto_footer() {
    let mut harness = Harness::builder()
        .with_size(egui::vec2(PANEL_WIDTH, PANEL_HEIGHT))
        .build_ui_state(|ui, rig: &mut Rig| rig.paint(ui), rig());
    harness.run_steps(3);
    let footer = footer_top(&harness);
    let page_top = harness.state().top;
    assert!(
        rect(&harness, "subtitles.generate").bottom() <= PANEL_HEIGHT,
        "the footer button leaves the window"
    );
    assert!(
        rect(&harness, FIND[0]).bottom() > footer,
        "fixture: the Find section starts above the footer, so nothing scrolls: find {:?} footer {footer}",
        rect(&harness, FIND[0])
    );
    for id in FIND {
        for _ in 0..80 {
            let rect = rect(&harness, id);
            if rect.top() >= page_top && rect.bottom() <= footer_top(&harness) {
                break;
            }
            // Wheel over the middle of the page, above the footer.
            harness.hover_at(egui::pos2(200.0, (page_top + footer_top(&harness)) / 2.0));
            harness.run_steps(2);
            harness.event(egui::Event::MouseWheel {
                unit: egui::MouseWheelUnit::Point,
                delta: egui::vec2(0.0, -120.0),
                phase: egui::TouchPhase::Move,
                source: egui::MouseWheelSource::Wheel,
                modifiers: egui::Modifiers::NONE,
            });
            harness.run_steps(30);
        }
        let rect = rect(&harness, id);
        let footer = footer_top(&harness);
        assert!(
            rect.top() >= page_top && rect.bottom() <= footer,
            "{id} is not reachable above the footer: {rect:?} footer {footer}"
        );
    }
}
