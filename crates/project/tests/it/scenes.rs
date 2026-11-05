#![allow(clippy::unwrap_used, clippy::indexing_slicing)]
use project::{Project, ProjectId, Scene, SceneId, SceneScope, Timeline, TrackId};
#[test]
fn scene_arrays_roundtrip_and_refuse_invalid_identity_and_main_layout() {
    let mut data = Project::empty(ProjectId(1), "Scenes".into())
        .unwrap()
        .into_data();
    let mut timeline = Timeline::default();
    timeline.main.id = TrackId(10);
    data.scenes
        .insert(Scene {
            id: SceneId(7),
            name: "Tiếng Việt".into(),
            is_main: false,
            timeline,
            bookmarks: vec![],
        })
        .unwrap();
    data.current_scene_id = SceneId(7);
    let project = Project::new(data).unwrap();
    assert_eq!(
        project.scene_ids(&SceneScope::Active).unwrap(),
        vec![SceneId(7)]
    );
    assert_eq!(
        project.scene_ids(&SceneScope::All).unwrap(),
        vec![SceneId(0), SceneId(7)]
    );
    assert_eq!(
        project
            .scene_ids(&SceneScope::Selected(vec![SceneId(7), SceneId(0)]))
            .unwrap(),
        vec![SceneId(7), SceneId(0)]
    );
    for ids in [vec![], vec![SceneId(99)], vec![SceneId(7), SceneId(7)]] {
        assert!(project.scene_ids(&SceneScope::Selected(ids)).is_err());
    }
    assert!(project.for_scene(SceneId(99)).is_err());
    let bytes = project::encode(&project, false).unwrap();
    assert_eq!(project::decode(&bytes).unwrap().project, project);
    let encoded: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert!(encoded["project"]["scenes"].is_array());
    for (field, value) in [
        ("is_main", serde_json::json!(true)),
        ("id", serde_json::json!(0)),
    ] {
        let mut invalid = encoded.clone();
        invalid["project"]["scenes"][1][field] = value;
        assert!(project::decode(&serde_json::to_vec(&invalid).unwrap()).is_err());
    }
    for value in [
        serde_json::json!([]),
        serde_json::json!([encoded["project"]["scenes"][1].clone()]),
    ] {
        let mut invalid = encoded.clone();
        invalid["project"]["scenes"] = value;
        assert!(project::decode(&serde_json::to_vec(&invalid).unwrap()).is_err());
    }
    let mut invalid = encoded;
    invalid["project"]["current_scene_id"] = serde_json::json!(99);
    assert!(project::decode(&serde_json::to_vec(&invalid).unwrap()).is_err());
}
