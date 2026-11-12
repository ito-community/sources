//! Image hosts from the nhentai CDN config. The selected server is a preference;
//! every known host stays permitted so a saved page still loads after a change.

use ito_rs::models::PluginImageRequest;
use std::collections::HashMap;

pub const REFERER: &str = "https://nhentai.net/";

const HOSTS: &[&str] = &[
    "i1.nhentai.net",
    "i2.nhentai.net",
    "i3.nhentai.net",
    "i4.nhentai.net",
    "t1.nhentai.net",
    "t2.nhentai.net",
    "t3.nhentai.net",
    "t4.nhentai.net",
    "i.nhentai.net",
    "t.nhentai.net",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageKind {
    Page,
    Thumb,
}

pub fn origins() -> Vec<String> {
    HOSTS.iter().map(|host| format!("https://{host}")).collect()
}

pub fn server_index(value: Option<&str>) -> u8 {
    match value {
        Some("2") => 2,
        Some("3") => 3,
        Some("4") => 4,
        _ => 1,
    }
}

pub fn url(path: &str, server: u8, kind: ImageKind) -> Option<String> {
    let path = path.trim();
    if path.is_empty() {
        return None;
    }
    if path.contains("://") {
        return allowed(path).then(|| path.to_owned());
    }
    let path = path.trim_start_matches('/');
    if path.is_empty() || path.contains("..") || path.contains('\\') {
        return None;
    }
    let server = server.clamp(1, 4);
    let host = match kind {
        ImageKind::Page => "i",
        ImageKind::Thumb => "t",
    };
    Some(format!("https://{host}{server}.nhentai.net/{path}"))
}

pub fn request(url: &str) -> Option<PluginImageRequest> {
    if !allowed(url) {
        return None;
    }
    Some(PluginImageRequest {
        url: url.to_owned(),
        headers: Some(headers()),
    })
}

pub fn headers() -> HashMap<String, String> {
    HashMap::from([("Referer".into(), REFERER.into())])
}

fn allowed(url: &str) -> bool {
    let Some(rest) = url.strip_prefix("https://") else {
        return false;
    };
    let (host, path) = rest.split_once('/').unwrap_or((rest, ""));
    let host = host.split(':').next().unwrap_or(host);
    !path.contains("..") && HOSTS.contains(&host)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relative_paths_use_the_selected_server_and_reject_traversal() {
        assert_eq!(
            url("galleries/1/1.webp", 2, ImageKind::Page).as_deref(),
            Some("https://i2.nhentai.net/galleries/1/1.webp")
        );
        assert_eq!(
            url("/galleries/1/thumb.webp", 4, ImageKind::Thumb).as_deref(),
            Some("https://t4.nhentai.net/galleries/1/thumb.webp")
        );
        assert!(url("../secret", 1, ImageKind::Page).is_none());
        assert!(url("", 1, ImageKind::Thumb).is_none());
    }

    #[test]
    fn requests_keep_the_site_referer_and_refuse_other_hosts() {
        let image = request("https://i1.nhentai.net/galleries/1/1.webp").expect("image");
        assert_eq!(
            image
                .headers
                .expect("headers")
                .get("Referer")
                .map(String::as_str),
            Some(REFERER)
        );
        assert!(request("https://evil.example/galleries/1/1.webp").is_none());
        assert!(request("http://i1.nhentai.net/galleries/1/1.webp").is_none());
        assert_eq!(origins().len(), HOSTS.len());
    }
}
