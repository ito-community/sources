//! Image addresses: which hosts the source may fetch from and how a request is made.
//!
//! The permitted origins are fixed here. They never depend on an address a caller supplies.

use crate::links::HOME_URL;
use crate::url;
use ito_rs::models::PluginImageRequest;
use std::collections::HashMap;

const MEDIA_HOST: &str = "media.templetoons.com";
/// The site's own proxy for images it does not host itself.
const PROXY_HOST: &str = "akira.templetoons.com";
const STATICALLY_HOST: &str = "cdn.statically.io";
/// Third-party hosts the site rewrites through its proxy before showing them.
const PROXIED_HOSTS: [&str; 3] = ["imagizer.imageshack.com", "imageshack.com", "i.ibb.co"];

/// Origins native image fetching is permitted for.
pub fn origins() -> Vec<String> {
    [MEDIA_HOST, PROXY_HOST]
        .iter()
        .map(|host| format!("https://{host}"))
        .collect()
}

/// The request for an image the source serves: the site's hosts over https, sent with the site
/// as referrer, which its image host requires of foreign pages. Any other address is refused.
pub fn request(address: &str) -> Option<PluginImageRequest> {
    Some(PluginImageRequest {
        url: permitted(address)?,
        headers: Some(headers()),
    })
}

/// The headers an image fetch needs.
pub fn headers() -> HashMap<String, String> {
    HashMap::from([("Referer".to_owned(), HOME_URL.to_owned())])
}

/// A page, cover or thumbnail address as the plugin hands it to Ito: the same address when it
/// is on a permitted host, the site's proxy address for hosts the site proxies, otherwise none.
pub fn locate(source: &str) -> Option<String> {
    let source = source.trim();
    let absolute = match source.strip_prefix("//") {
        Some(rest) => format!("https://{rest}"),
        None => source.to_owned(),
    };
    let parsed = url::parse_absolute(&absolute)?;
    if !parsed.https || parsed.port.is_some_and(|port| port != 443) {
        return None;
    }
    let query = parsed
        .query
        .map(|query| format!("?{query}"))
        .unwrap_or_default();
    let host = parsed.host.as_str();
    if host == MEDIA_HOST || host == PROXY_HOST {
        return Some(format!("https://{host}{}{query}", parsed.path));
    }
    if host == STATICALLY_HOST {
        // The site keeps the path after `/img/` and drops the query.
        let path = parsed.path.strip_prefix("/img/")?;
        return Some(format!("https://{PROXY_HOST}/{path}"));
    }
    PROXIED_HOSTS
        .contains(&host)
        .then(|| format!("https://{PROXY_HOST}/{host}{}{query}", parsed.path))
}

fn permitted(address: &str) -> Option<String> {
    let parsed = url::parse_absolute(address)?;
    let allowed = parsed.https
        && parsed.port.is_none_or(|port| port == 443)
        && (parsed.host == MEDIA_HOST || parsed.host == PROXY_HOST);
    allowed.then(|| address.trim().to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn origins_do_not_depend_on_any_address() {
        assert_eq!(
            origins(),
            [
                "https://media.templetoons.com",
                "https://akira.templetoons.com"
            ]
        );
    }

    #[test]
    fn site_images_keep_their_address_and_foreign_ones_are_dropped() {
        let cover = "https://media.templetoons.com/file/b/covers/x.webp";
        assert_eq!(locate(cover).as_deref(), Some(cover));
        assert_eq!(
            locate("//media.templetoons.com/file/b/covers/x.webp").as_deref(),
            Some(cover)
        );
        assert_eq!(
            locate("https://media.templetoons.com/a/b.jpg?v=2").as_deref(),
            Some("https://media.templetoons.com/a/b.jpg?v=2")
        );
        for foreign in [
            "http://media.templetoons.com/x.jpg",
            "https://media.templetoons.com.evil.example/x.jpg",
            "https://media.templetoons.com@evil.example/x.jpg",
            "https://media.templetoons.com:8443/x.jpg",
            "https://example.com/x.jpg",
            "/relative/x.jpg",
            "data:image/png;base64,AAAA",
            "",
            "   ",
        ] {
            assert_eq!(locate(foreign), None, "{foreign}");
        }
    }

    #[test]
    fn hosts_the_site_proxies_are_addressed_through_its_proxy() {
        assert_eq!(
            locate("https://imagizer.imageshack.com/img922/8254/AbC.jpg").as_deref(),
            Some("https://akira.templetoons.com/imagizer.imageshack.com/img922/8254/AbC.jpg")
        );
        assert_eq!(
            locate("https://i.ibb.co/abc/cover.png?x=1").as_deref(),
            Some("https://akira.templetoons.com/i.ibb.co/abc/cover.png?x=1")
        );
        assert_eq!(
            locate("https://cdn.statically.io/img/example.org/cover.jpg?w=2").as_deref(),
            Some("https://akira.templetoons.com/example.org/cover.jpg")
        );
        assert_eq!(locate("https://cdn.statically.io/other/cover.jpg"), None);
    }

    #[test]
    fn requests_name_the_site_as_referrer_and_refuse_other_hosts() {
        let page = "https://media.templetoons.com/file/b/uploads/series/s/c/01.jpg";
        let request = request(page).expect("site image");
        assert_eq!(request.url, page);
        let headers = request.headers.expect("headers");
        assert_eq!(
            headers.get("Referer").map(String::as_str),
            Some("https://templetoons.com/")
        );
        assert_eq!(headers.len(), 1);
        for refused in [
            "https://example.com/x.jpg",
            "http://media.templetoons.com/x.jpg",
            "https://media.templetoons.com:444/x.jpg",
            "https://templetoons.com/icon.webp",
            "https://evil.example/https://media.templetoons.com/x.jpg",
        ] {
            assert!(super::request(refused).is_none(), "{refused}");
        }
    }
}
