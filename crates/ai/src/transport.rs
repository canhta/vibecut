//! Shared provider transport: redacted key/request types, bounded HTTPS, PCM16 WAV
//! encoding and multipart form construction. Endpoint policy stays in each adapter.
mod text_model;
use crate::{AudioWindow, Error};
use std::fmt;
use std::io::Read;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;
pub(crate) use text_model::{MAX_OUTPUT, OPENAI_MODEL, TextModel, TextRequest};

/// Response bodies are transcripts or error envelopes; cap hostile responses.
pub const MAX_RESPONSE_BYTES: usize = 1024 * 1024;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(120);

/// In-memory API key injected by the composition root. Never serialized or logged.
#[derive(Clone)]
pub struct ApiKey(String);
impl ApiKey {
    /// Validates a printable ASCII key of at most 4096 bytes; Debug always redacts it.
    pub fn new(value: impl Into<String>) -> Result<Self, Error> {
        let value = value.into();
        if value.is_empty() || value.trim() != value || !value.is_ascii() || value.len() > 4096 {
            return Err(Error::Invalid("API key"));
        }
        Ok(Self(value))
    }
    pub(crate) fn expose(&self) -> &str {
        &self.0
    }
}
impl fmt::Debug for ApiKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("ApiKey([redacted])")
    }
}

/// Transport request; Debug never prints headers (authorization) or the body (audio).
pub struct HttpRequest {
    pub method: &'static str,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}
impl fmt::Debug for HttpRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("HttpRequest")
            .field("method", &self.method)
            .field("url", &self.url)
            .field("headers", &"[redacted]")
            .field("body_bytes", &self.body.len())
            .finish()
    }
}

/// Provider response; Debug never prints the body (transcript or error text).
pub struct HttpResponse {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}
impl fmt::Debug for HttpResponse {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("HttpResponse")
            .field("status", &self.status)
            .field("headers", &"[redacted]")
            .field("body_bytes", &self.body.len())
            .finish()
    }
}

/// Blocking HTTPS seam. Implementations must bound connect/read/write time and
/// must not retry; the engine job runner owns the attempt budget and backoff.
pub trait HttpTransport: Send + Sync {
    fn send(&self, request: HttpRequest) -> Result<HttpResponse, Error>;
    /// Send with an operation-specific response ceiling. Audio responses can exceed
    /// the small JSON envelope ceiling, so adapters pass the bound derived from the
    /// approved duration. The production transport enforces it while reading; a
    /// simple implementation may check after buffering.
    fn send_bounded(&self, request: HttpRequest, max_bytes: usize) -> Result<HttpResponse, Error> {
        if max_bytes == 0 {
            return Err(Error::Payload);
        }
        let response = self.send(request)?;
        if response.body.len() > max_bytes {
            return Err(Error::Payload);
        }
        Ok(response)
    }
}

/// Streaming GET response; the caller owns byte limits, cancellation and retries.
/// Body is never logged or buffered by this transport.
pub struct HttpStreamResponse {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Box<dyn Read + Send>,
}
pub trait DownloadTransport: Send + Sync {
    fn get(&self, url: &str, headers: &[(String, String)]) -> Result<HttpStreamResponse, Error>;
}
/// Resolve a bounded HTTPS download redirect; signed CDN queries are kept private.
pub fn download_redirect(base: &str, location: &str) -> Result<String, Error> {
    if location.len() > 8192 {
        return Err(Error::Payload);
    }
    let url = url::Url::parse(base)
        .and_then(|u| u.join(location))
        .map_err(|_| Error::Invalid("download redirect"))?;
    if url.scheme() != "https"
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
        || url.as_str().len() > 8192
    {
        return Err(Error::Invalid("download redirect"));
    }
    Ok(url.into())
}
/// Production transport with connect/read/write/total timeouts and redirects disabled
/// so an authorization header can never follow a redirect to another host.
pub struct UreqTransport {
    agent: ureq::Agent,
}
impl UreqTransport {
    /// Uses a 120 s total/read/write timeout, a 10 s connect timeout and no redirects.
    pub fn new() -> Self {
        Self::agent(REQUEST_TIMEOUT)
    }
    /// Same policy with an explicit nonzero total/read/write timeout.
    pub fn with_timeout(timeout: Duration) -> Result<Self, Error> {
        if timeout.is_zero() {
            return Err(Error::Invalid("transport timeout"));
        }
        Ok(Self::agent(timeout))
    }
    fn agent(timeout: Duration) -> Self {
        Self {
            agent: ureq::AgentBuilder::new()
                .timeout_connect(timeout.min(Duration::from_secs(10)))
                .timeout_read(timeout)
                .timeout_write(timeout)
                .timeout(timeout)
                .redirects(0)
                .build(),
        }
    }
    fn read_response(response: ureq::Response, max_bytes: usize) -> Result<HttpResponse, Error> {
        if max_bytes == 0 {
            return Err(Error::Payload);
        }
        let status = response.status();
        let mut headers = Vec::new();
        for name in response.headers_names() {
            if let Some(value) = response.header(&name) {
                headers.push((name, value.to_owned()));
            }
        }
        let mut body = Vec::new();
        response
            .into_reader()
            .take(max_bytes as u64 + 1)
            .read_to_end(&mut body)
            .map_err(|_| Error::Network)?;
        if body.len() > max_bytes {
            return Err(Error::Payload);
        }
        Ok(HttpResponse {
            status,
            headers,
            body,
        })
    }
}
impl DownloadTransport for UreqTransport {
    fn get(&self, url: &str, headers: &[(String, String)]) -> Result<HttpStreamResponse, Error> {
        let mut request = self.agent.get(url);
        for (name, value) in headers {
            request = request.set(name, value);
        }
        let response = match request.call() {
            Ok(r) | Err(ureq::Error::Status(_, r)) => r,
            Err(ureq::Error::Transport(_)) => return Err(Error::Network),
        };
        let status = response.status();
        let names: std::collections::BTreeSet<_> = response
            .headers_names()
            .into_iter()
            .map(|name| name.to_ascii_lowercase())
            .collect();
        let headers = names
            .into_iter()
            .flat_map(|name| {
                response
                    .all(&name)
                    .into_iter()
                    .map(move |v| (name.clone(), v.to_owned()))
            })
            .collect();
        Ok(HttpStreamResponse {
            status,
            headers,
            body: response.into_reader(),
        })
    }
}
impl Default for UreqTransport {
    fn default() -> Self {
        Self::new()
    }
}
impl HttpTransport for UreqTransport {
    fn send(&self, request: HttpRequest) -> Result<HttpResponse, Error> {
        self.send_bounded(request, MAX_RESPONSE_BYTES)
    }
    fn send_bounded(&self, request: HttpRequest, max_bytes: usize) -> Result<HttpResponse, Error> {
        let mut outbound = self.agent.request(request.method, &request.url);
        for (name, value) in &request.headers {
            outbound = outbound.set(name, value);
        }
        match outbound.send_bytes(&request.body) {
            Ok(response) | Err(ureq::Error::Status(_, response)) => {
                Self::read_response(response, max_bytes)
            }
            Err(ureq::Error::Transport(_)) => Err(Error::Network),
        }
    }
}

/// Encoded PCM16 WAV size including the fixed 44-byte header.
pub(crate) fn wav_bytes(samples: u64) -> Result<u64, Error> {
    samples
        .checked_mul(2)
        .and_then(|data| data.checked_add(44))
        .ok_or(Error::Payload)
}

/// Mono 16-bit PCM WAV. Input is already validated finite mono f32.
pub(crate) fn wav(audio: &AudioWindow<'_>) -> Result<Vec<u8>, Error> {
    let data_bytes = audio.samples.len().checked_mul(2).ok_or(Error::Payload)?;
    let total = data_bytes.checked_add(44).ok_or(Error::Payload)?;
    let data_len = u32::try_from(data_bytes).map_err(|_| Error::Payload)?;
    let riff_len = data_len.checked_add(36).ok_or(Error::Payload)?;
    let byte_rate = audio.sample_rate.checked_mul(2).ok_or(Error::Payload)?;
    let mut output = Vec::with_capacity(total);
    output.extend_from_slice(b"RIFF");
    output.extend_from_slice(&riff_len.to_le_bytes());
    output.extend_from_slice(b"WAVEfmt ");
    output.extend_from_slice(&16u32.to_le_bytes());
    output.extend_from_slice(&1u16.to_le_bytes());
    output.extend_from_slice(&1u16.to_le_bytes());
    output.extend_from_slice(&audio.sample_rate.to_le_bytes());
    output.extend_from_slice(&byte_rate.to_le_bytes());
    output.extend_from_slice(&2u16.to_le_bytes());
    output.extend_from_slice(&16u16.to_le_bytes());
    output.extend_from_slice(b"data");
    output.extend_from_slice(&data_len.to_le_bytes());
    for sample in audio.samples {
        let value = (sample.clamp(-1.0, 1.0) * 32767.0).round() as i16;
        output.extend_from_slice(&value.to_le_bytes());
    }
    Ok(output)
}

/// Bounded multipart builder shared by provider adapters. Values are trusted
/// adapter metadata; the boundary is chosen so it cannot occur in the audio.
pub(crate) struct Multipart {
    boundary: String,
    body: Vec<u8>,
}
impl Multipart {
    pub(crate) fn new(audio: &[u8]) -> Result<Self, Error> {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        for _ in 0..8 {
            let boundary = format!(
                "vibecut-{:x}-{:x}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            );
            if !audio
                .windows(boundary.len())
                .any(|window| window == boundary.as_bytes())
            {
                return Ok(Self {
                    boundary,
                    body: Vec::new(),
                });
            }
        }
        Err(Error::Invalid("multipart boundary"))
    }
    pub(crate) fn field(&mut self, name: &str, value: &str) {
        self.body
            .extend_from_slice(format!("--{}\r\n", self.boundary).as_bytes());
        self.body.extend_from_slice(
            format!("Content-Disposition: form-data; name=\"{name}\"").as_bytes(),
        );
        self.body.extend_from_slice(b"\r\n\r\n");
        self.body.extend_from_slice(value.as_bytes());
        self.body.extend_from_slice(b"\r\n");
    }
    pub(crate) fn file(&mut self, filename: &str, content_type: &str, bytes: &[u8]) {
        self.body
            .extend_from_slice(format!("--{}\r\n", self.boundary).as_bytes());
        self.body.extend_from_slice(
            format!("Content-Disposition: form-data; name=\"file\"; filename=\"{filename}\"")
                .as_bytes(),
        );
        self.body
            .extend_from_slice(format!("\r\nContent-Type: {content_type}\r\n\r\n").as_bytes());
        self.body.extend_from_slice(bytes);
        self.body.extend_from_slice(b"\r\n");
    }
    pub(crate) fn finish(mut self) -> (String, Vec<u8>) {
        self.body
            .extend_from_slice(format!("--{}--\r\n", self.boundary).as_bytes());
        (
            format!("multipart/form-data; boundary={}", self.boundary),
            self.body,
        )
    }
}

pub(crate) fn retry_after(headers: &[(String, String)]) -> Option<u32> {
    headers
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case("retry-after"))
        .and_then(|(_, value)| value.trim().parse().ok())
}
