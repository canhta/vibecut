//! Fetches verified downloads through bounded redirects.
use super::{Error as RuntimeError, Result};
use crate::jobs::JobContext;
use crate::session::wire::{ErrorCode, Failure, JobFailurePolicy};

impl Failure for ai::Error {
    fn code(&self) -> ErrorCode {
        match self {
            Self::Chapters(_) => ErrorCode::Refused,
            _ => ErrorCode::Ai,
        }
    }
    fn transient(&self) -> bool {
        self.is_transient()
    }
    fn job_policy(&self) -> JobFailurePolicy {
        match self {
            Self::RateLimit {
                retry_after_seconds,
            } => JobFailurePolicy {
                retry_after_seconds: *retry_after_seconds,
                ..Default::default()
            },
            _ => JobFailurePolicy::default(),
        }
    }
}

use transport::{DownloadTransport, HttpStreamResponse};
pub(crate) fn header<'a>(response: &'a HttpStreamResponse, name: &str) -> Result<Option<&'a str>> {
    let mut values = response
        .headers
        .iter()
        .filter(|(n, _)| n.eq_ignore_ascii_case(name));
    let value = values.next().map(|(_, v)| v.as_str());
    if values.next().is_some()
        || value.is_some_and(|v| v.len() > 8192 || v.bytes().any(|b| b == b'\r' || b == b'\n'))
    {
        return Err(RuntimeError::ModelResponse);
    }
    Ok(value)
}
/// Fetches one response over at most five redirects, refusing duplicate or oversized headers.
///
/// # Errors
///
/// `Cancelled` when the job is cancelled before any request; `NetworkTransient` for retryable
/// transport failures; `ModelResponse` for a malformed or excessive redirect chain;
/// `Transfer` retains a nontransient transport or redirect-validation cause.
pub fn response(
    http: &dyn DownloadTransport,
    url: &str,
    headers: &[(String, String)],
    ctx: &JobContext,
) -> Result<HttpStreamResponse> {
    let mut url = url.to_owned();
    for redirect in 0..=5 {
        ctx.check_cancelled()?;
        let reply = http.get(&url, headers).map_err(|error| match error {
            transport::Error::Network => RuntimeError::NetworkTransient,
            error => RuntimeError::Transfer(error.into()),
        })?;
        if matches!(reply.status, 301 | 302 | 303 | 307 | 308) {
            if redirect == 5 {
                return Err(RuntimeError::ModelResponse);
            }
            url = transport::download_redirect(
                &url,
                header(&reply, "location")?.ok_or(RuntimeError::ModelResponse)?,
            )
            .map_err(|error| RuntimeError::Transfer(error.into()))?;
        } else {
            return Ok(reply);
        }
    }
    Err(RuntimeError::ModelResponse)
}
