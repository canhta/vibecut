#![allow(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    clippy::panic,
    clippy::expect_used
)]
//! The missing-media state across the editor surfaces (ticket 425, capture 2026-11-14):
//! the properties panel, the player canvas and the media tile, then the relink that clears
//! them. The timeline clip and the Link media dialog have their own rows beside their code.
use crate::timeline::{harness, widget};
use egui_kittest::kittest::{By, Queryable};
use engine::{ByteBudget, Command, NewItem, Placement, Session};
use media::{AssetId, ContentHash, SourceAsset};
use project::{ItemParams, Project, ProjectId};
use std::{path::Path, sync::Arc};
use time::{SourceRange, TICKS_PER_SECOND, Tick};
use vibecut::{
    Editor,
    preferences::Preferences,
    services::playback::{Files, Playback},
};

const BYTES: &[u8] = b"footage that was moved away";
const SECOND: i64 = TICKS_PER_SECOND;

fn recorded(path: &Path) -> SourceAsset {
    testkit::source::asset(1, path.to_path_buf())
        .format("mov,mp4")
        .duration(Tick(10 * SECOND))
        .hash(ContentHash::from_bytes(BYTES).0)
        .stream(testkit::source::video().duration(Tick(10 * SECOND)).build())
        .build()
}

/// One clip over a source whose file is absent, with a process media cache attached.
fn missing_editor(root: &Path) -> (Editor, project::ItemId) {
    let mut session =
        Session::new(Project::empty(ProjectId(425), "Lost".into()).unwrap(), 20).unwrap();
    session
        .apply(Command::AddAsset {
            asset: recorded(&root.join("moved").join("clip.mp4")),
        })
        .unwrap();
    let item = match session
        .apply(Command::InsertItem {
            placement: Placement::Auto,
            item: NewItem {
                retime: Default::default(),
                animations: Default::default(),
                source_asset: Some(AssetId(1)),
                source_range: SourceRange::new(Tick::ZERO, Tick(6 * SECOND)).unwrap(),
                start: Tick::ZERO,
                content: ItemParams::Video(project::VideoParams::plain()),
            },
        })
        .unwrap()
        .created
        .first()
    {
        Some(engine::CreatedId::Item(id)) => *id,
        _ => panic!("item"),
    };
    let playback = Playback::new(
        &session,
        ByteBudget::new(64 << 20).unwrap(),
        Arc::new(Files),
    )
    .unwrap();
    let mut editor = Editor::new(session, playback, Preferences::default());
    let media = engine::control::MediaAccess::open_in(
        &root.join("cache"),
        ByteBudget::new(64 << 20).unwrap(),
    )
    .unwrap();
    editor.set_media(Arc::new(std::sync::Mutex::new(media)));
    editor
        .session
        .set_selection([item].into_iter().collect())
        .unwrap();
    (editor, item)
}

fn has_widget(harness: &egui_kittest::Harness<'_, Editor>, id: &str) -> bool {
    harness
        .state()
        .inspect()
        .unwrap()
        .widgets
        .iter()
        .any(|widget| widget.id == id)
}

fn close(actual: [u8; 4], expected: [u8; 3]) -> bool {
    actual[..3]
        .iter()
        .zip(expected)
        .all(|(a, e)| a.abs_diff(e) < 8)
}

#[test]
fn a_lost_clip_empties_the_panel_reddens_the_canvas_and_the_tile_then_relinking_restores_them() {
    let root = tempfile::tempdir().unwrap();
    let (editor, _) = missing_editor(root.path());
    let mut harness = harness(editor);
    harness.run_steps(4);
    // Properties: two grey lines and our Link media button, no tab strip of sub-tabs.
    for line in ["Clips on this timeline", "are missing their media"] {
        assert!(
            harness.query_all_by_label(line).next().is_some(),
            "the panel names the lost clips: {line}"
        );
    }
    assert!(has_widget(&harness, "inspector.link_media"));
    assert!(
        !has_widget(&harness, "inspector.tab.transform"),
        "a lost clip has no properties tabs"
    );
    // Player: the canvas carries the missing line and paints the missing red.
    assert!(
        harness
            .query_all_by_label("File not found")
            .next()
            .is_some(),
        "the canvas says the file is not found"
    );
    let image = harness.render().unwrap();
    let canvas = widget(&harness, "preview.canvas");
    let at = image
        .get_pixel(canvas.center().x as u32, (canvas.top() + 24.0) as u32)
        .0;
    // The stage shows the canvas centred; a point just under the stage's top edge may be
    // letterbox, so look at the canvas centre's neighbourhood instead.
    let centre = image
        .get_pixel(
            (canvas.center().x + 60.0) as u32,
            (canvas.center().y + 40.0) as u32,
        )
        .0;
    assert!(
        close(centre, [0x43, 0x17, 0x1A]) || close(at, [0x43, 0x17, 0x1A]),
        "{centre:?} / {at:?} is not the missing canvas red"
    );
    // Media tile: the missing body fill replaces the thumbnail.
    let tile = widget(&harness, "media.asset.1");
    let sample = image
        .get_pixel(tile.left() as u32 + 4, tile.top() as u32 + 40)
        .0;
    assert!(
        close(sample, [0x3E, 0x13, 0x16]),
        "{sample:?} is not the missing tile fill"
    );
    let screenshot =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/425-missing-surfaces.png");
    std::fs::create_dir_all(screenshot.parent().unwrap()).unwrap();
    image.save(&screenshot).unwrap();
    // The panel's button opens the dialog through the one request path.
    harness
        .get(By::new().predicate(|node| node.author_id() == Some("inspector.link_media")))
        .click();
    harness.run_steps(4);
    assert!(harness.state().inspect().unwrap().link_media.open);
    // Relinking through the command returns the panel: the file is back under another path.
    let found = root.path().join("found.mp4");
    std::fs::write(&found, BYTES).unwrap();
    let asset = harness.state().session.project().data().assets[0]
        .relink(found)
        .unwrap();
    harness
        .state_mut()
        .session
        .apply(Command::RelinkAsset { asset })
        .unwrap();
    harness.run_steps(4);
    assert!(!has_widget(&harness, "inspector.link_media"));
    assert!(has_widget(&harness, "inspector.tab.transform"));
}
