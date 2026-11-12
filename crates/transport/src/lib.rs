//! Bounded HTTPS requests, streaming responses and redirect and retry-after rules.
use std::fmt;
use std::io::Read;
use std::time::Duration;

/// Response bodies are transcripts or error envelopes; cap hostile responses.
pub const MAX_RESPONSE_BYTES: usize = 1024 * 1024;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(120);

/// Safe transport categories only: never headers, keys, URLs or response bodies.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("invalid transport input: {0}")]
    Invalid(&'static str),
    #[error("payload exceeds capability limits")]
    Payload,
    #[error("network request failed")]
    Network,
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

/// Parse the first case-insensitive `retry-after` second count.
pub fn retry_after(headers: &[(String, String)]) -> Option<u32> {
    headers
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case("retry-after"))
        .and_then(|(_, value)| value.trim().parse().ok())
}
