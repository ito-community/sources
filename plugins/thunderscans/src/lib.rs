//! English-language Thunder Scans catalog and reader provider.

use std::collections::HashMap;

use ito_rs::models::manga::{ContentRating, Manga, Status, Viewer};
use ito_rs::models::{
    Chapter, ContentKind, FilterItem, HomeComponent, HomeComponentValue, HomeLayout, LinkValue,
    Listing, MediaEntry, MediaKind, MediaPageResult, Page, PageContent, PluginImageRequest,
};
use ito_rs::net::Request;
use ito_rs::provider::SourceProvider;
use ito_rs::{Error, Result};
use scraper::{ElementRef, Html, Selector};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chapter_list_keeps_fractional_and_locked_items() {
        let root = document(
            r##"<ul id="chapterlist">
                <li data-num="10.2"><a href="https://en-thunderscans.com/series-chapter-10-2/"><span class="chapternum">Chapter 10.2</span></a></li>
                <li data-num="10"><a data-bs-target="#lockedChapterModal" data-id="9001" data-coin="50"><span class="chapternum">Chapter 10</span></a></li>
            </ul>"##,
        );
        let chapters = parse_chapters(&root).expect("synthetic chapter list parses");
        assert_eq!(chapters.len(), 2);
        assert_eq!(chapters[0].chapter, Some(10.2));
        assert_eq!(chapters[0].paywalled, Some(false));
        assert_eq!(chapters[1].paywalled, Some(true));
        assert_eq!(chapters[1].url, None);
        assert_eq!(chapters[1].key, "locked:9001");
    }

    #[test]
    fn search_query_encoding_preserves_unreserved_bytes() {
        assert_eq!(query_encode("a b&c"), "a%20b%26c");
        assert_eq!(query_encode("10.2"), "10%2E2");
    }

    #[test]
    fn locked_metadata_refuses_access_before_request() {
        let chapter = Chapter {
            key: "locked:9001".into(),
            title: Some("Chapter 42".into()),
            volume: None,
            chapter: Some(42.0),
            date_updated: None,
            scanlator: None,
            url: None,
            lang: Some("en".into()),
            paywalled: Some(true),
            kind: ContentKind::Images,
        };
        assert!(matches!(
            readable_chapter_path(&chapter),
            Err(Error::NeedsLogin(_))
        ));
    }

    #[test]
    fn reader_protected_flag_does_not_hide_public_images() {
        let html = r#"<script>ts_reader.run({"protected":true,"is_novel":false,"defaultSource":"Server 1","sources":[{"source":"Server 1","images":["https://en-thunderscans.com/wp-content/uploads/1.webp"]}]});</script>"#;
        let pages =
            reader_pages(html, "/series-chapter-1/").expect("public images remain readable");
        assert_eq!(pages.len(), 1);
    }

    #[test]
    fn reader_preserves_image_order_and_rejects_foreign_hosts() {
        let html = r#"<script>ts_reader.run({"protected":false,"is_novel":false,"defaultSource":"Server 1","sources":[{"source":"Server 1","images":["https://en-thunderscans.com/wp-content/uploads/2.webp","https://en-thunderscans.com/wp-content/uploads/1.webp"]}]});</script>"#;
        let pages =
            reader_pages(html, "/series-chapter-1/").expect("synthetic public reader parses");
        assert_eq!(pages.len(), 2);
        assert!(matches!(&pages[0].content, PageContent::Url(url) if url.ends_with("2.webp")));
        let foreign = html.replace(
            "https://en-thunderscans.com/wp-content/uploads/1.webp",
            "https://evil.example/image.webp",
        );
        assert!(reader_pages(&foreign, "/series-chapter-1/").is_err());
    }

    #[test]
    fn status_and_challenge_pages_map_to_error_categories() {
        assert!(response_text(200, b"<html>ok</html>".to_vec()).is_ok());
        assert!(matches!(
            response_text(401, b"<html>sign in</html>".to_vec()),
            Err(Error::NeedsLogin(_))
        ));
        assert!(matches!(
            response_text(403, b"<html>denied</html>".to_vec()),
            Err(Error::Server(_))
        ));
        assert!(matches!(
            response_text(403, b"<title>Just a moment...</title>".to_vec()),
            Err(Error::NeedsLogin(_))
        ));
        assert!(matches!(
            response_text(200, b"<script src='/cf-chl-x.js'></script>".to_vec()),
            Err(Error::NeedsLogin(_))
        ));
        assert!(matches!(
            response_text(404, b"<html>missing</html>".to_vec()),
            Err(Error::NotFound(_))
        ));
    }

    #[test]
    fn chapter_series_comes_from_the_back_link_not_the_slug() {
        let html = r#"<div class="widget"><a href="https://en-thunderscans.com/comics/sidebar-series/">Sidebar</a></div>
            <div class="entry"><a class="btn-enhanced btn-back-enhanced" href="https://en-thunderscans.com/comics/the-real-series/">Back</a></div>"#;
        assert_eq!(
            chapter_series_path(html).expect("back link is the series"),
            "/comics/the-real-series/"
        );
        assert!(
            chapter_series_path("<div class='widget'><a href='/comics/sidebar/'>x</a></div>")
                .is_err()
        );
    }

    #[test]
    fn editor_pick_section_has_no_all_series_listing() {
        let html = r#"<div class="bixbox"><div class="releases"><h2>Editor's Pick</h2></div>
            <div class="listupd"><div class="bsx"><a href="https://en-thunderscans.com/comics/a/" title="A">A</a></div></div></div>
            <div class="bixbox"><div class="releases"><h2>Latest Comics</h2></div>
            <div class="listupd"><div class="bsx"><a href="https://en-thunderscans.com/comics/b/" title="B">B</a></div></div></div>"#;
        let components = home_components(html).expect("synthetic home parses");
        let listings: Vec<_> = components
            .iter()
            .map(|component| match &component.value {
                HomeComponentValue::MediaScroller(_, listing) => {
                    listing.as_ref().map(|l| l.id.clone())
                }
                _ => panic!("expected a media scroller"),
            })
            .collect();
        assert_eq!(listings, vec![None, Some("latest-comics".to_owned())]);
    }
}
mod links;

const ORIGIN: &str = "https://en-thunderscans.com";
const REFERER: &str = "https://en-thunderscans.com/";
const IMAGE_PREFIX: &str = "https://en-thunderscans.com/wp-content/uploads/";

struct ThunderScans;

fn request(path: &str) -> Result<String> {
    let url = links::absolute_url(path).ok_or(Error::Unsupported)?;
    let mut req = Request::get(url);
    req.header("Referer", REFERER);
    let response = req.send()?;
    response_text(response.status, response.body)
}

/// Maps a response to an error category. Verification pages are recognized before the
/// status code, because a challenge can arrive with any status.
fn response_text(status: i32, body: Vec<u8>) -> Result<String> {
    let html = String::from_utf8(body)
        .map_err(|_| Error::Host("Thunder Scans returned invalid HTML".into()))?;
    if is_challenge_page(&html)? {
        return Err(Error::NeedsLogin(
            "Complete Thunder Scans verification in the browser.".into(),
        ));
    }
    match status {
        200..=299 => Ok(html),
        401 => Err(Error::NeedsLogin(
            "Sign in on Thunder Scans in the browser to continue.".into(),
        )),
        403 => Err(Error::Server("Thunder Scans denied the request".into())),
        404 => Err(Error::NotFound("Thunder Scans page was not found".into())),
        status => Err(Error::Server(format!(
            "Thunder Scans returned HTTP {status}"
        ))),
    }
}

/// Cloudflare interstitials carry a `cf-chl-` marker or a "Just a moment" document title.
fn is_challenge_page(html: &str) -> Result<bool> {
    if html.contains("cf-chl-") {
        return Ok(true);
    }
    let root = document(html);
    let titles = root.query("title")?;
    Ok(titles.iter().filter_map(text).any(|title| {
        title
            .to_ascii_lowercase()
            .trim_start()
            .starts_with("just a moment")
    }))
}

// Keeping HTML parsing local lets native tests exercise the actual parsers.
trait QueryNodes {
    fn query(&self, selector: &str) -> Result<Vec<ElementRef<'_>>>;
}

impl QueryNodes for Html {
    fn query(&self, selector: &str) -> Result<Vec<ElementRef<'_>>> {
        let selector =
            Selector::parse(selector).map_err(|_| Error::Html("Invalid source selector".into()))?;
        Ok(self.select(&selector).collect())
    }
}

impl QueryNodes for ElementRef<'_> {
    fn query(&self, selector: &str) -> Result<Vec<ElementRef<'_>>> {
        let selector =
            Selector::parse(selector).map_err(|_| Error::Html("Invalid source selector".into()))?;
        Ok(self.select(&selector).collect())
    }
}

fn card_manga(container: &ElementRef<'_>) -> Result<Option<Manga>> {
    let Some(anchor) = container.query("a[href*='/comics/']")?.into_iter().next() else {
        return Ok(None);
    };
    let Some(href) = attr(&anchor, "href") else {
        return Ok(None);
    };
    let Some(path) = links::series_path(&href) else {
        return Ok(None);
    };
    let title = attr(&anchor, "title")
        .or_else(|| text(&anchor))
        .unwrap_or_default();
    if title.trim().is_empty() {
        return Ok(None);
    }
    let cover = container
        .query("img")?
        .into_iter()
        .next()
        .and_then(|image| attr(&image, "src").or_else(|| attr(&image, "data-src")))
        .filter(|url| url.starts_with(IMAGE_PREFIX));
    let key = path
        .trim_matches('/')
        .rsplit('/')
        .next()
        .unwrap_or_default()
        .to_owned();
    Ok(Some(manga(key, title, cover, format!("{ORIGIN}{path}"))))
}

fn document(html: &str) -> Html {
    Html::parse_document(html)
}

fn attr(node: &ElementRef<'_>, name: &str) -> Option<String> {
    node.value().attr(name).map(str::to_owned)
}

fn text(node: &ElementRef<'_>) -> Option<String> {
    let value = node.text().collect::<String>().trim().to_owned();
    (!value.is_empty()).then_some(value)
}

fn status(value: &str) -> Status {
    match value.to_ascii_lowercase().as_str() {
        "ongoing" => Status::Ongoing,
        "completed" => Status::Completed,
        "hiatus" => Status::Hiatus,
        "canceled" | "cancelled" | "dropped" => Status::Cancelled,
        _ => Status::Unknown,
    }
}

fn rating() -> ContentRating {
    // The site does not declare a universal safe rating; neutral is safer than mislabeling.
    ContentRating::Suggestive
}

fn manga(key: String, title: String, cover: Option<String>, url: String) -> Manga {
    Manga {
        key,
        title,
        authors: None,
        artist: None,
        description: None,
        tags: None,
        cover,
        url: Some(url),
        status: Status::Unknown,
        content_rating: rating(),
        nsfw: 0,
        viewer: Viewer::Webtoon,
        chapters: None,
    }
}

/// Series cards live in the catalog results grid. Sidebar and widget links to series are
/// not catalog entries, so only `.listupd` descendants are read.
fn card_entries(html: &str) -> Result<Vec<MediaEntry>> {
    let root = document(html);
    let mut entries = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for anchor in root.query(".listupd a[href*='/comics/']")? {
        let Some(href) = attr(&anchor, "href") else {
            continue;
        };
        let Some(path) = links::series_path(&href) else {
            continue;
        };
        if !seen.insert(path.clone()) {
            continue;
        }
        let title = attr(&anchor, "title")
            .or_else(|| text(&anchor))
            .unwrap_or_default();
        if title.trim().is_empty() {
            continue;
        }
        let cover = anchor
            .query("img")?
            .into_iter()
            .next()
            .and_then(|image| attr(&image, "src").or_else(|| attr(&image, "data-src")))
            .filter(|url| url.starts_with(IMAGE_PREFIX));
        entries.push(MediaEntry::Manga(manga(
            path.trim_matches('/')
                .rsplit('/')
                .next()
                .unwrap_or_default()
                .to_owned(),
            title,
            cover,
            format!("{ORIGIN}{path}"),
        )));
    }
    Ok(entries)
}

fn has_next_page(html: &str) -> Result<bool> {
    let root = document(html);
    Ok(root
        .query("a.next.page-numbers, a[rel='next']")?
        .into_iter()
        .any(|node| attr(&node, "href").is_some_and(|href| links::canonical_path(&href).is_some())))
}

fn page_result(path: &str) -> Result<MediaPageResult> {
    parse_catalog_page(&request(path)?)
}

/// A catalog or search page. The site renders an explicit "Not Found" heading inside the
/// results container when a search has no matches; any other card-less page is unexpected.
fn parse_catalog_page(html: &str) -> Result<MediaPageResult> {
    let entries = card_entries(html)?;
    if !entries.is_empty() {
        return Ok(MediaPageResult {
            entries,
            has_next_page: has_next_page(html)?,
        });
    }
    let root = document(html);
    let reports_no_results = root
        .query(".listupd h3")?
        .iter()
        .filter_map(text)
        .any(|heading| heading.eq_ignore_ascii_case("not found"));
    if reports_no_results {
        return Ok(MediaPageResult {
            entries,
            has_next_page: false,
        });
    }
    Err(Error::Host(
        "Thunder Scans returned an unrecognized catalog page".into(),
    ))
}

fn query_encode(value: &str) -> String {
    value
        .bytes()
        .map(|byte| match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'~' => {
                (byte as char).to_string()
            }
            _ => format!("%{byte:02X}"),
        })
        .collect()
}

/// Chapter rows. Only explicit placeholders and the lock modal mark a row as locked. Other
/// rows without a valid chapter URL are not readable and are skipped. Duplicate chapter
/// links collapse to one row; duplicate locked rows keep distinct keys by list position.
fn parse_chapters(root: &impl QueryNodes) -> Result<Vec<Chapter>> {
    let mut chapters = Vec::new();
    let mut keys = std::collections::HashSet::new();
    for (index, item) in root.query("#chapterlist li")?.into_iter().enumerate() {
        let Some(number_text) = attr(&item, "data-num") else {
            continue;
        };
        let number = number_text.parse::<f32>().ok();
        let Some(anchor) = item.query("a")?.into_iter().next() else {
            continue;
        };
        let href = attr(&anchor, "href");
        let url = href
            .clone()
            .filter(|href| links::chapter_path(href).is_some());
        let placeholder = matches!(href.as_deref(), Some("#/" | "#"));
        let locked = placeholder
            || attr(&anchor, "data-bs-target").as_deref() == Some("#lockedChapterModal");
        if url.is_none() && !locked {
            continue;
        }
        let base = match url.as_deref().and_then(links::chapter_path) {
            Some(path) => path,
            None => format!(
                "locked:{}",
                attr(&anchor, "data-id").unwrap_or_else(|| number_text.clone())
            ),
        };
        let key = if keys.insert(base.clone()) {
            base
        } else if url.is_some() {
            continue;
        } else {
            let unique = format!("{base}#{}", index + 1);
            keys.insert(unique.clone());
            unique
        };
        let chapter_title = anchor
            .query(".chapternum")?
            .into_iter()
            .next()
            .and_then(|node| text(&node));
        chapters.push(Chapter {
            key,
            title: chapter_title,
            volume: None,
            chapter: number,
            date_updated: None,
            scanlator: None,
            url,
            lang: Some("en".into()),
            paywalled: Some(locked),
            kind: ContentKind::Images,
        });
    }
    Ok(chapters)
}

fn update(entry: MediaEntry, details: bool, units: bool) -> Result<MediaEntry> {
    let MediaEntry::Manga(mut item) = entry else {
        return Err(Error::Unsupported);
    };
    if !details && !units {
        return Ok(MediaEntry::Manga(item));
    }
    let path = item
        .url
        .as_deref()
        .and_then(links::series_path)
        .ok_or_else(|| Error::Host("Thunder Scans entry has no valid series URL".into()))?;
    let html = request(&path)?;
    let root = document(&html);
    if details {
        if let Some(title) = root
            .query("h1.entry-title")?
            .into_iter()
            .next()
            .and_then(|node| text(&node))
        {
            item.title = title;
        }
        item.cover = root
            .query(".thumb img.wp-post-image")?
            .into_iter()
            .next()
            .and_then(|node| attr(&node, "src"))
            .filter(|url| url.starts_with(IMAGE_PREFIX));
        item.description = root
            .query(".entry-content-single")?
            .into_iter()
            .next()
            .and_then(|node| text(&node));
        let info = root
            .query(".tsinfo")?
            .into_iter()
            .next()
            .and_then(|node| text(&node))
            .unwrap_or_default();
        item.status = ["Ongoing", "Completed", "Hiatus", "Canceled", "Cancelled"]
            .into_iter()
            .find(|candidate| info.contains(candidate))
            .map(status)
            .unwrap_or(Status::Unknown);
        for field in root.query(".tsinfo .imptdt")? {
            let Some(label) = field
                .query("h1")?
                .into_iter()
                .next()
                .and_then(|node| text(&node))
            else {
                continue;
            };
            let value = field
                .query("i")?
                .into_iter()
                .next()
                .and_then(|node| text(&node));
            match label.trim() {
                "Author" => item.authors = value.map(|name| vec![name]),
                "Artist" => item.artist = value,
                _ => {}
            }
        }
        let mut tags = Vec::new();
        for tag in root.query(".genres-container a[rel='tag']")? {
            if let Some(name) = text(&tag) {
                tags.push(name);
            }
        }
        item.tags = (!tags.is_empty()).then_some(tags);
        item.url = Some(format!("{ORIGIN}{path}"));
    }
    if units {
        item.chapters = Some(parse_chapters(&root)?);
    }
    Ok(MediaEntry::Manga(item))
}

fn reader_pages(html: &str, chapter_url: &str) -> Result<Vec<Page>> {
    let path = links::chapter_path(&format!("{ORIGIN}{chapter_url}"))
        .or_else(|| links::chapter_path(chapter_url))
        .ok_or_else(|| Error::Host("Thunder Scans chapter has no valid URL".into()))?;
    let marker = "ts_reader.run(";
    let start = html
        .find(marker)
        .ok_or_else(|| Error::Host("Thunder Scans reader data was missing".into()))?
        + marker.len();
    let end = html[start..]
        .find(");</script>")
        .map(|offset| start + offset)
        .ok_or_else(|| Error::Host("Thunder Scans reader data was malformed".into()))?;
    let data: serde_json::Value = serde_json::from_str(&html[start..end])
        .map_err(|_| Error::Host("Thunder Scans reader data was malformed".into()))?;
    if data.get("is_novel").and_then(serde_json::Value::as_bool) == Some(true) {
        return Err(Error::Unsupported);
    }
    let sources = data
        .get("sources")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| Error::Host("Thunder Scans reader sources were missing".into()))?;
    let default = data
        .get("defaultSource")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default();
    let source = sources
        .iter()
        .find(|source| source.get("source").and_then(serde_json::Value::as_str) == Some(default))
        .or_else(|| sources.first())
        .ok_or(Error::NotFound("Thunder Scans chapter has no pages".into()))?;
    let images = source
        .get("images")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| Error::Host("Thunder Scans reader images were missing".into()))?;
    if images.is_empty() {
        return Err(Error::NotFound("Thunder Scans chapter has no pages".into()));
    }
    images
        .iter()
        .enumerate()
        .map(|(index, image)| {
            let url = image
                .as_str()
                .filter(|url| url.starts_with(IMAGE_PREFIX))
                .ok_or_else(|| Error::Host("Thunder Scans returned an invalid image URL".into()))?;
            Ok(Page {
                index: index as i32,
                content: PageContent::Url(url.to_owned()),
                has_description: false,
                description: None,
                headers: Some(HashMap::from([(
                    "Referer".into(),
                    format!("{ORIGIN}{path}"),
                )])),
            })
        })
        .collect()
}

fn readable_chapter_path(chapter: &Chapter) -> Result<String> {
    if chapter.paywalled == Some(true) {
        return Err(Error::NeedsLogin(
            "This Thunder Scans chapter is locked. Open it on the website to check access.".into(),
        ));
    }
    chapter
        .url
        .as_deref()
        .and_then(links::chapter_path)
        .ok_or_else(|| Error::Host("Thunder Scans chapter has no valid URL".into()))
}

fn chapter_pages(chapter: &Chapter) -> Result<Vec<Page>> {
    let path = readable_chapter_path(chapter)?;
    reader_pages(&request(&path)?, &path)
}

/// A chapter page links back to its series. That link is the authoritative mapping, so the
/// series path is never inferred from the chapter slug.
fn chapter_series_path(html: &str) -> Result<String> {
    let root = document(html);
    root.query(".btn-back-enhanced[href*='/comics/']")?
        .iter()
        .filter_map(|node| attr(node, "href"))
        .find_map(|href| links::series_path(&href))
        .ok_or_else(|| Error::Host("Thunder Scans chapter page has no series link".into()))
}

fn home_components(html: &str) -> Result<Vec<HomeComponent>> {
    let root = document(html);
    let mut components = Vec::new();
    for section in root.query(".bixbox")? {
        let Some(title) = section
            .query(".releases h2")?
            .into_iter()
            .next()
            .and_then(|node| text(&node))
        else {
            continue;
        };
        if !matches!(
            title.as_str(),
            "Popular Today" | "Editor's Pick" | "Latest Comics"
        ) {
            continue;
        }
        let mut entries = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for card in section.query(".bsx")?.into_iter().take(12) {
            if let Some(item) = card_manga(&card)?
                && seen.insert(item.key.clone())
            {
                entries.push(MediaEntry::Manga(item));
            }
        }
        if entries.is_empty() {
            continue;
        }
        // Editor's Pick is a curated subset with no all-series listing of its own; only
        // "Latest Comics" maps to the catalog listing.
        let listing = (title == "Latest Comics").then(|| Listing {
            id: "latest-comics".into(),
            name: title.clone(),
            kind: 0,
        });
        components.push(HomeComponent {
            title: Some(title),
            subtitle: None,
            value: HomeComponentValue::MediaScroller(entries, listing),
        });
    }
    if components.is_empty() {
        return Err(Error::Host(
            "Thunder Scans home sections were not found".into(),
        ));
    }
    Ok(components)
}

impl SourceProvider for ThunderScans {
    const KINDS: &'static [MediaKind] = &[MediaKind::Manga];

    fn get_home() -> Result<HomeLayout> {
        Ok(HomeLayout {
            components: home_components(&request("/")?)?,
        })
    }

    fn get_listing(listing: Listing, page: i32) -> Result<MediaPageResult> {
        if page < 1 {
            return Err(Error::Host("Page numbers start at one".into()));
        }
        match listing.id.as_str() {
            "latest-comics" => {
                let path = if page == 1 {
                    "/project/".to_owned()
                } else {
                    format!("/project/page/{page}/")
                };
                page_result(&path)
            }
            _ => Err(Error::Unsupported),
        }
    }

    fn search(
        query: &str,
        page: i32,
        _kind: Option<MediaKind>,
        _filters: Vec<FilterItem>,
    ) -> Result<MediaPageResult> {
        if page < 1 {
            return Err(Error::Host("Page numbers start at one".into()));
        }
        let path = if page == 1 {
            format!("/?s={}", query_encode(query))
        } else {
            format!("/page/{page}/?s={}", query_encode(query))
        };
        page_result(&path)
    }

    fn get_media_update(entry: MediaEntry, details: bool, units: bool) -> Result<MediaEntry> {
        update(entry, details, units)
    }

    fn get_chapter_pages(_entry: MediaEntry, chapter: Chapter) -> Result<Vec<Page>> {
        chapter_pages(&chapter)
    }

    fn handle_url(url: &str) -> Result<LinkValue> {
        let path = if let Some(path) = links::series_path(url) {
            path
        } else if let Some(chapter) = links::chapter_path(url) {
            chapter_series_path(&request(&chapter)?)?
        } else {
            return Err(Error::Unsupported);
        };
        let key = path
            .trim_matches('/')
            .rsplit('/')
            .next()
            .unwrap_or_default()
            .to_owned();
        let initial = manga(key.clone(), key, None, format!("{ORIGIN}{path}"));
        let MediaEntry::Manga(item) = update(MediaEntry::Manga(initial), true, true)? else {
            return Err(Error::Unsupported);
        };
        Ok(LinkValue::Manga(item))
    }

    fn get_image_origins() -> Result<Vec<String>> {
        Ok(vec![ORIGIN.into()])
    }

    fn get_image_request(url: &str) -> Result<PluginImageRequest> {
        if !url.starts_with(IMAGE_PREFIX) {
            return Err(Error::Unsupported);
        }
        Ok(PluginImageRequest {
            url: url.into(),
            headers: Some(HashMap::from([("Referer".into(), REFERER.into())])),
        })
    }
}

ito_rs::export_plugin!(ThunderScans);
#[cfg(test)]
mod pagination_tests {
    use super::*;

    #[test]
    fn next_page_requires_a_canonical_next_link() {
        assert!(has_next_page(
            r#"<div class="pagination"><a class="next page-numbers" href="https://en-thunderscans.com/project/page/2/">Next</a></div>"#
        ).expect("valid pagination markup"));
        assert!(
            !has_next_page(
                r#"<div class="pagination"><span class="page-numbers current">2</span></div>"#
            )
            .expect("valid final-page markup")
        );
        assert!(
            !has_next_page(
                r#"<a class="next page-numbers" href="https://attacker.example/page/2/">Next</a>"#
            )
            .expect("foreign pagination link is ignored")
        );
    }

    #[test]
    fn malformed_reader_data_is_an_error_not_empty_success() {
        assert!(
            reader_pages(
                "<script>ts_reader.run({broken});</script>",
                "/series-chapter-1/"
            )
            .is_err()
        );
        assert!(reader_pages("<div id='readerarea'></div>", "/series-chapter-1/").is_err());
    }

    #[test]
    fn catalog_cards_dedupe_series_and_drop_foreign_covers() {
        let html = r#"<div class="listupd"><div class="bsx">
            <a href="https://en-thunderscans.com/comics/the-genius/?ref=home" title="The Genius">
              <img src="https://en-thunderscans.com/wp-content/uploads/cover.webp">
            </a>
            <a href="https://en-thunderscans.com/comics/the-genius/">The Genius</a>
          </div>
          <div class="bsx">
            <a href="https://en-thunderscans.com/comics/other/">  Other  <img src="https://evil.example/c.png"></a>
          </div>
          <a href="https://evil.example/comics/foreign/" title="Foreign">x</a></div>"#;
        let entries = card_entries(html).expect("synthetic cards parse");
        assert_eq!(entries.len(), 2);
        let MediaEntry::Manga(first) = &entries[0] else {
            panic!("expected manga");
        };
        assert_eq!(first.key, "the-genius");
        assert_eq!(
            first.cover.as_deref(),
            Some("https://en-thunderscans.com/wp-content/uploads/cover.webp")
        );
        let MediaEntry::Manga(second) = &entries[1] else {
            panic!("expected manga");
        };
        assert_eq!(second.title, "Other");
        assert_eq!(second.cover, None);
    }

    #[test]
    fn catalog_ignores_sidebar_widget_series_links() {
        let html = r#"<aside class="widget"><a href="https://en-thunderscans.com/comics/sidebar-series/" title="Sidebar">Sidebar</a></aside>
          <div class="listupd"><div class="bsx"><a href="https://en-thunderscans.com/comics/real/" title="Real">Real</a></div></div>"#;
        let entries = card_entries(html).expect("synthetic page parses");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].key(), "real");
    }

    #[test]
    fn placeholder_chapter_links_are_locked_without_urls() {
        let root = document(
            r##"<ul id="chapterlist"><li data-num="3"><a href="#/"><span class="chapternum">Chapter 3</span></a></li>
            <li data-num="x"><a href="#"><span class="chapternum">Extra</span></a></li></ul>"##,
        );
        let chapters = parse_chapters(&root).expect("synthetic chapter list parses");
        assert_eq!(chapters[0].key, "locked:3");
        assert_eq!(chapters[0].paywalled, Some(true));
        assert_eq!(chapters[0].url, None);
        assert_eq!(chapters[1].chapter, None);
        assert_eq!(chapters[1].paywalled, Some(true));
    }

    #[test]
    fn unknown_chapter_hrefs_are_skipped_not_unlocked() {
        let root = document(
            r##"<ul id="chapterlist">
                <li data-num="5"><a href="https://evil.example/series-chapter-5/"><span class="chapternum">Chapter 5</span></a></li>
                <li data-num="6"><a href="/comics/series/"><span class="chapternum">Not a chapter</span></a></li>
                <li data-num="7"><a><span class="chapternum">No link</span></a></li>
            </ul>"##,
        );
        let chapters = parse_chapters(&root).expect("synthetic chapter list parses");
        assert!(chapters.is_empty());
    }

    #[test]
    fn duplicate_chapters_collapse_and_locked_rows_keep_unique_keys() {
        let root = document(
            r##"<ul id="chapterlist">
                <li data-num="4"><a href="https://en-thunderscans.com/series-chapter-4/"><span class="chapternum">Chapter 4</span></a></li>
                <li data-num="4"><a href="https://en-thunderscans.com/series-chapter-4/"><span class="chapternum">Chapter 4</span></a></li>
                <li data-num="8"><a data-bs-target="#lockedChapterModal" data-id="77"><span class="chapternum">Chapter 8</span></a></li>
                <li data-num="8"><a data-bs-target="#lockedChapterModal" data-id="77"><span class="chapternum">Chapter 8</span></a></li>
            </ul>"##,
        );
        let chapters = parse_chapters(&root).expect("synthetic chapter list parses");
        let keys: Vec<_> = chapters.iter().map(|c| c.key.as_str()).collect();
        assert_eq!(keys, vec!["/series-chapter-4/", "locked:77", "locked:77#4"]);
    }

    #[test]
    fn search_without_matches_is_empty_but_unknown_pages_fail() {
        let none = parse_catalog_page(
            r#"<div class="bixbox"><div class="listupd"><center><h3>Not Found</h3></center></div></div>"#,
        )
        .expect("the site's no-results page is an empty result");
        assert!(none.entries.is_empty());
        assert!(!none.has_next_page);
        assert!(parse_catalog_page("<html><body><h1>Maintenance</h1></body></html>").is_err());
    }
    #[test]
    fn next_page_links_with_query_dots_are_canonical() {
        assert!(has_next_page(
            r#"<a class="next page-numbers" href="https://en-thunderscans.com/project/page/2/?s=a..b">Next</a>"#
        )
        .expect("valid pagination markup"));
    }
    #[test]
    fn search_terms_with_dots_build_request_paths_the_client_accepts() {
        let path = format!("/?s={}", query_encode("Wait..."));
        assert_eq!(path, "/?s=Wait%2E%2E%2E");
        assert!(links::absolute_url(&path).is_some());
    }
    #[test]
    fn challenge_title_must_start_with_the_phrase() {
        assert!(
            !is_challenge_page("<title>Notes</title><p>Just a moment</p>").expect("valid html")
        );
        assert!(is_challenge_page("<title>Just a moment...</title>").expect("valid html"));
    }
}
