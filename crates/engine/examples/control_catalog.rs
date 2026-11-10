//! Regenerate the checked-in registry and reference used by the CI drift test.
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let ids: Vec<_> = engine::registry().iter().map(|spec| spec.id).collect();
    if let Some((before, id)) = engine::declaration_order_error(&ids) {
        return Err(format!(
            "command declarations are not in ascending id order: `{id}` follows `{before}`; move the `{id}` declaration to its ordered place"
        )
        .into());
    }
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
        root.join("../../docs/product/agent-control/commands.md"),
        engine::control::reference_markdown()?,
    )?;
    Ok(())
}
