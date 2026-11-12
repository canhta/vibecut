//! Plays selected music through the app-owned preview sink.
use super::{
    Arc, AtomicBool, Error, Music, MusicDownloads, MusicPreview, MusicResult, Mutex, Ordering,
    PreviewClip, PreviewSink, validate_result,
};
impl MusicPreview {
    pub fn new(downloads: Arc<MusicDownloads>, sink: Arc<dyn PreviewSink>) -> Self {
        Self {
            downloads,
            sink,
            active: Mutex::new(None),
        }
    }
    /// Remain active until terminal sink completion. `cancelled` also covers an
    /// owner stopping a queued worker before this call begins; it is checked
    /// before cache/network work and throughout download and sink playback.
    pub fn play(&self, music: &MusicResult, cancelled: &dyn Fn() -> bool) -> Music<()> {
        if cancelled() {
            return Err(Error::Cancelled);
        }
        validate_result(music)?;
        let cancel = Arc::new(AtomicBool::new(false));
        {
            let mut active = self.active.lock().map_err(|_| Error::LibraryCorrupt)?;
            if let Some(old) = active.replace(cancel.clone()) {
                old.store(true, Ordering::Release);
            }
        }
        let is_cancelled = || cancel.load(Ordering::Acquire) || cancelled();
        let result = (|| {
            if is_cancelled() {
                return Err(Error::Cancelled);
            }
            let cached = match self.downloads.library.cached(&music.audio_url)? {
                Some(cached) => cached,
                None => self
                    .downloads
                    .download(&music.audio_url, &is_cancelled, &|_, _| {})?,
            };
            if is_cancelled() {
                return Err(Error::Cancelled);
            }
            let bytes = cached.bytes()?;
            self.sink
                .play(PreviewClip {
                    name: &music.title,
                    audio_url: &music.audio_url,
                    bytes: &bytes,
                    cancelled: &is_cancelled,
                })
                .map_err(Error::from)
        })();
        let mut active = self.active.lock().map_err(|_| Error::LibraryCorrupt)?;
        if active
            .as_ref()
            .is_some_and(|current| Arc::ptr_eq(current, &cancel))
        {
            *active = None;
        }
        result
    }
    pub fn stop(&self) -> Music<()> {
        if let Some(cancel) = self
            .active
            .lock()
            .map_err(|_| Error::LibraryCorrupt)?
            .take()
        {
            cancel.store(true, Ordering::Release);
        }
        self.sink.stop().map_err(Error::from)
    }
    pub fn cancel(&self) {
        if let Ok(mut active) = self.active.lock()
            && let Some(cancel) = active.take()
        {
            cancel.store(true, Ordering::Release);
        }
    }
}
