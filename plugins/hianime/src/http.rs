//! Fetch seam and block classification for HiAnime and MegaPlay requests.
//!
//! Every request goes through [`get`], which attaches [`USER_AGENT`] and classifies the
//! response before the caller sees it. The network sits behind [`Transport`], so native tests
//! answer requests with canned responses and run the same classification as the host.

use ito_rs::{
    Error,
    net::{NetResponse, Request},
};
use std::collections::HashMap;

/// The one User-Agent that every plugin request sends, including the playback headers.
///
/// It is an honest mobile Safari string that matches Ito's WebView. It is never rotated and
/// never swapped for another identity.
pub const USER_AGENT: &str = "Mozilla/5.0 (iPhone; CPU iPhone OS 17_4 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.4 Mobile/15E148 Safari/604.1";

/// Bytes of a response body scanned for challenge markers. Challenge pages are small.
const CHALLENGE_SCAN_LIMIT: usize = 64 * 1024;

/// Markers that only a Cloudflare challenge page carries. The bot-management beacon script
/// (`/cdn-cgi/challenge-platform/...`) is present on ordinary pages, so it is not a marker.
const CHALLENGE_MARKERS: [&str; 3] = [
    "cf_chl_opt",
    "<title>just a moment...</title>",
    "<title>attention required! | cloudflare</title>",
];

/// The upstream a request was addressed to. Blocks name it, so a MegaPlay block is never
/// reported as a block on the HiAnime site.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Upstream {
    /// The hianime.ms site, including its `/stream/<token>/<lang>` hop.
    Site,
    /// MegaPlay's embed, getSources, playlist, and caption files.
    MegaPlay,
}

impl Upstream {
    fn name(self) -> &'static str {
        match self {
            Self::Site => "HiAnime site",
            Self::MegaPlay => "MegaPlay",
        }
    }
}

/// A failed fetch. Classification is by variant, never by message text.
#[derive(Debug)]
pub enum Failure {
    /// A 403, a 429, or a challenge page. The call stops, and nothing retries around it.
    Blocked { upstream: Upstream, status: i32 },
    /// A transport error or an ordinary non-success response.
    Other(Error),
}

impl From<Error> for Failure {
    fn from(error: Error) -> Self {
        Self::Other(error)
    }
}

impl From<Failure> for Error {
    fn from(failure: Failure) -> Self {
        match failure {
            Failure::Blocked { upstream, status } => Error::Net(format!(
                "{} blocked the request (HTTP {status}); not retried",
                upstream.name()
            )),
            Failure::Other(error) => error,
        }
    }
}

/// Sends one GET and returns the raw response. The host implements it in the plugin, and
/// tests implement it with canned responses.
pub trait Transport {
    fn send(&self, url: &str, headers: &[(&str, &str)]) -> ito_rs::Result<NetResponse>;
}

/// The host network, through `ito_rs`.
pub struct HostNet;

impl Transport for HostNet {
    fn send(&self, url: &str, headers: &[(&str, &str)]) -> ito_rs::Result<NetResponse> {
        let mut request = Request::get(url);
        for &(name, value) in headers {
            request.header(name, value);
        }
        request.send()
    }
}

/// GETs `url` with [`USER_AGENT`] and `headers`, and returns the response only when it is a
/// success that is not a block.
pub fn get<N: Transport>(
    net: &N,
    upstream: Upstream,
    url: &str,
    headers: &[(&str, &str)],
) -> Result<NetResponse, Failure> {
    let mut all: Vec<(&str, &str)> = Vec::with_capacity(headers.len() + 1);
    all.push(("User-Agent", USER_AGENT));
    all.extend_from_slice(headers);
    let response = net.send(url, &all)?;
    check(upstream, response)
}

/// Classifies a response.
///
/// A block is a 403, a 429, a `cf-mitigated: challenge` header, or a challenge page in the
/// body, whatever the status. Any other response must be 2xx to succeed.
pub fn check(upstream: Upstream, response: NetResponse) -> Result<NetResponse, Failure> {
    let status = response.status;
    let blocked = status == 403
        || status == 429
        || is_challenge(&response.headers)
        || has_challenge_markers(&response.body);
    if blocked {
        return Err(Failure::Blocked { upstream, status });
    }
    if (200..300).contains(&status) {
        return Ok(response);
    }
    Err(Failure::Other(Error::Net(format!(
        "{} returned HTTP {status}",
        upstream.name()
    ))))
}

/// Returns true when the response carries `cf-mitigated: challenge`.
fn is_challenge(headers: &HashMap<String, String>) -> bool {
    headers.iter().any(|(name, value)| {
        name.eq_ignore_ascii_case("cf-mitigated") && value.trim().eq_ignore_ascii_case("challenge")
    })
}

/// Returns true when the start of the body is a Cloudflare challenge page.
fn has_challenge_markers(body: &[u8]) -> bool {
    let head = &body[..body.len().min(CHALLENGE_SCAN_LIMIT)];
    let text = String::from_utf8_lossy(head).to_ascii_lowercase();
    CHALLENGE_MARKERS.iter().any(|marker| text.contains(marker))
}

/// Writes a label-only line to the host log.
///
/// The host log import exists only inside the plugin's wasm host, so native tests drop the
/// line. Log text never carries a URL, a header, or a response body.
#[cfg(not(test))]
pub fn note(message: &str) {
    ito_rs::host::print(message);
}

/// Native tests have no host log.
#[cfg(test)]
pub fn note(_message: &str) {}

/// Percent-encodes a query value, using `+` for spaces like HiAnime's search form.
pub fn encode_query(text: &str) -> String {
    let mut encoded = String::with_capacity(text.len());
    for byte in text.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                encoded.push(char::from(byte));
            }
            b' ' => encoded.push('+'),
            _ => encoded.push_str(&format!("%{byte:02X}")),
        }
    }
    encoded
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fake::Canned;

    fn reply(status: i32, body: &[u8]) -> NetResponse {
        NetResponse {
            status,
            headers: HashMap::new(),
            body: body.to_vec(),
        }
    }

    #[test]
    fn user_agent_is_one_honest_mobile_safari_string() {
        assert_eq!(
            USER_AGENT,
            "Mozilla/5.0 (iPhone; CPU iPhone OS 17_4 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.4 Mobile/15E148 Safari/604.1"
        );
        assert!(!USER_AGENT.contains("Firefox"));
    }

    #[test]
    fn every_get_sends_the_user_agent_ahead_of_the_caller_headers() {
        let net = Canned::new().on("https://hianime.ms/a", 200, "ok");
        get(
            &net,
            Upstream::Site,
            "https://hianime.ms/a",
            &[("Referer", "https://hianime.ms/")],
        )
        .expect("success");
        let sent = net.requests();
        assert_eq!(sent[0].header("User-Agent"), Some(USER_AGENT));
        assert_eq!(sent[0].header("Referer"), Some("https://hianime.ms/"));
    }

    #[test]
    fn forbidden_and_rate_limited_responses_are_blocks_naming_the_upstream() {
        let Err(failure) = check(Upstream::MegaPlay, reply(403, b"")) else {
            panic!("expected a block");
        };
        assert!(matches!(
            failure,
            Failure::Blocked {
                upstream: Upstream::MegaPlay,
                status: 403
            }
        ));
        let Error::Net(message) = Error::from(failure) else {
            panic!("expected a network error");
        };
        assert_eq!(
            message,
            "MegaPlay blocked the request (HTTP 403); not retried"
        );
        assert!(matches!(
            check(Upstream::Site, reply(429, b"")),
            Err(Failure::Blocked {
                upstream: Upstream::Site,
                status: 429
            })
        ));
    }

    #[test]
    fn challenge_page_in_a_success_body_is_a_block() {
        let body = b"<html><script>window._cf_chl_opt = {};</script></html>";
        assert!(matches!(
            check(Upstream::Site, reply(200, body)),
            Err(Failure::Blocked {
                upstream: Upstream::Site,
                status: 200
            })
        ));
    }

    #[test]
    fn challenge_interstitial_under_a_503_is_a_block() {
        let body = b"<html><title>Just a moment...</title></html>";
        assert!(matches!(
            check(Upstream::Site, reply(503, body)),
            Err(Failure::Blocked { .. })
        ));
    }

    #[test]
    fn cloudflare_beacon_alone_is_not_a_challenge() {
        let body = b"<script src=\"/cdn-cgi/challenge-platform/scripts/jsd/main.js\"></script>";
        assert!(check(Upstream::Site, reply(200, body)).is_ok());
    }

    #[test]
    fn challenge_header_is_matched_case_insensitively() {
        let challenged = NetResponse {
            status: 200,
            headers: HashMap::from([("CF-Mitigated".to_string(), " Challenge ".to_string())]),
            body: b"ok".to_vec(),
        };
        assert!(matches!(
            check(Upstream::Site, challenged),
            Err(Failure::Blocked { .. })
        ));
        let other = NetResponse {
            status: 200,
            headers: HashMap::from([("cf-mitigated".to_string(), "none".to_string())]),
            body: b"ok".to_vec(),
        };
        assert!(check(Upstream::Site, other).is_ok());
    }

    #[test]
    fn other_statuses_are_ordinary_failures_not_blocks() {
        assert!(matches!(
            check(Upstream::Site, reply(410, b"gone")),
            Err(Failure::Other(Error::Net(_)))
        ));
        assert!(matches!(
            Error::from(Failure::Other(Error::Unsupported)),
            Error::Unsupported
        ));
    }

    #[test]
    fn query_values_use_plus_for_spaces_and_percent_encode_the_rest() {
        assert_eq!(encode_query("one piece"), "one+piece");
        assert_eq!(encode_query("a&b=c/d"), "a%26b%3Dc%2Fd");
        assert_eq!(encode_query("kaiju-no.8_~"), "kaiju-no.8_~");
    }
}
