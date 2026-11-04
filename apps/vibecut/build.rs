fn main() -> Result<(), Box<dyn std::error::Error>> {
    #[cfg(windows)]
    embed_resource::compile("vibecut.rc", embed_resource::NONE).manifest_required()?;
    Ok(())
}
