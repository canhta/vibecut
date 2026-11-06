//! Queries the credentialed sound provider.
use super::{
    API, Arc, COMMERCIAL_FILTER, Error, FIELDS, HttpRequest, HttpTransport, LibrarySound,
    MAX_QUERY, MAX_SEARCH_BYTES, MAX_TEXT, MAX_URL, RawPage, RawSound, SearchPage, SearchRequest,
    SearchSource, Sound, SoundClient, SoundResult, Token, fmt,
};
impl Token {
    pub fn new(value: impl Into<String>) -> Sound<Self> {
        let value = value.into();
        if value.is_empty()
            || value.trim() != value
            || value.len() > 4096
            || value.bytes().any(|b| b.is_ascii_control() || b == b' ')
        {
            return Err(Error::NoCredentials);
        }
        Ok(Self(value))
    }
    /// The composition root may pass an OS-keychain token here. Only the environment
    /// is read by this contract; keychain persistence stays app-owned.
    pub fn from_env() -> Sound<Option<Self>> {
        match std::env::var("FREESOUND_API_KEY") {
            Ok(value) => Ok(Some(Self::new(value)?)),
            Err(std::env::VarError::NotPresent) => Ok(None),
            Err(std::env::VarError::NotUnicode(_)) => Err(Error::NoCredentials),
        }
    }
    pub(super) fn expose(&self) -> &str {
        &self.0
    }
}
impl fmt::Debug for Token {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("Token([redacted])")
    }
}
impl Default for SearchRequest {
    fn default() -> Self {
        Self {
            query: String::new(),
            page: 1,
            page_size: 15,
            commercial_only: true,
        }
    }
}
impl SearchRequest {
    pub(super) fn validate(&self) -> Sound<()> {
        if self.query.len() > MAX_QUERY
            || !(1..=1000).contains(&self.page)
            || !(1..=150).contains(&self.page_size)
        {
            return Err(Error::Invalid("search request"));
        }
        Ok(())
    }
}
impl SoundResult {
    pub fn library_sound(&self) -> LibrarySound {
        LibrarySound {
            id: self.id,
            name: self.name.as_str().into(),
            author: self.author.as_str().into(),
            licence: self.licence.as_str().into(),
            attribution: self.attribution.as_str().into(),
            source_url: self.source_url.clone(),
            audio_url: self.audio_url.clone(),
            downloads: self.downloads,
            duration_ms: self.duration_ms,
        }
    }
}
impl SoundClient {
    pub fn new(token: Token, transport: Arc<dyn HttpTransport>) -> Self {
        Self { token, transport }
    }
    /// Production composition helper; transport policy stays in ai's shared owner.
    pub fn ureq(token: Token) -> Self {
        Self::new(token, Arc::new(ai::transport::UreqTransport::new()))
    }
    pub fn search(&self, request: &SearchRequest) -> Sound<SearchPage> {
        request.validate()?;
        let mut url = format!(
            "{API}?query={}&sort=downloads_desc&page={}&page_size={}&fields={}",
            encode(&request.query),
            request.page,
            request.page_size,
            encode(FIELDS)
        );
        if request.commercial_only {
            url.push_str("&filter=");
            url.push_str(&encode(COMMERCIAL_FILTER));
        }
        let reply = self
            .transport
            .send_bounded(
                HttpRequest {
                    method: "GET",
                    url,
                    headers: vec![(
                        "Authorization".to_owned(),
                        format!("Token {}", self.token.expose()),
                    )],
                    body: Vec::new(),
                },
                MAX_SEARCH_BYTES,
            )
            .map_err(Error::from)?;
        match reply.status {
            200 => parse_page(&reply.body, request),
            401 => Err(Error::Auth),
            402 => Err(Error::Quota),
            403 => Err(Error::Auth),
            404 => Err(Error::InvalidResponse),
            408 | 502 | 503 | 504 => Err(Error::Server),
            429 => Err(Error::RateLimit {
                retry_after_seconds: retry_after(&reply.headers),
            }),
            500..=599 => Err(Error::Server),
            _ => Err(Error::InvalidResponse),
        }
    }
}
fn parse_page(bytes: &[u8], request: &SearchRequest) -> Sound<SearchPage> {
    let raw: RawPage = serde_json::from_slice(bytes).map_err(|_| Error::InvalidResponse)?;
    if raw.results.len() > request.page_size as usize || raw.count > 100_000_000 {
        return Err(Error::InvalidResponse);
    }
    let mut results = Vec::new();
    for sound in raw.results {
        let Some(result) = parse_sound(sound)? else {
            continue;
        };
        // Defence in depth if the server ignored the filter: never present an
        // NC/Sampling+ result under the commercial-only switch.
        if request.commercial_only && !commercial(&result.licence) {
            continue;
        }
        results.push(result);
    }
    Ok(SearchPage {
        count: raw.count,
        page: request.page,
        page_size: request.page_size,
        results,
        has_next: raw.next.is_some(),
        source: SearchSource::Remote,
    })
}
fn parse_sound(raw: RawSound) -> Sound<Option<SoundResult>> {
    if raw.id == 0
        || raw.name.len() > MAX_TEXT
        || raw.username.len() > MAX_TEXT
        || raw.license.len() > MAX_TEXT
        || raw.name.is_empty()
        || raw.username.is_empty()
        || raw.license.is_empty()
        || !url(&raw.url)
    {
        return Err(Error::InvalidResponse);
    }
    let audio_url = raw
        .previews
        .get("preview-hq-mp3")
        .or_else(|| raw.previews.get("preview-lq-mp3"))
        .ok_or(Error::InvalidResponse)?
        .clone();
    if !url(&audio_url) {
        return Err(Error::InvalidResponse);
    }
    let duration = raw.duration.ok_or(Error::InvalidResponse)?;
    if !duration.is_finite() || !(0.0..=86_400.0).contains(&duration) {
        return Err(Error::InvalidResponse);
    }
    let duration_ms = (duration * 1000.0).round() as u64;
    let attribution = format!("{} by {} ({})", raw.name, raw.username, raw.license);
    Ok(Some(SoundResult {
        id: raw.id,
        name: raw.name,
        author: raw.username,
        licence: raw.license,
        attribution,
        source_url: raw.url,
        audio_url,
        duration_ms,
        downloads: raw.num_downloads,
    }))
}
pub(super) fn url(value: &str) -> bool {
    value.len() <= MAX_URL
        && value.starts_with("https://")
        && !value
            .bytes()
            .any(|b| b.is_ascii_control() || b == b' ' || b == b'"' || b == b'\\')
}
pub fn commercial(licence: &str) -> bool {
    matches!(licence, "Creative Commons 0" | "Attribution")
}
pub(super) fn retry_after(headers: &[(String, String)]) -> Option<u32> {
    headers
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case("retry-after"))
        .and_then(|(_, value)| value.trim().parse().ok())
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
