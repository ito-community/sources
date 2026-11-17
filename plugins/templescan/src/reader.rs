//! A chapter page: the images to read, or the reason there are none.
//!
//! The reader fills its page slots with images as they scroll into view, so the markup only
//! holds placeholders. The reader component is given every page's address in obfuscated form,
//! next to the layout hint of each page; [`tokens`] turns them back into addresses.

use crate::detail::is_not_found;
use crate::dom::{Query, text};
use crate::flight::Flight;
use crate::images;
use crate::site::Site;
use crate::tokens;
use ito_rs::models::{Page, PageContent};
use ito_rs::{Error, Result};
use scraper::Html;
use serde_json::Value;

/// The most script text the reader module is searched in. The web view buffers a reply whole
/// before the source sees it, so the SDK offers no way to bound the read; this is a sanity check
/// after the read, refusing an oversized script before it is searched.
const MAX_SCRIPT_BYTES: usize = 2 * 1024 * 1024;

/// What a chapter page offers a reader.
pub fn pages<S: Site>(doc: &Html) -> Result<Vec<Page>> {
    if early_access_gate(doc)? {
        return Err(early_access());
    }
    if is_not_found(doc) {
        return Err(Error::NotFound("Temple Scan has no such chapter".into()));
    }
    let reader = Reader::of(doc)?;
    let addresses = match decode_all(&reader.tokens, tokens::SEED) {
        Some(addresses) => addresses,
        None => recovered::<S>(&reader)?.ok_or_else(changed)?,
    };
    if addresses.is_empty() {
        return Err(Error::NotFound(
            "Temple Scan has no pages for this chapter".into(),
        ));
    }
    let mut pages = Vec::with_capacity(addresses.len());
    for (index, url) in addresses.into_iter().enumerate() {
        pages.push(Page {
            index: i32::try_from(index)
                .map_err(|_| Error::Host("Temple Scan chapter has too many pages".into()))?,
            content: PageContent::Url(url),
            has_description: false,
            description: None,
            headers: Some(images::headers()),
        });
    }
    Ok(pages)
}

/// The error for a chapter the site keeps for subscribers.
pub fn early_access() -> Error {
    Error::NeedsLogin(
        "This Temple Scan chapter is early access for subscribers. Ito can't open it; \
         it opens for everyone once the site releases it."
            .into(),
    )
}

/// The error for a reader the source no longer understands. It is never reported as an empty
/// chapter.
fn changed() -> Error {
    Error::Host("Temple Scan changed its reader; the source needs an update".into())
}

/// Whether the page is the site's notice that a chapter is early access for subscribers.
fn early_access_gate(doc: &Html) -> Result<bool> {
    for paragraph in doc.query("main p")? {
        if text(&paragraph)
            .to_lowercase()
            .contains("chapter is early access")
        {
            return Ok(true);
        }
    }
    Ok(false)
}

/// The reader component's data: its token list, and the script module that decodes the tokens
/// when the page names one.
struct Reader {
    /// The tokens in reading order; each one's position is its decode index.
    tokens: Vec<String>,
    script: Option<Script>,
}

/// The reader's script module: its id in the page's module table, and the chunk that loads it.
struct Script {
    module: u64,
    chunk: String,
}

impl Reader {
    /// The reader of `doc`. The page must have exactly one object with layout hints, and exactly
    /// one other array of strings in it, of the same length as the hints.
    fn of(doc: &Html) -> Result<Self> {
        let flight = Flight::from_document(doc)?;
        let found = flight.all(&["layoutHints"]);
        let [props] = found.as_slice() else {
            return Err(changed());
        };
        let Some(Value::Array(hints)) = props.get("layoutHints") else {
            return Err(changed());
        };
        let mut lists = props.iter().filter_map(|(key, value)| match value {
            Value::Array(items)
                if key != "layoutHints"
                    && items.len() == hints.len()
                    && items.iter().all(Value::is_string) =>
            {
                Some(items)
            }
            _ => None,
        });
        let (Some(list), None) = (lists.next(), lists.next()) else {
            return Err(changed());
        };
        let tokens = list
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_owned)
            .collect();
        let script = flight
            .reference_of(props)
            .and_then(|reference| Script::of(&flight, reference));
        Ok(Reader { tokens, script })
    }
}

impl Script {
    /// The script the module `reference` names, when its module row names a valid chunk.
    fn of(flight: &Flight, reference: &str) -> Option<Self> {
        let Value::Array(parts) = flight.module(reference)? else {
            return None;
        };
        let [Value::Number(module), Value::Array(chunks), ..] = parts.as_slice() else {
            return None;
        };
        let module = module.as_u64()?;
        // The module's own chunk is the last of the chunks that load it.
        let chunk = chunks.last()?.as_str()?;
        chunk_path_ok(chunk).then(|| Script {
            module,
            chunk: chunk.to_owned(),
        })
    }
}

/// Whether `path` is a script of the site's build: under `static/chunks/`, a `.js` file, with
/// nothing that could leave the directory. A `%` may only begin the bracket escapes `%5B` and
/// `%5D` that the site's dynamic route names use; every other byte is a plain path character.
fn chunk_path_ok(path: &str) -> bool {
    path.starts_with("static/chunks/")
        && path.ends_with(".js")
        && !path.contains("..")
        && escapes_ok(path)
}

/// Whether every `%` in `path` starts `%5B` or `%5D`, and every other byte is a plain path
/// character.
fn escapes_ok(path: &str) -> bool {
    let mut bytes = path.bytes();
    while let Some(byte) = bytes.next() {
        match byte {
            b'%' => {
                let escape = [bytes.next(), bytes.next()];
                if escape != [Some(b'5'), Some(b'B')] && escape != [Some(b'5'), Some(b'D')] {
                    return false;
                }
            }
            _ if byte.is_ascii_alphanumeric() || b"/._-()".contains(&byte) => {}
            _ => return false,
        }
    }
    true
}

/// Every token decoded under `seed` as a page address, in list order; `None` when any of them is
/// not a site image address. An element that already is an address passes through unchanged.
fn decode_all(tokens: &[String], seed: &str) -> Option<Vec<String>> {
    tokens
        .iter()
        .enumerate()
        .map(|(index, token)| {
            if token.starts_with("https://") {
                return images::permitted(token);
            }
            images::permitted(&tokens::decode(token, index, seed)?)
        })
        .collect()
}

/// The page addresses under the seed the reader's script declares, when that seed decodes every
/// token. `None` when the page names no script, the script cannot be read, or its seed does not
/// decode the tokens. The script is requested once.
fn recovered<S: Site>(reader: &Reader) -> Result<Option<Vec<String>>> {
    let Some(script) = &reader.script else {
        return Ok(None);
    };
    let reply = S::reply(&format!("/_next/{}", script.chunk))?;
    match reply.status {
        200 => {}
        404 => return Ok(None),
        status => {
            return Err(Error::Server(format!(
                "Temple Scan's reader script answered with HTTP {status}"
            )));
        }
    }
    // A sanity check after the read: the whole body is already in memory by this point.
    if reply.body.len() > MAX_SCRIPT_BYTES {
        return Ok(None);
    }
    let Some(seed) = tokens::seed_from_chunk(&reply.body, script.module) else {
        return Ok(None);
    };
    Ok(decode_all(&reader.tokens, &seed))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::dom;
    use crate::flight::script;
    use crate::tokens::{reader_script, scramble};
    use crate::web;
    use std::cell::RefCell;

    /// The page chunk of the reader module in the site's current build.
    const CHUNK: &str =
        "static/chunks/app/(public)/comic/%5Bslug%5D/%5Bch_slug%5D/page-a9aeefded0aa0809.js";
    const FIXTURE_SEED: &str = "fixtureSeed2026";

    /// The site's tokens for `a-suckers-love-story/chapter-1`, as served on 2026-11-16.
    const A_SUCKERS_TOKENS: [&str; 14] = [
        "spfvfINeTrkSh2H2W9rQJ8ntZ0At1dEaK47f7qJxaZWDgGFHwt8mu7UQsL9h-vxAvwlNmylq16a0NFq7j_mO2PNaF93xpevL_yC8C9hX2hH5yHrEJxAcJHha0hgS7ZSOwio72oowtK3RkT0yUjXD-qn5u4y4jeOrCw4",
        "jowy6N22UpdUNT4M7xFHrRKmt1ppePuv2dLbDh38LjRksvzKXjES9zoutCHy8MwGpOPdSNO89ht2pPQ7VNd0au0zCfU4rOfGfZYfGDLVqx-EZzZoV8sdwkZAekdB__4G_dSGP5rswgXGYZfQ6Yh04EdNoC_PBvLewWc",
        "FtC5aZ2qW3MdckM4qIFMllV_KYw81tTzqT4fEd40lrPol8r0-rqHwe6I7wkA5eOWHRs3-tilhe4KoIC5snowml9rpGVtE3leQTpK_jaWFoFxFbQdkRIpLnj0PqDhmN49xPbEDqWtHE89R_OXco93iQZGOz9ROpeW8Kc",
        "FuGBMAkN7XXWhbih2-N3KsloUhykVAVSb_IicDSJQ3LIW9Hp5PAT1r4BMI2Trp3yFrfNHNYVvh1cDB-VpB7rwXaZXBR55h7dZpJcMGOZ7vr3qJtpc01qAMklYYuV5tPB-slKcaGq0jmdV6dNrZZFmy50S0RsYxfQk6g",
        "74tjvPRpdLUzkMPGV0W6ZYmflmRXAYHtOmZOCK4DGXjSG-MH1yOFbsorikwaLoDYaileorx119tlAyZM8eLWVF7DIsiknTETeIrKGt_ms0UbXwHw4rkRQBEzoZb_HJmaloXc4pebyJJJ9RY6adsnq_AsERk0EkFZESE",
        "9e89sk7tY6y5YO98_N-nWXi7kkx3M_54PF3Cnsc8_dkUxoiMElOlH-TYx5D8KSQ1Ega-pDO66E2IeU6sC7I7NosuA508StMILMCrhHlUiuN_4DdYETADIaSXQsf6sQBGLyZNbqkxkmJWeBb7MEn_PE2nawMG2r3Ws_U",
        "Bo_pizk6bNRXx2m12Q3BQPdHp28jKjfuiePCK3sPcoiXXDlZagjz6EvFAp20biM56a_Fg3jiYlRTv6Pci2U2lq605ZRMFEuRsq8ln0q6eJY-YX70IRJ6W86qM6f-0WkvDBFTiCo_Yk3wJyZaDRymZwriiEv3g2NE_pY",
        "5NtTibHPt5mSgZfUtruowb1PBXtuSNuN63LwdoexGQORBdEaAuIWt9znrr511lnxTrd85Pc-HnttCYlSTyu6TEiAYavpvveZjE-TjYBTm7KFSCHUsYplepsQ6LEMzqIWL2x_iObeO0CgI-2sfq5S5Jpzs1tuzv77sWw",
        "NHHv72HATY15oZalM1V6LxlDy3_ZiwCD8VmrVW0YSn8Vlfqg0EK0UiybcewXPohrRlcQX0jxxTmItvNZ6GXx2R7EUo_XILQE87GZJLKIzjzEp-sB7EqT7Rv0-kcLw5wAcgQdOdMjTvYOXsM-stg3ypI8gMISfQgX34c",
        "4Tru174VtHdw4glY_kfjDn-rEPMwEt7Lsk2zBV1yp9ANBeuAjU-GtMTWGxxwxfzzWQv0Gp6zDWQpYPaMCsFBKPyiXJ0a3RfJ5loMaMICHgbOP7NT6reS_8r2rV0xynTdW_piSTaB0OM8gsGWwbzFABjDeNeDifgu8k8",
        "ACEYIgQIbqifml1_D0aGBGUrsQK785pensH9OGqm9Y-b-wVKolWLMQN-14oD8HfRaYVbOlbzpOKViPLbsY_s7e-p2rZ4TUIcRU5O2jOkXuyD77Z8YjElV-PMaYX9i2TyV1JWahCMNy-7eZg8dTm0gXOHCxXDzcQacZY",
        "2PnmsSe7Ek4RSVks9L6MM-NWznfFPCkzmziUfPz8Yt4XeMeZmv82kjz7ksz2F-3HYx810gTlWAIO9aZwt3ryEI_uQSKIL6GnF9q0DXClUEQxIpGrYpLgC4JfPKH3Y1S_eNjamCPeQ8iz_E5Q81CS239KzKdMcPcW6Vg",
        "De2W6oJ_X-qKS5gulwI2KWbDDUbnctolAF62gO6Po6Im-4bn5qpbg7TTOtC6yfNLW4gIiMHg6TalD3fL5kqIfFl2JHw04sOOYq3jyQZHx-WGwpbo-aqlkfBgAQseu52WiopmLsQ3Ffr3qtvOtQu2bASEb_KnmFdGJLY",
        "thVAvVAQzflXEVrP4a782jAzNjOZ9SmNtAvd_PuMTSsrMx1BKTTr_VyyN_3Gs4hRaOMHmTPWAGu-g3SIlTiNNlXXQ8iH6VvC49tWsJNwJmhVQrQtgUu6j4OD-oTFFPxM-u33MGJz9pP1f6jolI8Hg5rF0JMdBkCD_jQ",
    ];

    pub(crate) fn chapter_page(images: &[&str], key: &str) -> String {
        let list = images
            .iter()
            .map(|image| format!("{image:?}"))
            .collect::<Vec<_>>()
            .join(",");
        let hints = images
            .iter()
            .map(|_| "{\"w\":720,\"h\":12000}")
            .collect::<Vec<_>>()
            .join(",");
        let row = format!(
            "c:[[\"$\",\"$L2c\",null,{{\"{key}\":[{list}],\"layoutHints\":[{hints}],\"chapterName\":\"Chapter 1\"}}]]\n"
        );
        format!(
            "<html><body><main><h1 class='sr-only'>Evil Husband - Chapter 1</h1>\
             <a href='https://ads.example/x'><img src='/promo/reader-ad-pc.webp'></a>\
             <div data-index='0'><span>Loading image 1...</span></div></main>{}</body></html>",
            script(&[&row])
        )
    }

    /// A chapter whose reader element `$L2c` has `tokens` with null hints. When `chunk` is given,
    /// the element's module `44083` is loaded from that chunk, as the site's pages name it.
    fn reader_chapter(tokens: &[String], chunk: Option<&str>) -> String {
        let list = tokens
            .iter()
            .map(|token| serde_json::to_string(token).expect("token serialises"))
            .collect::<Vec<_>>()
            .join(",");
        let hints = vec!["null"; tokens.len()].join(",");
        let mut rows = format!(
            "c:[[\"$\",\"$L2c\",null,{{\"k7q\":[{list}],\"layoutHints\":[{hints}],\"chapterName\":\"Chapter 1\"}}]]\n"
        );
        if let Some(chunk) = chunk {
            rows.push_str(&format!(
                "2c:I[44083,[\"7699\",\"static/chunks/8e1d74a4.js\",\"6969\",\"{chunk}\"],\"default\"]\n"
            ));
        }
        format!(
            "<html><body><main><h1 class='sr-only'>Evil Husband - Chapter 1</h1></main>{}</body></html>",
            script(&[&rows])
        )
    }

    pub(crate) fn url(n: usize) -> String {
        format!("https://media.templetoons.com/file/b/uploads/series/s/c/{n:02}.jpg")
    }

    /// The addresses a chapter's tokens stand for, in the site's own shape.
    fn a_suckers_urls() -> Vec<String> {
        (1..=14)
            .map(|n| {
                format!(
                    "https://media.templetoons.com/file/terms54/uploads/series/a-suckers-love-story/921c10d9-2732-4871-a6bc-dc154fe53561/{n:02}.jpg"
                )
            })
            .collect()
    }

    /// A site that serves no pages and no scripts. Tests that never reach the reader script use it.
    struct Offline;

    impl Site for Offline {
        fn page(_path: &str) -> Result<Html> {
            Err(Error::Net("offline".into()))
        }

        fn reply(_path: &str) -> Result<web::Reply> {
            Err(Error::Net("offline".into()))
        }
    }

    thread_local! {
        static SCRIPT_ANSWER: RefCell<Option<(u16, String)>> = const { RefCell::new(None) };
        static REQUESTS: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
    }

    /// A site whose reader script is the one answer the test sets, and which logs its requests.
    struct Served;

    impl Site for Served {
        fn page(_path: &str) -> Result<Html> {
            Err(Error::Net("no pages are served".into()))
        }

        fn reply(path: &str) -> Result<web::Reply> {
            REQUESTS.with_borrow_mut(|requests| requests.push(path.to_owned()));
            SCRIPT_ANSWER.with_borrow(|answer| match answer {
                Some((status, body)) => Ok(web::Reply {
                    status: *status,
                    url: String::new(),
                    body: body.clone(),
                }),
                None => Err(Error::Net("no script is served".into())),
            })
        }
    }

    fn serve_script(status: u16, body: String) {
        SCRIPT_ANSWER.with_borrow_mut(|answer| *answer = Some((status, body)));
    }

    fn requests() -> Vec<String> {
        REQUESTS.with_borrow_mut(std::mem::take)
    }

    fn offline(html: &str) -> Result<Vec<Page>> {
        pages::<Offline>(&dom::parse(html))
    }

    fn content_url(page: &Page) -> &str {
        match &page.content {
            PageContent::Url(url) => url,
            PageContent::Text(_) => panic!("image page expected"),
            _ => panic!("unexpected page content"),
        }
    }

    fn urls_of(pages: &[Page]) -> Vec<&str> {
        pages.iter().map(content_url).collect()
    }

    #[test]
    fn pages_follow_the_readers_list_in_order_with_the_sites_referrer() {
        let addresses: Vec<String> = (1..=12).map(url).collect();
        let refs: Vec<&str> = addresses.iter().map(String::as_str).collect();
        // The list's name is a renamed token and differs between builds.
        for key in ["k9wavlj", "zzzzzzz"] {
            let pages = offline(&chapter_page(&refs, key)).expect("pages");
            assert_eq!(pages.len(), 12);
            for (index, page) in pages.iter().enumerate() {
                assert_eq!(page.index, i32::try_from(index).expect("small"));
                assert_eq!(content_url(page), addresses[index]);
                assert!(!page.has_description && page.description.is_none());
                let headers = page.headers.as_ref().expect("headers");
                assert_eq!(
                    headers.get("Referer").map(String::as_str),
                    Some("https://templetoons.com/")
                );
            }
        }
    }

    #[test]
    fn banners_in_the_markup_are_not_pages() {
        let only = url(1);
        let pages = offline(&chapter_page(&[&only], "k")).expect("pages");
        assert_eq!(pages.len(), 1);
        assert_eq!(content_url(&pages[0]), only);
    }

    #[test]
    fn a_chapter_with_no_images_is_not_found_rather_than_an_empty_success() {
        let html = chapter_page(&[], "k");
        assert!(matches!(offline(&html), Err(Error::NotFound(_))));
    }

    #[test]
    fn the_early_access_notice_is_refused_as_needs_login() {
        let html = "<html><body><main><h1 class='sr-only'>Evil Husband - Chapter 52</h1>\
                    <div><p>This chapter is early access!</p><p>You don't have a subscription</p>\
                    <a href='/comic/evil-husband'>Series Homepage</a></div></main></body></html>";
        assert!(matches!(offline(html), Err(Error::NeedsLogin(_))));
        // The notice wins even if a page's data were present.
        let both = chapter_page(&[&url(1)], "k").replace(
            "<h1 class='sr-only'>",
            "<p>This chapter is early access!</p><h1 class='sr-only'>",
        );
        assert!(matches!(offline(&both), Err(Error::NeedsLogin(_))));
    }

    #[test]
    fn a_missing_chapter_is_not_found() {
        let html = "<html><body><h1>404</h1><p>This page does not exist or is no longer available.</p></body></html>";
        assert!(matches!(offline(html), Err(Error::NotFound(_))));
    }

    #[test]
    fn a_page_from_an_unexpected_address_fails_the_chapter() {
        let good = url(1);
        let html = chapter_page(&[&good, "https://example.com/02.jpg"], "k");
        assert!(matches!(offline(&html), Err(Error::Host(_))));
        let html = chapter_page(&[&good, "http://media.templetoons.com/02.jpg"], "k");
        assert!(matches!(offline(&html), Err(Error::Host(_))));
        let html = chapter_page(&[&good, "https://media.mediacloud.lat/02.jpg"], "k");
        assert!(matches!(offline(&html), Err(Error::Host(_))));
    }

    #[test]
    fn a_reader_the_source_cannot_understand_is_an_error() {
        // No reader data at all.
        let html = "<html><body><main><h1>Chapter</h1></main></body></html>";
        assert!(matches!(offline(html), Err(Error::Host(_))));
        // Two candidate lists of the same length cannot be told apart.
        let row = "c:[{\"a\":[\"x\"],\"b\":[\"y\"],\"layoutHints\":[null]}]\n";
        let html = format!("<body><main></main>{}</body>", script(&[row]));
        assert!(matches!(offline(&html), Err(Error::Host(_))));
        // Hints and images of different lengths do not describe the same pages.
        let row = "c:[{\"a\":[\"x\",\"y\"],\"layoutHints\":[null]}]\n";
        let html = format!("<body><main></main>{}</body>", script(&[row]));
        assert!(matches!(offline(&html), Err(Error::Host(_))));
        // Two objects with layout hints are not one reader.
        let row =
            "c:[{\"a\":[\"x\"],\"layoutHints\":[null]},{\"b\":[\"y\"],\"layoutHints\":[null]}]\n";
        let html = format!("<body><main></main>{}</body>", script(&[row]));
        assert!(matches!(offline(&html), Err(Error::Host(_))));
    }

    #[test]
    fn a_chapter_decodes_its_tokens_with_the_pinned_seed() {
        let html = reader_chapter(&A_SUCKERS_TOKENS.map(str::to_owned), None);
        let pages = offline(&html).expect("pages");
        assert_eq!(urls_of(&pages), a_suckers_urls());
        assert!(requests().is_empty(), "the pinned seed needs no script");
    }

    #[test]
    fn null_layout_hints_do_not_stop_a_chapter() {
        let tokens: Vec<String> = [0, 1]
            .into_iter()
            .map(|index| scramble(&url(index + 1), index, tokens::SEED))
            .collect();
        let html = reader_chapter(&tokens, None);
        let pages = offline(&html).expect("pages");
        assert_eq!(urls_of(&pages), [url(1), url(2)]);
    }

    #[test]
    fn addresses_already_in_the_list_pass_through_unchanged() {
        let tokens = vec![url(1), scramble(&url(2), 1, tokens::SEED)];
        let pages = offline(&reader_chapter(&tokens, None)).expect("pages");
        assert_eq!(urls_of(&pages), [url(1), url(2)]);
        // A pass-through address is still held to the site's image hosts.
        let tokens = vec!["https://example.com/01.jpg".to_owned()];
        assert!(matches!(
            offline(&reader_chapter(&tokens, None)),
            Err(Error::Host(_))
        ));
    }

    #[test]
    fn a_wrong_seed_recovers_from_the_readers_script_once() {
        let tokens: Vec<String> = (1..=3)
            .map(|n| scramble(&url(n), n - 1, FIXTURE_SEED))
            .collect();
        serve_script(200, reader_script(44083, FIXTURE_SEED));
        let pages =
            pages::<Served>(&dom::parse(&reader_chapter(&tokens, Some(CHUNK)))).expect("pages");
        assert_eq!(urls_of(&pages), [url(1), url(2), url(3)]);
        assert_eq!(requests(), [format!("/_next/{CHUNK}")]);
    }

    #[test]
    fn a_script_declaring_another_seed_is_a_reader_change() {
        let tokens: Vec<String> = (1..=2)
            .map(|n| scramble(&url(n), n - 1, FIXTURE_SEED))
            .collect();
        serve_script(200, reader_script(44083, "someOtherSeed77"));
        assert!(matches!(
            pages::<Served>(&dom::parse(&reader_chapter(&tokens, Some(CHUNK)))),
            Err(Error::Host(_))
        ));
        assert_eq!(requests().len(), 1);
    }

    #[test]
    fn tokens_the_page_no_longer_decodes_with_no_script_are_a_reader_change() {
        let tokens: Vec<String> = vec![scramble(&url(1), 0, FIXTURE_SEED)];
        assert!(matches!(
            pages::<Served>(&dom::parse(&reader_chapter(&tokens, None))),
            Err(Error::Host(_))
        ));
        assert!(requests().is_empty(), "no script is named, none is fetched");
    }

    #[test]
    fn a_script_that_is_not_a_site_chunk_is_not_fetched() {
        let tokens: Vec<String> = vec![scramble(&url(1), 0, FIXTURE_SEED)];
        serve_script(200, reader_script(44083, FIXTURE_SEED));
        for chunk in [
            "../secret.js",
            "static/chunks/x.html",
            "https://evil.example/x.js",
        ] {
            assert!(matches!(
                pages::<Served>(&dom::parse(&reader_chapter(&tokens, Some(chunk)))),
                Err(Error::Host(_))
            ));
        }
        assert!(requests().is_empty());
    }

    #[test]
    fn a_script_that_cannot_be_read_is_a_reader_change_or_a_server_error() {
        let tokens: Vec<String> = vec![scramble(&url(1), 0, FIXTURE_SEED)];
        let html = reader_chapter(&tokens, Some(CHUNK));
        serve_script(404, String::new());
        assert!(matches!(
            pages::<Served>(&dom::parse(&html)),
            Err(Error::Host(_))
        ));
        serve_script(503, String::new());
        assert!(matches!(
            pages::<Served>(&dom::parse(&html)),
            Err(Error::Server(_))
        ));
        serve_script(200, "no reader here".into());
        assert!(matches!(
            pages::<Served>(&dom::parse(&html)),
            Err(Error::Host(_))
        ));
    }

    #[test]
    fn a_reader_script_that_is_too_large_is_refused() {
        let tokens: Vec<String> = vec![scramble(&url(1), 0, FIXTURE_SEED)];
        serve_script(200, "x".repeat(MAX_SCRIPT_BYTES + 1));
        assert!(matches!(
            pages::<Served>(&dom::parse(&reader_chapter(&tokens, Some(CHUNK)))),
            Err(Error::Host(_))
        ));
    }

    #[test]
    fn the_early_access_notice_never_fetches_a_script() {
        let html = "<html><body><main><p>This chapter is early access!</p></main></body></html>";
        assert!(matches!(
            pages::<Served>(&dom::parse(html)),
            Err(Error::NeedsLogin(_))
        ));
        assert!(requests().is_empty());
    }

    #[test]
    fn chunk_paths_must_be_build_scripts() {
        assert!(chunk_path_ok(CHUNK));
        assert!(!chunk_path_ok("static/chunks/../x.js"));
        assert!(!chunk_path_ok("static/chunks/x.js?v=1"));
        assert!(!chunk_path_ok("static/css/x.js"));
        // Only the bracket escapes of dynamic route names are percent-encoded.
        assert!(chunk_path_ok(
            "static/chunks/app/%5Bslug%5D/%5Bch_slug%5D/page-a.js"
        ));
        assert!(chunk_path_ok("static/chunks/app/%5B%5D/x.js"));
        for escaped in [
            "static/chunks/a%2e%2e/x.js",
            "static/chunks/%2e/x.js",
            "static/chunks/%%5B5D.js",
            "static/chunks/%5b/x.js",
            "static/chunks/%5/x.js",
            "static/chunks/x%",
            "static/chunks/%20x.js",
        ] {
            assert!(!chunk_path_ok(escaped), "{escaped}");
        }
    }
}
