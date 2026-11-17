//! Signed GET requests to the site's JSON API: the `_` token, sealed-body decoding, the response
//! envelope, and the mapping from failures to typed plugin errors (spec sections 2, 3 and 9.4).

use crate::query::Query;
use crate::{BASE_URL, decode, sign};
use ito_rs::net::{NetResponse, Request};
use ito_rs::{Error, Result};
use serde::de::DeserializeOwned;
use serde_json::Value;

const KEY_RETIRED: &str = "Comix changed its request key. Update the Comix source to continue.";
const SIGNING_REJECTED: &str =
    "Comix rejected the source's request signature. Update the Comix source to continue.";
const VERIFICATION: &str = "Comix needs a verification before chapters can load. Open Comix in the browser and complete its verification check, then retry.";
const BROWSER_CHECK: &str = "Comix needs a browser check before it will respond. Open Comix in the browser and complete its verification check, then retry.";
const NOT_FOUND: &str = "Comix has no series or chapter with that id.";
const REJECTED_PARAMETERS: &str = "Comix rejected the request parameters.";
const FORBIDDEN: &str = "Comix refused the request.";
const MALFORMED: &str = "Comix sent a response this source could not read.";

/// The signed URL for `path` (relative to the API base) and `query`.
fn request_url(path: &str, query: &Query) -> String {
    let token = sign::token(&query.message(path));
    let encoded = query.encoded();
    let separator = if encoded.is_empty() { "" } else { "&" };
    format!("{BASE_URL}/api/v1{path}?{encoded}{separator}_={token}")
}

/// Sends a signed GET and returns the envelope's `result` as `T`. Any non-2xx answer is mapped to
/// a typed error; nothing is ever returned as an empty success.
pub fn get<T: DeserializeOwned>(path: &str, query: &Query) -> Result<T> {
    let response = Request::get(request_url(path, query))
        .header("Accept", "application/json")
        .send()?;
    if !(200..300).contains(&response.status) {
        return Err(failure(&response));
    }
    let body = read_body(&response)?;
    serde_json::from_value(unwrap_envelope(body)).map_err(|_| malformed())
}

fn malformed() -> Error {
    Error::Server(MALFORMED.into())
}

/// The JSON of a response: a sealed body (`x-enc: 2` with an `e` text) is decoded first. A body
/// that fails to decode is an error, never passed on as raw text.
fn read_body(response: &NetResponse) -> Result<Value> {
    if header(response, "x-enc") == Some("2")
        && let Some(text) = sealed_text(&response.body)
    {
        let plain = decode::decode(&text).map_err(|_| malformed())?;
        return serde_json::from_slice(&plain).map_err(|_| malformed());
    }
    serde_json::from_slice(&response.body).map_err(|_| malformed())
}

fn sealed_text(body: &[u8]) -> Option<String> {
    let Value::Object(mut fields) = serde_json::from_slice::<Value>(body).ok()? else {
        return None;
    };
    match fields.remove("e") {
        Some(Value::String(text)) => Some(text),
        _ => None,
    }
}

/// Unwraps `{"status":"ok","result":...}`. Any other document is the payload itself.
fn unwrap_envelope(value: Value) -> Value {
    match value {
        Value::Object(mut map)
            if map.get("status").and_then(Value::as_str) == Some("ok")
                && map.contains_key("result") =>
        {
            map.remove("result").unwrap_or_default()
        }
        other => other,
    }
}

/// Maps an error answer to the plugin's typed outcome (spec section 9.4).
fn failure(response: &NetResponse) -> Error {
    let body: Option<Value> = serde_json::from_slice(&response.body).ok();
    let field = |name: &str| {
        body.as_ref()
            .and_then(|b| b.get(name))
            .and_then(Value::as_str)
    };
    match response.status {
        403 if field("error") == Some("captcha_required") => Error::NeedsLogin(VERIFICATION.into()),
        403 if is_browser_check(response) => Error::NeedsLogin(BROWSER_CHECK.into()),
        403 => match field("code") {
            Some("key_retired") => Error::Host(KEY_RETIRED.into()),
            Some("missing_token" | "invalid_token") => Error::Host(SIGNING_REJECTED.into()),
            _ => Error::Server(FORBIDDEN.into()),
        },
        404 => Error::NotFound(NOT_FOUND.into()),
        422 => Error::Server(REJECTED_PARAMETERS.into()),
        status => Error::Server(format!("Comix returned HTTP status {status}")),
    }
}

/// A Cloudflare challenge page: marked by the header, or an HTML body where JSON is expected.
fn is_browser_check(response: &NetResponse) -> bool {
    header(response, "cf-mitigated") == Some("challenge")
        || response.body.trim_ascii_start().first() == Some(&b'<')
}

fn header<'a>(response: &'a NetResponse, name: &str) -> Option<&'a str> {
    response
        .headers
        .iter()
        .find(|(key, _)| key.eq_ignore_ascii_case(name))
        .map(|(_, value)| value.as_str())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    const XW6NQ_ENVELOPE: &str = include_str!("../tests/fixtures/xw6nq-envelope.txt");

    fn response(status: i32, headers: &[(&str, &str)], body: &str) -> NetResponse {
        NetResponse {
            status,
            headers: headers
                .iter()
                .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
                .collect::<HashMap<_, _>>(),
            body: body.as_bytes().to_vec(),
        }
    }

    #[test]
    fn signed_url_carries_the_token_after_the_encoded_query() {
        let query = Query::new()
            .int("limit", 20)
            .order("number", "desc")
            .int("page", 1)
            .int("v", 0)
            .int("w", 0);
        assert_eq!(
            request_url("/manga/8v88/chapters", &query),
            "https://comix.to/api/v1/manga/8v88/chapters?limit=20&order%5Bnumber%5D=desc&page=1&v=0&w=0&_=etu.RrmtJju8KbY3vC6y"
        );
    }

    #[test]
    fn signed_url_without_parameters_has_only_the_token() {
        assert_eq!(
            request_url("/manga/8v88", &Query::new()),
            "https://comix.to/api/v1/manga/8v88?_=etu.0w2lzYlCZW5Pil0m"
        );
    }

    #[test]
    fn sealed_body_decodes_to_its_result() {
        let body = format!(r#"{{"e":"{XW6NQ_ENVELOPE}"}}"#);
        let value = read_body(&response(200, &[("x-enc", "2")], &body)).expect("sealed body");
        let manga = unwrap_envelope(value);
        assert_eq!(manga["hid"], "xw6nq");
        assert_eq!(manga["title"], "Secret love");
    }

    #[test]
    fn plain_body_unwraps_its_envelope() {
        let body = r#"{"status":"ok","result":{"items":[],"meta":{"page":1,"lastPage":1}}}"#;
        let value = read_body(&response(200, &[], body)).expect("plain body");
        assert_eq!(unwrap_envelope(value)["meta"]["lastPage"], 1);
    }

    #[test]
    fn sealed_flag_without_a_text_field_is_read_as_plain_json() {
        let body = r#"{"status":"ok","result":[]}"#;
        let value = read_body(&response(200, &[("X-Enc", "2")], body)).expect("plain json");
        assert_eq!(unwrap_envelope(value), serde_json::json!([]));
    }

    #[test]
    fn undecodable_sealed_body_is_an_error_not_raw_text() {
        let result = read_body(&response(200, &[("x-enc", "2")], r#"{"e":"%%%"}"#));
        assert!(matches!(result, Err(Error::Server(_))));
    }

    #[test]
    fn key_retired_is_a_source_update_not_a_silent_empty_result() {
        let error = failure(&response(
            403,
            &[],
            r#"{"message":"Invalid token.","code":"key_retired"}"#,
        ));
        assert!(matches!(&error, Error::Host(message) if message == KEY_RETIRED));
    }

    #[test]
    fn rejected_signatures_ask_for_a_source_update() {
        for code in ["missing_token", "invalid_token"] {
            let body = format!(r#"{{"message":"Invalid token.","code":"{code}"}}"#);
            let error = failure(&response(403, &[], &body));
            assert!(matches!(&error, Error::Host(message) if message == SIGNING_REJECTED));
        }
    }

    #[test]
    fn captcha_required_is_the_host_verification_outcome() {
        let error = failure(&response(
            403,
            &[],
            r#"{"error":"captcha_required","challenge":"/@waf/challenge"}"#,
        ));
        assert!(matches!(&error, Error::NeedsLogin(message) if message == VERIFICATION));
    }

    #[test]
    fn cloudflare_challenges_are_the_browser_check_outcome() {
        let by_header = failure(&response(403, &[("cf-mitigated", "challenge")], "{}"));
        assert!(matches!(&by_header, Error::NeedsLogin(message) if message == BROWSER_CHECK));
        let by_body = failure(&response(403, &[], "<!DOCTYPE html><html></html>"));
        assert!(matches!(&by_body, Error::NeedsLogin(message) if message == BROWSER_CHECK));
    }

    #[test]
    fn unknown_series_is_not_found() {
        let error = failure(&response(
            404,
            &[],
            r#"{"message":"No query results for model [App\\Models\\Manga] x"}"#,
        ));
        assert!(matches!(error, Error::NotFound(_)));
    }

    #[test]
    fn invalid_parameters_and_other_statuses_are_server_errors() {
        assert!(matches!(
            failure(&response(422, &[], r#"{"message":"bad"}"#)),
            Error::Server(message) if message == REJECTED_PARAMETERS
        ));
        assert!(matches!(
            failure(&response(500, &[], "")),
            Error::Server(message) if message == "Comix returned HTTP status 500"
        ));
    }
}
