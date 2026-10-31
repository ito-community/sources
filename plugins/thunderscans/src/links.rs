//! Canonical Thunder Scans URL identity helpers.

use url::Url;

const ORIGIN: &str = "https://en-thunderscans.com";
const HOSTS: &[&str] = &["en-thunderscans.com", "www.en-thunderscans.com"];

/// The part of a URL or path before any query or fragment. Dot segments are checked here, so a
/// query value containing dots is not treated as traversal.
fn route(url: &str) -> &str {
    url.split(['?', '#']).next().unwrap_or_default()
}

/// Returns the site path of an absolute `http(s)` URL on a Thunder Scans host.
///
/// Scheme and host case and a `www.` prefix are accepted. Userinfo, explicit ports
/// (including default ports), and dot segments in the path are rejected.
pub(crate) fn canonical_path(url: &str) -> Option<String> {
    if route(url).contains("..") {
        return None;
    }
    let authority = url.split_once("://")?.1.split(['/', '?', '#']).next()?;
    if authority.contains([':', '@']) {
        return None;
    }
    let parsed = Url::parse(url).ok()?;
    if !matches!(parsed.scheme(), "https" | "http")
        || !matches!(parsed.host_str(), Some(host) if HOSTS.contains(&host))
    {
        return None;
    }
    let path = parsed.path();
    (path.starts_with('/') && !path.starts_with("//")).then(|| path.to_owned())
}

pub(crate) fn series_path(url: &str) -> Option<String> {
    let path = canonical_path(url)?;
    let slug = path.strip_prefix("/comics/")?.strip_suffix('/')?;
    if slug.is_empty() || slug.contains('/') {
        return None;
    }
    Some(path)
}

pub(crate) fn chapter_path(url: &str) -> Option<String> {
    let path = canonical_path(url)?;
    let slug = path.strip_prefix('/')?.strip_suffix('/')?;
    if slug.is_empty() || !slug.contains("-chapter-") {
        return None;
    }
    Some(path)
}

/// Resolves a site-relative path (keeping its query, for search) or an absolute site URL.
pub(crate) fn absolute_url(path: &str) -> Option<String> {
    if path.starts_with('/') {
        return (!path.starts_with("//") && !route(path).contains(".."))
            .then(|| format!("{ORIGIN}{path}"));
    }
    canonical_path(path).map(|path| format!("{ORIGIN}{path}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn series_identity_ignores_tracking_query_but_not_other_hosts() {
        assert_eq!(
            series_path("https://en-thunderscans.com/comics/the-genius/?utm_source=reader"),
            Some("/comics/the-genius/".into())
        );
        assert_eq!(series_path("https://evil.example/comics/the-genius/"), None);
    }

    #[test]
    fn accepts_scheme_host_case_and_www_variants() {
        assert_eq!(
            series_path("http://WWW.en-thunderscans.com/comics/the-genius/"),
            Some("/comics/the-genius/".into())
        );
        assert_eq!(
            series_path("HTTPS://EN-THUNDERSCANS.COM/comics/the-genius/"),
            Some("/comics/the-genius/".into())
        );
        assert_eq!(
            chapter_path("https://www.en-thunderscans.com/title-chapter-3/"),
            Some("/title-chapter-3/".into())
        );
    }

    #[test]
    fn rejects_userinfo_ports_and_lookalike_hosts() {
        assert_eq!(
            series_path("https://user:pw@en-thunderscans.com/comics/x/"),
            None
        );
        assert_eq!(
            series_path("https://user@en-thunderscans.com/comics/x/"),
            None
        );
        assert_eq!(
            series_path("https://en-thunderscans.com:443/comics/x/"),
            None
        );
        assert_eq!(
            series_path("https://en-thunderscans.com.evil.example/comics/x/"),
            None
        );
        assert_eq!(
            series_path("https://evil.example/en-thunderscans.com/comics/x/"),
            None
        );
        assert_eq!(
            series_path("https://en-thunderscans.com\\@evil.example/comics/x/"),
            None
        );
    }

    #[test]
    fn dot_segments_are_rejected_in_the_path_only() {
        assert_eq!(
            series_path("https://en-thunderscans.com/comics/x/../y/"),
            None
        );
        assert_eq!(
            canonical_path("https://en-thunderscans.com/project/page/2/?s=a..b"),
            Some("/project/page/2/".into())
        );
        assert_eq!(absolute_url("/../secret?x=1"), None);
        assert_eq!(
            absolute_url("/?s=a..b"),
            Some("https://en-thunderscans.com/?s=a..b".into())
        );
    }

    #[test]
    fn chapter_identity_preserves_fractional_slug() {
        assert_eq!(
            chapter_path("https://en-thunderscans.com/title-chapter-10-2/"),
            Some("/title-chapter-10-2/".into())
        );
        assert_eq!(
            chapter_path("https://en-thunderscans.com/comics/title/"),
            None
        );
    }

    #[test]
    fn rejects_protocol_relative_and_traversal_paths() {
        assert_eq!(absolute_url("//evil.example/image"), None);
        assert_eq!(absolute_url("/../secret"), None);
    }
}
