//! Regenerate the checked-in registry and reference used by the CI drift test.
fn main() -> Result<(), engine::EditError> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    std::fs::write(
        root.join("catalog.digest"),
        format!("{}\n", engine::control::registry_hash()),
    )?;
    let artifact = root.join("../../target/fixtures/control/catalog.json");
    if let Some(parent) = artifact.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(
        artifact,
        engine::control::bounded_json(engine::registry(), engine::control::MAX_RESPONSE_BYTES)?,
    )?;
    std::fs::write(
        root.join("../../docs/product/commands.md"),
        engine::control::reference_markdown()?,
    )?;
    Ok(())
}
