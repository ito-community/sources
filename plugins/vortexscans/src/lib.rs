//! Vortex Scans discovery, complete chapter catalogs and public image readers.
use ito_rs::models::manga::{ContentRating, Manga, Status, Viewer};
use ito_rs::models::{
    Chapter, ContentKind, FilterItem, HomeComponent, HomeComponentValue, HomeLayout, LinkValue,
    Listing, MediaEntry, MediaKind, MediaPageResult, Page, PageContent, PluginImageRequest,
    Setting, SettingsSchema,
};
use ito_rs::net::Request;
use ito_rs::provider::SourceProvider;
use ito_rs::{Error, Result};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use url::Url;
mod rsc;

const ORIGIN: &str = "https://vortexscans.org";
const API: &str = "https://api.vortexscans.org/api";
const IMAGE_ORIGINS: &[&str] = &[
    ORIGIN,
    "https://storage.vortexscans.org",
    "https://i0.wp.com",
    "https://i1.wp.com",
    "https://i2.wp.com",
];
const SIZE: u64 = 24;
pub struct Vortex;
fn invalid() -> Error {
    Error::Host("Vortex returned incomplete or malformed data".into())
}
fn text<'a>(v: &'a Value, key: &str) -> &'a str {
    v[key].as_str().unwrap_or("")
}
fn string(v: &Value, key: &str) -> Option<String> {
    let s = text(v, key).trim();
    (!s.is_empty()).then(|| s.to_owned())
}
fn get(url: &str) -> Result<Vec<u8>> {
    let response = Request::get(url)
        .header("Referer", &format!("{ORIGIN}/"))
        .send()?;
    classify(response.status, response.body)
}
/// Maps a response to an error category. A 403 is an access gate only when the body is a
/// verification challenge; any other 403 is a server refusal.
fn classify(status: i32, body: Vec<u8>) -> Result<Vec<u8>> {
    match status {
        200..=299 => Ok(body),
        401 => Err(needs_login()),
        403 if challenge_page(&String::from_utf8_lossy(&body)) => Err(needs_login()),
        403 => Err(Error::Server("Vortex denied the request".into())),
        404 => Err(Error::NotFound(
            "Vortex could not find this title or chapter".into(),
        )),
        status => Err(Error::Server(format!("Vortex returned HTTP {status}"))),
    }
}
fn needs_login() -> Error {
    Error::NeedsLogin(
        "Open Vortex Scans in the browser and complete any required verification or sign in."
            .into(),
    )
}
fn json(url: &str) -> Result<Value> {
    serde_json::from_slice(&get(url)?).map_err(|_| invalid())
}
fn html(url: &str) -> Result<String> {
    let body = String::from_utf8(get(url)?).map_err(|_| invalid())?;
    if challenge_page(&body) {
        return Err(Error::NeedsLogin(
            "Complete Vortex verification in the browser.".into(),
        ));
    }
    Ok(body)
}
/// A verification interstitial has Cloudflare's `cf-chl-` markup or a document title that starts
/// with "Just a moment". The same words elsewhere on a page are ordinary copy.
fn challenge_page(html: &str) -> bool {
    if html.contains("cf-chl-") {
        return true;
    }
    let lower = html.to_ascii_lowercase();
    let Some(start) = lower.find("<title") else {
        return false;
    };
    let Some(open_end) = lower[start..].find('>') else {
        return false;
    };
    lower[start + open_end + 1..]
        .trim_start()
        .starts_with("just a moment")
}
fn path_url(series: &str, chapter: Option<&str>) -> Result<String> {
    if series.is_empty()
        || series.contains('/')
        || chapter.is_some_and(|s| s.is_empty() || s.contains('/'))
    {
        return Err(invalid());
    }
    let mut url = Url::parse(ORIGIN).map_err(|_| invalid())?;
    {
        let mut path = url.path_segments_mut().map_err(|_| invalid())?;
        path.push("series").push(series);
        if let Some(chapter) = chapter {
            path.push(chapter);
        }
    }
    Ok(url.into())
}
fn unescape(segment: &str) -> Result<String> {
    let mut out = Vec::with_capacity(segment.len());
    let mut bytes = segment.bytes();
    while let Some(b) = bytes.next() {
        if b == b'%' {
            let a = bytes
                .next()
                .and_then(|c| (c as char).to_digit(16))
                .ok_or_else(invalid)?;
            let b = bytes
                .next()
                .and_then(|c| (c as char).to_digit(16))
                .ok_or_else(invalid)?;
            out.push((a * 16 + b) as u8);
        } else {
            out.push(b);
        }
    }
    String::from_utf8(out).map_err(|_| invalid())
}
fn identity(url: &str) -> Result<(String, Option<String>)> {
    let url = Url::parse(url).map_err(|_| invalid())?;
    if !matches!(url.scheme(), "https" | "http")
        || !matches!(
            url.host_str(),
            Some("vortexscans.org" | "www.vortexscans.org")
        )
        || url.port().is_some()
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err(Error::NotFound("Not a Vortex series URL".into()));
    }
    let parts: Vec<_> = url.path().trim_matches('/').split('/').collect();
    if !(parts.len() == 2 || parts.len() == 3) || parts[0] != "series" {
        return Err(Error::NotFound("Not a Vortex series URL".into()));
    }
    let slug = unescape(parts[1])?;
    let chapter = parts.get(2).map(|s| unescape(s)).transpose()?;
    path_url(&slug, chapter.as_deref())?;
    Ok((slug, chapter))
}
fn plain(html: &str) -> String {
    let mut output = String::new();
    let mut tag = false;
    for c in html.chars() {
        match c {
            '<' => {
                tag = true;
                output.push(' ');
            }
            '>' => tag = false,
            _ if !tag => output.push(c),
            _ => {}
        }
    }
    output
        .replace("&amp;", "&")
        .replace("&nbsp;", " ")
        .replace("&#39;", "'")
        .replace("&quot;", "\"")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}
fn manga(v: &Value) -> Result<Manga> {
    let slug = text(v, "slug");
    let title = text(v, "postTitle");
    if title.is_empty() {
        return Err(invalid());
    }
    let tags: Vec<String> = v["genres"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|g| string(g, "name").or_else(|| string(&g["genre"], "name")))
        .collect();
    let adult = v["isAdult"].as_bool() == Some(true)
        || tags
            .iter()
            .any(|s| matches!(s.to_lowercase().as_str(), "adult" | "hentai" | "smut"));
    Ok(Manga {
        key: slug.into(),
        title: title.into(),
        authors: string(v, "author").map(|a| vec![a]),
        artist: string(v, "artist"),
        description: string(v, "postContent").map(|s| plain(&s)),
        tags: Some(tags),
        cover: string(v, "featuredImage"),
        url: Some(path_url(slug, None)?),
        status: match text(v, "seriesStatus") {
            "ONGOING" => Status::Ongoing,
            "COMPLETED" => Status::Completed,
            "HIATUS" => Status::Hiatus,
            "CANCELLED" | "DROPPED" => Status::Cancelled,
            _ => Status::Unknown,
        },
        content_rating: if adult {
            ContentRating::Nsfw
        } else {
            ContentRating::Suggestive
        },
        nsfw: i32::from(adult),
        viewer: Viewer::Webtoon,
        chapters: None,
    })
}
fn locked(v: &Value) -> bool {
    [
        "isLocked",
        "isPermanentlyLocked",
        "isLockedByCoins",
        "isShortLinkLocked",
    ]
    .iter()
    .any(|key| v[*key].as_bool() == Some(true))
        || v["isAccessible"].as_bool() == Some(false)
}
fn chapter(v: &Value, slug: &str) -> Result<Chapter> {
    let key = text(v, "slug");
    let number = v["number"].as_f64().ok_or_else(invalid)?;
    if !number.is_finite() {
        return Err(invalid());
    }
    Ok(Chapter {
        key: key.into(),
        title: string(v, "title"),
        volume: None,
        chapter: Some(number as f32),
        date_updated: None,
        scanlator: Some("Vortex Scans".into()),
        url: Some(path_url(slug, Some(key))?),
        lang: Some("en".into()),
        paywalled: Some(locked(v)),
        kind: ContentKind::Images,
    })
}
fn chapters(value: &Value, slug: &str) -> Result<Vec<Chapter>> {
    let items = value["post"]["chapters"].as_array().ok_or_else(invalid)?;
    let total = value["totalChapterCount"].as_u64().ok_or_else(invalid)?;
    if items.len() as u64 != total {
        return Err(Error::Server(
            "Vortex did not return the complete chapter catalog".into(),
        ));
    }
    let mut result = items
        .iter()
        .map(|v| chapter(v, slug))
        .collect::<Result<Vec<_>>>()?;
    result.sort_by(|a, b| {
        a.chapter
            .unwrap_or(0.0)
            .total_cmp(&b.chapter.unwrap_or(0.0))
            .then(a.key.cmp(&b.key))
    });
    let mut seen = HashSet::new();
    if result.iter().any(|c| !seen.insert(&c.key)) {
        return Err(invalid());
    }
    Ok(result)
}
fn detail(slug: &str, units: bool) -> Result<Manga> {
    let document = html(&path_url(slug, None)?)?;
    let post = rsc::field(&document, "post")?;
    let mut result = manga(&post)?;
    if result.key != slug {
        return Err(invalid());
    }
    if units {
        let id = post["id"].as_u64().ok_or_else(invalid)?;
        result.chapters = Some(chapters(
            &json(&format!(
                "{API}/chapters?postId={id}&skip=0&take=all&order=asc"
            ))?,
            slug,
        )?);
    }
    Ok(result)
}
fn tag(value: &str) -> Result<&str> {
    match value {
        "latestUpdate" | "trending" | "new" => Ok(value),
        _ => Err(Error::NeedsSetup(
            "Choose latestUpdate, trending, or new in source settings.".into(),
        )),
    }
}
fn browse(query: &str, page: i32, order: &str) -> Result<MediaPageResult> {
    let page = page.max(1) as u64;
    let mut url = Url::parse(&format!("{API}/posts")).map_err(|_| invalid())?;
    url.query_pairs_mut()
        .append_pair("page", &page.to_string())
        .append_pair("perPage", &SIZE.to_string())
        .append_pair("searchTerm", query)
        .append_pair("tag", tag(order)?)
        .append_pair("isNovel", "false");
    catalog(&json(url.as_str())?, page)
}
fn catalog(data: &Value, page: u64) -> Result<MediaPageResult> {
    let posts = data["posts"].as_array().ok_or_else(invalid)?;
    let total = data["totalCount"].as_u64().ok_or_else(invalid)?;
    // Every page but the last is full. A short page means the site dropped entries.
    let expected = SIZE.min(total.saturating_sub(page.saturating_sub(1) * SIZE));
    if (posts.len() as u64) < expected {
        return Err(invalid());
    }
    Ok(MediaPageResult {
        entries: posts
            .iter()
            .map(|v| manga(v).map(MediaEntry::Manga))
            .collect::<Result<_>>()?,
        has_next_page: page * SIZE < total,
    })
}
fn headers() -> HashMap<String, String> {
    HashMap::from([
        ("Referer".into(), format!("{ORIGIN}/")),
        ("Origin".into(), ORIGIN.into()),
    ])
}
fn image_request(url: &str) -> Result<PluginImageRequest> {
    let parsed = Url::parse(url).map_err(|_| invalid())?;
    if !IMAGE_ORIGINS.contains(&parsed.origin().ascii_serialization().as_str())
        || !parsed.username().is_empty()
        || parsed.password().is_some()
    {
        return Err(Error::NotFound(
            "Image origin is not allowed by Vortex Scans".into(),
        ));
    }
    Ok(PluginImageRequest {
        url: url.into(),
        headers: Some(headers()),
    })
}
/// Reads the public pages of the requested chapter. The route payload also holds other
/// `chapter` objects, such as neighbors and lists, so the object must name the requested slug
/// and its images are read from that same object.
fn reader(document: &str, requested: &str) -> Result<Vec<Page>> {
    let object = rsc::objects_with(document, "chapter")
        .into_iter()
        .find(|object| {
            object
                .get("chapter")
                .and_then(|chapter| chapter["slug"].as_str())
                == Some(requested)
        })
        .ok_or_else(|| Error::Server("Vortex did not return the requested chapter".into()))?;
    let data = object.get("chapter").ok_or_else(invalid)?;
    if !data.is_object() || data["isLocked"].as_bool().is_none() {
        return Err(invalid());
    }
    if locked(data) {
        return Err(Error::NeedsLogin(
            "This chapter is locked. Open it on Vortex Scans to check the required access.".into(),
        ));
    }
    // The reader list includes any configured intro/outro pages in display order.
    if let Some(images) = object.get("readerImages") {
        let images = images.as_array().ok_or_else(invalid)?;
        if images.is_empty() {
            return Err(Error::Server(
                "Vortex returned no public reader images".into(),
            ));
        }
        return images
            .iter()
            .enumerate()
            .map(|(index, image)| {
                let request = image_request(text(image, "displaySrc"))?;
                Ok(Page {
                    index: index as i32,
                    content: PageContent::Url(request.url),
                    headers: request.headers,
                    has_description: false,
                    description: None,
                })
            })
            .collect();
    }
    let images = object.get("publicImages").ok_or_else(invalid)?;
    let mut images = images
        .as_array()
        .ok_or_else(invalid)?
        .iter()
        .collect::<Vec<_>>();
    images.sort_by_key(|v| v["order"].as_i64().unwrap_or(i64::MAX));
    if images.is_empty() {
        return Err(Error::Server(
            "Vortex returned no public chapter images".into(),
        ));
    }
    let mut seen = HashSet::new();
    images
        .into_iter()
        .enumerate()
        .map(|(index, v)| {
            let order = v["order"].as_i64().ok_or_else(invalid)?;
            if !seen.insert(order) {
                return Err(invalid());
            }
            let request = image_request(text(v, "url"))?;
            Ok(Page {
                index: index as i32,
                content: PageContent::Url(request.url),
                headers: request.headers,
                has_description: false,
                description: None,
            })
        })
        .collect()
}
impl SourceProvider for Vortex {
    const KINDS: &'static [MediaKind] = &[MediaKind::Manga];
    fn get_home() -> Result<HomeLayout> {
        let mut components = Vec::new();
        for (id, name) in [
            ("latestUpdate", "Latest updates"),
            ("trending", "Trending"),
            ("new", "New series"),
        ] {
            let entries = browse("", 1, id)?
                .entries
                .into_iter()
                .filter_map(|v| {
                    if let MediaEntry::Manga(m) = v {
                        Some(m)
                    } else {
                        None
                    }
                })
                .collect();
            components.push(HomeComponent {
                title: Some(name.into()),
                subtitle: None,
                value: HomeComponentValue::Scroller(
                    entries,
                    Some(Listing {
                        id: id.into(),
                        name: name.into(),
                        kind: 0,
                    }),
                ),
            });
        }
        Ok(HomeLayout { components })
    }
    fn get_settings() -> Option<SettingsSchema> {
        Some(SettingsSchema {
            settings: vec![Setting::Picker {
                id: "browse_order".into(),
                name: "Browse order".into(),
                summary: Some(
                    "latestUpdate: recently updated; trending: hot series; new: recently added."
                        .into(),
                ),
                options: vec!["latestUpdate".into(), "trending".into(), "new".into()],
                default_value: "latestUpdate".into(),
            }],
        })
    }
    fn get_listing(listing: Listing, page: i32) -> Result<MediaPageResult> {
        browse("", page, &listing.id)
    }
    fn search(
        query: &str,
        page: i32,
        kind: Option<MediaKind>,
        _filters: Vec<FilterItem>,
    ) -> Result<MediaPageResult> {
        if kind.is_some_and(|kind| kind != MediaKind::Manga) {
            return Err(Error::Unsupported);
        }
        if query.starts_with("https://") || query.starts_with("http://") {
            let (slug, _) = identity(query)?;
            return Ok(MediaPageResult {
                entries: if page <= 1 {
                    vec![MediaEntry::Manga(detail(&slug, false)?)]
                } else {
                    vec![]
                },
                has_next_page: false,
            });
        }
        let order = ito_rs::defaults::get("browse_order")?.unwrap_or_else(|| "latestUpdate".into());
        browse(query, page, &order)
    }
    fn get_media_update(
        entry: MediaEntry,
        needs_details: bool,
        needs_units: bool,
    ) -> Result<MediaEntry> {
        let MediaEntry::Manga(mut entry) = entry else {
            return Err(Error::Unsupported);
        };
        if needs_details || needs_units {
            let mut updated = detail(&entry.key, needs_units)?;
            if needs_details {
                if !needs_units {
                    updated.chapters = entry.chapters;
                }
                entry = updated;
            } else {
                entry.chapters = updated.chapters;
            }
        }
        Ok(MediaEntry::Manga(entry))
    }
    fn get_chapter_pages(entry: MediaEntry, chapter: Chapter) -> Result<Vec<Page>> {
        let MediaEntry::Manga(entry) = entry else {
            return Err(Error::Unsupported);
        };
        if chapter.paywalled == Some(true) {
            return Err(Error::NeedsLogin(
                "This chapter requires access on Vortex Scans.".into(),
            ));
        }
        reader(
            &html(&path_url(&entry.key, Some(&chapter.key))?)?,
            &chapter.key,
        )
    }
    fn handle_url(url: &str) -> Result<LinkValue> {
        let (slug, chapter_slug) = identity(url)?;
        let mut item = detail(&slug, chapter_slug.is_some())?;
        if let Some(chapter_slug) = chapter_slug {
            let chapters = item.chapters.take().ok_or_else(invalid)?;
            let selected = chapters
                .into_iter()
                .find(|c| c.key == chapter_slug)
                .ok_or_else(|| Error::NotFound("Vortex chapter was not found".into()))?;
            item.chapters = Some(vec![selected]);
        }
        Ok(LinkValue::Manga(item))
    }
    fn get_image_origins() -> Result<Vec<String>> {
        Ok(IMAGE_ORIGINS.iter().map(|s| (*s).into()).collect())
    }
    fn get_image_request(url: &str) -> Result<PluginImageRequest> {
        image_request(url)
    }
    fn handle_image(url: &str) -> Result<Vec<u8>> {
        let request = image_request(url)?;
        let response = Request::get(&request.url)
            .header("Referer", &format!("{ORIGIN}/"))
            .send()?;
        if !(200..300).contains(&response.status) {
            return Err(Error::Server(format!(
                "Vortex image returned HTTP {}",
                response.status
            )));
        }
        Ok(response.body)
    }
}
ito_rs::export_plugin!(Vortex);

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn urls_preserve_apostrophes_and_reject_foreign_identity() {
        let (slug, chapter) =
            identity("https://vortexscans.org/series/a%27b/chapter-0.5?x=1").expect("URL");
        assert_eq!(slug, "a'b");
        assert_eq!(chapter.as_deref(), Some("chapter-0.5"));
        assert_eq!(
            identity(&path_url(&slug, chapter.as_deref()).expect("URL"))
                .expect("roundtrip")
                .0,
            slug
        );
        assert!(identity("https://vortexscans.org.evil/series/a").is_err());
        assert!(identity("https://vortexscans.org/series/a%2Fb").is_err());
        assert!(image_request("https://evil.test/page.png").is_err());
    }
    #[test]
    fn catalog_totals_control_pagination() {
        let data = serde_json::json!({"posts":[],"totalCount":0});
        assert!(!catalog(&data, 1).expect("empty catalog").has_next_page);
        assert!(catalog(&serde_json::json!({"posts":[],"totalCount":30}), 1).is_err());
        assert!(catalog(&serde_json::json!({"error":"failure"}), 1).is_err());
        let full: Vec<Value> = (0..SIZE)
            .map(|i| serde_json::json!({"slug":format!("synthetic-{i}"),"postTitle":"Synthetic"}))
            .collect();
        let first = serde_json::json!({"posts":full,"totalCount":25});
        assert!(catalog(&first, 1).expect("first page").has_next_page);
        let last = serde_json::json!({"posts":[{"slug":"synthetic-last","postTitle":"Synthetic"}],"totalCount":25});
        assert!(!catalog(&last, 2).expect("last page").has_next_page);
        assert!(catalog(&serde_json::json!({"posts":[],"totalCount":25}), 2).is_err());
        assert!(
            catalog(
                &serde_json::json!({"posts":[{"slug":"x","postTitle":"X"}],"totalCount":30}),
                1
            )
            .is_err()
        );
    }
    #[test]
    fn complete_chapters_preserve_zero_fraction_and_locks() {
        let data = serde_json::json!({"post":{"chapters":[{"slug":"chapter-2","number":2,"isLocked":true},{"slug":"chapter-0","number":0,"isAccessible":true},{"slug":"chapter-0.5","number":0.5}]},"totalChapterCount":3});
        let list = chapters(&data, "synthetic").expect("chapters");
        assert_eq!(
            list.iter().map(|c| c.chapter).collect::<Vec<_>>(),
            vec![Some(0.0), Some(0.5), Some(2.0)]
        );
        assert_eq!(list[2].paywalled, Some(true));
        let mut truncated = data;
        truncated["totalChapterCount"] = 4.into();
        assert!(chapters(&truncated, "synthetic").is_err());
    }
    #[test]
    fn reader_orders_public_pages_and_refuses_locked_data() {
        let fixture = r#"{chapter:{slug:"chapter-1",isLocked:!1,isAccessible:!0},publicImages:[{url:"https://storage.vortexscans.org/synthetic-b.png",order:2},{url:"https://storage.vortexscans.org/synthetic-a.png",order:0}]}"#;
        let pages = reader(fixture, "chapter-1").expect("pages");
        assert_eq!(pages.len(), 2);
        assert!(
            matches!(&pages[0].content,PageContent::Url(url) if url.ends_with("synthetic-a.png"))
        );
        assert!(reader(r#"{chapter:{slug:"chapter-1",isLocked:!0}}"#, "chapter-1").is_err());
        assert!(
            reader(
                r#"{chapter:{slug:"chapter-1",isAccessible:!1},publicImages:[]}"#,
                "chapter-1"
            )
            .is_err()
        );
        assert!(
            reader(
                r#"{chapter:{slug:"chapter-1",isLocked:!1},publicImages:[]}"#,
                "chapter-1"
            )
            .is_err()
        );
        assert!(reader(r#"{chapter:{slug:"chapter-1"}}"#, "chapter-1").is_err());
        let displayed = r#"{chapter:{slug:"chapter-1",isLocked:!1,isAccessible:!0},readerImages:[{displaySrc:"https://storage.vortexscans.org/intro.png"},{displaySrc:"https://storage.vortexscans.org/body.png"}]}"#;
        let pages = reader(displayed, "chapter-1").expect("display pages");
        assert_eq!(pages.len(), 2);
        assert!(matches!(&pages[0].content,PageContent::Url(url) if url.ends_with("intro.png")));
    }
    #[test]
    fn reader_takes_images_from_the_requested_chapter_object() {
        // A neighboring chapter object appears first; its images must not be used.
        let fixture = r#"{post:{chapters:[{chapter:{slug:"chapter-0",isLocked:!1},readerImages:[{displaySrc:"https://storage.vortexscans.org/wrong.png"}]}]}},{chapter:{slug:"chapter-1",isLocked:!1},readerImages:[{displaySrc:"https://storage.vortexscans.org/right.png"}]}"#;
        let pages = reader(fixture, "chapter-1").expect("requested chapter");
        assert_eq!(pages.len(), 1);
        assert!(matches!(&pages[0].content, PageContent::Url(url) if url.ends_with("right.png")));
        assert!(reader(fixture, "chapter-9").is_err());
    }
    #[test]
    fn unauthorized_is_a_login_gate_and_plain_forbidden_is_a_server_error() {
        assert!(matches!(
            classify(401, b"<html>plain</html>".to_vec()),
            Err(Error::NeedsLogin(_))
        ));
        assert!(matches!(
            classify(403, b"<html>denied</html>".to_vec()),
            Err(Error::Server(_))
        ));
        assert!(matches!(
            classify(403, b"<title>Just a moment...</title>".to_vec()),
            Err(Error::NeedsLogin(_))
        ));
        assert!(matches!(
            classify(403, b"<script src='/cf-chl-x.js'></script>".to_vec()),
            Err(Error::NeedsLogin(_))
        ));
        assert!(matches!(
            classify(404, b"".to_vec()),
            Err(Error::NotFound(_))
        ));
        assert!(classify(200, b"ok".to_vec()).is_ok());
    }
    #[test]
    fn challenge_detection_reads_the_title_prefix_or_cloudflare_markup() {
        assert!(challenge_page(
            "<html><head><title>Just a moment...</title></head></html>"
        ));
        assert!(challenge_page("<title>  JUST A MOMENT</title>"));
        assert!(challenge_page(
            "<head><title>Synthetic</title></head><div cf-chl-widget></div>"
        ));
        assert!(!challenge_page(
            "<html><title>Synthetic series</title><p>Just a moment, please</p></html>"
        ));
        assert!(!challenge_page("<title>Chapter: just a moment ago</title>"));
    }
}
