//! Projects sound-library records onto the engine control views.
use super::{SavedSound, SearchPage, SearchSource, SoundResult};
use engine::control::{SavedSoundEntry, SoundEntry, SoundPage, SoundSource};

impl From<SoundResult> for SoundEntry {
    fn from(sound: SoundResult) -> Self {
        Self {
            id: sound.id,
            name: sound.name,
            author: sound.author,
            licence: sound.licence,
            attribution: sound.attribution,
            source_url: sound.source_url,
            audio_url: sound.audio_url,
            duration_ms: sound.duration_ms,
            downloads: sound.downloads,
        }
    }
}

impl From<&SoundEntry> for SoundResult {
    fn from(sound: &SoundEntry) -> Self {
        Self {
            id: sound.id,
            name: sound.name.clone(),
            author: sound.author.clone(),
            licence: sound.licence.clone(),
            attribution: sound.attribution.clone(),
            source_url: sound.source_url.clone(),
            audio_url: sound.audio_url.clone(),
            duration_ms: sound.duration_ms,
            downloads: sound.downloads,
        }
    }
}

impl SearchPage {
    /// Project a page onto the control view, echoing the commercial-only switch the
    /// request asked for because the provider payload does not carry it.
    pub fn into_control(self, commercial_only: bool) -> SoundPage {
        SoundPage {
            source: match self.source {
                SearchSource::Remote => SoundSource::Remote,
                SearchSource::Local => SoundSource::Local,
            },
            count: self.count,
            page: self.page,
            page_size: self.page_size,
            has_next: self.has_next,
            commercial_only,
            results: self.results.into_iter().map(SoundEntry::from).collect(),
        }
    }
}

impl From<SavedSound> for SavedSoundEntry {
    fn from(saved: SavedSound) -> Self {
        Self {
            id: saved.sound.id,
            saved_at_unix_ms: saved.saved_at_unix_ms,
            attribution: saved.sound.attribution,
            licence: saved.sound.licence,
        }
    }
}
