//! Plays selected library sounds through a preview sink.
use super::{
    Arc, AtomicBool, Error, Mutex, Ordering, PreviewClip, PreviewSink, Sound, SoundDownloads,
    SoundPreview, SoundResult, validate_result,
};
impl SoundPreview {
    pub fn new(downloads: Arc<SoundDownloads>, sink: Arc<dyn PreviewSink>) -> Self {
        Self {
            downloads,
            sink,
            active: Mutex::new(None),
        }
    }
    pub fn play(&self, sound: &SoundResult) -> Sound<()> {
        validate_result(sound)?;
        let cancel = Arc::new(AtomicBool::new(false));
        {
            let mut active = self.active.lock().map_err(|_| Error::LibraryCorrupt)?;
            if let Some(old) = active.replace(cancel.clone()) {
                old.store(true, Ordering::Release);
            }
        }
        let result = (|| {
            let cached = match self.downloads.library.cached(&sound.audio_url)? {
                Some(cached) => cached,
                None => self.downloads.download(
                    &sound.audio_url,
                    &|| cancel.load(Ordering::Acquire),
                    &|_, _| {},
                )?,
            };
            if cancel.load(Ordering::Acquire) {
                return Err(Error::Cancelled);
            }
            let bytes = cached.bytes()?;
            self.sink.play(PreviewClip {
                name: &sound.name,
                audio_url: &sound.audio_url,
                bytes: &bytes,
                cancelled: &|| cancel.load(Ordering::Acquire),
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
