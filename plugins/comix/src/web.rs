use crate::BASE_URL;
use ito_rs::webview::Webview;
use ito_rs::{Error, Result};
use serde::{Deserialize, Serialize, de::DeserializeOwned};

const BRIDGE: &str = include_str!("bridge.js");
/// Opt-in marker: the host runs the script as an async function body and awaits its promise.
const ASYNC_MARKER: &str = "//# ito-async\n";
const ASYNC_PROBE: &str = "//# ito-async\nreturn \"ready\";";
/// SHA-256 of the captured site API client that the DTO mappings in `bridge.js` were derived from.
const API_CLIENT_REVISION: &str =
    "e6baa686c07629c4ad8660d64552ff549eb7e85b972564d8bc31267c65cc6588";
const UPDATE_ITO: &str = "Comix requires Ito 1.4.0 or newer for async WebView support; update Ito";

/// Read-only site operations the bridge accepts. Any other name is refused.
#[derive(Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Operation {
    List,
    Top,
    Detail,
    Chapters,
    Pages,
}

#[derive(Serialize)]
struct BridgeRequest<'a> {
    origin: &'static str,
    revision: &'static str,
    operation: Operation,
    #[serde(skip_serializing_if = "Option::is_none")]
    key: Option<&'a str>,
    query: &'a str,
}

#[derive(Deserialize)]
struct BridgeReply<T> {
    ok: bool,
    kind: Option<String>,
    status: Option<u16>,
    data: Option<T>,
}

pub struct ComixWebView;

impl ComixWebView {
    /// Runs one read-only operation through the page's own API client and decodes its result.
    pub fn fetch<T: DeserializeOwned>(
        operation: Operation,
        key: Option<&str>,
        query: &str,
    ) -> Result<T> {
        ensure_page()?;
        let request = serde_json::to_string(&BridgeRequest {
            origin: BASE_URL,
            revision: API_CLIENT_REVISION,
            operation,
            key,
            query,
        })
        .map_err(|_| helper_error("request"))?;
        let script = format!("{ASYNC_MARKER}const request = {request};\n{BRIDGE}");
        let raw = Webview::execute_js(&script).map_err(|_| bridge_unavailable())?;
        let reply: BridgeReply<T> =
            serde_json::from_str(&raw).map_err(|_| helper_error("reply"))?;
        if reply.ok {
            return reply.data.ok_or_else(|| helper_error("reply data"));
        }
        Err(refusal(reply.kind.as_deref(), reply.status))
    }
}

/// Loads the site page when needed, then requires the async marker to work on it.
fn ensure_page() -> Result<()> {
    let origin = serde_json::to_string(BASE_URL).map_err(|_| helper_error("origin"))?;
    let on_origin = format!("location.origin === {origin} ? 'ready' : 'missing'");
    if Webview::execute_js(&on_origin).is_ok_and(|state| state == "ready") {
        return Ok(());
    }
    Webview::load_url(BASE_URL)?;
    match Webview::execute_js(ASYNC_PROBE) {
        Ok(state) if state == "ready" => Ok(()),
        _ => Err(Error::Host(UPDATE_ITO.into())),
    }
}

/// Classifies a failed bridge call: a host without async support asks for an update.
fn bridge_unavailable() -> Error {
    match Webview::execute_js(ASYNC_PROBE) {
        Ok(state) if state == "ready" => helper_error("bridge call"),
        _ => Error::Host(UPDATE_ITO.into()),
    }
}

fn refusal(kind: Option<&str>, status: Option<u16>) -> Error {
    match kind {
        Some("challenge") => {
            Error::Host("Comix needs you to complete a verification; retry to show it.".into())
        }
        Some("http") => Error::Host(format!(
            "Comix request failed with HTTP status {}",
            status.map_or_else(|| "unknown".to_owned(), |code| code.to_string())
        )),
        Some("revision") => {
            Error::Host("Comix API client revision is not recognized; refusing to use it".into())
        }
        Some("missing-module") => helper_error("API client module"),
        Some("missing-facade") => helper_error("API client facade"),
        Some("shape") => helper_error("site response shape"),
        Some("origin") => helper_error("page origin"),
        _ => helper_error("bridge refusal"),
    }
}

fn helper_error(stage: &str) -> Error {
    Error::Host(format!("Comix helper failed: {stage}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn script_for(operation: Operation, key: Option<&str>, query: &str) -> String {
        let request = serde_json::to_string(&BridgeRequest {
            origin: BASE_URL,
            revision: API_CLIENT_REVISION,
            operation,
            key,
            query,
        })
        .expect("request serializes");
        format!("{ASYNC_MARKER}const request = {request};\n{BRIDGE}")
    }

    #[test]
    fn request_is_data_after_the_async_marker() {
        let script = script_for(Operation::Chapters, Some("abc"), "limit=100&page=1");
        assert!(script.starts_with("//# ito-async\nconst request = {"));
        assert!(script.contains(r#""operation":"chapters""#));
        assert!(script.contains(r#""key":"abc""#));
    }

    #[test]
    fn requests_without_a_key_omit_it() {
        let script = script_for(Operation::Top, None, "type=trending");
        assert!(!script.contains(r#""key""#));
    }

    #[test]
    fn operation_names_match_the_bridge_allowlist() {
        assert_eq!(serde_json::to_string(&Operation::List).unwrap(), "\"list\"");
        assert_eq!(serde_json::to_string(&Operation::Top).unwrap(), "\"top\"");
        assert_eq!(
            serde_json::to_string(&Operation::Detail).unwrap(),
            "\"detail\""
        );
        assert_eq!(
            serde_json::to_string(&Operation::Chapters).unwrap(),
            "\"chapters\""
        );
        assert_eq!(
            serde_json::to_string(&Operation::Pages).unwrap(),
            "\"pages\""
        );
    }

    #[test]
    fn refusal_kinds_stay_distinct() {
        let challenge = refusal(Some("challenge"), None).to_string();
        let http = refusal(Some("http"), Some(403)).to_string();
        let revision = refusal(Some("revision"), None).to_string();
        let shape = refusal(Some("shape"), None).to_string();
        assert!(challenge.contains("verification"));
        assert!(http.contains("403"));
        assert!(revision.contains("revision"));
        assert!(shape.contains("response shape"));
        assert_ne!(challenge, revision);
        assert_ne!(http, shape);
    }

    #[test]
    fn failed_replies_decode_without_a_payload() {
        let reply: BridgeReply<Vec<u8>> =
            serde_json::from_str(r#"{"ok":false,"kind":"http","status":403}"#)
                .expect("failure reply decodes");
        assert!(!reply.ok);
        assert!(reply.data.is_none());
        assert_eq!(reply.status, Some(403));
    }
}
