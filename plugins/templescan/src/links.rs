//! Series and chapter addresses: building them from keys and recognising them in links.

use crate::url;

pub const ORIGIN: &str = "https://templetoons.com";
pub const HOME_URL: &str = "https://templetoons.com/";
/// Hosts whose links belong to this source: the site and its former domain, each with its
/// subdomains.
const HOSTS: [&str; 2] = ["templetoons.com", "templescan.net"];

/// What a site address points at.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Link {
    Series(String),
    Chapter { series: String, chapter: String },
}

impl Link {
    /// The series the address belongs to, whether it names the series or one of its chapters.
    pub fn series(&self) -> &str {
        match self {
            Link::Series(series) | Link::Chapter { series, .. } => series,
        }
    }
}

/// Whether `slug` can stand for one path segment of a site address. Site slugs are used exactly
/// as the site spells them, including a trailing hyphen.
pub fn valid_slug(slug: &str) -> bool {
    !slug.is_empty()
        && slug != "."
        && slug != ".."
        && !slug
            .chars()
            .any(|c| c.is_control() || matches!(c, '/' | '\\'))
}

pub fn series_url(slug: &str) -> String {
    format!("{ORIGIN}/comic/{}", url::encode(slug))
}

pub fn chapter_url(series: &str, chapter: &str) -> String {
    format!(
        "{ORIGIN}/comic/{}/{}",
        url::encode(series),
        url::encode(chapter)
    )
}

/// Recognises a series or chapter address: absolute on the site or its former domain, or the
/// root-relative form the site's own pages use. Anything else is not ours.
pub fn parse(link: &str) -> Option<Link> {
    let link = link.trim();
    let path = if link.starts_with('/') && !link.starts_with("//") {
        link.split(['?', '#']).next().unwrap_or("")
    } else {
        let parsed = url::parse_absolute(link)?;
        let default_port = if parsed.https { 443 } else { 80 };
        if parsed.port.is_some_and(|port| port != default_port) || !site_host(&parsed.host) {
            return None;
        }
        parsed.path
    };
    let mut parts: Vec<&str> = path.strip_prefix('/')?.split('/').collect();
    if parts.last() == Some(&"") {
        parts.pop();
    }
    match parts.as_slice() {
        ["comic", series] => Some(Link::Series(segment(series)?)),
        ["comic", series, chapter] => Some(Link::Chapter {
            series: segment(series)?,
            chapter: segment(chapter)?,
        }),
        _ => None,
    }
}

fn segment(raw: &str) -> Option<String> {
    url::decode(raw).filter(|slug| valid_slug(slug))
}

fn site_host(host: &str) -> bool {
    HOSTS.iter().any(|site| {
        host == *site
            || host
                .strip_suffix(site)
                .is_some_and(|rest| rest.ends_with('.'))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn series(slug: &str) -> Option<Link> {
        Some(Link::Series(slug.into()))
    }

    #[test]
    fn a_series_address_keeps_its_slug_exactly() {
        assert_eq!(
            parse("https://templetoons.com/comic/evil-husband"),
            series("evil-husband")
        );
        // Some real slugs end in a hyphen; stripping it would name another page.
        assert_eq!(
            parse("https://templetoons.com/comic/all-about-destruction-"),
            series("all-about-destruction-")
        );
        assert_eq!(
            parse("https://templetoons.com/comic/evil-husband/?utm=x#top"),
            series("evil-husband")
        );
        assert_eq!(
            parse("http://www.templetoons.com/comic/evil-husband"),
            series("evil-husband")
        );
        assert_eq!(
            parse("https://templescan.net/comic/evil-husband"),
            series("evil-husband")
        );
        assert_eq!(parse("  /comic/evil-husband?x=1 "), series("evil-husband"));
    }

    #[test]
    fn a_chapter_address_names_its_series_and_chapter() {
        let link = parse("https://templetoons.com/comic/evil-husband/chapter-51").expect("chapter");
        assert_eq!(
            link,
            Link::Chapter {
                series: "evil-husband".into(),
                chapter: "chapter-51".into()
            }
        );
        assert_eq!(link.series(), "evil-husband");
        // Numeric-prefixed and bare-number chapter slugs exist on the site.
        assert!(matches!(
            parse("/comic/evil-husband/95958-chapter-1"),
            Some(Link::Chapter { chapter, .. }) if chapter == "95958-chapter-1"
        ));
        assert!(matches!(
            parse("/comic/evil-husband/103/"),
            Some(Link::Chapter { chapter, .. }) if chapter == "103"
        ));
    }

    #[test]
    fn foreign_hosts_and_other_pages_are_not_ours() {
        for link in [
            "https://templetoons.com.evil.example/comic/a",
            "https://eviltempletoons.com/comic/a",
            "https://templetoons.com@evil.example/comic/a",
            "https://templetoons.com:8443/comic/a",
            "https://example.com/comic/a",
            "https://templetoons.com/",
            "https://templetoons.com/comics",
            "https://templetoons.com/comics?page=2",
            "https://templetoons.com/novel/a",
            "https://templetoons.com/Comic/a",
            "https://templetoons.com/comic/",
            "https://templetoons.com/comic/a/b/c",
            "https://templetoons.com/comic/../comics",
            "https://templetoons.com/comic/a/%2E%2E",
            "https://templetoons.com/comic/a%2Fb",
            "https://templetoons.com/comic/a%zz",
            "//templetoons.com/comic/a",
            "ftp://templetoons.com/comic/a",
            "javascript:alert(1)",
            "",
        ] {
            assert_eq!(parse(link), None, "{link}");
        }
    }

    #[test]
    fn urls_built_from_keys_round_trip_through_parse() {
        for slug in ["evil-husband", "tail-", "a b", "한글", "x%y", "q?r#s"] {
            assert_eq!(parse(&series_url(slug)), series(slug), "{slug}");
            assert_eq!(
                parse(&chapter_url(slug, "chapter-1")),
                Some(Link::Chapter {
                    series: slug.into(),
                    chapter: "chapter-1".into()
                })
            );
        }
        assert_eq!(
            series_url("a b/c"),
            "https://templetoons.com/comic/a%20b%2Fc"
        );
    }

    #[test]
    fn only_segment_safe_slugs_are_valid() {
        assert!(valid_slug("evil-husband"));
        assert!(valid_slug("tail-"));
        for slug in ["", ".", "..", "a/b", "a\\b", "a\nb"] {
            assert!(!valid_slug(slug), "{slug:?}");
        }
    }
}
