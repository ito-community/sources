//! Fetch helper that classifies blocks and challenges so they can never look like empty results.

use ito_rs::{
    Error, Result,
    net::{NetResponse, Request},
};
use std::collections::HashMap;

/// Error message prefix for a 403, 429, or Cloudflare challenge. Plugin callers stop on it.
const BLOCKED: &str = "HiAnime blocked the request";

/// What a response status means for the plugin.
#[derive(Debug, PartialEq, Eq)]
pub enum Verdict {
    Ok,
    /// Forbidden, rate limited, or a challenge page. The call must fail, never retry around it.
    Blocked,
    /// Any other non-success status, such as a missing episode page.
    Failed,
}

/// Classifies a response from its status and whether it carries a Cloudflare challenge marker.
pub fn verdict(status: i32, challenge: bool) -> Verdict {
    if challenge || status == 403 || status == 429 {
        Verdict::Blocked
    } else if (200..300).contains(&status) {
        Verdict::Ok
    } else {
        Verdict::Failed
    }
}

/// Returns true when the response carries `cf-mitigated: challenge`.
pub fn is_challenge(headers: &HashMap<String, String>) -> bool {
    headers.iter().any(|(name, value)| {
        name.eq_ignore_ascii_case("cf-mitigated") && value.trim().eq_ignore_ascii_case("challenge")
    })
}

/// Sends a GET with the given headers and returns the body only for a success verdict.
pub fn fetch(url: &str, headers: &[(&str, &str)]) -> Result<NetResponse> {
    let mut request = Request::get(url);
    for &(name, value) in headers {
        request.header(name, value);
    }
    let response = request.send()?;
    match verdict(response.status, is_challenge(&response.headers)) {
        Verdict::Ok => Ok(response),
        Verdict::Blocked => Err(Error::Net(format!(
            "{BLOCKED} (HTTP {}); not retried",
            response.status
        ))),
        Verdict::Failed => Err(Error::Net(format!("HTTP {}", response.status))),
    }
}

/// Returns true when `error` came from a block or challenge rather than an ordinary failure.
pub fn is_blocked(error: &Error) -> bool {
    matches!(error, Error::Net(message) if message.starts_with(BLOCKED))
}

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

    #[test]
    fn forbidden_rate_limited_and_challenged_responses_are_blocks() {
        assert_eq!(verdict(403, false), Verdict::Blocked);
        assert_eq!(verdict(429, false), Verdict::Blocked);
        assert_eq!(verdict(200, true), Verdict::Blocked);
    }

    #[test]
    fn success_and_other_statuses_are_not_blocks() {
        assert_eq!(verdict(200, false), Verdict::Ok);
        assert_eq!(verdict(410, false), Verdict::Failed);
        assert_eq!(verdict(503, false), Verdict::Failed);
    }

    #[test]
    fn challenge_header_is_matched_case_insensitively() {
        let headers = HashMap::from([("CF-Mitigated".to_string(), " Challenge ".to_string())]);
        assert!(is_challenge(&headers));
        let other = HashMap::from([("cf-mitigated".to_string(), "none".to_string())]);
        assert!(!is_challenge(&other));
    }

    #[test]
    fn only_block_messages_are_reported_as_blocked() {
        let blocked = Error::Net(format!("{BLOCKED} (HTTP 403); not retried"));
        assert!(is_blocked(&blocked));
        assert!(!is_blocked(&Error::Net("HTTP 410".to_string())));
        assert!(!is_blocked(&Error::Unsupported));
    }

    #[test]
    fn query_values_use_plus_for_spaces_and_percent_encode_the_rest() {
        assert_eq!(encode_query("one piece"), "one+piece");
        assert_eq!(encode_query("a&b=c/d"), "a%26b%3Dc%2Fd");
        assert_eq!(encode_query("kaiju-no.8_~"), "kaiju-no.8_~");
    }
}
