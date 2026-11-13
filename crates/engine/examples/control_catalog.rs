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
    // This local artifact aggregates every page; transport caps still apply to
    // each wire response, not to the generated file containing the whole catalog.
    let mut artifact = std::io::BufWriter::new(std::fs::File::create(artifact)?);
    serde_json::to_writer(
        &mut artifact,
        &serde_json::json!({
            "output_schema": engine::control::output_schema(),
            "entries": engine::registry(),
        }),
    )?;
    std::io::Write::flush(&mut artifact)?;
    std::fs::write(
        root.join("../../docs/product/agent-control/commands.md"),
        engine::control::reference_markdown()?,
    )?;
    Ok(())
}
