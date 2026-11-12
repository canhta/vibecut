//! Serves the music control commands from the one library adapter.
use super::{
    Error, Music, MusicClient, MusicLibrary, MusicResult, SavedMusic, SearchPage, SearchRequest,
    SearchSource,
};
use engine::control::{
    MusicAccess, MusicEntry, MusicPage, MusicQuery, MusicSource, SavedMusicEntry,
};

impl From<MusicResult> for MusicEntry {
    fn from(music: MusicResult) -> Self {
        Self {
            id: music.id,
            title: music.title,
            author: music.author,
            licence: music.licence,
            licence_url: music.licence_url,
            attribution: music.attribution,
            source_url: music.source_url,
            audio_url: music.audio_url,
            duration_ms: music.duration_ms,
        }
    }
}

impl From<&MusicEntry> for MusicResult {
    fn from(music: &MusicEntry) -> Self {
        Self {
            id: music.id.clone(),
            title: music.title.clone(),
            author: music.author.clone(),
            licence: music.licence.clone(),
            licence_url: music.licence_url.clone(),
            attribution: music.attribution.clone(),
            source_url: music.source_url.clone(),
            audio_url: music.audio_url.clone(),
            duration_ms: music.duration_ms,
        }
    }
}

impl SearchPage {
    /// Project a page onto the control view.
    pub fn into_control(self) -> MusicPage {
        MusicPage {
            source: match self.source {
                SearchSource::Remote => MusicSource::Remote,
                SearchSource::Local => MusicSource::Local,
            },
            count: self.count,
            page: self.page,
            page_size: self.page_size,
            has_next: self.has_next,
            results: self.results.into_iter().map(MusicEntry::from).collect(),
        }
    }
}

impl From<SavedMusic> for SavedMusicEntry {
    fn from(saved: SavedMusic) -> Self {
        Self {
            music: MusicEntry::from(saved.music),
            saved_at_unix_ms: saved.saved_at_unix_ms,
        }
    }
}

impl MusicLibrary {
    /// Serve the three control operations over this library. `remote` is the
    /// scoped approval for a deliberate human remote search: its closure resolves
    /// the user credential and transport lazily. When it is absent, remote search
    /// refuses before any credential or network work while Saved, save and unsave
    /// keep working offline. A key is never a command parameter.
    pub fn with_control<T>(
        &self,
        remote: Option<&dyn Fn() -> Music<MusicClient>>,
        serve: impl FnOnce(&MusicAccess<'_>) -> T,
    ) -> T {
        let search = |query: &MusicQuery| -> engine::control::Result<MusicPage> {
            let request = SearchRequest {
                query: query.query.clone(),
                tag: query.tag.clone(),
                page: query.page,
                page_size: query.page_size,
            };
            let page = if query.saved {
                self.search_local(&request)?
            } else {
                let remote = remote.ok_or(Error::Invalid("music search approval"))?;
                remote()?.search(&request)?
            };
            Ok(page.into_control())
        };
        let save = |music: &MusicEntry| -> engine::control::Result<SavedMusicEntry> {
            Ok(self.save(&MusicResult::from(music))?.into())
        };
        let unsave = |id: &str| -> engine::control::Result<bool> { Ok(self.unsave(id)?) };
        let access = MusicAccess::new(&search, &save, &unsave);
        serve(&access)
    }
}
