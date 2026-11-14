//! Plays selected library sounds through a preview sink.
use super::{
    Arc, AtomicBool, Error, Mutex, Ordering, PreviewClip, PreviewSink, Sound, SoundDownloads,
    SoundPreview, SoundResult, validate_preview,
};
impl SoundPreview {
    pub fn new(downloads: Arc<SoundDownloads>, sink: Arc<dyn PreviewSink>) -> Self {
        Self {
            downloads,
            sink,
            active: Mutex::new(None),
        }
    }
    /// Remain active until terminal sink completion. `cancelled` also covers an
    /// owner stopping a queued worker before this call begins; it is checked
    /// before cache/network work and throughout download and sink playback.
    pub fn play(&self, sound: &SoundResult, cancelled: &dyn Fn() -> bool) -> Sound<()> {
        if cancelled() {
            return Err(Error::Cancelled);
        }
        validate_preview(sound)?;
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
            let cached = match self.downloads.library.cached(&sound.audio_url)? {
                Some(cached) => cached,
                None => self
                    .downloads
                    .download(&sound.audio_url, &is_cancelled, &|_, _| {})?,
            };
            if is_cancelled() {
                return Err(Error::Cancelled);
            }
            let bytes = cached.bytes()?;
            self.sink.play(PreviewClip {
                name: &sound.name,
                audio_url: &sound.audio_url,
                bytes: &bytes,
                cancelled: &is_cancelled,
            })
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
    pub fn stop(&self) -> Sound<()> {
        if let Some(cancel) = self
            .active
            .lock()
            .map_err(|_| Error::LibraryCorrupt)?
            .take()
        {
            cancel.store(true, Ordering::Release);
        }
        self.sink.stop()
    }
    pub fn cancel(&self) {
        if let Ok(mut active) = self.active.lock()
            && let Some(cancel) = active.take()
        {
            cancel.store(true, Ordering::Release);
        }
    }
}
