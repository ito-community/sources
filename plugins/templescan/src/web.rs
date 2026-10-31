//! Pages and data through Ito's web view.
//!
//! The site answers a plain request with a Cloudflare Turnstile verification page, so every
//! page is loaded the way a person's browser loads it: in Ito's web view, which keeps the site's
//! cookies. When the site does show its verification page, the user finishes it in that page,
//! shown by the host on request; nothing here solves, scripts or skips it.

use crate::dom::{Query, text};
use crate::links::ORIGIN;
use crate::url;
use ito_rs::webview::{Webview, WebviewResponse};
use ito_rs::{Error, Result};
use scraper::Html;
use serde::Deserialize;
use std::cell::Cell;

/// Opt-in marker: the host runs the script as an async function body and awaits its result.
const ASYNC: &str = "//# ito-async\n";
const HOSTS: [&str; 1] = ["templetoons.com"];

thread_local! {
    /// Whether this plugin instance has loaded a page. Running a script before the web view has
    /// a page makes the host fail the call, so the first use loads a page instead of checking.
    static LOADED: Cell<bool> = const { Cell::new(false) };
}

/// The shared web view as the host provides it. Loads and scripts go through this trait so the
/// load, verify and retry sequences can be run against scripted answers.
pub trait Browser {
    /// Loads `address` in the web view and returns where it ended up and its HTML.
    fn load_url(address: &str) -> Result<WebviewResponse>;

    /// Runs `script` in the page last loaded and returns its result as text.
    fn execute_js(script: &str) -> Result<String>;
}

/// The host's web view.
pub struct HostView;

impl Browser for HostView {
    fn load_url(address: &str) -> Result<WebviewResponse> {
        Webview::load_url(address)
    }

    fn execute_js(script: &str) -> Result<String> {
        Webview::execute_js(script)
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum Kind {
    Ready,
    /// The site's verification page.
    Challenge,
}

/// Judges the page the web view ended on after a load that asked for a page of the site.
pub fn classify(address: &str, doc: &Html) -> Result<Kind> {
    let Some(parsed) = url::parse_absolute(address) else {
        return Err(Error::Net("Temple Scan's page did not load".into()));
    };
    if !HOSTS.contains(&parsed.host.as_str()) {
        return Err(Error::Host(
            "Temple Scan sent the web view to another address; try again".into(),
        ));
    }
    if is_challenge_path(parsed.path) {
        return Ok(Kind::Challenge);
    }
    let title = doc
        .first("title")?
        .map(|title| text(&title).to_lowercase())
        .unwrap_or_default();
    Ok(if title.starts_with("just a moment") {
        Kind::Challenge
    } else {
        Kind::Ready
    })
}

fn is_challenge_path(path: &str) -> bool {
    path == "/challenge" || path.starts_with("/challenge/")
}

/// Whether `address` is an address on the site.
fn on_site(address: &str) -> bool {
    url::parse_absolute(address).is_some_and(|parsed| HOSTS.contains(&parsed.host.as_str()))
}

fn unavailable(error: Error) -> Error {
    match error {
        // The host has no web view, or the call was cancelled before it answered.
        Error::Unsupported => {
            Error::Net("Temple Scan's web view was interrupted or is not available".into())
        }
        other => other,
    }
}

fn verification_needed() -> Error {
    Error::NeedsLogin(
        "Temple Scan needs you to finish its verification. Retry with Ito open: the page is \
         shown so you can complete it. If you closed it, leave Ito and come back first."
            .into(),
    )
}

fn still_verifying() -> Error {
    Error::NeedsLogin(
        "Temple Scan still asks for verification after you finished it; try again in a moment"
            .into(),
    )
}

fn moved_away() -> Error {
    Error::Host("Temple Scan's web view kept leaving the site; try again".into())
}

/// How a verification ended.
#[derive(Debug, PartialEq, Eq)]
enum Outcome {
    /// The site reported its verification finished.
    Verified,
    /// The page left the verification, the host ended the wait, or the page was not a
    /// verification page after all. The address is loaded again to learn where things stand.
    Left,
}

/// Asks the host to show the page the user has to finish, then waits until the site reports it
/// finished. The host shows a page only while the app is in the foreground and only for a
/// script that is waiting, and ends the script if the user closes the page.
fn verify<B: Browser>() -> Result<Outcome> {
    match B::execute_js(&verify_script()) {
        Ok(outcome) => match outcome.as_str() {
            "verified" => Ok(Outcome::Verified),
            "refused" => Err(verification_needed()),
            "moved" => Ok(Outcome::Left),
            _ => Err(Error::Host(
                "Temple Scan's verification did not finish; try again".into(),
            )),
        },
        // The host ends a waiting script when the page navigates on, as the site does after a
        // successful verification, and when the user closes the page.
        Err(_) => Ok(Outcome::Left),
    }
}

/// Runs in the verification page. `itoShowPage` asks the host to present the page; it answers
/// whether it did. The wait ends when the verification heading reports success, or the page
/// leaves the verification address. Both are events of the page itself.
const VERIFY: &str = r#"
if (location.origin !== ORIGIN) return "moved";
const onChallenge = () => location.pathname.startsWith("/challenge") || /^just a moment/i.test(document.title);
if (!onChallenge()) return "moved";
const verified = () => /^verified!/i.test((document.querySelector("h1")?.textContent || "").trim());
const settled = () => !/^just a moment/i.test(document.title) && (!location.pathname.startsWith("/challenge") || verified());
if (settled()) return "verified";
const shown = async () => {
  try { return (await window.webkit?.messageHandlers?.itoShowPage?.postMessage(null)) === true; }
  catch (_) { return false; }
};
if (!(await shown())) return "refused";
return await new Promise(resolve => {
  const observer = new MutationObserver(() => {
    if (settled()) { observer.disconnect(); resolve("verified"); }
  });
  observer.observe(document.documentElement, { childList: true, subtree: true, characterData: true });
  if (settled()) { observer.disconnect(); resolve("verified"); }
});
"#;

fn verify_script() -> String {
    format!("{ASYNC}const ORIGIN = {};\n{VERIFY}", origin_literal())
}

fn origin_literal() -> String {
    serde_json::to_string(ORIGIN).unwrap_or_else(|_| format!("\"{ORIGIN}\""))
}

/// Loads `path` of the site and returns its page. A verification page is finished by the user
/// first and the requested page loaded again.
pub fn load(path: &str) -> Result<Html> {
    load_with::<HostView>(path)
}

fn load_with<B: Browser>(path: &str) -> Result<Html> {
    let address = format!("{ORIGIN}{path}");
    match open::<B>(&address)? {
        (Kind::Ready, doc) => Ok(doc),
        (Kind::Challenge, _) => verify_then_reopen::<B>(&address),
    }
}

/// Finishes the verification on the page loaded at `address`, then loads `address` again. An
/// address that is still a verification page afterwards is an error for the user to act on.
fn verify_then_reopen<B: Browser>(address: &str) -> Result<Html> {
    let outcome = verify::<B>()?;
    match open::<B>(address)? {
        (Kind::Ready, doc) => Ok(doc),
        (Kind::Challenge, _) => Err(match outcome {
            Outcome::Verified => still_verifying(),
            Outcome::Left => verification_needed(),
        }),
    }
}

fn open<B: Browser>(address: &str) -> Result<(Kind, Html)> {
    let response = B::load_url(address).map_err(unavailable)?;
    LOADED.set(true);
    let doc = crate::dom::parse(&response.html);
    let kind = classify(&response.url, &doc)?;
    Ok((kind, doc))
}

/// What the site's own endpoint answered to a request made from its page.
#[derive(Debug, PartialEq, Eq)]
pub struct Reply {
    pub status: u16,
    pub url: String,
    pub body: String,
}

impl Reply {
    /// Whether the request was sent to the verification page: either it ended there, or the
    /// site refused it with the challenge's markers rather than an answer of its own.
    pub fn challenged(&self) -> bool {
        url::parse_absolute(&self.url).is_some_and(|parsed| is_challenge_path(parsed.path))
            || (self.status == 403 && shows_challenge(&self.body))
    }
}

/// Whether an HTML page is the site's verification page by its markers: the challenge's own
/// markup, or its "Just a moment" title. Any other refusal is the site's answer.
fn shows_challenge(body: &str) -> bool {
    if body.contains("cf-chl-") {
        return true;
    }
    crate::dom::parse(body)
        .first("title")
        .ok()
        .flatten()
        .is_some_and(|title| text(&title).to_lowercase().starts_with("just a moment"))
}

/// Requests `path` of the site from inside one of its pages, as its own scripts do, so the
/// request carries the page's cookies and passes whatever the site checks of its visitors.
pub fn fetch(path: &str) -> Result<Reply> {
    fetch_with::<HostView>(path)
}

fn fetch_with<B: Browser>(path: &str) -> Result<Reply> {
    ensure_site_page::<B>()?;
    let reply = settled_reply::<B>(path)?;
    if !reply.challenged() {
        return Ok(reply);
    }
    // The request was sent to the verification page. Its address is loaded (the site's home page
    // when it refused with the challenge's markers), verified if it really is a verification page,
    // and then the site's home page is loaded before the request is made again from it.
    let address = challenge_address(&reply);
    if open::<B>(&address)?.0 == Kind::Challenge {
        verify::<B>()?;
        load_with::<B>("/")?;
    }
    let reply = settled_reply::<B>(path)?;
    if reply.challenged() {
        return Err(still_verifying());
    }
    Ok(reply)
}

fn challenge_address(reply: &Reply) -> String {
    match url::parse_absolute(&reply.url) {
        Some(parsed) if is_challenge_path(parsed.path) => reply.url.clone(),
        _ => format!("{ORIGIN}/"),
    }
}

/// The reply to `path`. When the web view had left the site, the site's home page is loaded and
/// the request made once more.
fn settled_reply<B: Browser>(path: &str) -> Result<Reply> {
    match request::<B>(path)? {
        Answer::Reply(reply) => Ok(reply),
        Answer::Moved => {
            load_with::<B>("/")?;
            match request::<B>(path)? {
                Answer::Reply(reply) => Ok(reply),
                Answer::Moved => Err(moved_away()),
            }
        }
    }
}

/// Makes sure the web view is on a usable page of the site. Another source may have moved the
/// shared web view since this plugin last used it.
fn ensure_site_page<B: Browser>() -> Result<()> {
    if LOADED.get() && B::execute_js(&site_page_check()).is_ok_and(|state| state == "ready") {
        return Ok(());
    }
    load_with::<B>("/").map(|_| ())
}

fn site_page_check() -> String {
    format!(
        "(location.origin === {} && !location.pathname.startsWith('/challenge') && !/^just a moment/i.test(document.title)) ? 'ready' : 'missing'",
        origin_literal()
    )
}

/// What the fetch script answered: the site's reply, or that the web view was not on a page of
/// the site when the request was made.
#[derive(Debug, PartialEq, Eq)]
enum Answer {
    Moved,
    Reply(Reply),
}

/// The fetch script's answer as it arrives from the page.
#[derive(Deserialize)]
struct Raw {
    #[serde(default)]
    moved: bool,
    #[serde(default)]
    status: u16,
    #[serde(default)]
    url: String,
    #[serde(default)]
    body: String,
}

fn request<B: Browser>(path: &str) -> Result<Answer> {
    let raw = B::execute_js(&fetch_script(path)?).map_err(unavailable)?;
    answer(&raw)
}

fn answer(raw: &str) -> Result<Answer> {
    let raw: Raw = serde_json::from_str(raw)
        .map_err(|_| Error::Host("Temple Scan's web view answered unexpectedly".into()))?;
    if raw.moved {
        return Ok(Answer::Moved);
    }
    if raw.status == 0 {
        return Err(Error::Net("Temple Scan could not be reached".into()));
    }
    // A reply that ended anywhere else is not the site's answer, whatever it says.
    if !on_site(&raw.url) {
        return Err(Error::Host(
            "Temple Scan's request ended at another address; try again".into(),
        ));
    }
    Ok(Answer::Reply(Reply {
        status: raw.status,
        url: raw.url,
        body: raw.body,
    }))
}

/// The script that asks for `path` from the page. The path is embedded as a JSON string
/// literal, so no text of it can end the literal or add code.
fn fetch_script(path: &str) -> Result<String> {
    if !path.starts_with('/') || path.starts_with("//") {
        return Err(Error::Host(
            "Temple Scan request was not a site path".into(),
        ));
    }
    let literal = serde_json::to_string(path)
        .map_err(|_| Error::Host("Temple Scan request could not be prepared".into()))?;
    Ok(format!(
        "{ASYNC}const path = {literal};\nconst ORIGIN = {};\n{FETCH}",
        origin_literal()
    ))
}

/// Runs in the site's page. The request is made only from a page of the site that is not a
/// verification page; otherwise the page is reported as moved and the caller reloads it.
const FETCH: &str = r#"
const onChallenge = location.pathname.startsWith("/challenge") || /^just a moment/i.test(document.title);
if (location.origin !== ORIGIN || onChallenge) return JSON.stringify({ moved: true });
try {
  const response = await fetch(path, {
    method: "GET",
    credentials: "same-origin",
    headers: { accept: "application/json" },
  });
  const body = await response.text();
  return JSON.stringify({ status: response.status, url: response.url, body });
} catch (_) {
  return JSON.stringify({ status: 0, url: "", body: "" });
}
"#;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dom;
    use std::cell::RefCell;
    use std::collections::VecDeque;

    const SITE: &str = "https://templetoons.com/comics";
    const HOME: &str = "https://templetoons.com/";
    const CHALLENGE_URL: &str = "https://templetoons.com/challenge?redirect=%2Fcomics";
    const READY: &str = "<html><head><title>Temple Scan</title></head><body>ok</body></html>";
    const CHALLENGE: &str =
        "<html><head><title>Just a moment...</title></head><body></body></html>";

    thread_local! {
        static PAGES: RefCell<VecDeque<WebviewResponse>> = const { RefCell::new(VecDeque::new()) };
        static SCRIPTS: RefCell<VecDeque<Result<String>>> = const { RefCell::new(VecDeque::new()) };
        static LOG: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
    }

    /// A web view that answers from scripted queues and logs every load and script it runs.
    struct Scripted;

    impl Browser for Scripted {
        fn load_url(address: &str) -> Result<WebviewResponse> {
            LOG.with_borrow_mut(|log| log.push(format!("load {address}")));
            PAGES
                .with_borrow_mut(VecDeque::pop_front)
                .ok_or_else(|| Error::Net("no page scripted".into()))
        }

        fn execute_js(script: &str) -> Result<String> {
            let label = if script.contains("itoShowPage") {
                "verify"
            } else if script.contains("fetch(path") {
                "fetch"
            } else {
                "check"
            };
            LOG.with_borrow_mut(|log| log.push(label.to_owned()));
            SCRIPTS
                .with_borrow_mut(VecDeque::pop_front)
                .unwrap_or_else(|| Err(Error::Net("no script answer scripted".into())))
        }
    }

    fn script(pages: Vec<(&str, &str)>, answers: Vec<Result<String>>) {
        PAGES.with_borrow_mut(|queue| {
            queue.clear();
            queue.extend(pages.into_iter().map(|(url, html)| WebviewResponse {
                url: url.into(),
                html: html.into(),
            }));
        });
        SCRIPTS.with_borrow_mut(|queue| {
            queue.clear();
            queue.extend(answers);
        });
        LOG.with_borrow_mut(Vec::clear);
    }

    fn log() -> Vec<String> {
        LOG.with_borrow_mut(std::mem::take)
    }

    fn reply_json(status: u16, url: &str, body: &str) -> String {
        serde_json::json!({ "status": status, "url": url, "body": body }).to_string()
    }

    fn moved_json() -> String {
        r#"{"moved":true}"#.to_owned()
    }

    fn kind(address: &str, html: &str) -> Result<Kind> {
        classify(address, &dom::parse(html))
    }

    #[test]
    fn the_verification_page_is_told_from_the_site() {
        let page = "<html><head><title>Temple Scan</title></head><body>Verifying your connection...</body></html>";
        assert_eq!(
            kind("https://templetoons.com/challenge?redirect=%2Fcomics", page).ok(),
            Some(Kind::Challenge)
        );
        assert_eq!(
            kind("https://templetoons.com/challenge", page).ok(),
            Some(Kind::Challenge)
        );
        assert!(matches!(
            kind("https://www.templetoons.com/challenge/x", page),
            Err(Error::Host(_))
        ));
        assert_eq!(
            kind("https://templetoons.com/comics", page).ok(),
            Some(Kind::Ready)
        );
        assert_eq!(
            kind("https://templetoons.com/challenger-deep", page).ok(),
            Some(Kind::Ready)
        );
        assert_eq!(
            kind("https://templetoons.com/comic/challenge", page).ok(),
            Some(Kind::Ready)
        );
        let cloudflare = "<html><head><title>Just a moment...</title></head><body></body></html>";
        assert_eq!(
            kind("https://templetoons.com/comics", cloudflare).ok(),
            Some(Kind::Challenge)
        );
    }

    #[test]
    fn a_page_that_is_not_the_site_is_an_error_not_content() {
        let page = "<html><body>ad</body></html>";
        assert!(matches!(
            kind("https://ads.example/landing", page),
            Err(Error::Host(_))
        ));
        assert!(matches!(
            kind("https://templetoons.com.evil.example/", page),
            Err(Error::Host(_))
        ));
        assert!(matches!(kind("about:blank", page), Err(Error::Net(_))));
        assert!(matches!(kind("", page), Err(Error::Net(_))));
    }

    #[test]
    fn the_unavailable_web_view_is_a_network_failure_not_an_opt_out() {
        assert!(matches!(unavailable(Error::Unsupported), Error::Net(_)));
        assert!(matches!(
            unavailable(Error::Host("x".into())),
            Error::Host(_)
        ));
    }

    #[test]
    fn an_ordinary_page_loads_without_any_verification() {
        script(vec![(SITE, READY)], vec![]);
        assert!(load_with::<Scripted>("/comics").is_ok());
        assert_eq!(log(), [format!("load {SITE}")]);
    }

    #[test]
    fn a_verification_page_is_verified_then_loaded_again() {
        script(
            vec![(CHALLENGE_URL, CHALLENGE), (SITE, READY)],
            vec![Ok("verified".into())],
        );
        assert!(load_with::<Scripted>("/comics").is_ok());
        assert_eq!(
            log(),
            [
                format!("load {SITE}"),
                "verify".into(),
                format!("load {SITE}")
            ]
        );
    }

    #[test]
    fn a_verification_the_host_ended_is_loaded_again_once() {
        script(
            vec![(CHALLENGE_URL, CHALLENGE), (SITE, READY)],
            vec![Err(Error::Net("cancelled".into()))],
        );
        assert!(load_with::<Scripted>("/comics").is_ok());
        assert_eq!(
            log(),
            [
                format!("load {SITE}"),
                "verify".into(),
                format!("load {SITE}")
            ]
        );
    }

    #[test]
    fn a_verification_that_is_still_pending_asks_for_the_user() {
        script(
            vec![(CHALLENGE_URL, CHALLENGE), (CHALLENGE_URL, CHALLENGE)],
            vec![Err(Error::Net("cancelled".into()))],
        );
        assert!(matches!(
            load_with::<Scripted>("/comics"),
            Err(Error::NeedsLogin(_))
        ));
        assert_eq!(log().len(), 3);
    }

    #[test]
    fn a_site_that_stays_challenged_after_verifying_is_an_error_for_the_user() {
        script(
            vec![(CHALLENGE_URL, CHALLENGE), (CHALLENGE_URL, CHALLENGE)],
            vec![Ok("verified".into())],
        );
        assert!(matches!(
            load_with::<Scripted>("/comics"),
            Err(Error::NeedsLogin(message)) if message.contains("still asks")
        ));
    }

    #[test]
    fn a_refused_verification_is_not_reopened() {
        script(vec![(CHALLENGE_URL, CHALLENGE)], vec![Ok("refused".into())]);
        assert!(matches!(
            load_with::<Scripted>("/comics"),
            Err(Error::NeedsLogin(_))
        ));
        assert_eq!(log(), [format!("load {SITE}"), "verify".into()]);
    }

    #[test]
    fn a_page_that_is_not_the_site_is_never_judged_as_content() {
        script(vec![("https://ads.example/landing", READY)], vec![]);
        assert!(matches!(
            load_with::<Scripted>("/comics"),
            Err(Error::Host(_))
        ));
        assert_eq!(log(), [format!("load {SITE}")]);
    }

    #[test]
    fn the_fetch_script_embeds_the_path_as_data() {
        let script = fetch_script("/api/search?q=a\"+alert(1)+\"&x=\n").expect("script");
        assert!(script.starts_with("//# ito-async\nconst path = \""));
        // The quote and the newline stay inside the literal.
        assert!(script.contains(r#"\"+alert(1)+\""#));
        assert!(script.contains(r"&x=\n"));
        let literal_line = script.lines().nth(1).expect("literal line");
        assert!(literal_line.starts_with("const path = \"") && literal_line.ends_with("\";"));
        assert!(fetch_script("https://evil.example/").is_err());
        assert!(fetch_script("//evil.example/").is_err());
        assert!(fetch_script("api/search").is_err());
    }

    #[test]
    fn replies_are_taken_only_from_the_site() {
        let ok = answer(&reply_json(
            200,
            "https://templetoons.com/api/search?q=ab",
            "{}",
        ))
        .expect("reply");
        assert!(matches!(&ok, Answer::Reply(reply) if reply.status == 200 && !reply.challenged()));
        let redirected = answer(&reply_json(
            200,
            "https://templetoons.com/challenge?redirect=%2Fapi%2Fsearch",
            "<html>",
        ))
        .expect("reply");
        assert!(matches!(&redirected, Answer::Reply(reply) if reply.challenged()));
        let refused = answer(&reply_json(
            403,
            "https://templetoons.com/api/x",
            marked_refusal(),
        ))
        .expect("reply");
        assert!(matches!(&refused, Answer::Reply(reply) if reply.challenged()));
        let titled = answer(&reply_json(
            403,
            "https://templetoons.com/api/x",
            "<html><head><title>Just a moment...</title></head></html>",
        ))
        .expect("reply");
        assert!(matches!(&titled, Answer::Reply(reply) if reply.challenged()));
        let forbidden = answer(&reply_json(
            403,
            "https://templetoons.com/api/x",
            "<html>forbidden</html>",
        ))
        .expect("reply");
        assert!(matches!(&forbidden, Answer::Reply(reply) if !reply.challenged()));
        let plain = answer(&reply_json(
            403,
            "https://templetoons.com/api/x",
            r#"{"error":1}"#,
        ))
        .expect("reply");
        assert!(matches!(&plain, Answer::Reply(reply) if !reply.challenged()));
        assert_eq!(answer(&moved_json()).expect("moved"), Answer::Moved);
        assert!(matches!(answer(&reply_json(0, "", "")), Err(Error::Net(_))));
        assert!(matches!(
            answer(&reply_json(200, "https://ads.example/x", "{}")),
            Err(Error::Host(_))
        ));
        assert!(matches!(
            answer(&reply_json(503, "", "")),
            Err(Error::Host(_))
        ));
        assert!(matches!(answer("not json"), Err(Error::Host(_))));
    }

    #[test]
    fn a_fetch_from_the_site_returns_its_reply() {
        script(
            vec![(HOME, READY)],
            vec![Ok(reply_json(200, "https://templetoons.com/api/x", "{}"))],
        );
        let reply = fetch_with::<Scripted>("/api/x").expect("reply");
        assert_eq!(reply.body, "{}");
        assert_eq!(log(), [format!("load {HOME}"), "fetch".into()]);
    }

    #[test]
    fn a_fetch_the_web_view_left_is_made_again_from_the_home_page() {
        script(
            vec![(HOME, READY), (HOME, READY)],
            vec![
                Ok(moved_json()),
                Ok(reply_json(200, "https://templetoons.com/api/x", "{}")),
            ],
        );
        assert!(fetch_with::<Scripted>("/api/x").is_ok());
        assert_eq!(
            log(),
            [
                format!("load {HOME}"),
                "fetch".into(),
                format!("load {HOME}"),
                "fetch".into()
            ]
        );
    }

    #[test]
    fn a_fetch_that_keeps_leaving_the_site_is_an_error() {
        script(
            vec![(HOME, READY), (HOME, READY)],
            vec![Ok(moved_json()), Ok(moved_json())],
        );
        assert!(matches!(
            fetch_with::<Scripted>("/api/x"),
            Err(Error::Host(_))
        ));
    }

    /// A refusal whose markup names the site's challenge.
    fn marked_refusal() -> &'static str {
        "<html><head><title>Temple Scan</title></head><body><div id='cf-chl-widget'></div></body></html>"
    }

    #[test]
    fn a_challenged_request_verifies_then_retries_from_the_home_page() {
        script(
            vec![(HOME, READY), (CHALLENGE_URL, CHALLENGE), (HOME, READY)],
            vec![
                Ok(reply_json(200, CHALLENGE_URL, "<html>")),
                Ok("verified".into()),
                Ok(reply_json(
                    200,
                    "https://templetoons.com/api/x",
                    "{\"ok\":1}",
                )),
            ],
        );
        let reply = fetch_with::<Scripted>("/api/x").expect("reply");
        assert_eq!(reply.body, "{\"ok\":1}");
        // The challenge address is not reopened after verifying: a site that keeps verified
        // visitors there is retried from its home page.
        assert_eq!(
            log(),
            [
                format!("load {HOME}"),
                "fetch".into(),
                format!("load {CHALLENGE_URL}"),
                "verify".into(),
                format!("load {HOME}"),
                "fetch".into(),
            ]
        );
    }

    #[test]
    fn a_refusal_with_a_challenge_marker_is_loaded_without_verifying_when_ready() {
        script(
            vec![(HOME, READY), (HOME, READY)],
            vec![
                Ok(reply_json(
                    403,
                    "https://templetoons.com/api/x",
                    marked_refusal(),
                )),
                Ok(reply_json(200, "https://templetoons.com/api/x", "{}")),
            ],
        );
        assert!(fetch_with::<Scripted>("/api/x").is_ok());
        assert!(!log().iter().any(|entry| entry == "verify"));
    }

    #[test]
    fn a_plain_forbidden_page_is_the_sites_answer_not_a_challenge() {
        script(
            vec![(HOME, READY)],
            vec![Ok(reply_json(
                403,
                "https://templetoons.com/api/x",
                "<html>forbidden</html>",
            ))],
        );
        let reply = fetch_with::<Scripted>("/api/x").expect("reply");
        assert_eq!(reply.status, 403);
        assert_eq!(log(), [format!("load {HOME}"), "fetch".into()]);
    }

    #[test]
    fn a_challenge_that_persists_after_verifying_is_an_error_for_the_user() {
        script(
            vec![(HOME, READY), (CHALLENGE_URL, CHALLENGE), (HOME, READY)],
            vec![
                Ok(reply_json(200, CHALLENGE_URL, "<html>")),
                Ok("verified".into()),
                Ok(reply_json(
                    403,
                    "https://templetoons.com/api/x",
                    marked_refusal(),
                )),
            ],
        );
        assert!(matches!(
            fetch_with::<Scripted>("/api/x"),
            Err(Error::NeedsLogin(_))
        ));
    }

    #[test]
    fn the_page_check_names_the_site_origin() {
        let check = site_page_check();
        assert!(check.contains("\"https://templetoons.com\""));
        assert!(check.contains("/challenge"));
    }
}
