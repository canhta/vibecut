#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic,
    reason = "public-interface conformance assertions"
)]
//! `text.search` and `text.insert_template` through the control request path.
use crate::support::control::{error, ok, run, runner, session};
use engine::control::{ControlResult, ErrorCode};
use project::{ItemParams, TextLook, TextTemplate};
use serde_json::json;
use std::sync::Arc;

fn search(
    s: &mut engine::Session,
    r: &mut engine::jobs::JobRunner,
    params: serde_json::Value,
    key: &str,
) -> (Vec<String>, Vec<String>) {
    let ControlResult::TextLibrary { looks, templates } = ok(run(s, r, "text.search", params, key))
    else {
        panic!("text.search result");
    };
    (
        looks.into_iter().map(|entry| entry.id).collect(),
        templates.into_iter().map(|entry| entry.id).collect(),
    )
}

#[test]
fn text_search_filters_by_query_shelf_and_licence_without_editing() {
    let mut s = session();
    let mut r = runner();
    let before = s.project().clone();
    let (looks, templates) = search(&mut s, &mut r, json!({}), "all");
    assert_eq!(looks.len(), TextLook::ALL.len());
    assert_eq!(templates.len(), TextTemplate::ALL.len());
    let (looks, templates) = search(
        &mut s,
        &mut r,
        json!({"query": "NEON", "shelf": "glow", "commercial_only": true}),
        "neon",
    );
    assert_eq!(looks, ["neon_cyan", "neon_pink"]);
    assert_eq!(templates, ["neon_sign"]);
    let (looks, templates) = search(
        &mut s,
        &mut r,
        json!({"query": "neon", "shelf": "label"}),
        "x",
    );
    assert!(looks.is_empty() && templates.is_empty());
    assert!(Arc::ptr_eq(&before, s.project()), "search is not an edit");
    let ControlResult::TextLibrary { looks, .. } = ok(run(
        &mut s,
        &mut r,
        "text.search",
        json!({"query": "clean white"}),
        "entry",
    )) else {
        panic!("text.search result");
    };
    assert_eq!(looks.len(), 1);
    assert_eq!(looks[0].license, project::TEXT_LIBRARY_LICENSE);
    assert!(looks[0].commercial);
}

#[test]
fn text_insert_template_adds_one_styled_item_in_one_undo_step() {
    let mut s = session();
    let mut r = runner();
    let before = s.project().clone();
    let history = s.history_len();
    ok(run(
        &mut s,
        &mut r,
        "text.insert_template",
        json!({
            "template": "lower_third",
            "content": "Mai Linh",
            "start": 0,
            "duration": 127008000000_i64
        }),
        "insert",
    ));
    assert_eq!(s.history_len(), (history.0 + 1, 0));
    let items: Vec<_> = s
        .project()
        .data()
        .scene()
        .timeline
        .iter()
        .flat_map(|track| &track.items)
        .collect();
    assert_eq!(items.len(), 1);
    let ItemParams::Text(params) = &items[0].content else {
        panic!("text item");
    };
    assert_eq!(params.content.as_str(), "Mai Linh");
    assert_eq!(
        project::TextStyle::from_params(params),
        TextTemplate::LowerThird.style()
    );
    assert_eq!(
        items[0].animations.iter().count(),
        1,
        "a lower third carries its vertical offset"
    );
    s.undo().unwrap();
    assert_eq!(
        s.project().data().scene().timeline,
        before.data().scene().timeline
    );
}

#[test]
fn text_insert_template_defaults_to_its_sample_words_and_refuses_bad_input() {
    let mut s = session();
    let mut r = runner();
    ok(run(
        &mut s,
        &mut r,
        "text.insert_template",
        json!({"template": "big_title", "start": 0, "duration": 127008000000_i64}),
        "sample",
    ));
    let item = s
        .project()
        .data()
        .scene()
        .timeline
        .iter()
        .flat_map(|track| &track.items)
        .next()
        .unwrap();
    let ItemParams::Text(params) = &item.content else {
        panic!("text item");
    };
    assert_eq!(params.content.as_str(), TextTemplate::BigTitle.sample());
    assert_eq!(
        item.animations.iter().count(),
        0,
        "a centred title has no offset"
    );
    let history = s.history_len();
    error(
        run(
            &mut s,
            &mut r,
            "text.insert_template",
            json!({"template": "no_such", "start": 0, "duration": 127008000000_i64}),
            "unknown",
        ),
        ErrorCode::InvalidParams,
    );
    assert_eq!(s.history_len(), history);
}
