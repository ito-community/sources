//! Small helpers over `scraper`, so every parser reads pages the same way and can be tested on
//! plain HTML text.

use ito_rs::{Error, Result};
use scraper::{ElementRef, Html, Selector};

pub fn parse(html: &str) -> Html {
    Html::parse_document(html)
}

fn selector(css: &str) -> Result<Selector> {
    Selector::parse(css).map_err(|_| Error::Html("Invalid Temple Scan selector".into()))
}

/// Selector queries on a document or on one element.
pub trait Query {
    fn query<'a>(&'a self, css: &str) -> Result<Vec<ElementRef<'a>>>;

    fn first<'a>(&'a self, css: &str) -> Result<Option<ElementRef<'a>>> {
        Ok(self.query(css)?.into_iter().next())
    }
}

impl Query for Html {
    fn query<'a>(&'a self, css: &str) -> Result<Vec<ElementRef<'a>>> {
        Ok(self.select(&selector(css)?).collect())
    }
}

impl Query for ElementRef<'_> {
    fn query<'a>(&'a self, css: &str) -> Result<Vec<ElementRef<'a>>> {
        Ok(self.select(&selector(css)?).collect())
    }
}

/// All text under `node` with runs of whitespace collapsed.
pub fn text(node: &ElementRef<'_>) -> String {
    collapse(node.text())
}

/// Only the text that sits directly in `node`, not in its child elements.
pub fn own_text(node: &ElementRef<'_>) -> String {
    collapse(
        node.children()
            .filter_map(|child| child.value().as_text().map(|text| &**text)),
    )
}

fn collapse<'a>(parts: impl Iterator<Item = &'a str>) -> String {
    let mut out = String::new();
    for word in parts.flat_map(str::split_whitespace) {
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(word);
    }
    out
}

pub fn attr<'a>(node: &ElementRef<'a>, name: &str) -> Option<&'a str> {
    node.value().attr(name)
}

/// `Some` of the collapsed text, or `None` when it is empty.
pub fn non_empty(text: String) -> Option<String> {
    (!text.is_empty()).then_some(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_collapses_whitespace_and_own_text_skips_children() {
        let doc = parse("<div id=a>  Hello <span>big\n  world</span>  there </div>");
        let div = doc.first("#a").expect("selector").expect("node");
        assert_eq!(text(&div), "Hello big world there");
        assert_eq!(own_text(&div), "Hello there");
    }

    #[test]
    fn invalid_selectors_are_errors_not_panics() {
        let doc = parse("<p>x</p>");
        assert!(matches!(doc.query("p["), Err(Error::Html(_))));
    }
}
