//! Fetches verified downloads through bounded redirects.
use crate::jobs::JobContext;
use crate::{EditError, Result};
use ai::transport::{DownloadTransport, HttpStreamResponse};
pub(crate) fn header<'a>(response: &'a HttpStreamResponse, name: &str) -> Result<Option<&'a str>> {
    let mut values = response
        .headers
        .iter()
        .filter(|(n, _)| n.eq_ignore_ascii_case(name));
    let value = values.next().map(|(_, v)| v.as_str());
    if values.next().is_some()
        || value.is_some_and(|v| v.len() > 8192 || v.bytes().any(|b| b == b'\r' || b == b'\n'))
    {
        return Err(EditError::ModelResponse);
    }
    Ok(value)
}
/// Fetches one response over at most five redirects, refusing duplicate or oversized headers.
///
/// # Errors
///
/// `Cancelled` when the job is cancelled before any request; `NetworkTransient` for retryable
/// transport failures; `ModelResponse` for a malformed or excessive redirect chain.
pub fn response(
    http: &dyn DownloadTransport,
    url: &str,
    headers: &[(String, String)],
    ctx: &JobContext,
) -> Result<HttpStreamResponse> {
    let mut url = url.to_owned();
    for redirect in 0..=5 {
        ctx.check_cancelled()?;
        let reply = http.get(&url, headers).map_err(|error| {
            if error.is_transient() {
                EditError::NetworkTransient
            } else {
                EditError::Ai(error)
            }
        })?;
        if matches!(reply.status, 301 | 302 | 303 | 307 | 308) {
            if redirect == 5 {
                return Err(EditError::ModelResponse);
            }
            url = ai::transport::download_redirect(
                &url,
                header(&reply, "location")?.ok_or(EditError::ModelResponse)?,
            )?;
        } else {
            return Ok(reply);
        }
    }
    Err(EditError::ModelResponse)
}
