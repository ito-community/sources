//! Minimal absolute-URL parsing and percent-coding, enough to judge which site and path an
//! address names. It never normalises: callers compare what the address actually says.

/// An absolute `http` or `https` address split into the parts the plugin checks.
#[derive(Debug, PartialEq, Eq)]
pub struct Parsed<'a> {
    pub https: bool,
    /// Lower-cased host name without port.
    pub host: String,
    pub port: Option<u16>,
    /// The path including its leading slash; empty when the address has none.
    pub path: &'a str,
    pub query: Option<&'a str>,
}

/// Splits an absolute `http(s)` address. User information, IPv6 literals, other schemes and
/// malformed hosts or ports are refused, so the host that is returned is the host contacted.
pub fn parse_absolute(url: &str) -> Option<Parsed<'_>> {
    let url = url.trim();
    let (https, rest) = match strip_prefix_ignore_case(url, "https://") {
        Some(rest) => (true, rest),
        None => (false, strip_prefix_ignore_case(url, "http://")?),
    };
    let authority_end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let (authority, tail) = rest.split_at(authority_end);
    if authority.is_empty() || authority.contains(['@', '[', ']', '\\']) {
        return None;
    }
    let (host, port) = match authority.split_once(':') {
        Some((host, port)) => (host, Some(port.parse::<u16>().ok()?)),
        None => (authority, None),
    };
    if !valid_host(host) {
        return None;
    }
    let tail = tail.split('#').next().unwrap_or("");
    let (path, query) = match tail.split_once('?') {
        Some((path, query)) => (path, Some(query)),
        None => (tail, None),
    };
    Some(Parsed {
        https,
        host: host.to_ascii_lowercase(),
        port,
        path,
        query,
    })
}

fn valid_host(host: &str) -> bool {
    !host.is_empty()
        && host.split('.').all(|label| {
            !label.is_empty()
                && label
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        })
}

fn strip_prefix_ignore_case<'a>(text: &'a str, prefix: &str) -> Option<&'a str> {
    let head = text.get(..prefix.len())?;
    head.eq_ignore_ascii_case(prefix)
        .then(|| &text[prefix.len()..])
}

/// Percent-encodes every byte outside the RFC 3986 unreserved set.
pub fn encode(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for byte in text.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            out.push(char::from(byte));
        } else {
            out.push('%');
            out.push(char::from(HEX[usize::from(byte >> 4)]));
            out.push(char::from(HEX[usize::from(byte & 15)]));
        }
    }
    out
}

const HEX: &[u8; 16] = b"0123456789ABCDEF";

/// Decodes percent escapes; `None` for a truncated or non-hex escape or a result that is not
/// UTF-8.
pub fn decode(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            let high = hex_value(*bytes.get(index + 1)?)?;
            let low = hex_value(*bytes.get(index + 2)?)?;
            out.push(high << 4 | low);
            index += 3;
        } else {
            out.push(bytes[index]);
            index += 1;
        }
    }
    String::from_utf8(out).ok()
}

fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_contacted_host_is_the_host_reported() {
        let parsed = parse_absolute("HTTPS://Templetoons.COM:443/comic/a?x=1#top").expect("valid");
        assert!(parsed.https);
        assert_eq!(parsed.host, "templetoons.com");
        assert_eq!(parsed.port, Some(443));
        assert_eq!(parsed.path, "/comic/a");
        assert_eq!(parsed.query, Some("x=1"));
        // Credentials would make the real host differ from the visible one.
        assert_eq!(
            parse_absolute("https://templetoons.com@evil.example/comic/a"),
            None
        );
        assert_eq!(
            parse_absolute("https://evil.example\\@templetoons.com/"),
            None
        );
        assert_eq!(parse_absolute("https://[::1]/comic/a"), None);
        assert_eq!(parse_absolute("https://templetoons.com:99999/"), None);
        assert_eq!(parse_absolute("https://templetoons.com:abc/"), None);
        assert_eq!(parse_absolute("ftp://templetoons.com/comic/a"), None);
        assert_eq!(parse_absolute("https:///comic/a"), None);
        assert_eq!(parse_absolute("https://a..b/"), None);
        assert_eq!(parse_absolute("//templetoons.com/comic/a"), None);
    }

    #[test]
    fn an_address_without_a_path_has_an_empty_one() {
        let parsed = parse_absolute("https://templetoons.com").expect("valid");
        assert_eq!(parsed.path, "");
        let parsed = parse_absolute("https://templetoons.com?x=1").expect("valid");
        assert_eq!((parsed.path, parsed.query), ("", Some("x=1")));
    }

    #[test]
    fn coding_round_trips_and_refuses_broken_escapes() {
        for text in ["plain-slug_1.~", "a b/c?d#e%f", "한글 제목", "tail-"] {
            assert_eq!(decode(&encode(text)).as_deref(), Some(text));
        }
        assert_eq!(encode("a b/한"), "a%20b%2F%ED%95%9C");
        assert_eq!(decode("a%2"), None);
        assert_eq!(decode("a%zz"), None);
        assert_eq!(decode("%ff"), None, "invalid UTF-8 is not text");
        assert_eq!(
            decode("a+b").as_deref(),
            Some("a+b"),
            "plus is not a space in a path"
        );
    }
}
