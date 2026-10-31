//! Parses data literals in the site's streamed route payload without executing JavaScript.
use ito_rs::{Error, Result};
use serde_json::{Map, Value};

/// Returns the first value bound to `name` at a key boundary that parses.
pub fn field(html: &str, name: &str) -> Result<Value> {
    objects_with(html, name)
        .into_iter()
        .find_map(|mut object| object.remove(name))
        .ok_or_else(|| Error::Host(format!("Vortex route data is missing {name}")))
}

/// Returns each object that binds `name` at a key boundary, in payload order.
///
/// A key boundary is preceded by `{` or `,`, so a longer key such as `prepost:` never matches.
/// The whole enclosing object is returned, whatever the order of its keys, so sibling fields
/// such as image lists are visible next to the matched key. Values this parser cannot read are
/// skipped without discarding the rest of the object.
pub fn objects_with(html: &str, name: &str) -> Vec<Map<String, Value>> {
    let marker = format!("{name}:");
    let mut seen = std::collections::HashSet::new();
    html.match_indices(&marker)
        .filter(|&(at, _)| at > 0 && matches!(html.as_bytes()[at - 1], b'{' | b','))
        .filter_map(|(at, _)| {
            // Without an enclosing object, the key run that starts at the match is read.
            let (start, braced) = match enclosing_object(html, at) {
                Some(start) => (start, true),
                None => (at, false),
            };
            seen.insert(start).then_some((start, braced))
        })
        .filter_map(|(start, braced)| {
            let mut parser = Parser {
                text: &html[start..],
                whole: html,
                depth: 0,
            };
            parser.tolerant_object(braced).ok()
        })
        .filter(|object| object.contains_key(name))
        .collect()
}

/// Byte offset of the innermost `{` that encloses `at`, found by scanning backwards.
fn enclosing_object(html: &str, at: usize) -> Option<usize> {
    let mut depth = 0usize;
    for (index, c) in html[..at].char_indices().rev() {
        match c {
            '}' => depth += 1,
            '{' if depth == 0 => return Some(index),
            '{' => depth -= 1,
            _ => {}
        }
    }
    None
}

struct Parser<'a> {
    text: &'a str,
    whole: &'a str,
    depth: usize,
}
impl Parser<'_> {
    fn skip(&mut self) {
        self.text = self.text.trim_start();
    }
    fn error() -> Error {
        Error::Host("Vortex sent malformed route data".into())
    }
    fn string(&mut self) -> Result<String> {
        self.text = &self.text[1..];
        let mut out = String::new();
        loop {
            let c = self.text.chars().next().ok_or_else(Self::error)?;
            self.text = &self.text[c.len_utf8()..];
            match c {
                '"' => return Ok(out),
                '\\' => {
                    let e = self.text.chars().next().ok_or_else(Self::error)?;
                    self.text = &self.text[e.len_utf8()..];
                    match e {
                        'x' | 'u' => {
                            let n = if e == 'x' { 2 } else { 4 };
                            let hex = self.text.get(..n).ok_or_else(Self::error)?;
                            let code = u32::from_str_radix(hex, 16).map_err(|_| Self::error())?;
                            out.push(char::from_u32(code).ok_or_else(Self::error)?);
                            self.text = &self.text[n..];
                        }
                        'n' => out.push('\n'),
                        'r' => out.push('\r'),
                        't' => out.push('\t'),
                        'b' => out.push('\u{8}'),
                        'f' => out.push('\u{c}'),
                        other => out.push(other),
                    }
                }
                other => out.push(other),
            }
        }
    }
    /// Reads an object key: a quoted string or an identifier running to its colon.
    fn key(&mut self) -> Result<String> {
        self.skip();
        if self.text.starts_with('"') {
            return self.string();
        }
        let end = self.text.find(':').ok_or_else(Self::error)?;
        let key = self.text[..end].trim().to_owned();
        self.text = &self.text[end..];
        Ok(key)
    }
    /// Advances past one value without interpreting it, stopping before the `,` or closing
    /// bracket that ends it in the enclosing container.
    fn skip_value(&mut self) {
        let mut depth = 0usize;
        let mut in_string = false;
        let mut escaped = false;
        for (index, c) in self.text.char_indices() {
            if in_string {
                if escaped {
                    escaped = false;
                } else if c == '\\' {
                    escaped = true;
                } else if c == '"' {
                    in_string = false;
                }
                continue;
            }
            match c {
                '"' => in_string = true,
                '{' | '[' | '(' => depth += 1,
                '}' | ']' | ')' if depth == 0 => {
                    self.text = &self.text[index..];
                    return;
                }
                '}' | ']' | ')' => depth -= 1,
                ',' if depth == 0 => {
                    self.text = &self.text[index..];
                    return;
                }
                _ => {}
            }
        }
        self.text = "";
    }
    /// Parses keys in any order. A braced object starts at `{` and ends at `}`. An unbraced run
    /// is the keys that follow a top-level match, ending at the first non-separator. A value that
    /// cannot be read is skipped, so unsupported syntax does not hide the keys that can be read.
    fn tolerant_object(&mut self, braced: bool) -> Result<Map<String, Value>> {
        let mut object = Map::new();
        if braced {
            self.skip();
            self.text = self.text.strip_prefix('{').ok_or_else(Self::error)?;
        }
        loop {
            self.skip();
            if braced {
                if let Some(rest) = self.text.strip_prefix('}') {
                    self.text = rest;
                    return Ok(object);
                }
            } else if self.text.is_empty() {
                return Ok(object);
            }
            let key = self.key()?;
            self.skip();
            self.text = self.text.strip_prefix(':').ok_or_else(Self::error)?;
            let saved = self.text;
            match self.value() {
                Ok(value) => {
                    object.insert(key, value);
                }
                Err(_) => {
                    self.text = saved;
                    self.skip_value();
                }
            }
            self.skip();
            if let Some(rest) = self.text.strip_prefix(',') {
                self.text = rest;
            } else if braced && !self.text.starts_with('}') {
                return Err(Self::error());
            } else if !braced {
                return Ok(object);
            }
        }
    }
    fn value(&mut self) -> Result<Value> {
        self.skip();
        if self.depth > 64 {
            return Err(Self::error());
        }
        if self.text.starts_with("$R[") {
            let end = self.text.find(']').ok_or_else(Self::error)? + 1;
            let reference = &self.text[..end];
            self.text = &self.text[end..];
            if self.text.starts_with('=') {
                self.text = &self.text[1..];
                return self.value();
            }
            let marker = format!("{reference}=");
            let Some(at) = self.whole.find(&marker) else {
                return Ok(Value::Null);
            };
            return Parser {
                text: &self.whole[at + marker.len()..],
                whole: self.whole,
                depth: self.depth + 1,
            }
            .value();
        }
        if self.text.starts_with('"') {
            return self.string().map(Value::String);
        }
        if self.text.starts_with('{') {
            self.text = &self.text[1..];
            let mut object = Map::new();
            loop {
                self.skip();
                if self.text.starts_with('}') {
                    self.text = &self.text[1..];
                    return Ok(Value::Object(object));
                }
                let key = self.key()?;
                self.skip();
                if !self.text.starts_with(':') {
                    return Err(Self::error());
                }
                self.text = &self.text[1..];
                let value = self.value()?;
                object.insert(key, value);
                self.skip();
                if self.text.starts_with(',') {
                    self.text = &self.text[1..];
                } else if !self.text.starts_with('}') {
                    return Err(Self::error());
                }
            }
        }
        if self.text.starts_with('[') {
            self.text = &self.text[1..];
            let mut list = Vec::new();
            loop {
                self.skip();
                if self.text.starts_with(']') {
                    self.text = &self.text[1..];
                    return Ok(Value::Array(list));
                }
                list.push(self.value()?);
                self.skip();
                if self.text.starts_with(',') {
                    self.text = &self.text[1..];
                } else if !self.text.starts_with(']') {
                    return Err(Self::error());
                }
            }
        }
        if self.text.starts_with("new Date(") {
            self.text = &self.text[9..];
            let value = self.value()?;
            self.skip();
            if !self.text.starts_with(')') {
                return Err(Self::error());
            }
            self.text = &self.text[1..];
            return Ok(value);
        }
        for (token, value) in [
            ("!0", Value::Bool(true)),
            ("!1", Value::Bool(false)),
            ("true", Value::Bool(true)),
            ("false", Value::Bool(false)),
            ("null", Value::Null),
            ("void 0", Value::Null),
        ] {
            if self.text.starts_with(token) {
                self.text = &self.text[token.len()..];
                return Ok(value);
            }
        }
        let end = self
            .text
            .find(|c: char| !c.is_ascii_digit() && !matches!(c, '-' | '+' | '.' | 'e' | 'E'))
            .unwrap_or(self.text.len());
        if end == 0 {
            return Err(Self::error());
        }
        let number = serde_json::from_str(&self.text[..end]).map_err(|_| Self::error())?;
        self.text = &self.text[end..];
        Ok(number)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn resolves_references_and_escaped_html() {
        let html = r#"$R[2]={name:"Action"},post:$R[3]={id:7,postContent:"\x3Cp>Hi\x3C/p>",genres:$R[4]=[$R[2]],locked:!1,number:0.5}"#;
        let data = field(html, "post").expect("data");
        assert_eq!(data["genres"][0]["name"], "Action");
        assert_eq!(data["postContent"], "<p>Hi</p>");
        assert_eq!(data["locked"], false);
    }
    #[test]
    fn malformed_and_missing_data_fail() {
        assert!(field("{post:{id:", "post").is_err());
        assert!(field("<html>verification</html>", "post").is_err());
    }
    #[test]
    fn key_matches_only_at_a_key_boundary() {
        assert!(field("{prepost:{id:1}}", "post").is_err());
        assert!(field("text post:{id:1}", "post").is_err());
        assert_eq!(field("{post:{id:1}}", "post").expect("data")["id"], 1);
    }
    #[test]
    fn nested_chapter_objects_are_listed_with_their_enclosing_object() {
        let html = r#"{post:{chapters:[{chapter:{slug:"chapter-0",isLocked:!1}}]}},{chapter:{slug:"chapter-1",isLocked:!1},publicImages:[{url:"synthetic.png",order:0}]}"#;
        let objects = objects_with(html, "chapter");
        assert_eq!(objects.len(), 2);
        assert_eq!(objects[0]["chapter"]["slug"], "chapter-0");
        assert!(!objects[0].contains_key("publicImages"));
        assert_eq!(objects[1]["chapter"]["slug"], "chapter-1");
        assert_eq!(objects[1]["publicImages"][0]["url"], "synthetic.png");
    }
    #[test]
    fn image_keys_before_the_chapter_key_are_read_from_the_same_object() {
        let html = r#"{readerImages:[{displaySrc:"https://storage.vortexscans.org/intro.png"}],publicImages:[{url:"https://storage.vortexscans.org/a.png",order:0}],chapter:{slug:"chapter-1",isLocked:!1}}"#;
        let objects = objects_with(html, "chapter");
        assert_eq!(objects.len(), 1);
        assert_eq!(objects[0]["chapter"]["slug"], "chapter-1");
        assert_eq!(
            objects[0]["readerImages"][0]["displaySrc"],
            "https://storage.vortexscans.org/intro.png"
        );
        assert_eq!(objects[0]["publicImages"][0]["order"], 0);
    }
    #[test]
    fn unreadable_sibling_values_do_not_hide_the_object() {
        let html = r#"{chapter:{slug:"chapter-1",isLocked:!1},helper:function(){return {a:1};},readerImages:[{displaySrc:"https://storage.vortexscans.org/a.png"}]}"#;
        let objects = objects_with(html, "chapter");
        assert_eq!(objects.len(), 1);
        assert_eq!(
            objects[0]["readerImages"][0]["displaySrc"],
            "https://storage.vortexscans.org/a.png"
        );
        assert!(!objects[0].contains_key("helper"));
    }
}
