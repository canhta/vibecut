//! Parallel oracle callers must never observe an unfinished shared fixture.
#![allow(clippy::unwrap_used, clippy::panic)]

#[test]
fn concurrent_fixture_reader_waits_for_completed_generation() {
    if !crate::common::require_oracles("concurrent fixture generation") {
        return;
    }
    let name = format!("concurrent-fixture-{}.m4a", std::process::id());
    let path = crate::common::fixtures_dir().join(&name);
    let _ = std::fs::remove_file(&path);
    let writer_name = name.clone();
    let args = [
        "-re",
        "-f",
        "lavfi",
        "-i",
        "sine=frequency=440:sample_rate=48000:duration=2",
        "-c:a",
        "aac",
    ];
    let writer = std::thread::spawn(move || {
        crate::common::generate("concurrent fixture writer", &writer_name, &args).unwrap()
    });
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while !std::fs::metadata(&path).is_ok_and(|meta| meta.len() > 0) {
        assert!(std::time::Instant::now() < deadline, "writer did not start");
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    let reader_path = crate::common::generate("concurrent fixture reader", &name, &args).unwrap();
    let imported = media::SourceAsset::import(media::AssetId(1), reader_path);
    writer.join().unwrap();
    std::fs::remove_file(path).unwrap();
    assert!(
        imported.is_ok(),
        "reader observed unfinished fixture: {imported:?}"
    );
}
