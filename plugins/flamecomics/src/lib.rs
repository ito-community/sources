//! Flame Comics' mixed comic and novel catalog and public readers.

use ito_rs::controls::{ChoiceOption, Control, ControlSection, OptionSource, SearchCapabilities};
use ito_rs::models::manga::{ContentRating, Manga, Status, Viewer};
use ito_rs::models::novel::{self, Novel};
use ito_rs::models::{
    Chapter, ContentKind, FilterItem, FilterValue, HomeComponent, HomeComponentValue, HomeLayout,
    LinkValue, Listing, MediaEntry, MediaKind, MediaPageResult, Page, PageContent,
    PluginImageRequest,
};
use ito_rs::net::Request;
use ito_rs::provider::SourceProvider;
use ito_rs::{Error, Result};
use scraper::{ElementRef, Html, Selector};
use serde_json::Value;
use std::collections::{BTreeSet, HashMap, HashSet};

const PAGE_SIZE: usize = 24;
const ORIGIN: &str = "https://flamecomics.xyz";
const IMAGE_ORIGIN: &str = "https://cdn.flamecomics.xyz";
const REFERER: &str = "https://flamecomics.xyz/";
struct FlameComics;

fn malformed() -> Error {
    Error::Server("Flame Comics data format changed".into())
}
/// Maps a non-success response to its app error category; `None` for a 2xx response.
fn response_error(status: i32, body: &[u8]) -> Option<Error> {
    if status == 401 {
        return Some(Error::NeedsLogin(
            "Open Flame Comics in the browser to sign in or complete verification".into(),
        ));
    }
    if is_challenge_page(body) {
        return Some(verification_required());
    }
    match status {
        200..=299 => None,
        404 => Some(Error::NotFound("Flame Comics page was not found".into())),
        _ => Some(Error::Server(format!(
            "Flame Comics returned HTTP {status}"
        ))),
    }
}
fn is_challenge_page(body: &[u8]) -> bool {
    let html = String::from_utf8_lossy(body);
    html.contains("<title>Just a moment") || html.contains("cf-chl-")
}
fn verification_required() -> Error {
    Error::NeedsLogin("Complete Flame Comics verification in the browser".into())
}
fn request(path: &str) -> Result<String> {
    let mut req = Request::get(format!("{ORIGIN}{path}"));
    req.header("Referer", REFERER);
    let response = req.send()?;
    if let Some(error) = response_error(response.status, &response.body) {
        return Err(error);
    }
    String::from_utf8(response.body).map_err(|_| malformed())
}
fn selector(css: &str) -> Result<Selector> {
    Selector::parse(css).map_err(|_| Error::Html("Invalid source selector".into()))
}
fn attr(node: &ElementRef<'_>, name: &str) -> Option<String> {
    node.value().attr(name).map(str::to_owned)
}
/// The page data the site renders into its own document; script text is read directly.
fn next_data(html: &str) -> Result<Value> {
    let root = Html::parse_document(html);
    let script = selector("script#__NEXT_DATA__")?;
    let node = root.select(&script).next().ok_or_else(malformed)?;
    let text = node.text().collect::<String>();
    let data: Value = serde_json::from_str(&text).map_err(|_| malformed())?;
    data.pointer("/props/pageProps")
        .cloned()
        .filter(Value::is_object)
        .ok_or_else(malformed)
}
fn strings(value: Option<&Value>) -> Option<Vec<String>> {
    value
        .and_then(Value::as_array)
        .map(|v| {
            v.iter()
                .filter_map(Value::as_str)
                .filter(|s| !s.trim().is_empty())
                .map(str::to_string)
                .collect::<Vec<_>>()
        })
        .filter(|v| !v.is_empty())
}
fn string(value: &Value, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.trim().is_empty())
        .map(str::to_string)
}
fn numeric(value: &Value) -> Option<f64> {
    value.as_f64().or_else(|| value.as_str()?.parse().ok())
}
fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.bytes().all(|b| b.is_ascii_digit())
}
fn valid_token(token: &str) -> bool {
    token.len() == 16 && token.bytes().all(|b| b.is_ascii_hexdigit())
}
fn route(kind: MediaKind) -> &'static str {
    if kind == MediaKind::Novel {
        "novel"
    } else {
        "series"
    }
}
fn status(value: &str) -> Status {
    match value.to_ascii_lowercase().as_str() {
        "ongoing" => Status::Ongoing,
        "completed" => Status::Completed,
        "hiatus" => Status::Hiatus,
        "dropped" | "cancelled" | "canceled" => Status::Cancelled,
        _ => Status::Unknown,
    }
}
fn image_url(src: &str) -> Option<String> {
    let source = if let Some(query) = src.strip_prefix("/_next/image?") {
        let encoded = query
            .split('&')
            .find_map(|part| part.strip_prefix("url="))?;
        let mut bytes = Vec::with_capacity(encoded.len());
        let mut chars = encoded.bytes();
        while let Some(byte) = chars.next() {
            if byte == b'%' {
                let hex = [chars.next()?, chars.next()?];
                bytes.push(u8::from_str_radix(std::str::from_utf8(&hex).ok()?, 16).ok()?);
            } else {
                bytes.push(byte);
            }
        }
        String::from_utf8(bytes).ok()?
    } else {
        src.to_string()
    };
    source
        .starts_with("https://cdn.flamecomics.xyz/")
        .then_some(source)
}
fn cover(item: &Value, kind: MediaKind, id: &str) -> Option<String> {
    let name = item.get("cover")?.as_str()?.trim();
    if name.is_empty() {
        return None;
    }
    if name.starts_with("https://") || name.starts_with("/_next/") {
        return image_url(name);
    }
    let stamp = item
        .get("last_edit")
        .and_then(Value::as_i64)
        .map(|n| format!("?{n}"))
        .unwrap_or_default();
    let folder = if kind == MediaKind::Novel {
        "novels"
    } else {
        "series"
    };
    Some(format!(
        "{IMAGE_ORIGIN}/uploads/images/{folder}/{id}/{name}{stamp}"
    ))
}
struct CatalogEntry {
    manga: Manga,
    kind: MediaKind,
    media_type: String,
    status_label: String,
    year: Option<i64>,
    likes: i64,
}
impl CatalogEntry {
    fn into_media(self) -> MediaEntry {
        to_media(self.manga, self.kind)
    }
}
fn to_media(m: Manga, kind: MediaKind) -> MediaEntry {
    if kind == MediaKind::Novel {
        let status = match m.status {
            Status::Ongoing => novel::Status::Ongoing,
            Status::Completed => novel::Status::Completed,
            Status::Cancelled => novel::Status::Cancelled,
            Status::Hiatus => novel::Status::Hiatus,
            _ => novel::Status::Unknown,
        };
        let content_rating = match m.content_rating {
            ContentRating::Nsfw => novel::ContentRating::Nsfw,
            ContentRating::Suggestive => novel::ContentRating::Suggestive,
            _ => novel::ContentRating::Safe,
        };
        MediaEntry::Novel(Novel {
            key: m.key,
            title: m.title,
            authors: m.authors,
            artist: m.artist,
            description: m.description,
            tags: m.tags,
            cover: m.cover,
            url: m.url,
            status,
            content_rating,
            nsfw: m.nsfw,
            chapters: m.chapters,
        })
    } else {
        MediaEntry::Manga(m)
    }
}
fn parse_entry(item: &Value) -> Result<CatalogEntry> {
    let media_type = string(item, "type").ok_or_else(malformed)?;
    let kind = if media_type.to_ascii_lowercase().contains("novel") {
        MediaKind::Novel
    } else {
        MediaKind::Manga
    };
    let id = item
        .get(if kind == MediaKind::Novel {
            "novel_id"
        } else {
            "series_id"
        })
        .and_then(Value::as_i64)
        .ok_or_else(malformed)?;
    if id < 1 {
        return Err(malformed());
    }
    let id = id.to_string();
    let tags = strings(item.get("categories").or_else(|| item.get("tags")));
    let adult = tags.as_deref().unwrap_or_default().iter().any(|tag| {
        matches!(
            tag.to_ascii_lowercase().as_str(),
            "mature" | "ecchi" | "ecchi comedy"
        )
    });
    let status_label = string(item, "status").unwrap_or_default();
    let manga = Manga {
        key: id.clone(),
        title: string(item, "title").ok_or_else(malformed)?,
        authors: strings(item.get("author")),
        artist: strings(item.get("artist")).map(|v| v.join(", ")),
        description: string(item, "description"),
        tags,
        cover: cover(item, kind, &id),
        url: Some(format!("{ORIGIN}/{}/{id}", route(kind))),
        status: status(&status_label),
        content_rating: if adult {
            ContentRating::Suggestive
        } else {
            ContentRating::Safe
        },
        nsfw: 0,
        viewer: Viewer::Webtoon,
        chapters: None,
    };
    Ok(CatalogEntry {
        manga,
        kind,
        media_type,
        status_label,
        year: item.get("year").and_then(Value::as_i64),
        likes: item
            .get("likes")
            .and_then(Value::as_i64)
            .unwrap_or_default(),
    })
}
fn parse_catalog(data: &Value) -> Result<Vec<CatalogEntry>> {
    let entries = data
        .get("series")
        .and_then(Value::as_array)
        .ok_or_else(malformed)?;
    entries.iter().map(parse_entry).collect()
}
fn catalog() -> Result<Vec<CatalogEntry>> {
    parse_catalog(&next_data(&request("/browse")?)?)
}
fn paged(entries: Vec<MediaEntry>, requested: i32) -> Result<MediaPageResult> {
    if requested < 1 {
        return Err(Error::Host("Page numbers start at one".into()));
    }
    let start = (requested as usize - 1).saturating_mul(PAGE_SIZE);
    let total = entries.len();
    let entries: Vec<_> = entries.into_iter().skip(start).take(PAGE_SIZE).collect();
    let has_next_page = start.saturating_add(entries.len()) < total;
    Ok(MediaPageResult {
        entries,
        has_next_page,
    })
}
fn parse_chapters(data: &Value, kind: MediaKind, id: &str) -> Result<Vec<Chapter>> {
    let values = data
        .get("chapters")
        .and_then(Value::as_array)
        .ok_or_else(malformed)?;
    let mut seen = HashSet::new();
    let mut chapters = Vec::with_capacity(values.len());
    for value in values {
        let key = string(value, "token")
            .filter(|v| valid_token(v))
            .ok_or_else(malformed)?;
        if !seen.insert(key.clone()) {
            continue;
        }
        let number = value
            .get("chapter")
            .and_then(numeric)
            .filter(|n| n.is_finite())
            .ok_or_else(malformed)? as f32;
        let title = string(value, "title")
            .map(|t| format!("Chapter {number} — {t}"))
            .unwrap_or_else(|| format!("Chapter {number}"));
        chapters.push(Chapter {
            key: key.clone(),
            title: Some(title),
            volume: None,
            chapter: Some(number),
            date_updated: value.get("release_date").and_then(numeric),
            scanlator: None,
            url: Some(format!("{ORIGIN}/{}/{id}/{key}", route(kind))),
            lang: Some("en".into()),
            paywalled: chapter_paywall(value),
            kind: if kind == MediaKind::Novel {
                ContentKind::Text
            } else {
                ContentKind::Images
            },
        });
    }
    chapters.sort_by(|a, b| {
        a.chapter
            .unwrap_or_default()
            .total_cmp(&b.chapter.unwrap_or_default())
            .then_with(|| a.key.cmp(&b.key))
    });
    Ok(chapters)
}
/// Access from the chapter DTO alone, with no reader fetch. A recorded `public_release` means the
/// chapter was released: every sampled chapter with one was `isPublic` (10 of 10 on novel 8).
/// Nothing in the DTO proves a priced chapter is locked (unreleased priced chapters were locked in
/// 77 samples and public in 91), so those stay unknown and the reader refuses them when opened.
fn chapter_paywall(value: &Value) -> Option<bool> {
    let price = value.get("price").and_then(numeric)?;
    if price <= 0.0 || value.get("public_release").and_then(numeric).is_some() {
        Some(false)
    } else {
        None
    }
}
fn detail(kind: MediaKind, id: &str, needs_units: bool) -> Result<MediaEntry> {
    if !matches!(kind, MediaKind::Manga | MediaKind::Novel) || !valid_id(id) {
        return Err(Error::Unsupported);
    }
    let data = next_data(&request(&format!("/{}/{id}", route(kind)))?)?;
    let item = data
        .get(if kind == MediaKind::Novel {
            "novels"
        } else {
            "series"
        })
        .ok_or_else(malformed)?;
    let mut entry = parse_entry(item)?;
    if entry.kind != kind || entry.manga.key != id {
        return Err(malformed());
    }
    if needs_units {
        entry.manga.chapters = Some(parse_chapters(&data, kind, id)?);
    }
    Ok(entry.into_media())
}
fn select_control(
    id: &str,
    title: &str,
    values: impl IntoIterator<Item = String>,
    default: &str,
) -> Control {
    let options = values
        .into_iter()
        .map(|v| ChoiceOption::new(v.clone(), v))
        .collect();
    Control::select(
        id,
        title,
        OptionSource::inline(options),
        Some(default.to_string()),
    )
}
/// Sorts catalog entries in the Browse order shared by the home row and the `browse` listing.
fn sort_browse(entries: &mut [CatalogEntry]) {
    entries.sort_by(|a, b| a.manga.title.cmp(&b.manga.title));
}
/// SDK inline option lists hold at most 256 entries, including the leading `all` option.
const MAX_INLINE_OPTIONS: usize = 256;
/// SDK option values and labels are limited to 256 bytes of non-control text.
const MAX_OPTION_BYTES: usize = 256;
/// Distinct catalog values without empty, control-bearing, oversized, or reserved `all` entries, capped for the SDK.
fn filter_options(values: impl IntoIterator<Item = String>) -> Vec<String> {
    values
        .into_iter()
        .filter(|v| {
            !v.trim().is_empty()
                && v.len() <= MAX_OPTION_BYTES
                && v.as_str() != "all"
                && !v.chars().any(char::is_control)
        })
        .collect::<BTreeSet<_>>()
        .into_iter()
        .take(MAX_INLINE_OPTIONS - 1)
        .collect()
}
fn filter_control(id: &str, title: &str, values: impl IntoIterator<Item = String>) -> Control {
    select_control(
        id,
        title,
        std::iter::once("all".to_string()).chain(filter_options(values)),
        "all",
    )
}
/// Builds search controls from parsed catalog values; every option list satisfies the SDK validator.
fn build_search_capabilities(entries: &[CatalogEntry]) -> SearchCapabilities {
    let years = entries.iter().filter_map(|e| e.year).map(|v| v.to_string());
    let types = entries.iter().map(|e| e.media_type.clone());
    let statuses = entries.iter().map(|e| e.status_label.clone());
    let genres = entries
        .iter()
        .flat_map(|e| e.manga.tags.as_deref().unwrap_or_default().iter().cloned());
    let mut capabilities = SearchCapabilities::new();
    capabilities.flags = SearchCapabilities::BROWSE
        | SearchCapabilities::FILTERS_WITH_QUERY
        | SearchCapabilities::SORT_WITH_QUERY;
    capabilities.sort = Some(select_control(
        "sort",
        "Sort",
        ["title".into(), "popular".into()],
        "title",
    ));
    capabilities.sections.push(
        ControlSection::new("catalog", "Catalog")
            .control(filter_control("status", "Status", statuses))
            .control(filter_control("type", "Type", types))
            .control(filter_control("year", "Year", years))
            .control(filter_control("genre", "Genre", genres)),
    );
    capabilities
}
fn search_capabilities(kind: Option<MediaKind>) -> Result<SearchCapabilities> {
    let entries = catalog()?
        .into_iter()
        .filter(|e| kind.is_none_or(|k| e.kind == k))
        .collect::<Vec<_>>();
    Ok(build_search_capabilities(&entries))
}
fn filter_text(filter: &FilterItem) -> Option<String> {
    match &filter.value {
        FilterValue::String(v) => Some(v.clone()),
        FilterValue::Int(v) => Some(v.to_string()),
        _ => None,
    }
}
fn parse_link(url: &str) -> Option<(MediaKind, String)> {
    let rest = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))?;
    let (host, path) = rest.split_once('/')?;
    if host != "flamecomics.xyz" && host != "www.flamecomics.xyz" {
        return None;
    }
    let path = path.split(['?', '#']).next()?;
    let mut parts = path.trim_end_matches('/').split('/');
    let kind = match parts.next()? {
        "series" => MediaKind::Manga,
        "novel" => MediaKind::Novel,
        _ => return None,
    };
    let id = parts.next()?;
    if !valid_id(id) {
        return None;
    }
    if let Some(token) = parts.next()
        && (!valid_token(token) || parts.next().is_some())
    {
        return None;
    }
    Some((kind, id.to_string()))
}
fn visible_reader_image(
    src: &str,
    style: &str,
    hidden: bool,
    id: &str,
    token: &str,
) -> Option<String> {
    if hidden
        || style.split(';').any(|s| {
            s.split_once(':').is_some_and(|(k, v)| {
                k.trim().eq_ignore_ascii_case("display") && v.trim().eq_ignore_ascii_case("none")
            })
        })
    {
        return None;
    }
    let url = image_url(src)?;
    url.starts_with(&format!(
        "{IMAGE_ORIGIN}/uploads/images/series/{id}/{token}/"
    ))
    .then_some(url)
}
fn locked() -> Error {
    Error::NeedsLogin(
        "This chapter is locked on Flame Comics. Open it in the browser to sign in or unlock it"
            .into(),
    )
}
fn reader_access(data: &Value) -> Result<()> {
    if data.get("isPublic").and_then(Value::as_bool) == Some(false) {
        return Err(locked());
    }
    Ok(())
}
/// Elements whose start and end close the current paragraph.
const BLOCK_TAGS: &[&str] = &[
    "address",
    "article",
    "aside",
    "blockquote",
    "dd",
    "div",
    "dl",
    "dt",
    "figure",
    "footer",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "header",
    "li",
    "main",
    "ol",
    "p",
    "pre",
    "section",
    "table",
    "tr",
    "ul",
];
/// Elements whose text is never reader content.
const SKIPPED_TAGS: &[&str] = &["script", "style"];
fn paragraphs(html: &str) -> Result<Vec<PageContent>> {
    let root = Html::parse_fragment(html);
    let mut current = String::new();
    let mut blocks = Vec::new();
    collect_blocks(root.root_element(), &mut current, &mut blocks);
    flush_block(&mut current, &mut blocks);
    Ok(blocks.into_iter().map(PageContent::Text).collect())
}
fn collect_blocks(element: ElementRef<'_>, current: &mut String, blocks: &mut Vec<String>) {
    let name = element.value().name();
    if SKIPPED_TAGS.contains(&name) {
        return;
    }
    if name == "br" {
        current.push('\n');
        return;
    }
    let block = BLOCK_TAGS.contains(&name);
    if block {
        flush_block(current, blocks);
    }
    for child in element.children() {
        if let Some(text) = child.value().as_text() {
            current.extend(
                text.chars()
                    .map(|c| if c.is_whitespace() { ' ' } else { c }),
            );
        } else if let Some(child) = ElementRef::wrap(child) {
            collect_blocks(child, current, blocks);
        }
    }
    if block {
        flush_block(current, blocks);
    }
}
/// Emits buffered text as one paragraph; `<br>` breaks become lines and runs of whitespace collapse.
fn flush_block(current: &mut String, blocks: &mut Vec<String>) {
    let text = current
        .split('\n')
        .map(|line| line.split_whitespace().collect::<Vec<_>>().join(" "))
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join("\n");
    current.clear();
    if !text.is_empty() {
        blocks.push(text);
    }
}
/// Visible chapter images in reader order, without hidden promotions or duplicates.
fn reader_images(html: &str, series: &str, token: &str) -> Result<Vec<String>> {
    let root = Html::parse_document(html);
    let images = selector("main img")?;
    let mut seen = HashSet::new();
    let mut urls = Vec::new();
    for img in root.select(&images) {
        let Some(src) = attr(&img, "src") else {
            continue;
        };
        let style = attr(&img, "style").unwrap_or_default();
        if let Some(url) =
            visible_reader_image(&src, &style, attr(&img, "hidden").is_some(), series, token)
            && seen.insert(url.clone())
        {
            urls.push(url);
        }
    }
    Ok(urls)
}
impl SourceProvider for FlameComics {
    const KINDS: &'static [MediaKind] = &[MediaKind::Manga, MediaKind::Novel];
    fn get_home() -> Result<HomeLayout> {
        let mut entries = catalog()?;
        entries.sort_by_key(|entry| std::cmp::Reverse(entry.likes));
        let popular = entries
            .iter()
            .take(12)
            .map(|e| to_media(e.manga.clone(), e.kind))
            .collect();
        let novels = entries
            .iter()
            .filter(|e| e.kind == MediaKind::Novel)
            .take(12)
            .map(|e| to_media(e.manga.clone(), e.kind))
            .collect();
        let browse = {
            let mut all = entries;
            sort_browse(&mut all);
            all.into_iter()
                .take(12)
                .map(CatalogEntry::into_media)
                .collect::<Vec<_>>()
        };
        Ok(HomeLayout {
            components: vec![
                HomeComponent {
                    title: Some("Popular".into()),
                    subtitle: None,
                    value: HomeComponentValue::MediaScroller(
                        popular,
                        Some(Listing {
                            id: "popular".into(),
                            name: "Popular".into(),
                            kind: 0,
                        }),
                    ),
                },
                HomeComponent {
                    title: Some("Novels".into()),
                    subtitle: None,
                    value: HomeComponentValue::MediaScroller(
                        novels,
                        Some(Listing {
                            id: "novels".into(),
                            name: "Novels".into(),
                            kind: 0,
                        }),
                    ),
                },
                HomeComponent {
                    title: Some("Browse all".into()),
                    subtitle: None,
                    value: HomeComponentValue::MediaScroller(
                        browse,
                        Some(Listing {
                            id: "browse".into(),
                            name: "Browse all".into(),
                            kind: 0,
                        }),
                    ),
                },
            ],
        })
    }
    fn get_listing(listing: Listing, page: i32) -> Result<MediaPageResult> {
        let mut entries = catalog()?;
        match listing.id.as_str() {
            "browse" => sort_browse(&mut entries),
            "popular" => entries.sort_by(|a, b| {
                b.likes
                    .cmp(&a.likes)
                    .then_with(|| a.manga.title.cmp(&b.manga.title))
            }),
            "novels" => {
                entries.retain(|e| e.kind == MediaKind::Novel);
                entries.sort_by_key(|entry| std::cmp::Reverse(entry.likes));
            }
            _ => return Err(Error::Unsupported),
        }
        paged(
            entries.into_iter().map(CatalogEntry::into_media).collect(),
            page,
        )
    }
    fn get_search_capabilities(kind: Option<MediaKind>) -> Result<SearchCapabilities> {
        search_capabilities(kind)
    }
    fn search(
        query: &str,
        page: i32,
        kind: Option<MediaKind>,
        filters: Vec<FilterItem>,
    ) -> Result<MediaPageResult> {
        let query = query.trim().to_lowercase();
        let mut entries = catalog()?
            .into_iter()
            .filter(|e| kind.is_none_or(|k| e.kind == k))
            .collect::<Vec<_>>();
        let mut sort = "title".to_string();
        for filter in filters {
            let value = filter_text(&filter)
                .ok_or_else(|| Error::Host("Invalid Flame Comics filter".into()))?;
            match filter.name.as_str() {
                "status" => {
                    if value != "all" {
                        entries.retain(|e| e.status_label.eq_ignore_ascii_case(&value));
                    }
                }
                "type" => {
                    if value != "all" {
                        entries.retain(|e| e.media_type.eq_ignore_ascii_case(&value));
                    }
                }
                "year" => {
                    if value != "all" {
                        let year = value
                            .parse::<i64>()
                            .map_err(|_| Error::Host("Invalid year".into()))?;
                        entries.retain(|e| e.year == Some(year));
                    }
                }
                "genre" => {
                    if value != "all" {
                        entries.retain(|e| {
                            e.manga
                                .tags
                                .as_deref()
                                .unwrap_or_default()
                                .iter()
                                .any(|t| t.eq_ignore_ascii_case(&value))
                        });
                    }
                }
                "sort" if value == "title" || value == "popular" => sort = value,
                _ => return Err(Error::Unsupported),
            }
        }
        if !query.is_empty() {
            entries.retain(|e| {
                e.manga.title.to_lowercase().contains(&query)
                    || e.manga
                        .description
                        .as_deref()
                        .is_some_and(|s| s.to_lowercase().contains(&query))
                    || e.manga
                        .authors
                        .as_deref()
                        .unwrap_or_default()
                        .iter()
                        .any(|s| s.to_lowercase().contains(&query))
                    || e.manga
                        .tags
                        .as_deref()
                        .unwrap_or_default()
                        .iter()
                        .any(|s| s.to_lowercase().contains(&query))
                    || e.manga
                        .artist
                        .as_deref()
                        .is_some_and(|s| s.to_lowercase().contains(&query))
            });
        }
        if sort == "popular" {
            entries.sort_by(|a, b| {
                b.likes
                    .cmp(&a.likes)
                    .then_with(|| a.manga.title.cmp(&b.manga.title))
            });
        } else {
            entries.sort_by_cached_key(|e| e.manga.title.to_lowercase());
        }
        paged(
            entries.into_iter().map(CatalogEntry::into_media).collect(),
            page,
        )
    }
    fn get_media_update(
        entry: MediaEntry,
        needs_details: bool,
        needs_units: bool,
    ) -> Result<MediaEntry> {
        if !needs_details && !needs_units {
            return Ok(entry);
        }
        let mut updated = detail(entry.kind(), entry.key(), needs_units)?;
        // Detail-only refreshes must not discard previously fetched chapters.
        if !needs_units {
            match (&mut updated, entry) {
                (MediaEntry::Manga(new), MediaEntry::Manga(old)) => new.chapters = old.chapters,
                (MediaEntry::Novel(new), MediaEntry::Novel(old)) => new.chapters = old.chapters,
                _ => return Err(Error::Unsupported),
            }
        }
        Ok(updated)
    }
    fn get_chapter_pages(entry: MediaEntry, chapter: Chapter) -> Result<Vec<Page>> {
        if !matches!(entry.kind(), MediaKind::Manga | MediaKind::Novel)
            || !valid_id(entry.key())
            || !valid_token(&chapter.key)
        {
            return Err(Error::Unsupported);
        }
        let html = request(&format!(
            "/{}/{}/{}",
            route(entry.kind()),
            entry.key(),
            chapter.key
        ))?;
        let data = next_data(&html)?;
        reader_access(&data)?;
        let dto = data.get("chapter").ok_or_else(malformed)?;
        if dto.get("token").and_then(Value::as_str) != Some(chapter.key.as_str()) {
            return Err(malformed());
        }
        let contents = if entry.kind() == MediaKind::Novel {
            if data.get("isPublic").and_then(Value::as_bool) != Some(true) {
                return Err(malformed());
            }
            let content = dto
                .get("content")
                .and_then(Value::as_str)
                .ok_or_else(malformed)?;
            paragraphs(content)?
        } else {
            reader_images(&html, entry.key(), &chapter.key)?
                .into_iter()
                .map(PageContent::Url)
                .collect()
        };
        if contents.is_empty() {
            return Err(Error::Server(
                "Flame Comics chapter contains no readable content".into(),
            ));
        }
        Ok(contents
            .into_iter()
            .enumerate()
            .map(|(index, content)| Page {
                index: index as i32,
                content,
                has_description: false,
                description: None,
                headers: None,
            })
            .collect())
    }
    fn handle_url(url: &str) -> Result<LinkValue> {
        let (kind, id) = parse_link(url).ok_or(Error::Unsupported)?;
        match detail(kind, &id, true)? {
            MediaEntry::Manga(m) => Ok(LinkValue::Manga(m)),
            MediaEntry::Novel(n) => Ok(LinkValue::Novel(n)),
            _ => Err(Error::Unsupported),
        }
    }
    fn get_image_origins() -> Result<Vec<String>> {
        Ok(vec![IMAGE_ORIGIN.into()])
    }
    fn get_image_request(url: &str) -> Result<PluginImageRequest> {
        let url = image_url(url).ok_or(Error::Unsupported)?;
        Ok(PluginImageRequest {
            url,
            headers: Some(HashMap::from([("Referer".into(), REFERER.into())])),
        })
    }
}
ito_rs::export_plugin!(FlameComics);

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn comic(id: i64) -> Value {
        json!({"series_id":id,"title":"Synthetic comic","type":"Manhwa","categories":["Action"],"cover":"thumbnail.webp","last_edit":12})
    }
    #[test]
    fn mixed_catalog_preserves_colliding_numeric_ids_and_novel_metadata() {
        let data = json!({"series":[comic(5), {"novel_id":5,"title":"Synthetic novel","type":"Web Novel","categories":["Mature"],"author":["Writer"],"cover":"novel.webp"}]});
        let mut entries = parse_catalog(&data).expect("valid catalog");
        assert_eq!(entries.len(), 2);
        let MediaEntry::Novel(novel) = entries.pop().expect("novel entry").into_media() else {
            panic!("novel dropped or misclassified")
        };
        assert_eq!(novel.key, "5");
        assert_eq!(novel.authors, Some(vec!["Writer".into()]));
        assert!(novel.cover.expect("novel cover").contains("/novels/5/"));
        assert!(matches!(
            novel.content_rating,
            novel::ContentRating::Suggestive
        ));
        assert!(parse_catalog(&json!({"series":[{"title":"Missing identity"}]})).is_err());
        assert!(parse_catalog(&json!({"error":"blocked"})).is_err());
        let linked = json!({"series_id":166,"novel_id":5,"title":"Linked comic","type":"Manhwa"});
        let entry = parse_entry(&linked).expect("cross-linked comic detail");
        assert_eq!(entry.kind, MediaKind::Manga);
        assert_eq!(entry.manga.key, "166");
    }
    #[test]
    fn chapter_dto_orders_zero_fractional_and_paid_chapters_without_duplicates() {
        let data = json!({"chapters":[
            {"token":"aaaaaaaaaaaaaaaa","chapter":"12.50","title":"Extra","price":"2.00","release_date":1234},
            {"token":"bbbbbbbbbbbbbbbb","chapter":"0.00","title":"Prologue","price":"0.00"},
            {"token":"aaaaaaaaaaaaaaaa","chapter":"12.50"}]});
        let chapters = parse_chapters(&data, MediaKind::Novel, "5").expect("chapters");
        assert_eq!(chapters.len(), 2);
        assert_eq!(chapters[0].chapter, Some(0.0));
        assert_eq!(chapters[1].chapter, Some(12.5));
        assert_eq!(chapters[1].paywalled, None);
        assert_eq!(chapters[1].date_updated, Some(1234.0));
        assert_eq!(chapters[1].kind, ContentKind::Text);
        assert!(
            parse_chapters(
                &json!({"chapters":[{"token":"bad","chapter":"1"}]}),
                MediaKind::Manga,
                "1"
            )
            .is_err()
        );
    }
    #[test]
    fn pagination_handles_last_empty_and_invalid_pages() {
        let entries = (1..=25)
            .map(|n| parse_entry(&comic(n)).expect("entry").into_media())
            .collect::<Vec<_>>();
        assert!(paged(entries.clone(), 1).expect("first").has_next_page);
        let last = paged(entries.clone(), 2).expect("last");
        assert_eq!(last.entries.len(), 1);
        assert!(!last.has_next_page);
        let empty = paged(entries.clone(), 3).expect("empty");
        assert!(empty.entries.is_empty());
        assert!(!empty.has_next_page);
        assert!(paged(entries, 0).is_err());
    }
    #[test]
    fn links_reject_foreign_hosts_extra_segments_and_invalid_tokens() {
        assert_eq!(
            parse_link("https://flamecomics.xyz/novel/5/aaaaaaaaaaaaaaaa?x=1#top"),
            Some((MediaKind::Novel, "5".into()))
        );
        assert_eq!(
            parse_link("https://flamecomics.xyz/series/166/"),
            Some((MediaKind::Manga, "166".into()))
        );
        for url in [
            "https://flamecomics.xyz.evil/series/1",
            "https://flamecomics.xyz/series/1/bad",
            "https://flamecomics.xyz/series/1/aaaaaaaaaaaaaaaa/extra",
        ] {
            assert!(parse_link(url).is_none());
        }
    }
    #[test]
    fn reader_image_policy_excludes_hidden_promos_covers_and_foreign_sources() {
        let actual =
            "https://cdn.flamecomics.xyz/uploads/images/series/1/aaaaaaaaaaaaaaaa/page.webp";
        assert_eq!(
            visible_reader_image(actual, "display:block", false, "1", "aaaaaaaaaaaaaaaa"),
            Some(actual.into())
        );
        assert!(
            visible_reader_image(
                actual,
                "width:100%; display: none;",
                false,
                "1",
                "aaaaaaaaaaaaaaaa"
            )
            .is_none()
        );
        assert!(visible_reader_image(actual, "", true, "1", "aaaaaaaaaaaaaaaa").is_none());
        assert!(visible_reader_image(actual, "", false, "2", "aaaaaaaaaaaaaaaa").is_none());
        assert!(image_url("/_next/image?url=https%3A%2F%2Fother.example%2Fa.png").is_none());
        assert!(image_url("https://cdn.flamecomics.xyz.evil/x").is_none());
        assert_eq!(
            image_url("/_next/image?url=https%3A%2F%2Fcdn.flamecomics.xyz%2Fa.webp&w=384"),
            Some("https://cdn.flamecomics.xyz/a.webp".into())
        );
    }
    #[test]
    fn reader_access_never_treats_locked_payload_as_empty_success() {
        assert!(matches!(
            reader_access(&json!({"isPublic":false})),
            Err(Error::NeedsLogin(_))
        ));
        assert!(reader_access(&json!({"isPublic":true})).is_ok());
    }
    #[test]
    fn html_boundaries_preserve_paragraphs_and_exclude_reader_promos() {
        let text =
            paragraphs("<p> First <em>paragraph</em> &amp; details. </p><p> </p><p>Second.</p>")
                .expect("HTML paragraphs");
        assert_eq!(text.len(), 2);
        assert!(
            matches!(&text[0], PageContent::Text(value) if value == "First paragraph & details.")
        );
        assert!(matches!(&text[1], PageContent::Text(value) if value == "Second."));
        let html = r#"<main><img src="https://cdn.flamecomics.xyz/uploads/images/series/1/aaaaaaaaaaaaaaaa/one.webp" style="display:block"><img src="https://cdn.flamecomics.xyz/uploads/images/series/1/aaaaaaaaaaaaaaaa/promo.webp" style="display:none"><img src="https://cdn.flamecomics.xyz/uploads/images/series/1/cover.webp"><img src="https://cdn.flamecomics.xyz/uploads/images/series/1/aaaaaaaaaaaaaaaa/one.webp" style="display:block"></main>"#;
        let visible = reader_images(html, "1", "aaaaaaaaaaaaaaaa").expect("reader images");
        assert_eq!(visible.len(), 1);
        assert!(visible[0].ends_with("/one.webp"));
        assert!(
            next_data(
                r#"<script id="__NEXT_DATA__">{"props":{"pageProps":{"series":[]}}}</script>"#
            )
            .is_ok()
        );
        assert!(next_data(r#"<script id="__NEXT_DATA__">{"error":"blocked"}</script>"#).is_err());
    }
    fn texts(content: &[PageContent]) -> Vec<&str> {
        content
            .iter()
            .map(|c| match c {
                PageContent::Text(value) => value.as_str(),
                _ => panic!("expected text content"),
            })
            .collect()
    }
    #[test]
    fn paragraphs_map_br_to_line_breaks() {
        let content = paragraphs("<p>Line one<br>Line two</p>").expect("HTML paragraphs");
        assert_eq!(texts(&content), ["Line one\nLine two"]);
    }
    #[test]
    fn paragraphs_skip_script_and_style_text() {
        let content = paragraphs(
            "<p>Visible</p><script>var secret=1;</script><style>p{color:red}</style><p>Also <script>track()</script>visible</p>",
        )
        .expect("HTML paragraphs");
        assert_eq!(texts(&content), ["Visible", "Also visible"]);
    }
    #[test]
    fn paragraphs_keep_text_outside_p_tags() {
        let content =
            paragraphs("<div>Alpha<br>Beta</div>Gamma<div>Delta</div>").expect("HTML paragraphs");
        assert_eq!(texts(&content), ["Alpha\nBeta", "Gamma", "Delta"]);
    }
    #[test]
    fn paragraphs_do_not_duplicate_nested_paragraphs() {
        let content = paragraphs("<div>Intro<p>Inner</p>Tail</div>").expect("HTML paragraphs");
        assert_eq!(texts(&content), ["Intro", "Inner", "Tail"]);
    }
    #[test]
    fn chapter_access_is_public_only_when_release_is_recorded() {
        let data = json!({"chapters":[
            {"token":"0000000000000001","chapter":"1","price":"0.00"},
            {"token":"0000000000000002","chapter":"2","price":"50.00"},
            {"token":"0000000000000003","chapter":"3","price":"50.00","public_release":1786152471},
            {"token":"0000000000000004","chapter":"4"}]});
        let chapters = parse_chapters(&data, MediaKind::Novel, "5").expect("chapters");
        let access: Vec<_> = chapters.iter().map(|c| c.paywalled).collect();
        assert_eq!(access, [Some(false), None, Some(false), None]);
    }
    #[test]
    fn capabilities_pass_sdk_validation_with_missing_status_and_sentinel_values() {
        let genres = (0..300)
            .map(|n| format!("Genre {n:03}"))
            .collect::<Vec<_>>();
        let data = json!({"series":[
            {"series_id":1,"title":"Missing status","type":"Manhwa"},
            {"series_id":2,"title":"Sentinels","type":"all","status":"all","categories":["all","","Action"],"year":2020},
            {"series_id":3,"title":"Many genres","type":"Manhwa","status":"Ongoing","categories":genres,"year":2021}]});
        let entries = parse_catalog(&data).expect("catalog");
        let capabilities = build_search_capabilities(&entries);
        ito_rs::controls::validate_capabilities(&capabilities).expect("SDK accepts capabilities");
    }
    #[test]
    fn browse_row_uses_browse_listing_order() {
        let data = json!({"series":[
            {"series_id":1,"title":"Zeta","type":"Manhwa","likes":100},
            {"series_id":2,"title":"Alpha","type":"Manhwa","likes":1},
            {"series_id":3,"title":"Mid","type":"Manhwa","likes":50}]});
        let mut entries = parse_catalog(&data).expect("catalog");
        sort_browse(&mut entries);
        let titles: Vec<_> = entries.iter().map(|e| e.manga.title.as_str()).collect();
        assert_eq!(titles, ["Alpha", "Mid", "Zeta"]);
    }
    #[test]
    fn http_errors_map_to_login_only_for_auth_and_challenges() {
        assert!(response_error(200, b"<html></html>").is_none());
        assert!(matches!(
            response_error(401, b""),
            Some(Error::NeedsLogin(_))
        ));
        assert!(matches!(
            response_error(403, b"<html>denied</html>"),
            Some(Error::Server(_))
        ));
        assert!(matches!(
            response_error(403, b"<title>Just a moment...</title>"),
            Some(Error::NeedsLogin(_))
        ));
        assert!(matches!(
            response_error(503, b"cf-chl-bypass"),
            Some(Error::NeedsLogin(_))
        ));
        assert!(matches!(response_error(404, b""), Some(Error::NotFound(_))));
    }
}
