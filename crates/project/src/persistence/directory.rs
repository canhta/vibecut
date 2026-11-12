//! Retains admitted directory identities at filesystem boundaries.
use super::identity::{Identity, held_identity, named_identity};
use crate::{Error, Result};
use std::{
    ffi::OsStr,
    fs::{self, File, Metadata},
    path::{Component, Path, PathBuf},
    sync::Arc,
};

/// A canonical directory retained by native handle and named object identity.
///
/// Root aliases resolve once at open; children must be real immediate directories.
/// A child retains its root even after the root guard drops. At most one child
/// level is admitted. No file contents, lock/control files or deletions are owned
/// here. Call `validate` at operation boundaries: this is advisory validation,
/// not protection against arbitrary post-check races or physical power loss.
pub struct OwnedDirectory {
    directory: Arc<Directory>,
    parent: Option<Arc<Directory>>,
}

struct Directory {
    path: PathBuf,
    file: File,
    identity: Identity,
}

impl OwnedDirectory {
    /// Create missing directories privately (0700 on Unix), or open an existing
    /// root, resolving its supplied alias once. No permissions on existing roots
    /// change. Returns `Unsupported` for unavailable native identity/platform or
    /// non-directory kinds, `Conflict` for an inadmissible resolved object, and
    /// `Io` for creation/open failures. No partially admitted guard is returned.
    /// An empty root returns `Invalid`; native filesystem path limits apply.
    pub fn open(root: &Path) -> Result<Self> {
        if !cfg!(any(unix, windows)) {
            return Err(Error::Unsupported("directory identity platform"));
        }
        if root.as_os_str().is_empty() {
            return Err(Error::Invalid("directory root"));
        }
        match fs::symlink_metadata(root) {
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => create(root, true)?,
            Err(error) => return Err(error.into()),
        }
        let path = fs::canonicalize(root).map_err(|error| {
            if error.kind() == std::io::ErrorKind::NotFound {
                Error::Conflict
            } else {
                error.into()
            }
        })?;
        Ok(Self {
            directory: Directory::open(path)?,
            parent: None,
        })
    }

    /// Admit one ordinary native component, creating it privately if absent.
    /// Invalid components/Windows aliases return `Invalid`; deeper nesting is
    /// `Unsupported`. Existing reparse/symlink aliases return `Conflict`, files
    /// return `Unsupported`. Validate the root before mutation and after handle
    /// admission. Creation/open failures are typed `Io`; no deletion is attempted.
    /// Unavailable native identity returns `Unsupported`, never an unbound guard.
    pub fn child(&self, name: &OsStr) -> Result<Self> {
        component(name)?;
        if self.parent.is_some() {
            return Err(Error::Unsupported("directory nesting"));
        }
        self.validate()?;
        let path = self.directory.path.join(name);
        match fs::symlink_metadata(&path) {
            Ok(metadata) => kind(&metadata)?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => create(&path, false)?,
            Err(error) => return Err(error.into()),
        }
        self.validate()?;
        let directory = Directory::open(path)?;
        self.validate()?;
        Ok(Self {
            directory,
            parent: Some(self.directory.clone()),
        })
    }

    /// The admitted absolute native path; using it alone does not revalidate it.
    pub fn path(&self) -> &Path {
        &self.directory.path
    }

    /// Check the retained root and directory against their current names.
    /// Disappearance, replacement, aliases, non-directories or loss of identity
    /// return `Conflict`. Windows no-delete sharing may prevent the replacement
    /// itself; Unix renames may succeed and invalidate this guard instead.
    pub fn validate(&self) -> Result<()> {
        if let Some(parent) = &self.parent {
            parent.validate()?;
        }
        self.directory.validate()
    }
}

impl Directory {
    fn open(path: PathBuf) -> Result<Arc<Self>> {
        kind(&fs::symlink_metadata(&path)?)?;
        let mut options = File::options();
        options.read(true);
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            // BACKUP_SEMANTICS opens directories; OPEN_REPARSE_POINT refuses
            // transparent leaf traversal. Share READ/WRITE, never DELETE, so the
            // held object cannot move while its full 128-bit named ID is read.
            options
                .custom_flags(0x02000000 | 0x00200000)
                .share_mode(0x1 | 0x2);
        }
        let file = options.open(&path)?;
        kind(&file.metadata()?)?;
        let identity = held_identity(&file, &path)
            .map_err(|_| Error::Unsupported("directory identity unavailable"))?;
        // Unix filesystems may return their native Unicode/case spelling.
        // Admit that leaf only, with unchanged parent and a real original name.
        let canonical = fs::canonicalize(&path)?;
        if canonical.parent() != path.parent() || (cfg!(windows) && canonical != path) {
            return Err(Error::Conflict);
        }
        kind(&fs::symlink_metadata(&path)?)?;
        let directory = Arc::new(Self {
            path: canonical,
            file,
            identity,
        });
        directory.validate()?;
        Ok(directory)
    }

    fn validate(&self) -> Result<()> {
        let check = || -> Result<()> {
            let metadata = fs::symlink_metadata(&self.path)?;
            kind(&metadata)?;
            kind(&self.file.metadata()?)?;
            if named_identity(&self.path, &metadata)? != self.identity
                || fs::canonicalize(&self.path)? != self.path
            {
                return Err(Error::Conflict);
            }
            Ok(())
        };
        check().map_err(|_| Error::Conflict)
    }
}

fn kind(metadata: &Metadata) -> Result<()> {
    let alias = metadata.file_type().is_symlink();
    #[cfg(windows)]
    let alias = {
        use std::os::windows::fs::MetadataExt;
        // All reparse kinds, including junctions, not only symbolic links.
        alias || metadata.file_attributes() & 0x400 != 0
    };
    if alias {
        Err(Error::Conflict)
    } else if !metadata.is_dir() {
        Err(Error::Unsupported("directory kind"))
    } else {
        Ok(())
    }
}

fn create(path: &Path, recursive: bool) -> Result<()> {
    let mut builder = fs::DirBuilder::new();
    builder.recursive(recursive);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(path)?;
    Ok(())
}

fn component(name: &OsStr) -> Result<()> {
    let mut components = Path::new(name).components();
    let bytes = name.as_encoded_bytes();
    if !matches!(components.next(), Some(Component::Normal(part)) if part == name)
        || components.next().is_some()
        || bytes.iter().any(|b| matches!(b, 0 | b'/' | b'\\'))
    {
        return Err(Error::Invalid("directory child component"));
    }
    #[cfg(windows)]
    {
        let stem = bytes.split(|b| *b == b'.').next().unwrap_or_default();
        let numbered_device = stem.get(..3).is_some_and(|prefix| {
            prefix.eq_ignore_ascii_case(b"COM") || prefix.eq_ignore_ascii_case(b"LPT")
        }) && stem
            .get(3..)
            .is_some_and(|suffix| matches!(suffix, [b'1'..=b'9'] | [0xc2, 0xb9 | 0xb2 | 0xb3]));
        if bytes.iter().any(|b| *b < 32 || b"<>:\"|?*".contains(b))
            || matches!(bytes.last(), Some(b'.' | b' '))
            || [
                b"CON".as_slice(),
                b"PRN",
                b"AUX",
                b"NUL",
                b"CONIN$",
                b"CONOUT$",
            ]
            .iter()
            .any(|reserved| stem.eq_ignore_ascii_case(reserved))
            || numbered_device
        {
            return Err(Error::Invalid("directory child component"));
        }
    }
    Ok(())
}
