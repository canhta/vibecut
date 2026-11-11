//! Loads verified font faces into the text catalog.
use super::{
    Cache, Digest, EditError, FontError, FontLoad, FontLoadState, FontOrigin, FontStore,
    GoogleFamily, LICENSE_FILE, Path, Read, Result, Sha256, fs, hex, io, sidecar,
};
impl FontStore {
    /// Register only imported families requested by this snapshot before rendering.
    /// Missing cached families retain text's deterministic bundled fallback; no
    /// system scan, download or project mutation is implied by opening a project.
    pub fn load_project(&self, project: &project::Project) -> Result<()> {
        let imported: std::collections::BTreeSet<_> = self
            .imported()?
            .into_iter()
            .map(|entry| entry.family.to_lowercase())
            .collect();
        if imported.is_empty() {
            return Ok(());
        }
        let mut requested = std::collections::BTreeSet::new();
        for scene in project.data().scenes.iter() {
            for item in scene.timeline.iter().flat_map(|track| &track.items) {
                match &item.content {
                    project::ItemParams::Text(text) => {
                        requested.insert(text.font.as_str().to_owned());
                    }
                    project::ItemParams::Caption(caption) => {
                        requested.extend(
                            caption
                                .cues
                                .styles
                                .iter()
                                .map(|style| style.font_family.as_str().to_owned()),
                        );
                    }
                    _ => {}
                }
            }
        }
        for family in requested {
            if imported.contains(&family.to_lowercase()) {
                self.load(&family)?;
            }
        }
        Ok(())
    }

    /// Register the family's faces with `text` so layout resolves it, reading font files lazily
    /// and only now. Network is never used. Unknown and not-downloaded families are load
    /// states, not errors: layout then falls back to the default family deterministically.
    ///
    /// # Errors
    ///
    /// Fails on file IO or when `text` refuses every face of the family.
    pub fn load(&self, family: &str) -> Result<FontLoad> {
        let family = family.trim();
        if family.is_empty() || family.len() > 256 {
            return Err(EditError::Invalid("font family"));
        }
        let state = |state, origin, effective: &str, styles| FontLoad {
            family: family.to_owned(),
            state,
            origin,
            effective_family: effective.to_owned(),
            styles,
        };
        if let Some((name, styles)) = text::families()
            .into_iter()
            .find(|(name, _)| name.eq_ignore_ascii_case(family))
        {
            return Ok(state(
                FontLoadState::Loaded,
                Some(FontOrigin::Bundled),
                &name,
                styles,
            ));
        }
        let custom: Vec<_> = self
            .imported()?
            .into_iter()
            .filter(|record| record.family.to_lowercase() == family.to_lowercase())
            .collect();
        if let Some(first) = custom.first() {
            let mut faces = Vec::new();
            for record in &custom {
                let Some(bytes) = self.imported_bytes(record)? else {
                    return Ok(state(
                        FontLoadState::CacheInvalid,
                        Some(FontOrigin::Custom),
                        text::DEFAULT_FAMILY,
                        Vec::new(),
                    ));
                };
                faces.push(record.font_data(bytes)?);
            }
            let mut styles = Vec::new();
            for face in faces {
                styles.push(face.style.clone());
                text::register(face).map_err(FontError::Face)?;
            }
            return Ok(state(
                FontLoadState::Loaded,
                Some(FontOrigin::Custom),
                &first.family,
                styles,
            ));
        }
        let mut blocked = None;
        if let Some(g) = self.google_family(family) {
            match self.cache(g)? {
                Cache::Valid(files) => {
                    let mut faces = Vec::new();
                    for (file, bytes) in g.files.iter().zip(files) {
                        faces.push(text::FontData {
                            family: g.family.clone(),
                            style: text::style_name(file.weight, file.italic),
                            weight: file.weight,
                            italic: file.italic,
                            index: 0,
                            bytes,
                        });
                    }
                    let styles = register(faces)?;
                    return Ok(state(
                        FontLoadState::Loaded,
                        Some(FontOrigin::Google),
                        &g.family,
                        styles,
                    ));
                }
                Cache::Invalid => blocked = Some(FontLoadState::CacheInvalid),
                Cache::Missing => blocked = Some(FontLoadState::NotDownloaded),
            }
        }
        let installed = self.system_family(family);
        if let Some(first) = installed.first() {
            let mut faces = Vec::new();
            for face in &installed {
                let Some(bytes) = read_font(&face.path)? else {
                    continue;
                };
                faces.push(text::FontData {
                    family: first.family.clone(),
                    style: text::style_name(face.weight, face.italic),
                    weight: face.weight,
                    italic: face.italic,
                    index: face.index,
                    bytes,
                });
            }
            let styles = register(faces)?;
            return Ok(state(
                FontLoadState::Loaded,
                Some(FontOrigin::System),
                &first.family,
                styles,
            ));
        }
        Ok(state(
            blocked.unwrap_or(FontLoadState::Unknown),
            blocked.map(|_| FontOrigin::Google),
            text::DEFAULT_FAMILY,
            Vec::new(),
        ))
    }
}

impl FontStore {
    pub(super) fn cache(&self, g: &GoogleFamily) -> Result<Cache> {
        read_cache(&self.root, g)
    }
}

/// Verified cache state of a catalog family: missing, tampered/stale, or the verified bytes.
pub(super) fn read_cache(root: &Path, g: &GoogleFamily) -> Result<Cache> {
    let dir = root.join(&g.slug);
    match fs::symlink_metadata(&dir) {
        Ok(meta) if meta.is_dir() => {}
        Ok(_) => return Ok(Cache::Invalid),
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Cache::Missing),
        Err(error) => return Err(error.into()),
    }
    let mut files = Vec::new();
    let expected = g
        .files
        .iter()
        .map(|f| (f.name.as_str(), f.size, f.sha256.as_str()))
        .chain(std::iter::once((
            LICENSE_FILE,
            g.license_size,
            g.license_sha256.as_str(),
        )));
    for (name, size, sha256) in expected {
        let path = dir.join(name);
        let Some(bytes) = read_verified(&path, size, sha256)? else {
            return Ok(Cache::Invalid);
        };
        if !g.files.iter().any(|f| f.name == name) {
            continue;
        }
        if !fs::symlink_metadata(sidecar(&path)).is_ok_and(|m| m.is_file()) {
            return Ok(Cache::Invalid);
        }
        files.push(bytes);
    }
    Ok(Cache::Valid(files))
}

pub(super) fn styles_of(faces: impl Iterator<Item = (u16, bool)>) -> Vec<String> {
    let mut faces: Vec<(u16, bool)> = faces.collect();
    faces.sort_unstable();
    faces.dedup();
    faces
        .into_iter()
        .map(|(weight, italic)| text::style_name(weight, italic))
        .collect()
}

/// Register faces, tolerating a name that is already held by a different file. Returns the style
/// names now resolvable; fails only when no face could be registered.
pub(super) fn register(faces: Vec<text::FontData>) -> Result<Vec<String>> {
    let mut styles: Vec<String> = Vec::new();
    let mut last = None;
    let mut registered = false;
    for face in faces {
        let style = face.style.clone();
        match text::register(face) {
            Ok(_) | Err(text::FontError::Conflict) => {
                registered = true;
                if !styles.contains(&style) {
                    styles.push(style);
                }
            }
            Err(error) => last = Some(error),
        }
    }
    match (registered, last) {
        (false, Some(error)) => Err(FontError::Face(error).into()),
        (false, None) => Err(EditError::Invalid("font family has no readable face")),
        _ => Ok(styles),
    }
}

/// A font file read for registration; `None` when it is too large or vanished since the scan.
fn read_font(path: &Path) -> Result<Option<Vec<u8>>> {
    let meta = match fs::metadata(path) {
        Ok(meta) => meta,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    if !meta.is_file() || meta.len() > text::MAX_FONT_BYTES as u64 {
        return Ok(None);
    }
    Ok(Some(fs::read(path)?))
}

/// The file's bytes when it is a regular file of exactly `size` bytes with this SHA-256.
pub(super) fn read_verified(path: &Path, size: u64, sha256: &str) -> Result<Option<Vec<u8>>> {
    let meta = match fs::symlink_metadata(path) {
        Ok(meta) => meta,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    if !meta.is_file() || meta.len() != size {
        return Ok(None);
    }
    // Bound a file that grows between metadata and read, including custom entries.
    let mut bytes = Vec::new();
    fs::File::open(path)?
        .take(size.saturating_add(1))
        .read_to_end(&mut bytes)?;
    Ok((bytes.len() as u64 == size && hex(&Sha256::digest(&bytes)) == sha256).then_some(bytes))
}
