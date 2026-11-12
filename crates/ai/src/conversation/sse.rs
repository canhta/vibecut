//! Bounded server-sent-event reader shared by both streaming adapters.
use crate::Error;
use ::transport::{HttpRequest, HttpResponse, MAX_RESPONSE_BYTES, StreamTransport};
use std::io::{BufRead, BufReader, Read};

/// One line, one event's data and the whole stream are each capped; a hostile or
/// runaway stream ends with `Payload`, never unbounded memory.
const MAX_LINE: usize = 256 * 1024;
const MAX_STREAM: usize = 8 * 1024 * 1024;

pub(super) struct Sse {
    reader: BufReader<Box<dyn Read + Send>>,
    read: usize,
}
impl Sse {
    pub(super) fn new(body: Box<dyn Read + Send>) -> Self {
        Self {
            reader: BufReader::new(body),
            read: 0,
        }
    }
    /// The next event's joined `data:` lines, or `None` at the end of the stream.
    /// Comments, `event:`, `id:` and `retry:` lines are ignored: both providers
    /// repeat the event type inside the JSON.
    pub(super) fn next_event(&mut self) -> Result<Option<String>, Error> {
        let mut data: Option<String> = None;
        let mut line = Vec::new();
        loop {
            line.clear();
            let limit = MAX_LINE.min(MAX_STREAM.saturating_sub(self.read));
            let count = (&mut self.reader)
                .take(limit as u64 + 1)
                .read_until(b'\n', &mut line)
                .map_err(|_| Error::Network)?;
            if count == 0 {
                return Ok(data);
            }
            if count > limit {
                return Err(Error::Payload);
            }
            self.read += count;
            let text = std::str::from_utf8(&line).map_err(|_| Error::Schema)?;
            let text = text.trim_end_matches(['\n', '\r']);
            if text.is_empty() {
                if data.is_some() {
                    return Ok(data);
                }
                continue;
            }
            if let Some(value) = text.strip_prefix("data:") {
                let value = value.strip_prefix(' ').unwrap_or(value);
                match &mut data {
                    Some(existing) => {
                        existing.push('\n');
                        existing.push_str(value);
                    }
                    None => data = Some(value.to_owned()),
                }
            }
        }
    }
}

/// POST and return the event body of a 2xx reply; any other status is read as a
/// bounded error envelope and classified like every other remote call.
pub(super) fn open(
    transport: &dyn StreamTransport,
    request: HttpRequest,
    config: &crate::ProviderConfig,
    status_error: fn(&HttpResponse) -> Error,
) -> Result<Sse, Error> {
    let response = transport.post_stream(request)?;
    if (200..=299).contains(&response.status) {
        return Ok(Sse::new(response.body));
    }
    let mut body = Vec::new();
    response
        .body
        .take(MAX_RESPONSE_BYTES as u64 + 1)
        .read_to_end(&mut body)
        .map_err(|_| Error::Network)?;
    if body.len() > MAX_RESPONSE_BYTES {
        return Err(Error::Payload);
    }
    let buffered = HttpResponse {
        status: response.status,
        headers: response.headers,
        body,
    };
    crate::remote::check_model_response(&buffered, config)?;
    Err(status_error(&buffered))
}
