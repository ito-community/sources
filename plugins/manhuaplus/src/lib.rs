//! ManhuaPlus catalog and reader for the original .com site.
#[cfg(target_arch = "wasm32")]
use ito_rs::export_plugin;
use ito_rs::{
    Error, Result,
    models::manga::{ContentRating, Manga, Status, Viewer},
    models::*,
    net::Request,
    provider::SourceProvider,
};
use scraper::{ElementRef, Html, Selector};
use std::collections::{HashMap, HashSet};

const ORIGIN: &str = "https://manhuaplus.com";
const CDN: &str = "https://cdn.manhuaplus.com";
const REFERER: &str = "https://manhuaplus.com/";
const USER_AGENT: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0.0.0 Safari/537.36";
fn image_headers(referer: &str) -> HashMap<String, String> {
    HashMap::from([
        ("Referer".into(), referer.into()),
        ("User-Agent".into(), USER_AGENT.into()),
        (
            "Accept".into(),
            "image/avif,image/webp,image/apng,image/svg+xml,image/*,*/*;q=0.8".into(),
        ),
    ])
}
/// Catalog and reader provider for ManhuaPlus.
pub struct ManhuaPlus;

fn selector(value: &str) -> Result<Selector> {
    Selector::parse(value).map_err(|_| Error::Host("Invalid ManhuaPlus selector".into()))
}
fn text(node: ElementRef<'_>) -> String {
    node.text()
        .collect::<Vec<_>>()
        .join(" ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}
fn first_text(root: &Html, css: &str) -> Result<Option<String>> {
    Ok(root
        .select(&selector(css)?)
        .map(text)
        .find(|s| !s.is_empty()))
}
fn site_path(url: &str) -> Option<&str> {
    let path = if url.starts_with('/') && !url.starts_with("//") {
        url
    } else {
        url.strip_prefix(ORIGIN)
            .or_else(|| url.strip_prefix("https://www.manhuaplus.com"))?
    };
    if !path.starts_with('/') {
        return None;
    }
    Some(path.split(['?', '#']).next().unwrap_or(path))
}
fn slug(value: &str) -> bool {
    !value.is_empty()
        && value
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_')
}
fn series_path(url: &str) -> Option<String> {
    let value = site_path(url)?
        .strip_prefix("/manga/")?
        .trim_end_matches('/');
    (slug(value) && value != "page").then(|| format!("/manga/{value}/"))
}
fn chapter_path(url: &str) -> Option<String> {
    let value = site_path(url)?
        .strip_prefix("/manga/")?
        .trim_end_matches('/');
    let (series, chapter) = value.split_once('/')?;
    (slug(series) && slug(chapter) && chapter.starts_with("chapter-"))
        .then(|| format!("/manga/{series}/{chapter}/"))
}
fn image_url(value: &str) -> Option<String> {
    let value = value.trim();
    let url = if value.starts_with("//") {
        format!("https:{value}")
    } else if value.starts_with("/wp-content/uploads/") {
        format!("{ORIGIN}{value}")
    } else {
        value.to_owned()
    };
    if url.starts_with(&format!("{ORIGIN}/wp-content/uploads/"))
        || url.starts_with(&format!("{CDN}/"))
    {
        Some(url)
    } else {
        None
    }
}
fn image(node: ElementRef<'_>) -> Option<String> {
    ["data-src", "data-lazy-src", "src"]
        .into_iter()
        .find_map(|key| node.value().attr(key).and_then(image_url))
}
fn check_response(status: u16, html: &str) -> Result<()> {
    // These are challenge-page markers, not ordinary embedded captcha widgets.
    if html.contains("cf-chl-")
        || html.contains("<title>Just a moment")
        || html.contains("<title>Attention Required!")
    {
        return Err(Error::NeedsLogin(
            "Open ManhuaPlus in the source browser to complete site verification".into(),
        ));
    }
    match status {
        200..=299 => Ok(()),
        401 => Err(Error::NeedsLogin(
            "Sign in to ManhuaPlus in the source browser".into(),
        )),
        403 => Err(Error::Server("ManhuaPlus denied the request".into())),
        404 => Err(Error::NotFound("ManhuaPlus page was not found".into())),
        _ => Err(Error::Server(format!("ManhuaPlus returned HTTP {status}"))),
    }
}
fn request(path: &str) -> Result<String> {
    if !path.starts_with('/') || path.starts_with("//") {
        return Err(Error::Unsupported);
    }
    let mut req = Request::get(format!("{ORIGIN}{path}"));
    req.header("Referer", REFERER);
    let response = req.send()?;
    let html = String::from_utf8(response.body)
        .map_err(|_| Error::Host("ManhuaPlus returned invalid HTML".into()))?;
    check_response(response.status as u16, &html)?;
    Ok(html)
}
fn manga(path: &str, title: String, cover: Option<String>) -> Manga {
    Manga {
        key: path
            .trim_matches('/')
            .rsplit('/')
            .next()
            .unwrap_or_default()
            .into(),
        title,
        authors: None,
        artist: None,
        description: None,
        tags: None,
        cover,
        url: Some(format!("{ORIGIN}{path}")),
        status: Status::Unknown,
        content_rating: ContentRating::Suggestive,
        nsfw: 0,
        viewer: Viewer::Webtoon,
        chapters: None,
    }
}
fn entries(root: &Html) -> Result<Vec<MediaEntry>> {
    let cards = selector(".page-item-detail, .row.c-tabs-item__content")?;
    let links = selector(".item-thumb a[href], .tab-thumb a[href], .post-title a[href]")?;
    let images = selector("img")?;
    let titles = selector(".post-title a")?;
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    for card in root.select(&cards) {
        let Some((a, path)) = card
            .select(&links)
            .find_map(|a| Some((a, series_path(a.value().attr("href")?)?)))
        else {
            continue;
        };
        if !seen.insert(path.clone()) {
            continue;
        }
        let title = card
            .select(&titles)
            .map(text)
            .find(|s| !s.is_empty())
            .or_else(|| a.value().attr("title").map(str::to_owned))
            .unwrap_or_else(|| text(a));
        if title.is_empty() {
            continue;
        }
        out.push(MediaEntry::Manga(manga(
            &path,
            title,
            card.select(&images).find_map(image),
        )));
    }
    Ok(out)
}
fn parse_listing(html: &str, search: bool) -> Result<MediaPageResult> {
    check_response(200, html)?;
    let root = Html::parse_document(html);
    // Search pages have sidebar recommendations which are not search results.
    let scoped = if search {
        root.select(&selector(".search-wrap")?)
            .next()
            .map(|n| Html::parse_fragment(&n.inner_html()))
    } else {
        None
    };
    let result_root = scoped.as_ref().unwrap_or(&root);
    let entries = entries(result_root)?;
    let empty_search = search
        && root
            .select(&selector(".search-no-results, .search-wrap.no-results")?)
            .next()
            .is_some();
    if entries.is_empty() && !empty_search {
        return Err(Error::Html(
            "ManhuaPlus catalog markup was not found".into(),
        ));
    }
    let has_next_page = root
        .select(&selector(".wp-pagenavi a.nextpostslink[rel='next']")?)
        .next()
        .is_some();
    Ok(MediaPageResult {
        entries,
        has_next_page,
    })
}
fn encode(value: &str) -> String {
    let mut out = String::new();
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || b"-_.~".contains(&byte) {
            out.push(char::from(byte));
        } else {
            use std::fmt::Write;
            let _ = write!(out, "%{byte:02X}");
        }
    }
    out
}
fn listing_path(page: i32, query: Option<&str>) -> Result<String> {
    if page < 1 {
        return Err(Error::Host("Page numbers start at one".into()));
    }
    Ok(match query {
        Some(query) if page == 1 => format!("/?s={}&post_type=wp-manga", encode(query)),
        Some(query) => format!("/page/{page}/?s={}&post_type=wp-manga", encode(query)),
        None if page == 1 => "/manga/".into(),
        None => format!("/manga/page/{page}/"),
    })
}
fn chapter_number(label: &str, path: &str) -> Option<f32> {
    let lower = label.to_ascii_lowercase();
    let rest = lower
        .find("chapter")
        .map(|i| lower[i + 7..].trim_start_matches([' ', ':', '#']));
    let from_label = rest.and_then(|s| {
        let end = s
            .find(|c: char| !c.is_ascii_digit() && c != '.')
            .unwrap_or(s.len());
        s[..end].parse::<f32>().ok()
    });
    from_label
        .or_else(|| {
            path.split("/chapter-")
                .nth(1)?
                .trim_end_matches('/')
                .replace('-', ".")
                .parse()
                .ok()
        })
        .filter(|n| n.is_finite() && *n >= 0.0)
}
fn chapters(root: &Html) -> Result<Vec<Chapter>> {
    let mut chapters = Vec::new();
    let mut seen = HashSet::new();
    let anchors = selector("a[href]")?;
    let locks = selector(".chapter-lock, .chapter-locked, .fa-lock, .ion-locked")?;
    for row in root.select(&selector(
        ".listing-chapters_wrap ul.main.version-chap li.wp-manga-chapter",
    )?) {
        let Some((anchor, path)) = row
            .select(&anchors)
            .find_map(|a| Some((a, chapter_path(a.value().attr("href")?)?)))
        else {
            continue;
        };
        if !seen.insert(path.clone()) {
            continue;
        }
        let title = text(anchor);
        let locked = row.select(&locks).next().is_some()
            || row
                .value()
                .classes()
                .any(|c| matches!(c, "locked" | "chapter-locked"))
            || row.value().attr("data-locked") == Some("true");
        chapters.push(Chapter {
            key: path.clone(),
            title: Some(title.clone()),
            volume: None,
            chapter: chapter_number(&title, &path),
            date_updated: None,
            scanlator: None,
            url: Some(format!("{ORIGIN}{path}")),
            lang: Some("en".into()),
            paywalled: Some(locked),
            kind: ContentKind::Images,
        });
    }
    if chapters.is_empty() {
        return Err(Error::Html("ManhuaPlus chapter list was not found".into()));
    }
    // Publication order occasionally puts a numbered chapter at the bottom of the list.
    chapters.reverse();
    chapters.sort_by(|a, b| match (a.chapter, b.chapter) {
        (Some(a), Some(b)) => a.total_cmp(&b),
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => std::cmp::Ordering::Equal,
    });
    Ok(chapters)
}
fn parse_update(mut m: Manga, html: &str, details: bool, units: bool) -> Result<Manga> {
    check_response(200, html)?;
    let root = Html::parse_document(html);
    if details {
        m.title = first_text(&root, ".post-title h1, .post-title h2, h1")?
            .ok_or_else(|| Error::Html("ManhuaPlus series title was not found".into()))?;
        m.cover = root
            .select(&selector(".summary_image img")?)
            .find_map(image);
        m.description = first_text(&root, ".description-summary .summary__content")?;
        m.tags = Some(
            root.select(&selector(".genres-content a")?)
                .map(text)
                .filter(|s| !s.is_empty())
                .collect(),
        );
        let authors: Vec<_> = root
            .select(&selector(".author-content a")?)
            .map(text)
            .filter(|s| !s.is_empty())
            .collect();
        m.authors = (!authors.is_empty()).then_some(authors);
        m.artist = first_text(&root, ".artist-content")?;
        for row in root.select(&selector(".post-content_item")?) {
            let heading = row
                .select(&selector(".summary-heading")?)
                .map(text)
                .collect::<String>()
                .to_lowercase();
            if heading.contains("status") {
                let value = row
                    .select(&selector(".summary-content")?)
                    .map(text)
                    .collect::<String>()
                    .to_lowercase();
                m.status = if value.contains("completed") {
                    Status::Completed
                } else if value.contains("ongoing") {
                    Status::Ongoing
                } else {
                    Status::Unknown
                };
            }
        }
    }
    if units {
        m.chapters = Some(chapters(&root)?);
    }
    Ok(m)
}
fn update(entry: MediaEntry, details: bool, units: bool) -> Result<MediaEntry> {
    let MediaEntry::Manga(m) = entry else {
        return Err(Error::Unsupported);
    };
    if !details && !units {
        return Ok(MediaEntry::Manga(m));
    }
    let path = m
        .url
        .as_deref()
        .and_then(series_path)
        .or_else(|| series_path(&format!("/manga/{}/", m.key)))
        .ok_or(Error::Unsupported)?;
    parse_update(m, &request(&path)?, details, units).map(MediaEntry::Manga)
}
fn reader_pages(html: &str, path: &str) -> Result<Vec<Page>> {
    check_response(200, html)?;
    let root = Html::parse_document(html);
    let content = root
        .select(&selector(".reading-content")?)
        .next()
        .ok_or_else(|| Error::Html("ManhuaPlus reader content was not found".into()))?;
    // Lock markers only count inside the reader; page chrome can carry unrelated widgets.
    if content
        .select(&selector(
            ".chapter-locked, .chapter-lock, .wp-manga-chapter-paywall",
        )?)
        .next()
        .is_some()
    {
        return Err(Error::NeedsLogin(
            "This ManhuaPlus chapter requires signing in on the website".into(),
        ));
    }
    let mut pages = Vec::new();
    let mut seen = HashSet::new();
    for node in content.select(&selector("img")?) {
        // Embedded advertising is not a reader page, even when it uses the site's CDN.
        if node.ancestors().filter_map(ElementRef::wrap).any(|a| {
            a.value()
                .classes()
                .any(|c| matches!(c, "ads" | "adsbygoogle" | "ad-container" | "code-block"))
        }) {
            continue;
        }
        let Some(url) = image(node) else {
            continue;
        };
        if !seen.insert(url.clone()) {
            continue;
        }
        let referer = if url.starts_with(&format!("{CDN}/")) {
            REFERER.to_owned()
        } else {
            format!("{ORIGIN}{path}")
        };
        pages.push(Page {
            index: pages.len() as i32,
            content: PageContent::Url(url),
            has_description: false,
            description: None,
            headers: Some(image_headers(&referer)),
        });
    }
    // Prose is only a fallback for a chapter with no reader image at all. Image chapters share
    // the same wrapper class, so their text is never used in place of invalid images.
    if pages.is_empty() {
        if content.select(&selector("img")?).next().is_some() {
            return Err(Error::Html(
                "ManhuaPlus reader images were not valid".into(),
            ));
        }
        for paragraph in content.select(&selector(".text-left p, .text-right p, p")?) {
            if paragraph.ancestors().filter_map(ElementRef::wrap).any(|a| {
                a.value()
                    .classes()
                    .any(|c| matches!(c, "ads" | "ad-container" | "code-block"))
            }) {
                continue;
            }
            let value = text(paragraph);
            if !value.is_empty() {
                pages.push(Page {
                    index: pages.len() as i32,
                    content: PageContent::Text(value),
                    has_description: false,
                    description: None,
                    headers: None,
                });
            }
        }
    }
    if pages.is_empty() {
        return Err(Error::Html(
            "ManhuaPlus chapter pages were not found".into(),
        ));
    }
    Ok(pages)
}
fn browse_listing() -> Listing {
    Listing {
        id: "manga".into(),
        name: "All manga".into(),
        kind: 0,
    }
}
impl SourceProvider for ManhuaPlus {
    const KINDS: &'static [MediaKind] = &[MediaKind::Manga];
    fn get_home() -> Result<HomeLayout> {
        let root = Html::parse_document(&request("/")?);
        let items = entries(&root)?;
        if items.is_empty() {
            return Err(Error::Html("ManhuaPlus home manga were not found".into()));
        }
        Ok(HomeLayout {
            components: vec![HomeComponent {
                title: Some("Latest manga".into()),
                subtitle: None,
                value: HomeComponentValue::MediaScroller(items, Some(browse_listing())),
            }],
        })
    }
    fn get_listing(listing: Listing, page: i32) -> Result<MediaPageResult> {
        if listing.id != "manga" && listing.id != "/manga/" {
            return Err(Error::Unsupported);
        }
        parse_listing(&request(&listing_path(page, None)?)?, false)
    }
    fn search(
        query: &str,
        page: i32,
        kind: Option<MediaKind>,
        filters: Vec<FilterItem>,
    ) -> Result<MediaPageResult> {
        if kind.is_some_and(|k| k != MediaKind::Manga) || !filters.is_empty() {
            return Err(Error::Unsupported);
        }
        parse_listing(&request(&listing_path(page, Some(query))?)?, true)
    }
    fn get_media_update(entry: MediaEntry, details: bool, units: bool) -> Result<MediaEntry> {
        update(entry, details, units)
    }
    fn get_chapter_pages(_entry: MediaEntry, chapter: Chapter) -> Result<Vec<Page>> {
        if chapter.paywalled == Some(true) {
            return Err(Error::NeedsLogin(
                "This ManhuaPlus chapter requires signing in on the website".into(),
            ));
        }
        let path = chapter
            .url
            .as_deref()
            .and_then(chapter_path)
            .or_else(|| chapter_path(&chapter.key))
            .ok_or(Error::Unsupported)?;
        reader_pages(&request(&path)?, &path)
    }
    fn handle_url(url: &str) -> Result<LinkValue> {
        if site_path(url) == Some("/manga/") {
            return Ok(LinkValue::Listing(browse_listing()));
        }
        let path = series_path(url)
            .or_else(|| {
                chapter_path(url)
                    .and_then(|p| p.rsplit_once("/chapter-").map(|(s, _)| format!("{s}/")))
            })
            .ok_or(Error::Unsupported)?;
        let entry = update(
            MediaEntry::Manga(manga(&path, String::new(), None)),
            true,
            true,
        )?;
        match entry {
            MediaEntry::Manga(m) => Ok(LinkValue::Manga(m)),
            _ => Err(Error::Unsupported),
        }
    }
    fn get_image_origins() -> Result<Vec<String>> {
        Ok(vec![ORIGIN.into(), CDN.into()])
    }
    fn get_image_request(url: &str) -> Result<PluginImageRequest> {
        let url = image_url(url).ok_or(Error::Unsupported)?;
        Ok(PluginImageRequest {
            url,
            headers: Some(image_headers(REFERER)),
        })
    }
}
#[cfg(target_arch = "wasm32")]
export_plugin!(ManhuaPlus);

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn strict_identity_and_pagination() {
        assert_eq!(
            series_path("https://manhuaplus.com/manga/example/?x=1#read").as_deref(),
            Some("/manga/example/")
        );
        assert!(series_path("https://manhuaplus.com.evil/manga/example/").is_none());
        assert!(chapter_path("https://manhuaplus.org/manga/example/chapter-1/").is_none());
        assert_eq!(
            chapter_path("/manga/example/chapter-536-1/#reader").as_deref(),
            Some("/manga/example/chapter-536-1/")
        );
        assert_eq!(listing_path(2, None).unwrap(), "/manga/page/2/");
        assert_eq!(
            listing_path(3, Some("a & b")).unwrap(),
            "/page/3/?s=a%20%26%20b&post_type=wp-manga"
        );
        assert!(listing_path(0, None).is_err());
    }
    #[test]
    fn complete_chapters_reverse_deduplicate_and_keep_fractional_zero() {
        let root = Html::parse_document(
            r#"<div class="listing-chapters_wrap"><ul class="main version-chap">
          <li class="wp-manga-chapter"><a href="/manga/example/chapter-536-1/">Chapter536.1</a></li>
          <li class="wp-manga-chapter"><a href="/manga/example/chapter-2/">Chapter 2</a></li>
          <li class="wp-manga-chapter"><a href="/manga/example/chapter-2/">Chapter 2</a></li>
          <li class="wp-manga-chapter"><a href="javascript:void(0)">Bad link</a></li>
          <li class="wp-manga-chapter locked"><a href="/manga/example/chapter-0/">Chapter 0</a></li>
          </ul></div>"#,
        );
        let rows = chapters(&root).unwrap();
        assert_eq!(
            rows.iter().map(|c| c.chapter).collect::<Vec<_>>(),
            [Some(0.0), Some(2.0), Some(536.1)]
        );
        assert_eq!(rows[0].paywalled, Some(true));
        assert_eq!(rows[1].paywalled, Some(false));
        assert_eq!(chapter_number("Chapter 0.5: Opening", "/bad"), Some(0.5));
    }
    #[test]
    fn reader_accepts_cdn_despite_text_wrapper_and_excludes_ads() {
        let html = r#"<div class="read-container"><img src="https://cdn.manhuaplus.com/ad.jpg">
        <div class="reading-content"><div class="text-left"><p><img src="https://cdn.manhuaplus.com/2026/01/a.jpg"></p>
        <div class="code-block"><img src="https://cdn.manhuaplus.com/ad2.jpg"></div>
        <img data-src="https://manhuaplus.com/wp-content/uploads/b.jpg" src="placeholder">
        <img src="https://cdn.manhuaplus.com/2026/01/a.jpg"><img src="https://advert.invalid/a.jpg"></div></div></div>"#;
        let pages = reader_pages(html, "/manga/example/chapter-1/").unwrap();
        assert_eq!(pages.len(), 2);
        assert_eq!(pages.iter().map(|p| p.index).collect::<Vec<_>>(), [0, 1]);
        assert!(matches!(&pages[0].content, PageContent::Url(url) if url.ends_with("/a.jpg")));
        assert_eq!(pages[0].headers.as_ref().unwrap()["Referer"], REFERER);
        assert!(image_url("https://cdn.manhuaplus.com.evil/a.jpg").is_none());
        assert!(image_url("https://manhuaplus.com/logo.jpg").is_none());
    }
    #[test]
    fn text_and_blocked_pages_are_not_empty_success() {
        let pages = reader_pages("<div class='reading-content'><p>First &amp; paragraph.</p><p>Second paragraph.</p></div>", "/manga/example/chapter-1/").unwrap();
        assert!(matches!(&pages[0].content, PageContent::Text(t) if t == "First & paragraph."));
        assert!(reader_pages("<div class='reading-content'></div>", "/").is_err());
        assert!(reader_pages("<div class='reading-content'><p>Caption.</p><img src='https://advert.invalid/a.jpg'></div>", "/").is_err());
        assert!(matches!(
            check_response(403, "<title>Just a moment</title><div id='cf-chl-test'>"),
            Err(Error::NeedsLogin(_))
        ));
        assert!(matches!(
            check_response(403, "Forbidden"),
            Err(Error::Server(_))
        ));
        assert!(matches!(
            reader_pages(
                "<div class='reading-content'><div class='chapter-locked'></div></div>",
                "/"
            ),
            Err(Error::NeedsLogin(_))
        ));
        assert!(
            reader_pages("<div class='chapter-locked'></div><div class='reading-content'><img src='https://cdn.manhuaplus.com/2026/01/a.jpg'></div>", "/manga/example/chapter-1/").is_ok()
        );
    }
    #[test]
    fn search_empty_and_catalog_next_link() {
        let result = r#"<div class="search-wrap"><div class="row c-tabs-item__content"><div class="tab-thumb"><a href="/manga/found/"><img data-src="/wp-content/uploads/cover.jpg"></a></div><div class="post-title"><a href="/manga/found/">Found</a></div></div></div>"#;
        let result_page = parse_listing(result, true).unwrap();
        assert_eq!(result_page.entries[0].key(), "found");
        let card = r#"<div class="page-item-detail"><div class="item-thumb"><a href="/manga/example/" title="Example"><img data-src="/wp-content/uploads/cover.jpg"></a></div></div>"#;
        let html = format!(
            "{card}<div class='wp-pagenavi'><a class='nextpostslink' rel='next' href='/manga/page/2/'>Next</a></div>"
        );
        let page = parse_listing(&html, false).unwrap();
        assert_eq!(page.entries.len(), 1);
        assert!(page.has_next_page);
        let html = format!(
            "<body class='search-no-results'>{card}<div class='search-wrap no-results not-found'></div></body>"
        );
        let page = parse_listing(&html, true).unwrap();
        assert!(page.entries.is_empty());
        assert!(!page.has_next_page);
        assert!(parse_listing("<html>Unavailable</html>", true).is_err());
    }
    #[test]
    fn complete_large_list_has_no_preview_limit() {
        let mut html =
            String::from("<div class='listing-chapters_wrap'><ul class='main version-chap'>");
        for number in (0..3400).rev() {
            use std::fmt::Write;
            write!(html, "<li class='wp-manga-chapter'><a href='/manga/example/chapter-{number}/'>Chapter {number}</a></li>").unwrap();
        }
        html.push_str("</ul></div>");
        let rows = chapters(&Html::parse_document(&html)).unwrap();
        assert_eq!(rows.len(), 3400);
        assert_eq!(rows[0].chapter, Some(0.0));
        assert_eq!(rows[3399].chapter, Some(3399.0));
    }
    #[test]
    fn native_image_policy_is_fixed_and_rejects_foreign_origins() {
        assert_eq!(ManhuaPlus::get_image_origins().unwrap(), [ORIGIN, CDN]);
        let request =
            ManhuaPlus::get_image_request("https://cdn.manhuaplus.com/2026/01/synthetic.jpg")
                .unwrap();
        let headers = request.headers.unwrap();
        assert_eq!(headers["Referer"], REFERER);
        assert_eq!(headers["User-Agent"], USER_AGENT);
        assert!(
            ManhuaPlus::get_image_request("https://cdn.manhuaplus.com.evil/synthetic.jpg").is_err()
        );
    }
    #[test]
    fn detail_metadata_and_out_of_order_chapters() {
        let html = r#"<div class="post-title"><h1>Example</h1></div>
          <div class="summary_image"><img data-src="/wp-content/uploads/cover.jpg"></div>
          <div class="author-content"><a>Writer</a></div><div class="artist-content"><a>Artist</a></div>
          <div class="description-summary"><div class="summary__content">Synthetic description.</div></div>
          <div class="genres-content"><a>Action</a></div>
          <div class="post-content_item"><div class="summary-heading">Status</div><div class="summary-content">Completed</div></div>
          <div class="listing-chapters_wrap"><ul class="main version-chap">
          <li class="wp-manga-chapter"><a href="/manga/example/chapter-3/">Chapter 3</a></li>
          <li class="wp-manga-chapter"><a href="/manga/example/chapter-1/">Chapter 1</a></li>
          <li class="wp-manga-chapter"><a href="/manga/example/chapter-2/">Chapter 2</a></li></ul></div>"#;
        let m = parse_update(
            manga("/manga/example/", String::new(), None),
            html,
            true,
            true,
        )
        .unwrap();
        assert_eq!(m.title, "Example");
        assert_eq!(m.authors.unwrap(), ["Writer"]);
        assert_eq!(m.artist.as_deref(), Some("Artist"));
        assert_eq!(m.description.as_deref(), Some("Synthetic description."));
        assert!(matches!(m.status, Status::Completed));
        assert_eq!(
            m.chapters
                .unwrap()
                .iter()
                .map(|c| c.chapter)
                .collect::<Vec<_>>(),
            [Some(1.0), Some(2.0), Some(3.0)]
        );
    }
}
