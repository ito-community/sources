//! RizzFables complete catalog and public comic reader provider.
//!
//! Keyword matching and pagination operate on the site's complete catalog, not
//! its short autocomplete results. The icon intentionally renders the site's
//! textual identity; it is not an official graphical logo.

use std::collections::{HashMap, HashSet};

use ito_rs::controls::{ChoiceOption, Control, ControlSection, OptionSource, SearchCapabilities};
use ito_rs::models::manga::{ContentRating, Manga, Status, Viewer};
use ito_rs::models::{
    Chapter, ContentKind, FilterItem, FilterValue, HomeComponent, HomeComponentValue, HomeLayout,
    LinkValue, Listing, MediaEntry, MediaKind, MediaPageResult, Page, PageContent,
    PluginImageRequest,
};
use ito_rs::net::Request;
use ito_rs::provider::SourceProvider;
use ito_rs::{Error, Result};
use scraper::{ElementRef, Html, Selector};
use url::Url;

const ORIGIN: &str = "https://rizzfables.com";
const REFERER: &str = "https://rizzfables.com/";
const IMAGE_HOSTS: &[&str] = &[
    "https://rizzfables.com",
    "https://cdn.rizzfables.com",
    "https://cdn.55779955.xyz",
];
const PAGE_SIZE: usize = 20;

// Keeping HTML parsing local also lets native tests exercise the actual parsers.
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
fn document(html: &str) -> Html {
    Html::parse_document(html)
}
fn attr(node: &ElementRef<'_>, name: &str) -> Option<String> {
    node.value().attr(name).map(str::to_owned)
}
fn text(node: &ElementRef<'_>) -> Option<String> {
    let value = node
        .text()
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    (!value.is_empty()).then_some(value)
}
fn access_check(html: &str) -> Result<()> {
    let root = document(html);
    // Challenge phrases count only in the document title or alongside Cloudflare markup.
    // Page bodies can carry the same words as ordinary copy, so they are not evidence.
    let challenge_title = root.query("title")?.iter().filter_map(text).any(|title| {
        let title = title.to_ascii_lowercase();
        [
            "just a moment",
            "verify you are human",
            "checking your browser",
        ]
        .iter()
        .any(|phrase| title.trim_start().starts_with(phrase))
    });
    if html.contains("cf-chl-") || challenge_title {
        return Err(Error::NeedsLogin(
            "Open RizzFables in the browser and complete the site's verification".into(),
        ));
    }
    // The ordinary header includes a sign-in dialog on every public page. It is not a gate.
    for gate in root.query("#readerarea, .reader-error, .chapter-error, .access-denied")? {
        let message = text(&gate).unwrap_or_default().to_ascii_lowercase();
        if message.contains("login to read")
            || message.contains("log in to read")
            || message.contains("sign in to read")
        {
            return Err(Error::NeedsLogin(
                "Sign in on RizzFables to read this chapter".into(),
            ));
        }
        if message.contains("purchase this chapter")
            || message.contains("unlock this chapter")
            || message.contains("access denied")
        {
            return Err(Error::NeedsLogin(
                "Open RizzFables in the browser to check access to this chapter".into(),
            ));
        }
    }
    Ok(())
}
fn response_text(status: i32, body: Vec<u8>) -> Result<String> {
    let html = String::from_utf8(body)
        .map_err(|_| Error::Host("RizzFables returned invalid text".into()))?;
    access_check(&html)?;
    match status {
        200..=299 => Ok(html),
        401 => Err(Error::NeedsLogin(
            "Sign in on RizzFables to continue".into(),
        )),
        403 => Err(Error::Server("RizzFables denied the request".into())),
        404 => Err(Error::NotFound("RizzFables page was not found".into())),
        status => Err(Error::Server(format!("RizzFables returned HTTP {status}"))),
    }
}
fn request(path: &str) -> Result<String> {
    if !path.starts_with('/') || path.starts_with("//") || path.contains("..") {
        return Err(Error::Unsupported);
    }
    let mut req = Request::get(format!("{ORIGIN}{path}"));
    req.header("Referer", REFERER);
    let response = req.send()?;
    response_text(response.status, response.body)
}
fn post_json(body: &str) -> Result<serde_json::Value> {
    let mut req = Request::post(format!("{ORIGIN}/Index/filter_series"));
    req.header("Referer", "https://rizzfables.com/series");
    req.header("Origin", ORIGIN);
    req.header("X-Requested-With", "XMLHttpRequest");
    req.header(
        "Content-Type",
        "application/x-www-form-urlencoded; charset=UTF-8",
    );
    let response = req.body(body.as_bytes()).send()?;
    let text = response_text(response.status, response.body)?;
    serde_json::from_str(&text)
        .map_err(|_| Error::Host("RizzFables returned invalid catalog JSON".into()))
}
fn canonical_path(url: &str) -> Option<String> {
    let path = if url.starts_with('/') {
        url.split(['?', '#']).next()?.to_owned()
    } else {
        absolute_path(url)?
    };
    (path.starts_with('/')
        && !path.starts_with("//")
        && !path.contains("..")
        && !path.contains(['\\', '%']))
    .then(|| path.trim_end_matches('/').to_owned())
}
/// Path of an absolute `http(s)` URL on a RizzFables host. Userinfo and explicit ports are
/// rejected, so look-alike authorities cannot pass as the site.
fn absolute_path(url: &str) -> Option<String> {
    let authority = url.split_once("://")?.1.split(['/', '?', '#']).next()?;
    if authority.contains([':', '@']) {
        return None;
    }
    let parsed = Url::parse(url).ok()?;
    let on_site = matches!(
        parsed.host_str(),
        Some("rizzfables.com" | "www.rizzfables.com")
    );
    (on_site && matches!(parsed.scheme(), "https" | "http")).then(|| parsed.path().to_owned())
}
fn series_path(url: &str) -> Option<String> {
    let path = canonical_path(url)?;
    let slug = path.strip_prefix("/series/")?;
    (!slug.is_empty() && !slug.contains('/')).then_some(path)
}
fn chapter_path(url: &str) -> Option<String> {
    let path = canonical_path(url)?;
    let slug = path.strip_prefix("/chapter/")?;
    (slug.contains("-chapter-") && !slug.contains('/')).then_some(path)
}
fn media_path(item: &Manga) -> Option<String> {
    item.url
        .as_deref()
        .and_then(series_path)
        .or_else(|| series_path(&format!("/{}", item.key.trim_start_matches('/'))))
}
fn image_url(url: &str) -> Option<String> {
    let absolute = if url.starts_with("/assets/images/") {
        format!("{ORIGIN}{url}")
    } else {
        url.to_owned()
    };
    if absolute.contains(['\\', '\r', '\n']) || absolute.contains("/../") {
        return None;
    }
    (absolute.starts_with("https://rizzfables.com/assets/images/")
        || absolute.starts_with("https://cdn.rizzfables.com/wp-content/uploads/")
        || absolute.starts_with("https://cdn.55779955.xyz/file/"))
    .then_some(absolute)
}
fn manga(path: &str, title: String, cover: Option<String>) -> Manga {
    Manga {
        key: path.trim_matches('/').to_owned(),
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
fn cards(root: &impl QueryNodes) -> Result<Vec<MediaEntry>> {
    let mut seen = HashSet::new();
    let mut entries = Vec::new();
    for anchor in root.query("a[href*='/series/']")? {
        let Some(path) = attr(&anchor, "href").and_then(|href| series_path(&href)) else {
            continue;
        };
        let title = attr(&anchor, "title")
            .or_else(|| anchor.query(".tt").ok()?.first().and_then(text))
            .or_else(|| text(&anchor))
            .unwrap_or_default();
        if title.is_empty() || !seen.insert(path.clone()) {
            continue;
        }
        let cover = anchor
            .query("img")?
            .first()
            .and_then(|img| attr(img, "data-src").or_else(|| attr(img, "src")))
            .and_then(|src| image_url(&src));
        entries.push(MediaEntry::Manga(manga(&path, title, cover)));
    }
    Ok(entries)
}
fn complete_catalog(html: &str) -> Result<Vec<MediaEntry>> {
    access_check(html)?;
    let root = document(html);
    // The site renders its entire alphabetic catalog in #set_bs, with no active pager.
    // Refuse a future paginated layout rather than presenting page one as global search.
    if !root.query(".pagination a[href], a[rel='next']")?.is_empty() {
        return Err(Error::Host(
            "RizzFables changed its complete catalog layout".into(),
        ));
    }
    let container = root
        .query("#set_bs")?
        .into_iter()
        .next()
        .ok_or_else(|| Error::Host("RizzFables catalog was missing".into()))?;
    let entries = cards(&container)?;
    if entries.is_empty() {
        return Err(Error::Host("RizzFables catalog was empty".into()));
    }
    Ok(entries)
}
fn filter_text(filter: &FilterItem) -> Result<&str> {
    match &filter.value {
        FilterValue::String(value) => Ok(value),
        _ => Err(Error::Unsupported),
    }
}
fn catalog_filter_body(filters: &[FilterItem]) -> Result<String> {
    let mut genres = Vec::new();
    let mut status = "all";
    let mut kind = "all";
    let mut order = "all";
    for filter in filters {
        match filter.name.as_str() {
            "genre" | "genres" => {
                let values: Vec<String> =
                    serde_json::from_str(filter_text(filter)?).map_err(|_| Error::Unsupported)?;
                if values.iter().any(|value| {
                    value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit())
                }) {
                    return Err(Error::Unsupported);
                }
                genres.extend(values);
            }
            "status" => status = filter_text(filter)?,
            "type" => kind = filter_text(filter)?,
            "order" => order = filter_text(filter)?,
            _ => return Err(Error::Unsupported),
        }
    }
    if !["all", "ongoing", "completed", "hiatus", "season"].contains(&status)
        || !["all", "manga", "manhwa", "manhua", "comic"].contains(&kind)
        || ![
            "all",
            "title",
            "titlereverse",
            "update",
            "latest",
            "popular",
        ]
        .contains(&order)
    {
        return Err(Error::Unsupported);
    }
    let mut fields = genres
        .iter()
        .map(|genre| format!("genres_checked%5B%5D={}", encode(genre)))
        .collect::<Vec<_>>();
    fields.push(format!("StatusValue={}", encode(status)));
    fields.push(format!("TypeValue={}", encode(kind)));
    fields.push(format!("OrderValue={}", encode(order)));
    Ok(fields.join("&"))
}
/// Normalizes a title for matching: decodes entities, collapses whitespace, and case-folds.
fn title_key(title: &str) -> String {
    let decoded = Html::parse_fragment(title)
        .root_element()
        .text()
        .collect::<String>();
    decoded
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}
fn filtered_catalog(
    value: &serde_json::Value,
    catalog: Vec<MediaEntry>,
) -> Result<Vec<MediaEntry>> {
    let rows = value
        .as_array()
        .ok_or_else(|| Error::Host("RizzFables catalog response was not a list".into()))?;
    // A title shared by two catalog entries identifies neither, so that title is withheld
    // rather than guessed. Matching uses the title text the site returns; HTML links remain
    // the authoritative identity of each entry.
    let mut by_title: HashMap<String, Option<MediaEntry>> = HashMap::new();
    for entry in catalog {
        let key = match &entry {
            MediaEntry::Manga(item) => title_key(&item.title),
            _ => continue,
        };
        by_title
            .entry(key)
            .and_modify(|slot| *slot = None)
            .or_insert(Some(entry));
    }
    let mut entries = Vec::with_capacity(rows.len());
    for row in rows {
        let title = row
            .get("title")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| Error::Host("RizzFables catalog result had no title".into()))?;
        let Some(slot) = by_title.get_mut(&title_key(title)) else {
            // A row without a catalog entry is skipped rather than failing the whole search.
            continue;
        };
        // Withheld collisions and repeated rows for one series both yield nothing.
        if let Some(entry) = slot.take() {
            entries.push(entry);
        }
    }
    Ok(entries)
}
fn paginate(entries: Vec<MediaEntry>, query: &str, page: i32) -> Result<MediaPageResult> {
    if page < 1 {
        return Err(Error::Host("Page numbers start at one".into()));
    }
    let query = query.trim().to_lowercase();
    let mut entries: Vec<_> = entries.into_iter().filter(|entry| {
        matches!(entry, MediaEntry::Manga(item) if item.title.to_lowercase().contains(&query))
    }).collect();
    let start = (page as usize - 1).saturating_mul(PAGE_SIZE);
    let has_next_page = start.saturating_add(PAGE_SIZE) < entries.len();
    let entries = if start >= entries.len() {
        Vec::new()
    } else {
        entries.drain(start..).take(PAGE_SIZE).collect()
    };
    Ok(MediaPageResult {
        entries,
        has_next_page,
    })
}

/// Body of the catalog's default filter state. Ito sends every control, so a plain text search
/// whose controls are all at these defaults must not use the filter endpoint.
const DEFAULT_FILTER_BODY: &str = "StatusValue=all&TypeValue=all&OrderValue=all";
/// The filter request body, or `None` when every control is at its default.
fn filter_request_body(filters: &[FilterItem]) -> Result<Option<String>> {
    let body = catalog_filter_body(filters)?;
    Ok((body != DEFAULT_FILTER_BODY).then_some(body))
}
fn catalog_search(query: &str, page: i32, filters: &[FilterItem]) -> Result<MediaPageResult> {
    if page < 1 {
        return Err(Error::Host("Page numbers start at one".into()));
    }
    let mut catalog = complete_catalog(&request("/series")?)?;
    if let Some(body) = filter_request_body(filters)? {
        catalog = filtered_catalog(&post_json(&body)?, catalog)?;
    }
    paginate(catalog, query, page)
}
fn select_control(id: &str, title: &str, values: &[(&str, &str)], default: &str) -> Control {
    let options = values
        .iter()
        .map(|(value, label)| ChoiceOption::new(*value, *label))
        .collect();
    Control::select(
        id,
        title,
        OptionSource::inline(options),
        Some(default.to_owned()),
    )
}
fn search_capabilities() -> Result<SearchCapabilities> {
    let html = request("/series")?;
    let root = document(&html);
    let labels = root.query("label[for]")?;
    let mut genres = Vec::new();
    for input in root.query("input.genre-item")? {
        let Some(value) = attr(&input, "value") else {
            continue;
        };
        let Some(id) = attr(&input, "id") else {
            continue;
        };
        let label = labels
            .iter()
            .find(|label| attr(label, "for").as_deref() == Some(id.as_str()))
            .and_then(text)
            .unwrap_or_else(|| value.clone());
        genres.push(ChoiceOption::new(value, label));
    }
    if genres.is_empty() {
        return Err(Error::Host(
            "RizzFables genre filters were not found".into(),
        ));
    }
    let mut capabilities = SearchCapabilities::new();
    capabilities.flags = SearchCapabilities::BROWSE
        | SearchCapabilities::SORT_WITH_QUERY
        | SearchCapabilities::FILTERS_WITH_QUERY;
    capabilities.sections.push(
        ControlSection::new("catalog", "Catalog")
            .control(Control::multi_select(
                "genre",
                "Genre",
                OptionSource::inline(genres),
                Vec::new(),
            ))
            .control(select_control(
                "status",
                "Status",
                &[
                    ("all", "All"),
                    ("ongoing", "Ongoing"),
                    ("completed", "Completed"),
                    ("hiatus", "Hiatus"),
                    ("season", "Season End"),
                ],
                "all",
            ))
            .control(select_control(
                "type",
                "Type",
                &[
                    ("all", "All"),
                    ("manga", "Manga"),
                    ("manhwa", "Manhwa"),
                    ("manhua", "Manhua"),
                    ("comic", "Comic"),
                ],
                "all",
            ))
            .control(select_control(
                "order",
                "Order by",
                &[
                    ("all", "Default"),
                    ("title", "A-Z"),
                    ("titlereverse", "Z-A"),
                    ("update", "Update"),
                    ("latest", "Added"),
                    ("popular", "Popular"),
                ],
                "all",
            )),
    );
    Ok(capabilities)
}
fn chapter_date(value: &str) -> Option<f64> {
    let mut parts = value.split_whitespace();
    let day = parts.next()?.parse::<i64>().ok()?;
    let month_name = parts.next()?;
    let month = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ]
    .iter()
    .position(|month| *month == month_name)?;
    let year = parts.next()?.parse::<i64>().ok()?;
    if parts.next().is_some() || !(1970..=9999).contains(&year) {
        return None;
    }
    let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    let days_in_month = [
        31,
        if leap { 29 } else { 28 },
        31,
        30,
        31,
        30,
        31,
        31,
        30,
        31,
        30,
        31,
    ];
    if day < 1 || day > days_in_month[month] {
        return None;
    }
    let days_before_year =
        |year: i64| 365 * year + (year - 1) / 4 - (year - 1) / 100 + (year - 1) / 400;
    let days = days_before_year(year) - days_before_year(1970)
        + days_in_month[..month].iter().sum::<i64>()
        + day
        - 1;
    Some(days as f64 * 86_400.0)
}
fn chapters(html: &str) -> Result<Vec<Chapter>> {
    access_check(html)?;
    let root = document(html);
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    for row in root.query("#chapterlist li")? {
        let Some(anchor) = row.query("a[href*='/chapter/']")?.into_iter().next() else {
            continue;
        };
        let Some(path) = attr(&anchor, "href").and_then(|url| chapter_path(&url)) else {
            // A link outside the chapter URL shape is skipped; the other rows still list.
            continue;
        };
        if !seen.insert(path.clone()) {
            continue;
        }
        let label = anchor
            .query(".chapternum")?
            .first()
            .and_then(text)
            .or_else(|| text(&anchor))
            .unwrap_or_default();
        let number = attr(&row, "data-num")
            .and_then(|value| value.parse::<f32>().ok())
            .or_else(|| {
                label
                    .strip_prefix("Chapter")
                    .unwrap_or(&label)
                    .split_whitespace()
                    .next()?
                    .parse::<f32>()
                    .ok()
            });
        let date_updated = anchor
            .query(".chapterdate")?
            .first()
            .and_then(text)
            .and_then(|value| chapter_date(&value));
        out.push(Chapter {
            key: path.trim_matches('/').to_owned(),
            title: (!label.is_empty()).then_some(label),
            volume: None,
            chapter: number,
            date_updated,
            scanlator: None,
            url: Some(format!("{ORIGIN}{path}")),
            lang: Some("en".into()),
            paywalled: None,
            kind: ContentKind::Images,
        });
    }
    if out.is_empty() {
        return Err(Error::Host(
            "RizzFables chapter list was missing or empty".into(),
        ));
    }
    Ok(out)
}
fn synopsis(html: &str) -> Option<String> {
    let suffix = html.split_once("var description =")?.1.trim_start();
    let value = serde_json::Deserializer::from_str(suffix)
        .into_iter::<String>()
        .next()?
        .ok()?;
    let value = value.replace("<br>", "\n").replace("<br />", "\n");
    let root = Html::parse_fragment(&value);
    let result = root.root_element().text().collect::<String>();
    (!result.trim().is_empty()).then(|| result.trim().to_owned())
}
fn update(entry: MediaEntry, details: bool, units: bool) -> Result<MediaEntry> {
    let MediaEntry::Manga(mut item) = entry else {
        return Err(Error::Unsupported);
    };
    let path = media_path(&item).ok_or(Error::Unsupported)?;
    if details || units {
        let html = request(&path)?;
        let root = document(&html);
        if details {
            item.title = root
                .query(".entry-title")?
                .first()
                .and_then(text)
                .unwrap_or(item.title);
            item.description = synopsis(&html).or_else(|| {
                root.query("#description-container, .description")
                    .ok()?
                    .first()
                    .and_then(text)
            });
            item.cover = root
                .query(".thumb img")?
                .first()
                .and_then(|node| attr(node, "src"))
                .and_then(|url| image_url(&url));
            let mut tags = Vec::new();
            for node in root.query("a[rel='tag']")? {
                if let Some(tag) = text(&node)
                    && !tags.contains(&tag)
                {
                    tags.push(tag);
                }
            }
            item.tags = (!tags.is_empty()).then_some(tags);
            let status = root
                .query(".bs-status")?
                .first()
                .and_then(text)
                .unwrap_or_default()
                .to_ascii_lowercase();
            item.status = match status.as_str() {
                "ongoing" => Status::Ongoing,
                "completed" => Status::Completed,
                "dropped" => Status::Cancelled,
                "hiatus" | "season end" => Status::Hiatus,
                _ => Status::Unknown,
            };
            for node in root.query(".tsinfo .imptdt")? {
                let label = text(&node).unwrap_or_default();
                let value = node.query("i")?.first().and_then(text);
                if label.starts_with("Author") {
                    item.authors = value.map(|value| vec![value]);
                } else if label.starts_with("Artist") {
                    item.artist = value;
                }
            }
        }
        if units {
            item.chapters = Some(chapters(&html)?);
        }
    }
    Ok(MediaEntry::Manga(item))
}
fn reader_pages(html: &str, chapter_url: &str) -> Result<Vec<Page>> {
    let path = chapter_path(chapter_url).ok_or(Error::Unsupported)?;
    access_check(html)?;
    let root = document(html);
    let mut pages = Vec::new();
    for image in root.query("#readerarea img")? {
        // Linked banners are advertisements, not pages, even if inserted inside the reader.
        if image
            .ancestors()
            .filter_map(ElementRef::wrap)
            .any(|node| node.value().name() == "a")
        {
            continue;
        }
        let source = attr(&image, "data-src")
            .filter(|value| !value.trim().is_empty())
            .or_else(|| attr(&image, "src"))
            .ok_or_else(|| Error::Host("RizzFables reader image had no URL".into()))?;
        let url = image_url(source.trim())
            .ok_or_else(|| Error::Host("RizzFables returned an unsupported reader image".into()))?;
        let index = i32::try_from(pages.len())
            .map_err(|_| Error::Host("RizzFables chapter contains too many pages".into()))?;
        pages.push(Page {
            index,
            content: PageContent::Url(url),
            has_description: false,
            description: None,
            headers: Some(HashMap::from([(
                "Referer".into(),
                format!("{ORIGIN}{path}"),
            )])),
        });
    }
    if pages.is_empty() {
        return Err(Error::NotFound(
            "RizzFables chapter has no public image pages".into(),
        ));
    }
    Ok(pages)
}
fn chapter_pages(chapter: &Chapter) -> Result<Vec<Page>> {
    if chapter.paywalled == Some(true) {
        return Err(Error::NeedsLogin(
            "This RizzFables chapter is restricted; open it on the website".into(),
        ));
    }
    let path = chapter
        .url
        .as_deref()
        .and_then(chapter_path)
        .or_else(|| chapter_path(&format!("/{}", chapter.key.trim_start_matches('/'))))
        .ok_or(Error::Unsupported)?;
    reader_pages(&request(&path)?, &format!("{ORIGIN}{path}"))
}
fn encode(value: &str) -> String {
    value
        .bytes()
        .map(|byte| match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (byte as char).to_string()
            }
            _ => format!("%{byte:02X}"),
        })
        .collect()
}

/// Public catalog and reader for RizzFables, retaining the stable RizzComic source identity.
pub struct RizzComic;
impl SourceProvider for RizzComic {
    const KINDS: &'static [MediaKind] = &[MediaKind::Manga];
    fn get_home() -> Result<HomeLayout> {
        let html = request("/")?;
        let root = document(&html);
        let mut components = Vec::new();
        for section in root.query(".bixbox")? {
            let Some(title) = section.query(".releases h2")?.first().and_then(text) else {
                continue;
            };
            let mut entries = cards(&section)?;
            entries.truncate(12);
            if !entries.is_empty() {
                let listing = (title == "Latest Comics").then(|| Listing {
                    id: "series".into(),
                    name: "Series".into(),
                    kind: 0,
                });
                components.push(HomeComponent {
                    title: Some(title),
                    subtitle: None,
                    value: HomeComponentValue::MediaScroller(entries, listing),
                });
            }
        }
        if components.is_empty() {
            return Err(Error::Host(
                "RizzFables home sections were not found".into(),
            ));
        }
        Ok(HomeLayout { components })
    }
    fn get_listing(listing: Listing, page: i32) -> Result<MediaPageResult> {
        if listing.id != "series" {
            return Err(Error::Unsupported);
        }
        catalog_search("", page, &[])
    }
    fn get_search_capabilities(kind: Option<MediaKind>) -> Result<SearchCapabilities> {
        if kind.is_some_and(|kind| kind != MediaKind::Manga) {
            return Err(Error::Unsupported);
        }
        search_capabilities()
    }
    fn search(
        query: &str,
        page: i32,
        kind: Option<MediaKind>,
        filters: Vec<FilterItem>,
    ) -> Result<MediaPageResult> {
        if kind.is_some_and(|kind| kind != MediaKind::Manga) {
            return Err(Error::Unsupported);
        }
        catalog_search(query, page, &filters)
    }
    fn get_media_update(entry: MediaEntry, details: bool, units: bool) -> Result<MediaEntry> {
        update(entry, details, units)
    }
    fn get_chapter_pages(_entry: MediaEntry, chapter: Chapter) -> Result<Vec<Page>> {
        chapter_pages(&chapter)
    }
    fn handle_url(url: &str) -> Result<LinkValue> {
        let path = if let Some(path) = series_path(url) {
            path
        } else if let Some(path) = chapter_path(url) {
            let html = request(&path)?;
            let root = document(&html);
            root.query(".ts-breadcrumb a[href*='/series/']")?
                .first()
                .and_then(|node| attr(node, "href"))
                .and_then(|href| series_path(&href))
                .ok_or(Error::Unsupported)?
        } else {
            return Err(Error::Unsupported);
        };
        let entry = update(
            MediaEntry::Manga(manga(&path, String::new(), None)),
            true,
            true,
        )?;
        if let MediaEntry::Manga(item) = entry {
            Ok(LinkValue::Manga(item))
        } else {
            Err(Error::Unsupported)
        }
    }
    fn get_image_origins() -> Result<Vec<String>> {
        Ok(IMAGE_HOSTS.iter().map(|host| (*host).to_owned()).collect())
    }
    fn get_image_request(url: &str) -> Result<PluginImageRequest> {
        let url = image_url(url).ok_or(Error::Unsupported)?;
        Ok(PluginImageRequest {
            url,
            headers: Some(HashMap::from([("Referer".into(), REFERER.into())])),
        })
    }
}
ito_rs::export_plugin!(RizzComic);

#[cfg(test)]
mod tests {
    use super::*;
    fn catalog(count: usize) -> Vec<MediaEntry> {
        (0..count)
            .map(|index| {
                MediaEntry::Manga(manga(
                    &format!("/series/book-{index}"),
                    format!("Book {index}"),
                    None,
                ))
            })
            .collect()
    }
    #[test]
    fn identities_are_stable_and_reject_foreign_or_ambiguous_urls() {
        assert_eq!(
            series_path("https://rizzfables.com/series/id-name/?x=1"),
            Some("/series/id-name".into())
        );
        assert_eq!(
            series_path("https://rizzfables.com.evil.example/series/id-name"),
            None
        );
        assert_eq!(series_path("//evil.example/series/id-name"), None);
        assert_eq!(series_path("https://rizzfables.com/series/%2e%2e"), None);
        assert_eq!(
            chapter_path("/chapter/x-chapter-7.5"),
            Some("/chapter/x-chapter-7.5".into())
        );
        let item = manga("/series/book-1", "Book".into(), None);
        assert_eq!(
            media_path(&Manga { url: None, ..item }),
            Some("/series/book-1".into())
        );
    }
    #[test]
    fn complete_search_paginates_after_keyword_matching_not_before() {
        let first = paginate(catalog(41), "BOOK", 1).expect("first");
        assert_eq!(first.entries.len(), 20);
        assert!(first.has_next_page);
        let second = paginate(catalog(41), "book", 2).expect("second");
        assert_eq!(second.entries[0].key(), "series/book-20");
        assert!(second.has_next_page);
        let last = paginate(catalog(41), "book", 3).expect("last");
        assert_eq!(last.entries.len(), 1);
        assert!(!last.has_next_page);
        assert!(paginate(catalog(40), "", 2).is_ok_and(|page| !page.has_next_page));
        assert_eq!(
            paginate(catalog(41), "book 40", 1)
                .expect("late match")
                .entries
                .len(),
            1
        );
        assert!(
            paginate(catalog(41), "missing", 1)
                .expect("no matches")
                .entries
                .is_empty()
        );
        assert!(paginate(catalog(41), "", 0).is_err());
        assert!(
            paginate(catalog(41), "", i32::MAX)
                .expect("past end")
                .entries
                .is_empty()
        );
    }
    #[test]
    fn catalog_uses_canonical_links_and_refuses_partial_layouts() {
        let html = r#"<div id="set_bs"><a href="/series/foos-book" title="Foo's Book"><img src="/assets/images/cover.webp"></a></div>"#;
        let entries = complete_catalog(html).expect("complete catalog");
        let response = serde_json::json!([{"title":"Foo's Book"}]);
        let entries = filtered_catalog(&response, entries).expect("canonical mapping");
        assert_eq!(entries[0].key(), "series/foos-book");
        assert!(
            complete_catalog(&format!(
                "{html}<nav class='pagination'><a href='/series?page=2'>2</a></nav>"
            ))
            .is_err()
        );
        assert!(complete_catalog("<html>unrecognized</html>").is_err());
        assert!(filtered_catalog(&serde_json::json!({"error":true}), catalog(1)).is_err());
        assert_eq!(
            filtered_catalog(&serde_json::json!([{"title":"Missing"}]), catalog(1))
                .expect("unmatched rows are skipped")
                .len(),
            0
        );
    }
    #[test]
    fn dates_are_utc_seconds_and_reject_impossible_days() {
        assert_eq!(chapter_date("01 Jan 1970"), Some(0.0));
        assert_eq!(chapter_date("02 Jan 1970"), Some(86_400.0));
        assert_eq!(chapter_date("29 Feb 2024"), Some(1_709_164_800.0));
        assert_eq!(chapter_date("29 Feb 2023"), None);
        assert_eq!(chapter_date("31 Apr 2024"), None);
    }
    #[test]
    fn chapters_keep_fractional_zero_and_dom_order_without_inventing_locks() {
        let html = r#"<ul id="chapterlist"><li><a href="/chapter/x-chapter-7.5"><span class="chapternum">Chapter 7.5</span></a></li><li><a href="/chapter/x-chapter-0"><span class="chapternum">Chapter 0</span></a></li><li><a href="/chapter/x-chapter-0"><span class="chapternum">Chapter 0</span></a></li></ul>"#;
        let items = chapters(html).expect("chapter list");
        assert_eq!(
            items
                .iter()
                .map(|chapter| chapter.chapter)
                .collect::<Vec<_>>(),
            vec![Some(7.5), Some(0.0)]
        );
        assert!(items.iter().all(|chapter| chapter.paywalled.is_none()));
        assert!(chapters("<ul id='chapterlist'></ul>").is_err());
    }
    #[test]
    fn reader_preserves_dom_order_and_excludes_ads() {
        let html = r#"<img src="https://cdn.rizzfables.com/wp-content/uploads/outside.webp"><div id="readerarea"><img src="https://cdn.rizzfables.com/wp-content/uploads/page-2.webp"><a href="https://ads.example"><img src="https://cdn.rizzfables.com/wp-content/uploads/banner.webp"></a><img src="placeholder.gif" data-src="https://cdn.55779955.xyz/file/book/page-1.webp"></div>"#;
        let pages =
            reader_pages(html, "https://rizzfables.com/chapter/x-chapter-2").expect("reader");
        let urls = pages
            .iter()
            .map(|page| match &page.content {
                PageContent::Url(url) => url.as_str(),
                _ => "",
            })
            .collect::<Vec<_>>();
        assert_eq!(
            urls,
            [
                "https://cdn.rizzfables.com/wp-content/uploads/page-2.webp",
                "https://cdn.55779955.xyz/file/book/page-1.webp"
            ]
        );
        assert_eq!(pages[1].index, 1);
        assert_eq!(
            pages[0]
                .headers
                .as_ref()
                .and_then(|headers| headers.get("Referer"))
                .map(String::as_str),
            Some("https://rizzfables.com/chapter/x-chapter-2")
        );
        assert!(
            reader_pages(
                "<div id='readerarea'><img src='https://evil.example/x'></div>",
                "/chapter/x-chapter-1"
            )
            .is_err()
        );
    }
    #[test]
    fn verification_and_explicit_reader_refusals_are_not_empty_success() {
        assert!(matches!(
            access_check("<title>Verify you are human</title>"),
            Err(Error::NeedsLogin(_))
        ));
        assert!(matches!(
            reader_pages(
                "<div id='readerarea'>Log in to read</div>",
                "/chapter/x-chapter-1"
            ),
            Err(Error::NeedsLogin(_))
        ));
        assert!(matches!(
            reader_pages(
                "<div id='readerarea'>Purchase this chapter</div>",
                "/chapter/x-chapter-1"
            ),
            Err(Error::NeedsLogin(_))
        ));
        assert!(
            access_check("<div id='signin-modal'>Sign in</div><div id='readerarea'></div>").is_ok()
        );
        assert!(response_text(500, b"failure".to_vec()).is_err());
    }
    #[test]
    fn filter_form_matches_site_fields_and_rejects_unknown_values() {
        let filters = vec![
            FilterItem {
                name: "genre".into(),
                type_name: "multi_select".into(),
                value: FilterValue::String(r#"["75","3"]"#.into()),
            },
            FilterItem {
                name: "status".into(),
                type_name: "select".into(),
                value: FilterValue::String("ongoing".into()),
            },
            FilterItem {
                name: "type".into(),
                type_name: "select".into(),
                value: FilterValue::String("manhwa".into()),
            },
            FilterItem {
                name: "order".into(),
                type_name: "select".into(),
                value: FilterValue::String("titlereverse".into()),
            },
        ];
        assert_eq!(
            catalog_filter_body(&filters).expect("filters"),
            "genres_checked%5B%5D=75&genres_checked%5B%5D=3&StatusValue=ongoing&TypeValue=manhwa&OrderValue=titlereverse"
        );
        assert!(
            catalog_filter_body(&[FilterItem {
                name: "status".into(),
                type_name: "select".into(),
                value: FilterValue::String("bogus".into())
            }])
            .is_err()
        );
        assert_eq!(encode("a b&c"), "a%20b%26c");
        assert!(
            image_url("https://cdn.rizzfables.com.evil.example/wp-content/uploads/x.webp")
                .is_none()
        );
    }
    #[test]
    fn embedded_synopsis_is_decoded_without_exposing_script_source() {
        let html = r#"<script>var description = "A \"quoted\" synopsis.<br>Next line."; var md = window.markdownit();</script>"#;
        assert_eq!(
            synopsis(html).as_deref(),
            Some("A \"quoted\" synopsis.\nNext line.")
        );
        assert_eq!(
            synopsis("<script>var description = invalid;</script>"),
            None
        );
    }

    #[test]
    fn challenge_phrases_in_body_copy_are_not_gates() {
        assert!(
            access_check("<p>Verify you are human before you buy.</p><div id='readerarea'></div>")
                .is_ok()
        );
        assert!(matches!(
            access_check("<title>Just a moment...</title>"),
            Err(Error::NeedsLogin(_))
        ));
        assert!(matches!(
            access_check("<script src='/cf-chl-bypass.js'></script>"),
            Err(Error::NeedsLogin(_))
        ));
    }
    #[test]
    fn plain_forbidden_is_a_server_error_while_sign_in_is_a_login_gate() {
        assert!(matches!(
            response_text(403, b"<html>denied</html>".to_vec()),
            Err(Error::Server(_))
        ));
        assert!(matches!(
            response_text(401, b"<html>sign in</html>".to_vec()),
            Err(Error::NeedsLogin(_))
        ));
    }
    #[test]
    fn filtered_titles_match_normalized_and_withhold_collisions() {
        let catalog = vec![
            MediaEntry::Manga(manga("/series/foo", "Foo & Bar".into(), None)),
            MediaEntry::Manga(manga("/series/dup-a", "Dup".into(), None)),
            MediaEntry::Manga(manga("/series/dup-b", "dup".into(), None)),
        ];
        let response = serde_json::json!([{"title":"Foo &amp; Bar"},{"title":"  FOO   & bar "},{"title":"DUP"}]);
        let entries = filtered_catalog(&response, catalog).expect("normalized mapping");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].key(), "series/foo");
    }
    #[test]
    fn url_identity_accepts_scheme_and_host_case_and_rejects_lookalikes() {
        assert_eq!(
            series_path("HTTP://WWW.RizzFables.com/series/id-name"),
            Some("/series/id-name".into())
        );
        assert_eq!(
            series_path("https://user@rizzfables.com/series/id-name"),
            None
        );
        assert_eq!(
            series_path("https://rizzfables.com:8443/series/id-name"),
            None
        );
        assert_eq!(
            series_path("https://rizzfables.com.evil.example/series/id-name"),
            None
        );
    }
    #[test]
    fn novel_is_not_a_catalog_type() {
        assert!(
            catalog_filter_body(&[FilterItem {
                name: "type".into(),
                type_name: "select".into(),
                value: FilterValue::String("novel".into()),
            }])
            .is_err()
        );
    }
    #[test]
    fn default_controls_send_no_filter_request() {
        let defaults = vec![
            FilterItem {
                name: "genre".into(),
                type_name: "multi_select".into(),
                value: FilterValue::String("[]".into()),
            },
            FilterItem {
                name: "status".into(),
                type_name: "select".into(),
                value: FilterValue::String("all".into()),
            },
            FilterItem {
                name: "type".into(),
                type_name: "select".into(),
                value: FilterValue::String("all".into()),
            },
            FilterItem {
                name: "order".into(),
                type_name: "select".into(),
                value: FilterValue::String("all".into()),
            },
        ];
        assert_eq!(filter_request_body(&defaults).expect("defaults"), None);
        assert_eq!(filter_request_body(&[]).expect("no filters"), None);
        assert!(
            filter_request_body(&[FilterItem {
                name: "status".into(),
                type_name: "select".into(),
                value: FilterValue::String("ongoing".into()),
            }])
            .expect("explicit filter")
            .is_some()
        );
    }
    #[test]
    fn chapter_rows_with_foreign_links_are_skipped_not_fatal() {
        let html = r#"<ul id="chapterlist"><li><a href="/chapter/x-extra"><span class="chapternum">Extra</span></a></li><li><a href="/chapter/x-chapter-2"><span class="chapternum">Chapter 2</span></a></li></ul>"#;
        let items = chapters(html).expect("valid rows survive");
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].key, "chapter/x-chapter-2");
    }
    #[test]
    fn challenge_title_must_start_with_the_phrase() {
        assert!(access_check("<title>Chapter notes</title><p>Just a moment, please</p>").is_ok());
        assert!(access_check("<title>Just a moment...</title>").is_err());
    }
}
