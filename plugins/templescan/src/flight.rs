//! The data a Next.js page ships to its own scripts.
//!
//! The site's pages are rendered by the Next.js app router, which embeds the props of every
//! component as `self.__next_f.push([1, "<chunk>"])` calls in inline scripts. The chunks join
//! into rows of the form `<id>:<json>`. Reading them gives the same data the page renders from,
//! including what the markup only shows part of: the whole catalog and a chapter's pages.
//!
//! The site renames some data fields to short meaningless tokens inside these rows, and the
//! tokens differ between builds. Callers therefore find objects by the fields that are not
//! renamed and recognise the renamed ones by their values; see [`Classified`].

use crate::dom::Query;
use ito_rs::Result;
use scraper::Html;
use serde_json::{Map, Value};

const PUSH: &str = "self.__next_f.push(";

/// The decoded rows of one page.
pub struct Flight {
    rows: Vec<Value>,
}

impl Flight {
    pub fn from_document(doc: &Html) -> Result<Self> {
        let mut payload = String::new();
        for script in doc.query("script")? {
            let code: String = script.text().collect();
            append_chunks(&code, &mut payload);
        }
        Ok(Flight {
            rows: rows(&payload),
        })
    }

    /// The first object, in document order and depth first, that has every one of `keys`.
    pub fn find(&self, keys: &[&str]) -> Option<&Map<String, Value>> {
        self.rows.iter().find_map(|row| search(row, keys))
    }
}

/// Adds the text chunks of every `self.__next_f.push(...)` call in `script`. Chunks that are not
/// text (the bootstrap calls pass numbers and nulls) carry no row data.
fn append_chunks(script: &str, payload: &mut String) {
    let mut rest = script;
    while let Some(at) = rest.find(PUSH) {
        let arguments = &rest[at + PUSH.len()..];
        let mut values = serde_json::Deserializer::from_str(arguments).into_iter::<Value>();
        match values.next() {
            Some(Ok(Value::Array(items))) => {
                if let [Value::Number(kind), Value::String(chunk), ..] = items.as_slice()
                    && kind.as_u64() == Some(1)
                {
                    payload.push_str(chunk);
                }
                rest = &arguments[values.byte_offset()..];
            }
            _ => rest = arguments,
        }
    }
}

/// Splits the payload into the JSON values of its rows.
///
/// A row is `<hex id>:<value>`. Most rows end at a line break, but a text row is
/// `<id>:T<hex byte length>,<that many bytes>` and the next row follows its last byte directly,
/// so line breaks alone cannot separate rows: a page's data may come right after a text row
/// (the page's structured-data notice is one) on the same line. Module references and hints
/// have no JSON value; anything malformed is skipped without hiding the rows after it.
fn rows(payload: &str) -> Vec<Value> {
    let mut values = Vec::new();
    let mut rest = payload;
    while !rest.is_empty() {
        match next_row(rest) {
            Some((value, after)) => {
                values.extend(value);
                rest = after;
            }
            None => rest = skip_line(rest),
        }
    }
    values
}

/// The row at the start of `text`: its JSON value, if it has one, and what follows the row.
fn next_row(text: &str) -> Option<(Option<Value>, &str)> {
    let (id, body) = text.split_once(':')?;
    if id.is_empty() || !id.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return None;
    }
    match body.chars().next()? {
        'T' => {
            let (length, content) = body[1..].split_once(',')?;
            let length = usize::from_str_radix(length, 16).ok()?;
            Some((None, content.get(length..)?))
        }
        tag if tag.is_ascii_uppercase() => Some((None, skip_line(body))),
        _ => {
            let mut stream = serde_json::Deserializer::from_str(body).into_iter::<Value>();
            let value = stream.next()?.ok()?;
            let after = &body[stream.byte_offset()..];
            Some((Some(value), after.strip_prefix('\n').unwrap_or(after)))
        }
    }
}

fn skip_line(text: &str) -> &str {
    text.find('\n').map_or("", |at| &text[at + 1..])
}

fn search<'a>(value: &'a Value, keys: &[&str]) -> Option<&'a Map<String, Value>> {
    match value {
        Value::Object(map) => {
            if keys.iter().all(|key| map.contains_key(*key)) {
                return Some(map);
            }
            map.values().find_map(|child| search(child, keys))
        }
        Value::Array(items) => items.iter().find_map(|child| search(child, keys)),
        _ => None,
    }
}

/// Whether `text` could be a URL slug as the site writes them: no whitespace or capitals.
pub fn slug_like(text: &str) -> bool {
    !text.is_empty()
        && !text
            .chars()
            .any(|c| c.is_whitespace() || c.is_uppercase() || c == '/')
}

/// The renamed fields of a list of records, told apart by what they hold.
///
/// Every record has the same fields. After removing the ones the caller knows by name, the
/// renamed ones remain: a string that is a slug in nearly every record, a string that mostly is
/// not, and optionally a list and a number. Ambiguity is refused rather than guessed.
pub struct Classified {
    pub slug: String,
    pub text: String,
}

impl Classified {
    /// `known` names the fields that are not renamed; `records` must all be objects.
    pub fn of(records: &[&Map<String, Value>], known: &[&str]) -> Option<Classified> {
        let first = records.first()?;
        let mut strings = Vec::new();
        let mut lists = 0_usize;
        let mut numbers = 0_usize;
        for key in first.keys().filter(|key| !known.contains(&key.as_str())) {
            let mut values = records.iter().map(|record| record.get(key));
            if values
                .clone()
                .all(|value| matches!(value, Some(Value::Array(_))))
            {
                lists += 1;
                if lists > 1 {
                    return None;
                }
            } else if values
                .clone()
                .all(|value| matches!(value, Some(Value::Number(_))))
            {
                numbers += 1;
                if numbers > 1 {
                    return None;
                }
            } else if values.all(|value| matches!(value, Some(Value::String(_)))) {
                strings.push(key.clone());
            } else {
                return None;
            }
        }
        let [first_key, second_key] = strings.as_slice() else {
            return None;
        };
        let slugs = |key: &String| {
            records
                .iter()
                .filter(|record| {
                    record
                        .get(key)
                        .and_then(Value::as_str)
                        .is_some_and(slug_like)
                })
                .count()
        };
        let (first_slugs, second_slugs) = (slugs(first_key), slugs(second_key));
        let (slug, text, most, least) = if first_slugs > second_slugs {
            (first_key, second_key, first_slugs, second_slugs)
        } else if second_slugs > first_slugs {
            (second_key, first_key, second_slugs, first_slugs)
        } else {
            return None;
        };
        // A few odd records are tolerated; a field most records fail is not a slug, and a text
        // field that mostly looks like slugs cannot be told from one.
        if most * 4 < records.len() * 3 || least * 2 > records.len() {
            return None;
        }
        Some(Classified {
            slug: slug.clone(),
            text: text.clone(),
        })
    }
}

#[cfg(test)]
pub fn script(chunks: &[&str]) -> String {
    // One inline script per chunk, as the site emits them, with the attributes a dialog library
    // adds to scripts while a modal is open.
    chunks
        .iter()
        .map(|chunk| {
            let literal = serde_json::to_string(chunk).expect("chunk serialises");
            format!("<script data-aria-hidden=\"true\">self.__next_f.push([1,{literal}])</script>")
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dom;

    fn flight(html: &str) -> Flight {
        Flight::from_document(&dom::parse(html)).expect("flight")
    }

    #[test]
    fn rows_are_rebuilt_from_chunks_split_anywhere() {
        let html = format!(
            "<html><body><script>(self.__next_f=self.__next_f||[]).push([0]);self.__next_f.push([2,null])</script>{}</body></html>",
            script(&[
                "1:HL[\"/_next/x.woff2\",\"font\"]\na:I[1,[],\"\"]\n2:[\"$\",\"div\",null,{\"props\":{\"lis",
                "t\":[1,2,3],\"initialPage\":1}}]\n3:T5,plain\n4:{\"other\":true}\n",
            ])
        );
        let flight = flight(&html);
        let found = flight.find(&["list", "initialPage"]).expect("props");
        assert_eq!(found["list"], serde_json::json!([1, 2, 3]));
        assert_eq!(
            flight.find(&["other"]).map(|m| m["other"].clone()),
            Some(Value::Bool(true))
        );
        assert!(flight.find(&["missing"]).is_none());
    }

    #[test]
    fn several_pushes_in_one_script_and_malformed_ones_are_tolerated() {
        let html = "<script>self.__next_f.push([1,\"1:{\\\"a\\\":1}\\n\"]);self.__next_f.push(oops);self.__next_f.push([1,\"2:{\\\"b\\\":2}\\n\"])</script>";
        let flight = flight(html);
        assert!(flight.find(&["a"]).is_some());
        assert!(flight.find(&["b"]).is_some());
    }

    #[test]
    fn rows_that_are_not_json_are_skipped_without_hiding_the_rest() {
        let html = format!(
            "<body>{}</body>",
            script(&["zz:{\"no\":1}\n5:[broken\n6:{\"yes\":1}\n:{\"bare\":1}\n"])
        );
        let flight = flight(&html);
        assert!(flight.find(&["yes"]).is_some());
        assert!(flight.find(&["no"]).is_none(), "ids are hexadecimal");
        assert!(flight.find(&["bare"]).is_none());
    }

    fn records(json: &str) -> Vec<Map<String, Value>> {
        match serde_json::from_str::<Value>(json).expect("json") {
            Value::Array(items) => items
                .into_iter()
                .map(|item| match item {
                    Value::Object(map) => map,
                    _ => panic!("object expected"),
                })
                .collect(),
            _ => panic!("array expected"),
        }
    }

    #[test]
    fn renamed_fields_are_recognised_whatever_their_tokens() {
        for (title, slug, chapters, price) in [
            ("aaaaaaa", "bbbbbbb", "ccccccc", "ddddddd"),
            ("zzzzzzz", "yyyyyyy", "xxxxxxx", "wwwwwww"),
        ] {
            let json = format!(
                r#"[{{"{title}":"Evil Husband","{slug}":"evil-husband","thumbnail":"t","{chapters}":[],"{price}":0}},
                    {{"{title}":"orchid","{slug}":"until-he-becomes-an-orchid","thumbnail":"t","{chapters}":[{{}}],"{price}":10}}]"#
            );
            let records = records(&json);
            let refs: Vec<_> = records.iter().collect();
            let found = Classified::of(&refs, &["thumbnail"]).expect("classified");
            assert_eq!(found.text, title);
            assert_eq!(found.slug, slug);
        }
    }

    #[test]
    fn ambiguous_or_inconsistent_records_are_refused() {
        // Two slug-like strings cannot be told apart.
        let both = records(r#"[{"a":"one","b":"two"},{"a":"three","b":"four"}]"#);
        assert!(Classified::of(&both.iter().collect::<Vec<_>>(), &[]).is_none());
        // Neither is slug-like.
        let neither = records(r#"[{"a":"One Two","b":"Three Four"}]"#);
        assert!(Classified::of(&neither.iter().collect::<Vec<_>>(), &[]).is_none());
        // A field that changes type between records cannot be classified.
        let mixed = records(r#"[{"a":"A B","b":"x-y","c":1},{"a":"C D","b":"z-w","c":"s"}]"#);
        assert!(Classified::of(&mixed.iter().collect::<Vec<_>>(), &[]).is_none());
        // Two lists are ambiguous too.
        let lists = records(r#"[{"a":"A B","b":"x-y","c":[],"d":[]}]"#);
        assert!(Classified::of(&lists.iter().collect::<Vec<_>>(), &[]).is_none());
        assert!(Classified::of(&[], &[]).is_none());
    }

    #[test]
    fn slugs_have_no_whitespace_or_capitals() {
        assert!(slug_like("evil-husband"));
        assert!(slug_like("tail-"));
        assert!(slug_like("95958-chapter-1"));
        assert!(!slug_like(""));
        assert!(!slug_like("Evil Husband"));
        assert!(!slug_like("Prologue"));
        assert!(!slug_like("chapter 58"));
    }
}
