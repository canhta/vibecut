//! The once-a-day request for the public repository's latest release tag.
use crate::services::views::about::VERSION;
use serde::Deserialize;
use transport::{HttpRequest, HttpTransport};

/// The one request the check makes: the latest published release (drafts and prereleases are
/// never returned by this endpoint).
const LATEST: &str = "https://api.github.com/repos/canhta/vibecut/releases/latest";
/// The release page the notice links to, followed by the validated tag.
const RELEASE_PAGE: &str = "https://github.com/canhta/vibecut/releases/tag/";
/// Seconds between checks.
const DAY: u64 = 24 * 60 * 60;
/// The reply carries the release notes and asset list; cap a hostile one.
const MAX_REPLY: usize = transport::MAX_RESPONSE_BYTES;

/// A published release newer than this build.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Release {
    /// `MAJOR.MINOR.PATCH` with an optional `v`; validated, so it is safe in a URL.
    tag: String,
}

impl Release {
    pub(super) fn tag(&self) -> &str {
        &self.tag
    }
    /// The release page on the public repository.
    pub(super) fn page(&self) -> String {
        format!("{RELEASE_PAGE}{}", self.tag)
    }
}

/// Whether a check may run at `now` (Unix seconds): turned on, and none started in the last
/// day. A clock that moved back before the last check counts as due.
pub(super) fn due(enabled: bool, last: Option<u64>, now: u64) -> bool {
    enabled && last.is_none_or(|last| now < last || now - last >= DAY)
}

/// The request: a plain GET with a generic User-Agent and nothing else, no credential, cookie
/// or identifier.
fn request() -> HttpRequest {
    HttpRequest {
        method: "GET",
        url: LATEST.to_owned(),
        headers: vec![("User-Agent".to_owned(), format!("VibeCut/{VERSION}"))],
        body: Vec::new(),
    }
}

/// The latest release when it is newer than this build. Offline, a refused or malformed reply
/// and a tag that is not a plain release version are all `None`: the check stays silent.
pub(super) fn newer(transport: &dyn HttpTransport) -> Option<Release> {
    #[derive(Deserialize)]
    struct Reply {
        tag_name: String,
        #[serde(default)]
        draft: bool,
        #[serde(default)]
        prerelease: bool,
    }
    let response = transport.send_bounded(request(), MAX_REPLY).ok()?;
    if response.status != 200 {
        return None;
    }
    let reply: Reply = serde_json::from_slice(&response.body).ok()?;
    if reply.draft || reply.prerelease {
        return None;
    }
    (version(&reply.tag_name)? > version(VERSION)?).then_some(Release {
        tag: reply.tag_name,
    })
}

/// `MAJOR.MINOR.PATCH`, optionally prefixed with `v`; a prerelease or build suffix is refused.
fn version(text: &str) -> Option<(u64, u64, u64)> {
    let text = text.strip_prefix('v').unwrap_or(text);
    let mut parts = text.split('.').map(|part| {
        (!part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit()))
            .then(|| part.parse::<u64>().ok())
            .flatten()
    });
    let parsed = (parts.next()??, parts.next()??, parts.next()??);
    parts.next().is_none().then_some(parsed)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;
    use transport::{Error, HttpResponse};

    struct Reply(Result<(u16, String), ()>);
    impl HttpTransport for Reply {
        fn send(&self, _: HttpRequest) -> Result<HttpResponse, Error> {
            let (status, body) = self.0.clone().map_err(|()| Error::Network)?;
            Ok(HttpResponse {
                status,
                headers: Vec::new(),
                body: body.into_bytes(),
            })
        }
    }

    /// Fails if a tag that is not a strictly newer plain release, or a failed or malformed
    /// reply, offers an update.
    #[test]
    fn only_a_strictly_newer_release_tag_is_offered() {
        let body =
            |tag: &str| format!(r#"{{"tag_name":"{tag}","draft":false,"prerelease":false}}"#);
        for (reply, offered) in [
            (Ok((200, body("v999.0.0"))), Some("v999.0.0")),
            (Ok((200, body("v1.0.0"))), Some("v1.0.0")),
            (Ok((200, body("999.0.0"))), Some("999.0.0")),
            (Ok((200, body(&format!("v{VERSION}")))), None),
            (Ok((200, body("v0.0.0"))), None),
            (Ok((200, body("v999.0.0-beta.1"))), None),
            (Ok((200, body("v999.0"))), None),
            (Ok((200, body("v+999.0.0"))), None),
            (Ok((200, body("v999.0.0/../../x"))), None),
            (
                Ok((
                    200,
                    r#"{"tag_name":"v999.0.0","prerelease":true}"#.to_owned(),
                )),
                None,
            ),
            (Ok((404, body("v999.0.0"))), None),
            (Ok((200, "<html>".to_owned())), None),
            (Err(()), None),
        ] {
            let release = newer(&Reply(reply.clone()));
            assert_eq!(release.as_ref().map(Release::tag), offered, "{reply:?}");
            if let Some(release) = release {
                assert_eq!(
                    release.page(),
                    format!(
                        "https://github.com/canhta/vibecut/releases/tag/{}",
                        release.tag()
                    )
                );
            }
        }
    }
}
