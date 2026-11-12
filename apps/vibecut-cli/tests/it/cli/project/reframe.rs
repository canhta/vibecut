//! Verifies native reframe command admission.
use crate::cli::fixtures::new_project;
use crate::cli::process::{cli, code, os};

#[test]
fn reframe_requires_an_item_and_a_known_target() {
    let missing = cli(&[os("reframe")]);
    assert_eq!(code(&missing), 2);
    let dir = tempfile::tempdir().unwrap();
    let project = dir.path().join("reframe.vibecut");
    let _ = new_project(&project);
    let item = cli(&[os("reframe"), project.as_os_str(), os("--item"), os("0")]);
    assert_eq!(code(&item), 2);
    let target = cli(&[
        os("reframe"),
        project.as_os_str(),
        os("--item"),
        os("0"),
        os("--target"),
        os("4:3"),
    ]);
    assert_eq!(code(&target), 2);
}
