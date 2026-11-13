//! Lends the process font, sound and music stores to registry requests.
use crate::args::CliError;
use engine::control::{FontAccess, MusicAccess, SoundAccess, StickerAccess};
use engine::jobs::JobRunner;
use libraries::fonts::FontStore;
use libraries::music::{MusicClient, MusicLibrary, MusicToken};
use libraries::sounds::{
    SearchRequest, SoundClient, SoundLibrary, SoundResult as LibrarySound, Token,
};
use std::path::Path;

pub(crate) fn prepare_project_fonts(project: &project::Project) -> Result<(), CliError> {
    FontStore::native()?.load_project(project)?;
    Ok(())
}

/// Build the font operations from the per-user store and serve them. A host
/// without a user cache lends `None`; `fonts.*` then refuses unavailable.
pub(crate) fn with_fonts<T>(serve: impl FnOnce(Option<&FontAccess<'_>>) -> T) -> T {
    let store = FontStore::native().ok();
    let list = |query: Option<&str>| match &store {
        Some(store) => store.list(query),
        None => Err(engine::FontError::Unavailable.into()),
    };
    let load = |family: &str| match &store {
        Some(store) => store.load(family),
        None => Err(engine::FontError::Unavailable.into()),
    };
    let download = |runner: &mut JobRunner, family: &str, accepted: bool| match &store {
        Some(store) => store
            .submit_download(runner, family, accepted)
            .map(|(job, _)| job),
        None => Err(engine::FontError::Unavailable.into()),
    };
    let import = |runner: &mut JobRunner, path: &Path, accepted: bool| match &store {
        Some(store) => store
            .submit_import(runner, path.to_owned(), accepted)
            .map(|(job, _)| job),
        None => Err(engine::FontError::Unavailable.into()),
    };
    let fonts = store
        .as_ref()
        .map(|_| FontAccess::new(&list, &load, &download, &import));
    serve(fonts.as_ref())
}

/// The bundled sticker catalog's search, the page `stickers.search` returns.
fn sticker_search(
    query: &engine::control::StickerQuery,
) -> engine::control::Result<engine::control::StickerPage> {
    Ok(libraries::stickers::search_catalog(
        &query.query,
        query.category,
        query.limit,
    ))
}

/// The bundled sticker catalog's entry for an id, refused when the catalog does not know it.
fn sticker_resolve(id: &str) -> engine::control::Result<engine::control::StickerEntry> {
    libraries::stickers::resolve_sticker(id).ok_or(engine::control::Error::Edit(
        engine::EditError::Invalid("unknown sticker id"),
    ))
}

/// The bundled sticker catalog as an MCP inbox lends it. It has no store to open.
pub(crate) fn sticker_lend() -> mcp::StickerLend {
    mcp::StickerLend {
        search: sticker_search,
        resolve: sticker_resolve,
    }
}

/// Lend the bundled sticker catalog. It has no store to open, so the access always exists.
pub(crate) fn with_stickers<T>(serve: impl FnOnce(&StickerAccess<'_>) -> T) -> T {
    serve(&StickerAccess::new(&sticker_search, &sticker_resolve))
}

/// Open one sound-library root and serve its three operations. Only search reads
/// the user credential; local mutations do not need it. The token never enters a
/// command parameter; the access object chooses remote search when it is present
/// and the saved local library when it is absent.
pub(crate) fn with_sounds<T>(
    root: &Path,
    serve: impl FnOnce(&SoundAccess<'_>) -> Result<T, CliError>,
) -> Result<T, CliError> {
    let library = SoundLibrary::open(root)?;
    let search =
        |query: &engine::control::SoundQuery| -> engine::control::Result<engine::control::SoundPage> {
            let request = SearchRequest {
                query: query.query.clone(),
                page: query.page,
                page_size: query.page_size,
                commercial_only: query.commercial_only,
            };
            let page = match Token::from_env()? {
                Some(token) => SoundClient::ureq(token).search(&request)?,
                None => library.search_local(&request)?,
            };
            Ok(page.into_control(query.commercial_only))
        };
    let save =
        |sound: &engine::control::SoundEntry| -> engine::control::Result<engine::control::SavedSoundEntry> {
            Ok(library.save(&LibrarySound::from(sound))?.into())
        };
    let unsave = |id: u64| -> engine::control::Result<bool> { Ok(library.unsave(id)?) };
    let access = SoundAccess::new(&search, &save, &unsave);
    serve(&access)
}

/// Open one music-library root and serve its three operations through the one
/// library adapter. The remote resolver is the scoped approval for a deliberate
/// human search: it resolves the optional user key lazily and is never called by
/// Saved, save or unsave. The key never enters a command parameter.
pub(crate) fn with_music<T>(
    root: &Path,
    remote: bool,
    serve: impl FnOnce(&MusicAccess<'_>) -> Result<T, CliError>,
) -> Result<T, CliError> {
    let library = MusicLibrary::open(root)?;
    let client = || -> libraries::music::Music<MusicClient> {
        Ok(MusicClient::ureq(MusicToken::from_env()?))
    };
    if remote {
        library.with_control(Some(&client), serve)
    } else {
        library.with_control(None, serve)
    }
}
