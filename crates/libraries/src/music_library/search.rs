//! Queries the open music aggregator.
use super::{
    API, Arc, Error, HttpRequest, HttpTransport, LICENCES, MAX_PAGE_SIZE, MAX_QUERY,
    MAX_SEARCH_BYTES, MAX_URL, Music, MusicClient, MusicResult, MusicToken, RawPage, RawTrack,
    SearchPage, SearchRequest, SearchSource, fmt,
};
use transport::retry_after;
impl MusicToken {
    pub fn new(value: impl Into<String>) -> Music<Self> {
        let value = value.into();
        if value.is_empty()
            || value.trim() != value
            || value.len() > 4096
            || value.bytes().any(|b| b.is_ascii_control() || b == b' ')
        {
            return Err(Error::Auth);
        }
        Ok(Self(value))
    }
    /// The composition root may pass a key stored by its settings surface here.
    /// Only the environment is read by this contract; persistence stays app-owned.
    pub fn from_env() -> Music<Option<Self>> {
        match std::env::var("OPENVERSE_API_KEY") {
            Ok(value) => Ok(Some(Self::new(value)?)),
            Err(std::env::VarError::NotPresent) => Ok(None),
            Err(std::env::VarError::NotUnicode(_)) => Err(Error::Auth),
        }
    }
    pub(super) fn expose(&self) -> &str {
        &self.0
    }
}
impl fmt::Debug for MusicToken {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("MusicToken([redacted])")
    }
}
impl Default for SearchRequest {
    fn default() -> Self {
        Self {
            query: String::new(),
            tag: None,
            page: 1,
            page_size: 15,
        }
    }
}
impl SearchRequest {
    pub(super) fn validate(&self) -> Music<()> {
        if self.query.len() > MAX_QUERY
            || self.tag.as_ref().is_some_and(|tag| tag.len() > MAX_QUERY)
            || !(1..=1000).contains(&self.page)
            || !(1..=MAX_PAGE_SIZE).contains(&self.page_size)
            || (!self.query.is_empty() && self.tag.is_some())
        {
            return Err(Error::Invalid("search request"));
        }
        Ok(())
    }
}
impl MusicResult {
    /// Admit NFC provenance through [`project::LibraryMusic::validate`].
    /// Its identifier, UTF-8 byte, HTTPS URL, licence/deed and millisecond bounds
    /// apply here. Returns `MusicError::Project` retaining the typed cause before
    /// any filesystem, network or project operation.
    pub fn library_music(&self) -> Music<project::LibraryMusic> {
        let music = project::LibraryMusic {
            id: self.id.as_str().into(),
            title: self.title.as_str().into(),
            author: self.author.as_str().into(),
            licence: self.licence.as_str().into(),
            licence_url: self.licence_url.clone(),
            attribution: self.attribution.as_str().into(),
            source_url: self.source_url.clone(),
            audio_url: self.audio_url.clone(),
            duration_ms: self.duration_ms,
        };
        music.validate()?;
        Ok(music)
    }
}
impl From<project::LibraryMusic> for MusicResult {
    fn from(music: project::LibraryMusic) -> Self {
        Self {
            id: music.id.as_str().to_owned(),
            title: music.title.as_str().to_owned(),
            author: music.author.as_str().to_owned(),
            licence: music.licence.as_str().to_owned(),
            licence_url: music.licence_url,
            attribution: music.attribution.as_str().to_owned(),
            source_url: music.source_url,
            audio_url: music.audio_url,
            duration_ms: music.duration_ms,
        }
    }
}
impl MusicClient {
    pub fn new(token: Option<MusicToken>, transport: Arc<dyn HttpTransport>) -> Self {
        Self { token, transport }
    }
    /// Production composition helper; transport policy stays in ai's shared owner.
    pub fn ureq(token: Option<MusicToken>) -> Self {
        Self::new(token, Arc::new(transport::UreqTransport::new()))
    }
    pub fn search(&self, request: &SearchRequest) -> Music<SearchPage> {
        request.validate()?;
        let mut url = format!(
            "{API}?category=music&license={}&page={}&page_size={}&filter_dead=true&mature=false",
            encode(LICENCES),
            request.page,
            request.page_size
        );
        if !request.query.is_empty() {
            url.push_str("&q=");
            url.push_str(&encode(&request.query));
        }
        if let Some(tag) = &request.tag {
            url.push_str("&tags=");
            url.push_str(&encode(tag));
        }
        let headers = self
            .token
            .as_ref()
            .map(|token| {
                vec![(
                    "Authorization".to_owned(),
                    format!("Bearer {}", token.expose()),
                )]
            })
            .unwrap_or_default();
        let reply = self
            .transport
            .send_bounded(
                HttpRequest {
                    method: "GET",
                    url,
                    headers,
                    body: Vec::new(),
                },
                MAX_SEARCH_BYTES,
            )
            .map_err(Error::from)?;
        match reply.status {
            200 => parse_page(&reply.body, request),
            401 | 403 => Err(Error::Auth),
            402 => Err(Error::Quota),
            404 => Err(Error::InvalidResponse),
            408 | 502 | 503 | 504 => Err(Error::Network),
            429 => Err(Error::RateLimit {
                retry_after_seconds: retry_after(&reply.headers),
            }),
            500..=599 => Err(Error::Server),
            _ => Err(Error::InvalidResponse),
        }
    }
}
fn parse_page(bytes: &[u8], request: &SearchRequest) -> Music<SearchPage> {
    let raw: RawPage = serde_json::from_slice(bytes).map_err(|_| Error::InvalidResponse)?;
    if raw.results.len() > request.page_size as usize
        || raw.result_count > 100_000_000
        || raw.page_count > 10_000_000
    {
        return Err(Error::InvalidResponse);
    }
    let mut results = Vec::new();
    for track in raw.results {
        // Defence in depth if the server ignored the filter or sent an
        // incomplete row: only complete, admitted results are ever shown.
        if let Some(result) = serde_json::from_value(track).ok().and_then(parse_track) {
            results.push(result);
        }
    }
    let has_next = u64::from(request.page) < raw.page_count;
    Ok(SearchPage {
        count: raw.result_count,
        page: request.page,
        page_size: request.page_size,
        results,
        has_next,
        source: SearchSource::Remote,
    })
}
fn parse_track(raw: RawTrack) -> Option<MusicResult> {
    // Transport spelling only: project admission owns every accepted identity,
    // version and matching deed. No inferred version or repaired provenance.
    let family = match raw.license.as_deref()? {
        "cc0" => "CC0",
        "by" => "CC BY",
        _ => return None,
    };
    let result = MusicResult {
        id: raw.id?,
        title: raw.title?,
        author: raw.creator?,
        licence: format!("{family} {}", raw.license_version?),
        licence_url: raw.license_url?,
        attribution: raw.attribution?,
        source_url: raw.foreign_landing_url?,
        audio_url: raw.url?,
        duration_ms: u64::try_from(raw.duration?).ok()?,
    };
    result.library_music().ok().map(Into::into)
}
pub(super) fn url(value: &str) -> bool {
    value.len() <= MAX_URL
        && value.starts_with("https://")
        && !value
            .bytes()
            .any(|b| b.is_ascii_control() || b == b' ' || b == b'"' || b == b'\\')
}
fn encode(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            out.push(char::from(byte));
        } else {
            out.push('%');
            out.push(hex(byte >> 4));
            out.push(hex(byte & 0x0f));
        }
    }
    out
}
pub(super) fn hex(value: u8) -> char {
    match value & 0x0f {
        0..=9 => char::from(b'0' + (value & 0x0f)),
        _ => char::from(b'A' + ((value & 0x0f) - 10)),
    }
}
