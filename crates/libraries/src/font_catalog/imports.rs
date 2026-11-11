//! Publishes validated user fonts in the font store.
use super::*;
use serde::{Deserialize, Serialize};
use skrifa::{MetadataProvider, string::StringId};
use unicode_normalization::UnicodeNormalization;

const MAX_IMPORTS: usize = 128;
const MAX_STORED_BYTES: u64 = 256 << 20;
const MAX_RECORD_BYTES: u64 = 64 << 10;
// Includes bounded Brotli window, transformed glyph points and reconstruction
// scratch. The directory's transformed data is capped separately before decode.
const IMPORT_WORK_BYTES: u64 = 512 << 20;
const RECORD: &str = "import.json";
const FONT: &str = "font.sfnt";

/// The source-file boundary. Implementations must return at most `max` bytes,
/// check cancellation and bound blocking IO; the importer checks the byte cap too.
pub trait FontFile: Send + Sync {
    fn read(&self, path: &Path, max: u64, context: &JobContext) -> Result<Vec<u8>>;
}
/// Local regular files; ancestors are subject to the filesystem owner's usual
/// path-race limits. Final symlinks are refused at the metadata check.
pub struct LocalFontFile;
impl FontFile for LocalFontFile {
    fn read(&self, path: &Path, max: u64, context: &JobContext) -> Result<Vec<u8>> {
        read_bounded(path, max, Some(context))
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Imported {
    schema: u32,
    pub family: String,
    pub style: String,
    weight: u16,
    italic: bool,
    sha256: String,
    original_sha256: String,
    size: u64,
    // Font metadata is provenance, not a verified licence grant.
    license: String,
    rights_acknowledged: bool,
    pub(super) source: String,
}

impl FontStore {
    /// Copy a TTF, OTF or WOFF2 after explicit acknowledgement of usage rights.
    /// Source and decoded files are limited to32MiB, WOFF2 transformed data to4MiB,
    /// the store to128 files/256MiB. Cancellation before publication leaves no entry.
    /// No source path or font bytes enter project state or the job result.
    pub fn submit_import(
        &self,
        runner: &mut JobRunner,
        path: PathBuf,
        accepted: bool,
    ) -> Result<(JobId, CancelToken)> {
        if !accepted {
            return Err(EngineError::Control(
                engine::control::ControlError::Permission,
            ));
        }
        let guard = Active::acquire(&self.active, "custom").map_err(|error| match error {
            EngineError::Font(FontError::DownloadActive) => {
                EngineError::Font(FontError::ImportActive)
            }
            other => other,
        })?;
        let root = self.root.clone();
        let file = self.file.clone();
        let reserved: Vec<String> = text::families()
            .into_iter()
            .map(|(family, _)| family)
            .chain(self.google.iter().map(|family| family.family.clone()))
            .chain(self.system_faces().iter().map(|face| face.family.clone()))
            .collect();
        let input = CacheKey::new(KeyInputs {
            kind: ArtifactKind::FontDownload,
            source: [0; 32],
            provider: "local-font",
            model: "import",
            version: "1",
            settings: b"ttf-otf-woff2",
            transcript: [0; 32],
            cues: [0; 32],
            clip_range: [0; 32],
            translation: [0; 32],
        })?;
        Ok(runner.submit::<EngineError>(
            JobSpec {
                kind: JobKind::Import,
                class: ResourceClass::Cpu,
                input,
                bytes: JobBytes {
                    compositor: IMPORT_WORK_BYTES,
                    ..Default::default()
                },
            },
            move |ctx| {
                let _hold = &guard;
                import(&root, &path, &reserved, file.as_ref(), ctx)?;
                Ok(Vec::new())
            },
        )?)
    }

    pub(super) fn imported(&self) -> Result<Vec<Imported>> {
        records(&self.root)
    }

    pub(super) fn imported_bytes(&self, record: &Imported) -> Result<Option<Vec<u8>>> {
        loading::read_verified(
            &self
                .root
                .join(format!("custom-{}", record.sha256))
                .join(FONT),
            record.size,
            &record.sha256,
        )
    }
}

fn valid_name(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 256
        && value.trim() == value
        && !value.chars().any(char::is_control)
}

fn records(root: &Path) -> Result<Vec<Imported>> {
    let dirs = match fs::read_dir(root) {
        Ok(dirs) => dirs,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error.into()),
    };
    let mut out = Vec::new();
    for entry in dirs {
        let entry = entry?;
        let name = entry.file_name();
        let Some(hash) = name.to_str().and_then(|name| name.strip_prefix("custom-")) else {
            continue;
        };
        if hash.len() != 64 || !hash.bytes().all(|b| b.is_ascii_hexdigit()) {
            continue;
        }
        if !entry.file_type()?.is_dir() {
            return Err(FontError::Integrity.into());
        }
        if out.len() == MAX_IMPORTS {
            return Err(EngineError::Control(engine::control::ControlError::Limit));
        }
        let bytes = read_bounded(&entry.path().join(RECORD), MAX_RECORD_BYTES, None)?;
        let record: Imported = serde_json::from_slice(&bytes).map_err(|_| FontError::Integrity)?;
        if record.schema != 1
            || !record.rights_acknowledged
            || !valid_name(&record.family)
            || !valid_name(&record.style)
            || record.style != text::style_name(record.weight, record.italic)
            || !(1..=1000).contains(&record.weight)
            || record.sha256 != hash
            || record.original_sha256.len() != 64
            || !record
                .original_sha256
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit())
            || record.size > text::MAX_FONT_BYTES as u64
            || record.license.len() > 32 << 10
            || record.source != format!("local:sha256:{}", record.original_sha256)
        {
            return Err(FontError::Integrity.into());
        }
        out.push(record);
    }
    let mut total = 0u64;
    let mut families = BTreeMap::new();
    for record in &out {
        total = total
            .checked_add(record.size)
            .ok_or(EngineError::Control(engine::control::ControlError::Limit))?;
        let count = families
            .entry(record.family.to_lowercase())
            .or_insert(0usize);
        *count += 1;
        if total > MAX_STORED_BYTES || *count > MAX_FAMILY_FACES {
            return Err(EngineError::Control(engine::control::ControlError::Limit));
        }
    }
    out.sort_by_cached_key(|record| (record.family.to_lowercase(), record.weight, record.italic));
    Ok(out)
}

fn read_bounded(path: &Path, max: u64, context: Option<&JobContext>) -> Result<Vec<u8>> {
    if let Some(context) = context {
        context.check_cancelled()?;
    }
    if !fs::symlink_metadata(path)?.is_file() {
        return Err(FontError::InvalidImport.into());
    }
    let mut file = fs::File::open(path)?;
    if !file.metadata()?.is_file() || file.metadata()?.len() > max {
        return Err(EngineError::Control(engine::control::ControlError::Limit));
    }
    let mut bytes = Vec::new();
    let mut input = Read::by_ref(&mut file).take(max + 1);
    let mut buffer = [0u8; BUFFER];
    loop {
        if let Some(context) = context {
            context.check_cancelled()?;
        }
        let count = input.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        bytes.extend_from_slice(&buffer[..count]);
    }
    if bytes.len() as u64 > max {
        return Err(EngineError::Control(engine::control::ControlError::Limit));
    }
    Ok(bytes)
}

fn describe(bytes: &[u8]) -> Result<(String, u16, bool, String)> {
    let font = skrifa::FontRef::new(bytes).map_err(|_| FontError::InvalidImport)?;
    let license = font
        .localized_strings(StringId::LICENSE_DESCRIPTION)
        .next()
        .map(|name| name.to_string())
        .filter(|value| value.len() <= 32 << 10)
        .unwrap_or_else(|| "Not specified in font; user acknowledged usage rights".into());
    let mut database = fontdb::Database::new();
    database.load_font_data(bytes.to_vec());
    let mut faces = database.faces();
    let face = faces.next().ok_or(FontError::InvalidImport)?;
    let family: String = face
        .families
        .first()
        .ok_or(FontError::InvalidImport)?
        .0
        .nfc()
        .collect();
    let weight = face.weight.0;
    let italic = face.style != fontdb::Style::Normal;
    if faces.next().is_some() || !valid_name(&family) || !(1..=1000).contains(&weight) {
        return Err(FontError::InvalidImport.into());
    }
    Ok((family, weight, italic, license))
}

fn import(
    root: &Path,
    path: &Path,
    reserved: &[String],
    file: &dyn FontFile,
    ctx: &JobContext,
) -> Result<()> {
    ctx.check_cancelled()?;
    let original = file.read(path, text::MAX_FONT_BYTES as u64, ctx)?;
    if original.len() > text::MAX_FONT_BYTES {
        return Err(EngineError::Control(engine::control::ControlError::Limit));
    }
    let original_sha256 = hex(&Sha256::digest(&original));
    let bytes = super::formats::decode(original)?;
    ctx.progress(JobStage::Verify, 0.5)?;
    let (family, weight, italic, license) = describe(&bytes)?;
    let style = text::style_name(weight, italic);
    if reserved
        .iter()
        .any(|name| name.to_lowercase() == family.to_lowercase())
    {
        return Err(FontError::Conflict.into());
    }
    if text::is_available(&family) {
        let current = text::face(text::resolve(&family, &style).face);
        if current.style == style && current.bytes() != bytes {
            return Err(FontError::Conflict.into());
        }
    }
    let record = Imported {
        schema: 1,
        family,
        style,
        weight,
        italic,
        sha256: hex(&Sha256::digest(&bytes)),
        source: format!("local:sha256:{original_sha256}"),
        original_sha256,
        size: bytes.len() as u64,
        license,
        rights_acknowledged: true,
    };
    fs::create_dir_all(root)?;
    let lock_path = root.join(".custom.lock");
    if fs::symlink_metadata(&lock_path).is_ok_and(|meta| !meta.is_file()) {
        return Err(FontError::Integrity.into());
    }
    let lock = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(lock_path)?;
    lock.try_lock().map_err(|_| FontError::ImportActive)?;
    let existing = records(root)?;
    let target = root.join(format!("custom-{}", record.sha256));
    if existing.iter().any(|entry| entry.sha256 == record.sha256) {
        if loading::read_verified(&target.join(FONT), record.size, &record.sha256)?.is_none() {
            return Err(FontError::Integrity.into());
        }
        return Ok(());
    }
    if existing.iter().any(|entry| {
        entry.family.to_lowercase() == record.family.to_lowercase()
            && entry.style.eq_ignore_ascii_case(&record.style)
    }) {
        return Err(FontError::Conflict.into());
    }
    if existing.len() >= MAX_IMPORTS
        || existing
            .iter()
            .filter(|entry| entry.family.to_lowercase() == record.family.to_lowercase())
            .count()
            >= MAX_FAMILY_FACES
        || existing
            .iter()
            .try_fold(record.size, |sum, item| sum.checked_add(item.size))
            .is_none_or(|total| total > MAX_STORED_BYTES)
    {
        return Err(EngineError::Control(engine::control::ControlError::Limit));
    }
    let mut staging = Staging::create(root, "custom")?;
    let metadata = serde_json::to_vec(&record).map_err(|_| FontError::InvalidImport)?;
    if metadata.len() as u64 > MAX_RECORD_BYTES {
        return Err(EngineError::Control(engine::control::ControlError::Limit));
    }
    for (name, data) in [(FONT, bytes), (RECORD, metadata)] {
        ctx.check_cancelled()?;
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(staging.path.join(name))?;
        file.write_all(&data)?;
        file.sync_all()?;
    }
    ctx.commit_publication(|| -> Result<()> {
        fs::rename(&staging.path, target)?;
        staging.published = true;
        Ok(())
    })?;
    // Published staging no longer owns its old pathname, even if it is reused.
    Ok(())
}

impl Imported {
    pub(super) fn entry(&self) -> FontEntry {
        FontEntry {
            family: self.family.clone(),
            origin: FontOrigin::Custom,
            styles: vec![self.style.clone()],
            state: FontState::Available,
            license: Some("User supplied; usage rights acknowledged".into()),
            source: Some(self.source.clone()),
            download_bytes: None,
        }
    }
    pub(super) fn font_data(&self, bytes: Vec<u8>) -> Result<text::FontData> {
        let (family, weight, italic, _) = describe(&bytes)?;
        if family != self.family || weight != self.weight || italic != self.italic {
            return Err(FontError::Integrity.into());
        }
        Ok(text::FontData {
            family,
            style: self.style.clone(),
            weight,
            italic,
            index: 0,
            bytes,
        })
    }
}
