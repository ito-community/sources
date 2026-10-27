//! Small string scanner for HiAnime's server-rendered markup.
//!
//! The host DOM (`ito_rs::html::Node`) is a wasm import, so it cannot run in native
//! unit tests. This scanner covers the subset the parsers need: start tags with quoted
//! or bare attributes, the matching end tag of an element, and text extraction. It skips
//! comments and the raw bodies of `script` and `style`.

/// A start tag: its lowercase-insensitive name and raw attribute text.
#[derive(Debug, Clone, Copy)]
pub struct Tag<'a> {
    pub name: &'a str,
    attrs: &'a str,
    /// Byte offset just past the tag's closing `>`, relative to the scanned document.
    end: usize,
}

impl Tag<'_> {
    /// Returns the decoded value of attribute `name`, or `None` when it is absent.
    pub fn attr(&self, name: &str) -> Option<String> {
        attr_value(self.attrs, name)
    }

    /// Returns true when the whitespace-separated `class` list contains `class`.
    pub fn has_class(&self, class: &str) -> bool {
        self.attr("class")
            .is_some_and(|value| value.split_ascii_whitespace().any(|token| token == class))
    }
}

/// An element with its markup and the document that follows its end tag.
#[derive(Debug, Clone, Copy)]
pub struct Element<'a> {
    pub tag: Tag<'a>,
    /// Markup between the start tag and the matching end tag.
    pub inner: &'a str,
    /// Markup after the matching end tag, used to continue a sequential scan.
    pub after: &'a str,
}

/// Iterator over the start tags of a document, in order.
pub struct Tags<'a> {
    html: &'a str,
    pos: usize,
}

/// Returns every start tag of `html` in document order.
pub fn tags(html: &str) -> Tags<'_> {
    Tags { html, pos: 0 }
}

impl<'a> Iterator for Tags<'a> {
    type Item = Tag<'a>;

    fn next(&mut self) -> Option<Tag<'a>> {
        let html = self.html;
        loop {
            let open = self.pos + html[self.pos..].find('<')?;
            let rest = &html[open..];

            if rest.starts_with("<!--") {
                self.pos = open + rest.find("-->").map_or(rest.len(), |end| end + 3);
                continue;
            }
            if !rest[1..].starts_with(|c: char| c.is_ascii_alphabetic()) {
                self.pos = open + 1;
                continue;
            }

            let name_len = rest[1..]
                .bytes()
                .take_while(|b| b.is_ascii_alphanumeric() || *b == b'-')
                .count();
            let name = &rest[1..=name_len];
            let attrs_start = 1 + name_len;
            let Some(close) = tag_close(&rest[attrs_start..]) else {
                self.pos = html.len();
                return None;
            };
            let attrs = &rest[attrs_start..attrs_start + close];
            let end = open + attrs_start + close + 1;
            self.pos = end;

            if name.eq_ignore_ascii_case("script") || name.eq_ignore_ascii_case("style") {
                self.pos = find_close(&html[end..], name).map_or(html.len(), |at| end + at);
            }
            return Some(Tag { name, attrs, end });
        }
    }
}

/// Returns the first element named `name` (or any element for `"*"`) that satisfies `pred`.
pub fn next_element<'a>(
    html: &'a str,
    name: &str,
    pred: impl Fn(&Tag<'a>) -> bool,
) -> Option<Element<'a>> {
    tags(html)
        .find(|tag| (name == "*" || tag.name.eq_ignore_ascii_case(name)) && pred(tag))
        .map(|tag| element(html, tag))
}

/// Returns the text of `markup` with tags removed, entities decoded, and whitespace collapsed.
pub fn text(markup: &str) -> String {
    let mut stripped = String::with_capacity(markup.len());
    let mut rest = markup;
    while let Some(open) = rest.find('<') {
        stripped.push_str(&rest[..open]);
        stripped.push(' ');
        rest = match rest[open..].find('>') {
            Some(close) => &rest[open + close + 1..],
            None => "",
        };
    }
    stripped.push_str(rest);
    decode_entities(&stripped)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// Elements that never have an end tag.
const VOID_ELEMENTS: &[&str] = &[
    "area", "base", "br", "col", "embed", "hr", "img", "input", "link", "meta", "source", "track",
    "wbr",
];

fn element<'a>(html: &'a str, tag: Tag<'a>) -> Element<'a> {
    let body = &html[tag.end..];
    if VOID_ELEMENTS
        .iter()
        .any(|void| void.eq_ignore_ascii_case(tag.name))
    {
        return Element {
            tag,
            inner: "",
            after: body,
        };
    }
    match find_close(body, tag.name) {
        Some(close) => {
            let after_start = body[close..]
                .find('>')
                .map_or(body.len(), |gt| close + gt + 1);
            Element {
                tag,
                inner: &body[..close],
                after: &body[after_start..],
            }
        }
        None => Element {
            tag,
            inner: body,
            after: "",
        },
    }
}

/// Finds `</name` followed by `>` or whitespace, so `</abbr>` never closes `a`.
fn find_close(body: &str, name: &str) -> Option<usize> {
    let bytes = body.as_bytes();
    let mut from = 0;
    while let Some(offset) = body[from..].find("</") {
        let at = from + offset;
        let start = at + 2;
        let end = start + name.len();
        let name_matches = bytes
            .get(start..end)
            .is_some_and(|candidate| candidate.eq_ignore_ascii_case(name.as_bytes()));
        let boundary = bytes
            .get(end)
            .is_none_or(|byte| *byte == b'>' || byte.is_ascii_whitespace());
        if name_matches && boundary {
            return Some(at);
        }
        from = start;
    }
    None
}

/// Returns the offset of the `>` that ends a start tag, ignoring `>` inside quoted values.
fn tag_close(after_name: &str) -> Option<usize> {
    let mut quote: Option<u8> = None;
    for (offset, byte) in after_name.bytes().enumerate() {
        match (quote, byte) {
            (Some(open), _) if byte == open => quote = None,
            (Some(_), _) => {}
            (None, b'"' | b'\'') => quote = Some(byte),
            (None, b'>') => return Some(offset),
            (None, _) => {}
        }
    }
    None
}

/// Reads one attribute from raw attribute text. Names match case-insensitively.
fn attr_value(attrs: &str, wanted: &str) -> Option<String> {
    let bytes = attrs.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        while i < bytes.len() && (bytes[i].is_ascii_whitespace() || bytes[i] == b'/') {
            i += 1;
        }
        let name_start = i;
        while i < bytes.len() && !bytes[i].is_ascii_whitespace() && !matches!(bytes[i], b'=' | b'/')
        {
            i += 1;
        }
        if i == name_start {
            i += 1;
            continue;
        }
        let name = &attrs[name_start..i];

        let mut j = i;
        while j < bytes.len() && bytes[j].is_ascii_whitespace() {
            j += 1;
        }
        let mut value = "";
        if j < bytes.len() && bytes[j] == b'=' {
            j += 1;
            while j < bytes.len() && bytes[j].is_ascii_whitespace() {
                j += 1;
            }
            if j < bytes.len() && matches!(bytes[j], b'"' | b'\'') {
                let quote = bytes[j];
                let start = j + 1;
                let end = bytes[start..]
                    .iter()
                    .position(|&byte| byte == quote)
                    .map_or(bytes.len(), |at| start + at);
                value = &attrs[start..end];
                i = (end + 1).min(bytes.len());
            } else {
                let start = j;
                while j < bytes.len() && !bytes[j].is_ascii_whitespace() {
                    j += 1;
                }
                value = &attrs[start..j];
                i = j;
            }
        } else {
            i = j;
        }

        if name.eq_ignore_ascii_case(wanted) {
            return Some(decode_entities(value));
        }
    }
    None
}

/// Decodes the named and numeric character references that HiAnime's markup uses.
fn decode_entities(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(amp) = rest.find('&') {
        out.push_str(&rest[..amp]);
        let after = &rest[amp + 1..];
        let decoded = after
            .find(';')
            .filter(|semi| *semi <= 10)
            .and_then(|semi| decode_entity(&after[..semi]).map(|c| (c, semi)));
        match decoded {
            Some((c, semi)) => {
                out.push(c);
                rest = &after[semi + 1..];
            }
            None => {
                out.push('&');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

fn decode_entity(name: &str) -> Option<char> {
    match name {
        "amp" => Some('&'),
        "lt" => Some('<'),
        "gt" => Some('>'),
        "quot" => Some('"'),
        "apos" => Some('\''),
        "nbsp" => Some(' '),
        _ => {
            let code = match name.strip_prefix("#x").or_else(|| name.strip_prefix("#X")) {
                Some(hex) => u32::from_str_radix(hex, 16).ok()?,
                None => name.strip_prefix('#')?.parse::<u32>().ok()?,
            };
            char::from_u32(code)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn attributes_decode_entities_in_single_and_double_quotes() {
        let html = r#"<div title='Say &quot;hi&quot; &amp; go' data-x="1&amp;2">"#;
        let tag = tags(html).next().expect("one start tag");
        assert_eq!(tag.attr("title").as_deref(), Some("Say \"hi\" & go"));
        assert_eq!(tag.attr("data-x").as_deref(), Some("1&2"));
        assert_eq!(tag.attr("missing"), None);
    }

    #[test]
    fn text_strips_tags_and_collapses_whitespace() {
        assert_eq!(
            text("<span>Airing</span>\n <span>now&nbsp;</span>"),
            "Airing now"
        );
        assert_eq!(text("A &amp; B &#39;C&#39;"), "A & B 'C'");
    }

    #[test]
    fn element_end_tag_must_match_the_whole_name() {
        let html = "<a class=\"x\"><abbr>q</abbr>r</a><b>after</b>";
        let link = next_element(html, "a", |tag| tag.has_class("x")).expect("link");
        assert_eq!(link.inner, "<abbr>q</abbr>r");
        assert!(link.after.starts_with("<b>"));
    }

    #[test]
    fn scanner_skips_comments_and_script_bodies() {
        let html = concat!(
            "<!-- <a class=\"hk-ep\" href=\"/hidden\"> -->",
            "<script>var s = '<a class=\"hk-ep\" href=\"/in-script\">';</script>",
            "<a class=\"hk-ep\" href=\"/shown\">"
        );
        let link = next_element(html, "a", |tag| tag.has_class("hk-ep")).expect("link");
        assert_eq!(link.tag.attr("href").as_deref(), Some("/shown"));
    }

    #[test]
    fn void_elements_have_no_inner_markup() {
        let html = "<img src=\"/a.webp\"><p>text</p>";
        let image = next_element(html, "img", |_| true).expect("image");
        assert_eq!(image.inner, "");
        assert!(image.after.starts_with("<p>"));
    }
}
