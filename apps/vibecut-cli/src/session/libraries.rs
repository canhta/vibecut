//! Lends the process font and sound stores to registry requests.
use crate::args::CliError;
use engine::control::{FontAccess, SoundAccess};
use engine::jobs::JobRunner;
use libraries::fonts::FontStore;
use libraries::sounds::{
    SearchRequest, SoundClient, SoundLibrary, SoundResult as LibrarySound, Token,
};
use std::path::Path;

/// Build the three font operations from the per-user store and serve them. A host
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
    let fonts = store
        .as_ref()
        .map(|_| FontAccess::new(&list, &load, &download));
    serve(fonts.as_ref())
}

/// Open one sound-library root, read the user credential for this process and serve
/// the three library operations. The token never enters a command parameter; the
/// access object chooses remote search when it is present and the saved local
/// library when it is absent.
pub(crate) fn with_sounds<T>(
    root: &Path,
    serve: impl FnOnce(&SoundAccess<'_>) -> Result<T, CliError>,
) -> Result<T, CliError> {
    let library = SoundLibrary::open(root)?;
    let token = Token::from_env()?;
    let search =
        |query: &engine::control::SoundQuery| -> engine::Result<engine::control::SoundPage> {
            let request = SearchRequest {
                query: query.query.clone(),
                page: query.page,
                page_size: query.page_size,
                commercial_only: query.commercial_only,
            };
            let page = match &token {
                Some(token) => SoundClient::ureq(token.clone()).search(&request)?,
                None => library.search_local(&request)?,
            };
            Ok(page.into_control(query.commercial_only))
        };
    let save =
        |sound: &engine::control::SoundEntry| -> engine::Result<engine::control::SavedSoundEntry> {
            Ok(library.save(&LibrarySound::from(sound))?.into())
        };
    let unsave = |id: u64| -> engine::Result<bool> { Ok(library.unsave(id)?) };
    let access = SoundAccess::new(&search, &save, &unsave);
    serve(&access)
}
