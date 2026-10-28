//! Site URLs: series and chapter links, and image locations.

use crate::query::encode;

pub const SITE: &str = "https://atsu.moe";
const CDN: &str = "https://cdn.atsu.moe";

pub fn series_url(id: &str, novel: bool) -> String {
    let kind = if novel { "novel" } else { "manga" };
    format!("{SITE}/{kind}/{}", encode(id))
}

pub fn chapter_url(series: &str, chapter: &str, novel: bool) -> String {
    let kind = if novel { "novel" } else { "read" };
    format!("{SITE}/{kind}/{}/{}", encode(series), encode(chapter))
}

/// The series id of a site link: `/manga/<id>`, `/novel/<id>`, `/novel/<id>/<chapter>`,
/// `/read/<id>/<chapter>` or the older `/read/<id>?chapterId=<chapter>`.
pub fn series_id(url: &str) -> Option<String> {
    let rest = strip_scheme(url.trim())?;
    let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let host = rest[..end].to_ascii_lowercase();
    if host != "atsu.moe" && host != "www.atsu.moe" {
        return None;
    }
    let path = rest[end..].split(['?', '#']).next().unwrap_or_default();
    let segments: Vec<&str> = path.split('/').filter(|part| !part.is_empty()).collect();
    let id = match segments.as_slice() {
        ["manga", id] | ["novel", id] | ["novel", id, _] | ["read", id] | ["read", id, _] => *id,
        _ => return None,
    };
    let valid = id
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_');
    valid.then(|| id.to_owned())
}

/// Resolves an image path from the API. Absolute URLs must point at the site.
pub fn image_url(path: &str) -> Option<String> {
    if let Some(rest) = strip_scheme(path) {
        let host = rest[..rest.find(['/', '?', '#']).unwrap_or(rest.len())].to_ascii_lowercase();
        let on_site = !host.contains('@') && (host == "atsu.moe" || host.ends_with(".atsu.moe"));
        return on_site.then(|| format!("https://{rest}"));
    }
    if path.is_empty()
        || path.contains("://")
        || path.contains('\\')
        || path.split('/').any(|part| part == "..")
    {
        return None;
    }
    Some(if path.starts_with("/static/") {
        format!("{CDN}{path}")
    } else if let Some(stripped) = path.strip_prefix('/') {
        format!("{CDN}/{stripped}")
    } else if path.starts_with("static/") {
        format!("{CDN}/{path}")
    } else {
        format!("{CDN}/static/{path}")
    })
}

fn strip_scheme(url: &str) -> Option<&str> {
    let (scheme, rest) = url.split_once("://")?;
    (scheme.eq_ignore_ascii_case("https") || scheme.eq_ignore_ascii_case("http")).then_some(rest)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn series_links_accept_every_site_route() {
        for url in [
            "https://atsu.moe/manga/Ab_1",
            "https://www.atsu.moe/novel/Ab_1/",
            "http://ATSU.moe/novel/Ab_1/ch9",
            "https://atsu.moe/read/Ab_1/ch9#page=2",
            "https://atsu.moe/read/Ab_1?chapterId=ch9",
        ] {
            assert_eq!(series_id(url).as_deref(), Some("Ab_1"), "{url}");
        }
    }

    #[test]
    fn foreign_or_malformed_links_are_rejected() {
        for url in [
            "https://atsu.moe.example.com/manga/x",
            "https://example.com/manga/x",
            "https://user@atsu.moe/manga/x",
            "ftp://atsu.moe/manga/x",
            "https://atsu.moe/manga/x%2F..",
            "https://atsu.moe/user/x",
            "atsu.moe/manga/x",
        ] {
            assert_eq!(series_id(url), None, "{url}");
        }
    }

    #[test]
    fn image_paths_resolve_on_the_cdn_only() {
        assert_eq!(
            image_url("/static/p/a.webp").as_deref(),
            Some("https://cdn.atsu.moe/static/p/a.webp")
        );
        assert_eq!(
            image_url("posters/a.webp").as_deref(),
            Some("https://cdn.atsu.moe/static/posters/a.webp")
        );
        assert_eq!(
            image_url("/p/a.webp").as_deref(),
            Some("https://cdn.atsu.moe/p/a.webp")
        );
        assert_eq!(
            image_url("https://cdn.atsu.moe/a.webp").as_deref(),
            Some("https://cdn.atsu.moe/a.webp")
        );
        assert_eq!(
            image_url("http://CDN.atsu.moe/a.webp").as_deref(),
            Some("https://CDN.atsu.moe/a.webp")
        );
        for path in [
            "https://example.com/a.webp",
            "file:///etc/hosts",
            "../a.webp",
            "a\\b",
            "",
            "https://cdn.atsu.moe.evil/a",
        ] {
            assert_eq!(image_url(path), None, "{path}");
        }
    }

    #[test]
    fn generated_links_round_trip() {
        assert_eq!(
            series_id(&series_url("Ab_1", true)).as_deref(),
            Some("Ab_1")
        );
        assert_eq!(
            series_id(&chapter_url("Ab_1", "c 1", false)).as_deref(),
            Some("Ab_1")
        );
        assert_eq!(
            chapter_url("Ab_1", "c 1", false),
            "https://atsu.moe/read/Ab_1/c%201"
        );
    }
}
