//! A chapter page: the images to read, or the reason there are none.
//!
//! The reader fills its page slots with images as they scroll into view, so the markup only
//! holds placeholders. The addresses of every page are in the data the reader component is
//! given, next to the layout hint of each page.

use crate::detail::is_not_found;
use crate::dom::{Query, text};
use crate::flight::Flight;
use crate::images;
use ito_rs::models::{Page, PageContent};
use ito_rs::{Error, Result};
use scraper::Html;
use serde_json::Value;

/// What a chapter page offers a reader.
pub fn pages(doc: &Html) -> Result<Vec<Page>> {
    if early_access_gate(doc)? {
        return Err(early_access());
    }
    if is_not_found(doc) {
        return Err(Error::NotFound("Temple Scan has no such chapter".into()));
    }
    let addresses = addresses(doc)?;
    if addresses.is_empty() {
        return Err(Error::NotFound(
            "Temple Scan has no pages for this chapter".into(),
        ));
    }
    let mut pages = Vec::with_capacity(addresses.len());
    for (index, address) in addresses.iter().enumerate() {
        // A page that cannot be fetched would leave a hole in the chapter, so none is dropped.
        let url = images::locate(address).ok_or_else(|| {
            Error::Host("Temple Scan returned a page image from an unexpected address".into())
        })?;
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

/// The page image addresses in reading order: the list the reader component receives beside its
/// `layoutHints`, whatever the site names that list.
fn addresses(doc: &Html) -> Result<Vec<String>> {
    let layout =
        || Error::Host("Temple Scan changed its reader; the source needs an update".into());
    let flight = Flight::from_document(doc)?;
    let props = flight.find(&["layoutHints"]).ok_or_else(layout)?;
    let Some(Value::Array(hints)) = props.get("layoutHints") else {
        return Err(layout());
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
        return Err(layout());
    };
    Ok(list
        .iter()
        .filter_map(Value::as_str)
        .map(str::to_owned)
        .collect())
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::dom;
    use crate::flight::script;

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

    pub(crate) fn url(n: usize) -> String {
        format!("https://media.templetoons.com/file/b/uploads/series/s/c/{n:02}.jpg")
    }

    fn content_url(page: &Page) -> &str {
        match &page.content {
            PageContent::Url(url) => url,
            PageContent::Text(_) => panic!("image page expected"),
            _ => panic!("unexpected page content"),
        }
    }

    #[test]
    fn pages_follow_the_readers_list_in_order_with_the_sites_referrer() {
        let addresses: Vec<String> = (1..=12).map(url).collect();
        let refs: Vec<&str> = addresses.iter().map(String::as_str).collect();
        // The list's name is a renamed token and differs between builds.
        for key in ["k9wavlj", "zzzzzzz"] {
            let pages = pages(&dom::parse(&chapter_page(&refs, key))).expect("pages");
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
        let pages = pages(&dom::parse(&chapter_page(&[&only], "k"))).expect("pages");
        assert_eq!(pages.len(), 1);
        assert_eq!(content_url(&pages[0]), only);
    }

    #[test]
    fn a_chapter_with_no_images_is_not_found_rather_than_an_empty_success() {
        let html = chapter_page(&[], "k");
        assert!(matches!(pages(&dom::parse(&html)), Err(Error::NotFound(_))));
    }

    #[test]
    fn the_early_access_notice_is_refused_as_needs_login() {
        let html = "<html><body><main><h1 class='sr-only'>Evil Husband - Chapter 52</h1>\
                    <div><p>This chapter is early access!</p><p>You don't have a subscription</p>\
                    <a href='/comic/evil-husband'>Series Homepage</a></div></main></body></html>";
        assert!(matches!(
            pages(&dom::parse(html)),
            Err(Error::NeedsLogin(_))
        ));
        // The notice wins even if a page's data were present.
        let both = chapter_page(&[&url(1)], "k").replace(
            "<h1 class='sr-only'>",
            "<p>This chapter is early access!</p><h1 class='sr-only'>",
        );
        assert!(matches!(
            pages(&dom::parse(&both)),
            Err(Error::NeedsLogin(_))
        ));
    }

    #[test]
    fn a_missing_chapter_is_not_found() {
        let html = "<html><body><h1>404</h1><p>This page does not exist or is no longer available.</p></body></html>";
        assert!(matches!(pages(&dom::parse(html)), Err(Error::NotFound(_))));
    }

    #[test]
    fn a_page_from_an_unexpected_address_fails_the_chapter() {
        let good = url(1);
        let html = chapter_page(&[&good, "https://example.com/02.jpg"], "k");
        assert!(matches!(pages(&dom::parse(&html)), Err(Error::Host(_))));
        let html = chapter_page(&[&good, "http://media.templetoons.com/02.jpg"], "k");
        assert!(matches!(pages(&dom::parse(&html)), Err(Error::Host(_))));
    }

    #[test]
    fn a_reader_the_source_cannot_understand_is_an_error() {
        // No reader data at all.
        let html = "<html><body><main><h1>Chapter</h1></main></body></html>";
        assert!(matches!(pages(&dom::parse(html)), Err(Error::Host(_))));
        // Two candidate lists of the same length cannot be told apart.
        let row = "c:[{\"a\":[\"x\"],\"b\":[\"y\"],\"layoutHints\":[null]}]\n";
        let html = format!("<body><main></main>{}</body>", script(&[row]));
        assert!(matches!(pages(&dom::parse(&html)), Err(Error::Host(_))));
        // Hints and images of different lengths do not describe the same pages.
        let row = "c:[{\"a\":[\"x\",\"y\"],\"layoutHints\":[null]}]\n";
        let html = format!("<body><main></main>{}</body>", script(&[row]));
        assert!(matches!(pages(&dom::parse(&html)), Err(Error::Host(_))));
    }
}
