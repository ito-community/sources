//! NovelBin's public novel catalog, chapter index, and text reader.

use ito_rs::models::novel::{ContentRating, Novel, Status};
use ito_rs::models::{
    Chapter, ContentKind, FilterItem, HomeComponent, HomeComponentValue, HomeLayout, Link,
    LinkValue, Listing, MediaEntry, MediaKind, MediaPageResult, Page, PageContent,
    PluginImageRequest,
};
use ito_rs::net::Request;
use ito_rs::provider::SourceProvider;
use ito_rs::{Error, Result};
use scraper::{ElementRef, Html};
use serde_json::Value;
use std::collections::HashMap;

const SITE: &str = "https://novelbin.xyz";
const CDN: &str = "https://boxnovela.b-cdn.net";
/// Local search batch size; remote paged listings use their own one-based page files.
const PAGE_SIZE: usize = 24;

pub struct NovelBin;

fn host_error() -> Error {
    Error::Host("NovelBin returned data the plugin does not understand".into())
}

fn decode_json(status: i32, body: &[u8]) -> Result<Value> {
    match status {
        200..=299 => serde_json::from_slice(body).map_err(|_| host_error()),
        // The plugin declares no settings, so an auth refusal must send the user to the website.
        _ if (400..500).contains(&status) && is_challenge(body) => Err(Error::NeedsLogin(
            "Open NovelBin in the source browser to complete site verification".into(),
        )),
        401 => Err(Error::NeedsLogin(
            "Sign in to NovelBin in the source browser".into(),
        )),
        403 => Err(Error::Server("NovelBin denied this request".into())),
        404 => Err(Error::NotFound(
            "NovelBin could not find that catalog entry".into(),
        )),
        status => Err(Error::Server(format!("NovelBin returned HTTP {status}"))),
    }
}

/// Recognises challenge pages from their markers only; the body is never logged or returned.
fn is_challenge(body: &[u8]) -> bool {
    let text = String::from_utf8_lossy(body);
    text.contains("cf-chl-")
        || text.contains("<title>Just a moment")
        || text.contains("<title>Attention Required!")
}

/// The site's own catalog requests are cross-origin fetches, so the CDN expects their Origin.
fn get_json(path: &str) -> Result<Value> {
    let url = format!("{CDN}/{path}");
    let response = Request::get(&url)
        .header("Referer", &format!("{SITE}/"))
        .header("Origin", SITE)
        .send()?;
    decode_json(response.status, &response.body)
}

fn get_list(path: &str) -> Result<Value> {
    match get_json(path) {
        Ok(value) => Ok(value),
        Err(Error::NotFound(_)) if path.starts_with("lists/web_") => {
            get_json(&path.replacen("lists/web_", "lists/", 1))
        }
        Err(error) => Err(error),
    }
}

fn safe_segment(value: &str) -> Option<&str> {
    (!value.is_empty()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_')))
    .then_some(value)
}

fn string(value: &Value, key: &str) -> Option<String> {
    value.get(key)?.as_str().map(str::to_owned)
}

fn array(value: &Value) -> Option<&[Value]> {
    value.as_array().map(Vec::as_slice).or_else(|| {
        value
            .get("data")
            .and_then(Value::as_array)
            .map(Vec::as_slice)
    })
}

fn image_url(raw: Option<String>) -> Option<String> {
    let raw = raw?;
    let normalized = raw.replace("https://api.boxnovela.com/", "https://boxnovela.b-cdn.net/");
    if normalized.starts_with("https://boxnovela.b-cdn.net/") {
        Some(normalized)
    } else if let Some(path) = normalized.strip_prefix("/thumbnail/") {
        Some(format!("{CDN}/thumbnail/{path}"))
    } else if let Some(path) = normalized.strip_prefix("thumbnail/") {
        Some(format!("{CDN}/thumbnail/{path}"))
    } else if normalized.starts_with('/') && !normalized.starts_with("//") {
        Some(format!("{SITE}{normalized}"))
    } else {
        None
    }
}

fn image_origin(url: &str) -> Option<&'static str> {
    let rest = url.strip_prefix("https://")?;
    let (host, path) = rest.split_once('/')?;
    if path.is_empty() || host.contains('@') || host.contains(':') {
        return None;
    }
    match host {
        "boxnovela.b-cdn.net" => Some(CDN),
        "novelbin.xyz" => Some(SITE),
        _ => None,
    }
}

fn content_rating(value: &Value) -> ContentRating {
    let adult = ["adult", "is_adult", "nsfw", "18_plus"]
        .iter()
        .any(|key| value.get(*key).and_then(Value::as_bool) == Some(true));
    if adult {
        ContentRating::Nsfw
    } else {
        ContentRating::Safe
    }
}

fn status(value: Option<&str>) -> Status {
    match value.unwrap_or_default().to_ascii_lowercase().as_str() {
        "completed" | "complete" | "finished" => Status::Completed,
        "hiatus" | "on hiatus" => Status::Hiatus,
        "cancelled" | "canceled" => Status::Cancelled,
        "ongoing" | "updating" | "in progress" => Status::Ongoing,
        _ => Status::Unknown,
    }
}

fn novel_from(value: &Value, slug: &str) -> Novel {
    let title = string(value, "title").unwrap_or_else(|| slug.replace('-', " "));
    let author = string(value, "author").filter(|author| !author.trim().is_empty());
    let authors = author.map(|author| vec![author]);
    let tags = value
        .get("genres")
        .and_then(Value::as_array)
        .map(|genres| {
            genres
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect::<Vec<_>>()
        })
        .filter(|genres| !genres.is_empty());
    let rating = content_rating(value);
    Novel {
        key: slug.to_owned(),
        title,
        authors,
        artist: None,
        description: string(value, "description")
            .and_then(|description| clean_description(&description)),
        tags,
        cover: image_url(string(value, "cover").or_else(|| string(value, "image"))),
        url: Some(format!("{SITE}/novel/{slug}")),
        status: status(string(value, "status").as_deref()),
        content_rating: rating.clone(),
        nsfw: i32::from(matches!(rating, ContentRating::Nsfw)),
        chapters: None,
    }
}
fn entry_from_item(item: &Value) -> Result<MediaEntry> {
    let slug = string(item, "slug").ok_or_else(host_error)?;
    let slug = safe_segment(&slug).ok_or_else(host_error)?;
    Ok(MediaEntry::Novel(novel_from(item, slug)))
}

/// How a catalog response is paged; decided by the listing path, never by the JSON shape.
#[derive(Clone, Copy)]
enum Paging {
    /// One page file per request; the site reports `has_more`.
    Remote,
    /// The response is the whole catalog, sliced into `PAGE_SIZE` batches here.
    Local,
}

fn page_result(data: &Value, page: i32, paging: Paging) -> Result<MediaPageResult> {
    let Some(items) = array(data) else {
        return Err(host_error());
    };
    let entries = items
        .iter()
        .map(entry_from_item)
        .collect::<Result<Vec<_>>>()?;
    let (entries, has_next_page) = match paging {
        Paging::Remote => {
            let has_more = data
                .get("has_more")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            (entries, has_more)
        }
        Paging::Local => {
            let start = (page.max(1) as usize - 1).saturating_mul(PAGE_SIZE);
            let has_more = start.saturating_add(PAGE_SIZE) < entries.len();
            let batch = entries.into_iter().skip(start).take(PAGE_SIZE).collect();
            (batch, has_more)
        }
    };
    Ok(MediaPageResult {
        entries,
        has_next_page,
    })
}

fn listing_path(listing: &Listing, page: i32) -> Result<(String, Paging)> {
    let page = page.max(1);
    match listing.id.as_str() {
        "popular" => Ok(("lists/web_popular.json".into(), Paging::Local)),
        "latest" => Ok((format!("lists/web_latest_{page}.json"), Paging::Remote)),
        "search" => Ok(("lists/web_search.json".into(), Paging::Local)),
        id if id.starts_with("genre:") => {
            let slug = id.strip_prefix("genre:").unwrap_or_default();
            safe_segment(slug)
                .map(|slug| {
                    (
                        format!("lists/web_genre_{slug}_{page}.json"),
                        Paging::Remote,
                    )
                })
                .ok_or_else(|| Error::NotFound("Unknown NovelBin genre".into()))
        }
        id if id.starts_with("status:") => {
            let slug = id.strip_prefix("status:").unwrap_or_default();
            safe_segment(slug)
                .map(|slug| {
                    (
                        format!("lists/web_status_{slug}_{page}.json"),
                        Paging::Remote,
                    )
                })
                .ok_or_else(|| Error::NotFound("Unknown NovelBin status".into()))
        }
        _ => Err(Error::NotFound("Unknown NovelBin listing".into())),
    }
}

fn page_at(index: usize, content: PageContent) -> Page {
    Page {
        index: index as i32,
        content,
        has_description: false,
        description: None,
        headers: None,
    }
}

/// Plain-text description: paragraphs are kept apart by a blank line, line breaks are kept.
fn clean_description(raw: &str) -> Option<String> {
    let paragraphs = extract_reader_content(raw)
        .ok()?
        .into_iter()
        .filter_map(|content| match content {
            PageContent::Text(text) => Some(text),
            _ => None,
        })
        .collect::<Vec<_>>();
    (!paragraphs.is_empty()).then(|| paragraphs.join("\n\n"))
}

fn chapters_for(slug: &str, novel_id: &str) -> Result<Vec<Chapter>> {
    let novel_id = safe_segment(novel_id).ok_or_else(host_error)?;
    let data = get_json(&format!("chapters/{novel_id}.json"))?;
    let items = data.as_array().ok_or_else(host_error)?;
    parse_chapters(slug, novel_id, items)
}

fn parse_chapters(slug: &str, novel_id: &str, items: &[Value]) -> Result<Vec<Chapter>> {
    items
        .iter()
        .map(|item| {
            // The row id is the reader's chapter number and does not shift when chapters are inserted.
            let id = item
                .get("id")
                .and_then(Value::as_u64)
                .ok_or_else(host_error)?;
            let title = string(item, "title");
            let number = title.as_deref().and_then(chapter_number);
            Ok(Chapter {
                key: format!("{novel_id}:{id}"),
                title,
                volume: None,
                chapter: number.map(|number| number as f32),
                date_updated: None,
                scanlator: None,
                url: Some(format!("{SITE}/read/{slug}/chapter-{}", id + 1)),
                lang: Some("en".into()),
                paywalled: None,
                kind: ContentKind::Text,
            })
        })
        .collect()
}

fn chapter_number(title: &str) -> Option<f64> {
    let title = title.trim();
    let word = title.get(..7)?;
    if !word.eq_ignore_ascii_case("chapter") {
        return None;
    }
    let rest = &title[7..];
    // The site separates the word and number with a regular or a non-breaking space.
    let normalized = rest.trim_start();
    if normalized.len() == rest.len() {
        return None;
    }
    let number = normalized
        .split(|character: char| !character.is_ascii_digit() && character != '.')
        .next()?;
    number.parse().ok()
}

fn readable_chapter_path(chapter: &Chapter) -> Result<String> {
    let (novel_id, id) = chapter.key.split_once(':').ok_or_else(host_error)?;
    let novel_id = safe_segment(novel_id).ok_or_else(host_error)?;
    let id: u64 = id.parse().map_err(|_| host_error())?;
    Ok(format!("read/{novel_id}/{id}.json"))
}

fn chapter_pages(chapter: &Chapter) -> Result<Vec<Page>> {
    let data = get_json(&readable_chapter_path(chapter)?)?;
    let html = string(&data, "content").ok_or_else(host_error)?;
    let contents = extract_reader_content(&html)?;
    if contents.is_empty() {
        return Err(Error::NotFound(
            "NovelBin returned no readable chapter content".into(),
        ));
    }
    Ok(contents
        .into_iter()
        .enumerate()
        .map(|(index, content)| page_at(index, content))
        .collect())
}

/// Elements whose text is never story prose. Class-based ad rules were removed because no
/// observed reader payload carries ad or promo markup.
fn non_story_element(element: ElementRef<'_>) -> bool {
    matches!(
        element.value().name(),
        "script" | "style" | "noscript" | "iframe" | "nav"
    )
}

fn finish_paragraph(text: &mut String, contents: &mut Vec<PageContent>) {
    let start = text.len() - text.trim_start().len();
    let end = text.trim_end().len();
    if start >= end {
        text.clear();
        return;
    }
    text.truncate(end);
    if start > 0 {
        text.drain(..start);
    }
    contents.push(PageContent::Text(std::mem::take(text)));
}

fn extract_reader_content(html: &str) -> Result<Vec<PageContent>> {
    let document = Html::parse_fragment(html);
    let mut contents = Vec::new();
    let mut text = String::new();
    let mut block = None;
    for node in document.root_element().descendants() {
        if ElementRef::wrap(node).is_some_and(non_story_element)
            || node
                .ancestors()
                .filter_map(ElementRef::wrap)
                .any(non_story_element)
        {
            continue;
        }
        if let Some(value) = node.value().as_text() {
            let parent_block = node
                .ancestors()
                .find(|parent| {
                    parent.value().as_element().is_some_and(|element| {
                        matches!(
                            element.name(),
                            "p" | "div" | "section" | "blockquote" | "li" | "h1" | "h2" | "h3"
                        )
                    })
                })
                .map(|parent| parent.id());
            if parent_block != block {
                finish_paragraph(&mut text, &mut contents);
                block = parent_block;
            }
            text.push_str(value);
        } else if let Some(element) = node.value().as_element() {
            match element.name() {
                "br" => text.push('\n'),
                "img" => {
                    if let Some(url) = element
                        .attr("src")
                        .or_else(|| element.attr("data-src"))
                        .or_else(|| element.attr("data-original"))
                        .and_then(|url| image_url(Some(url.to_owned())))
                    {
                        finish_paragraph(&mut text, &mut contents);
                        contents.push(PageContent::Url(url));
                    }
                }
                _ => {}
            }
        }
    }
    finish_paragraph(&mut text, &mut contents);
    Ok(contents)
}

fn parse_link(url: &str) -> Option<String> {
    let url = url.trim();
    let rest = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))?;
    let (host, path) = rest.split_once('/')?;
    if !matches!(
        host.to_ascii_lowercase().as_str(),
        "novelbin.xyz"
            | "www.novelbin.xyz"
            | "novelbin.me"
            | "www.novelbin.me"
            | "novelbin.com"
            | "www.novelbin.com"
            | "novelbin.org"
            | "www.novelbin.org"
    ) {
        return None;
    }
    let mut segments = path.split(['?', '#']).next()?.split('/');
    match segments.next()? {
        "novel" | "read" => safe_segment(segments.next()?).map(str::to_owned),
        _ => None,
    }
}

impl SourceProvider for NovelBin {
    const KINDS: &'static [MediaKind] = &[MediaKind::Novel];

    fn get_home() -> Result<HomeLayout> {
        let popular = Listing {
            id: "popular".into(),
            name: "Popular novels".into(),
            kind: 0,
        };
        let latest = Listing {
            id: "latest".into(),
            name: "Latest updates".into(),
            kind: 0,
        };
        let popular_page = page_result(&get_list("lists/web_popular.json")?, 1, Paging::Local)?;
        let latest_page = page_result(&get_list("lists/web_latest_1.json")?, 1, Paging::Remote)?;
        let genres = [
            "action",
            "adventure",
            "fantasy",
            "romance",
            "sci-fi",
            "martial-arts",
            "xuanhuan",
            "xianxia",
            "mystery",
            "supernatural",
            "comedy",
            "drama",
            "historical",
            "horror",
            "psychological",
        ]
        .into_iter()
        .map(|genre| Link {
            title: genre.replace('-', " ").to_ascii_uppercase(),
            value: Some(LinkValue::Listing(Listing {
                id: format!("genre:{genre}"),
                name: genre.replace('-', " "),
                kind: 0,
            })),
        })
        .collect();
        Ok(HomeLayout {
            components: vec![
                HomeComponent {
                    title: Some("Popular novels".into()),
                    subtitle: None,
                    value: HomeComponentValue::NovelScroller(
                        popular_page
                            .entries
                            .into_iter()
                            .filter_map(|entry| match entry {
                                MediaEntry::Novel(novel) => Some(novel),
                                _ => None,
                            })
                            .collect(),
                        Some(popular),
                    ),
                },
                HomeComponent {
                    title: Some("Latest updates".into()),
                    subtitle: None,
                    value: HomeComponentValue::NovelScroller(
                        latest_page
                            .entries
                            .into_iter()
                            .filter_map(|entry| match entry {
                                MediaEntry::Novel(novel) => Some(novel),
                                _ => None,
                            })
                            .collect(),
                        Some(latest),
                    ),
                },
                HomeComponent {
                    title: Some("Browse by genre".into()),
                    subtitle: None,
                    value: HomeComponentValue::Links(genres),
                },
            ],
        })
    }

    fn get_listing(listing: Listing, page: i32) -> Result<MediaPageResult> {
        let (path, paging) = listing_path(&listing, page)?;
        page_result(&get_list(&path)?, page, paging)
    }

    fn search(
        query: &str,
        page: i32,
        _kind: Option<MediaKind>,
        _filters: Vec<FilterItem>,
    ) -> Result<MediaPageResult> {
        let data = get_list("lists/web_search.json")?;
        let query = query.trim().to_lowercase();
        let entries = array(&data)
            .ok_or_else(host_error)?
            .iter()
            .filter(|item| {
                query.is_empty()
                    || ["title", "alternative_titles", "slug"]
                        .iter()
                        .filter_map(|key| string(item, key))
                        .any(|field| field.to_lowercase().contains(&query))
            })
            .map(entry_from_item)
            .collect::<Result<Vec<_>>>()?;
        let offset = (page.max(1) as usize - 1).saturating_mul(PAGE_SIZE);
        let has_next_page = offset.saturating_add(PAGE_SIZE) < entries.len();
        Ok(MediaPageResult {
            entries: entries.into_iter().skip(offset).take(PAGE_SIZE).collect(),
            has_next_page,
        })
    }

    fn get_media_update(
        mut entry: MediaEntry,
        needs_details: bool,
        needs_units: bool,
    ) -> Result<MediaEntry> {
        let MediaEntry::Novel(novel) = &mut entry else {
            return Err(Error::Unsupported);
        };
        if !needs_details && !needs_units {
            return Ok(entry);
        }
        let slug = safe_segment(&novel.key).ok_or_else(host_error)?;
        let details = if needs_details || needs_units {
            Some(get_json(&format!("novel/{slug}.json"))?)
        } else {
            None
        };
        if let Some(details) = details.as_ref() {
            novel.title = string(details, "title").unwrap_or_else(|| novel.title.clone());
            novel.authors = string(details, "author").map(|author| vec![author]);
            novel.description = string(details, "description")
                .and_then(|description| clean_description(&description));
            novel.tags = details
                .get("genres")
                .and_then(Value::as_array)
                .map(|genres| {
                    genres
                        .iter()
                        .filter_map(Value::as_str)
                        .map(str::to_owned)
                        .collect()
                });
            novel.cover = image_url(string(details, "cover").or_else(|| string(details, "image")))
                .or(novel.cover.take());
            novel.status = status(string(details, "status").as_deref());
            novel.content_rating = content_rating(details);
            novel.nsfw = i32::from(matches!(novel.content_rating, ContentRating::Nsfw));
            novel.url = Some(format!("{SITE}/novel/{slug}"));
        }
        if needs_units {
            let novel_id = details
                .as_ref()
                .and_then(|value| value.get("id"))
                .map(|id| {
                    id.as_str()
                        .map(str::to_owned)
                        .unwrap_or_else(|| id.to_string())
                })
                .unwrap_or_else(|| slug.to_owned());
            novel.chapters = Some(chapters_for(slug, &novel_id)?);
        }
        Ok(entry)
    }

    fn get_chapter_pages(entry: MediaEntry, chapter: Chapter) -> Result<Vec<Page>> {
        if !matches!(entry, MediaEntry::Novel(_)) {
            return Err(Error::Unsupported);
        }
        chapter_pages(&chapter)
    }

    fn handle_url(url: &str) -> Result<LinkValue> {
        let slug = parse_link(url).ok_or(Error::Unsupported)?;
        let data = get_json(&format!("novel/{slug}.json"))?;
        Ok(LinkValue::Novel(novel_from(&data, &slug)))
    }

    fn get_image_origins() -> Result<Vec<String>> {
        Ok(vec![CDN.into(), SITE.into()])
    }
    fn get_image_request(url: &str) -> Result<PluginImageRequest> {
        if image_origin(url).is_none() {
            return Err(Error::Unsupported);
        }
        Ok(PluginImageRequest {
            url: url.to_owned(),
            headers: Some(HashMap::from([("Referer".into(), format!("{SITE}/"))])),
        })
    }
}

ito_rs::export_plugin!(NovelBin);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_and_historical_series_links_keep_slug_identity() {
        for url in [
            "https://novelbin.xyz/novel/quiet-sample",
            "https://novelbin.me/read/quiet-sample/chapter-1",
            "http://www.novelbin.com/novel/quiet-sample",
        ] {
            assert_eq!(
                parse_link(url).as_deref(),
                Some("quiet-sample"),
                "input {url}"
            );
        }
        assert_eq!(parse_link("https://example.org/novel/quiet-sample"), None);
        assert_eq!(parse_link("https://novelbin.xyz/novel/../private"), None);
    }

    #[test]
    fn remote_paged_listings_return_every_item_and_follow_has_more() {
        let mut items = (0..25).map(|index| serde_json::json!({"id":format!("item-{index}"),"slug":format!("story-{index}"),"title":format!("Story {index}")})).collect::<Vec<_>>();
        let data = serde_json::json!({"data":items,"has_more":true});
        let first = page_result(&data, 1, Paging::Remote).expect("synthetic list shape is valid");
        assert_eq!(first.entries.len(), 25);
        assert!(first.has_next_page);
        items.truncate(24);
        let final_data = serde_json::json!({"data":items,"has_more":false});
        let final_page =
            page_result(&final_data, 2, Paging::Remote).expect("synthetic list shape is valid");
        assert_eq!(final_page.entries.len(), 24);
        assert!(!final_page.has_next_page);
    }

    #[test]
    fn malformed_catalog_and_untrusted_slugs_fail_closed() {
        assert!(
            page_result(
                &serde_json::json!({"data":[{"title":"missing slug"}],"has_more":false}),
                1,
                Paging::Remote,
            )
            .is_err()
        );
        assert!(page_result(&serde_json::json!({"unexpected": []}), 1, Paging::Local).is_err());
        assert_eq!(safe_segment("../sample"), None);
        assert_eq!(safe_segment("sample/name"), None);
        assert_eq!(safe_segment("sample-name_2"), Some("sample-name_2"));
    }

    #[test]
    fn chapter_rows_are_keyed_by_row_id_and_numbered_from_titles() {
        let rows = [
            serde_json::json!({"id":0,"tocIndex":0,"title":"Chapter 0: Beginning"}),
            serde_json::json!({"id":1,"tocIndex":1,"title":"Chapter 3.5: Interlude"}),
            serde_json::json!({"id":7,"tocIndex":2,"title":"Chapter\u{a0}4: Restricted"}),
        ];
        let chapters =
            parse_chapters("quiet-sample", "91", &rows).expect("synthetic rows are valid");
        assert_eq!(
            chapters
                .iter()
                .map(|chapter| chapter.key.as_str())
                .collect::<Vec<_>>(),
            ["91:0", "91:1", "91:7"]
        );
        assert_eq!(
            chapters
                .iter()
                .map(|chapter| chapter.chapter)
                .collect::<Vec<_>>(),
            [Some(0.0), Some(3.5), Some(4.0)]
        );
        assert_eq!(chapters[2].paywalled, None);
        assert_eq!(chapters[0].kind, ContentKind::Text);
        assert_eq!(
            readable_chapter_path(&chapters[2]).expect("row id is addressable"),
            "read/91/7.json"
        );
        assert!(
            parse_chapters(
                "quiet-sample",
                "91",
                &[serde_json::json!({"title":"Chapter 1"})]
            )
            .is_err()
        );
    }

    #[test]
    fn response_errors_and_invalid_json_are_distinguished_without_body_leakage() {
        assert!(matches!(
            decode_json(401, b"private fixture"),
            Err(Error::NeedsLogin(_))
        ));
        assert!(matches!(
            decode_json(403, b"challenge fixture"),
            Err(Error::Server(_))
        ));
        assert!(matches!(
            decode_json(403, b"<title>Just a moment</title>"),
            Err(Error::NeedsLogin(_))
        ));
        assert!(matches!(
            decode_json(404, b"not found fixture"),
            Err(Error::NotFound(_))
        ));
        assert!(matches!(
            decode_json(503, b"server fixture"),
            Err(Error::Server(_))
        ));
        assert!(matches!(
            decode_json(200, b"<html>fixture</html>"),
            Err(Error::Host(_))
        ));
    }

    #[test]
    fn description_paragraphs_and_line_breaks_are_kept_apart() {
        assert_eq!(
            clean_description("<p>First line.</p><p>Second line.</p>").as_deref(),
            Some("First line.\n\nSecond line.")
        );
        assert_eq!(
            clean_description("<p>One<br>Two</p><script>ignored</script>").as_deref(),
            Some("One\nTwo")
        );
        assert_eq!(clean_description("<p> </p>"), None);
    }

    #[test]
    fn paragraphs_and_illustrations_keep_order_and_skip_scripts() {
        let contents = extract_reader_content(
            r#"
            <p> First paragraph. </p>
            <p><img src="https://boxnovela.b-cdn.net/image-fixture.webp"></p>
            <p>Second paragraph.</p><script>ignored script</script>
        "#,
        )
        .expect("synthetic story HTML parses");
        assert!(matches!(&contents[0], PageContent::Text(text) if text == "First paragraph."));
        assert!(
            matches!(&contents[1], PageContent::Url(url) if url.ends_with("/image-fixture.webp"))
        );
        assert!(matches!(&contents[2], PageContent::Text(text) if text == "Second paragraph."));
        assert_eq!(contents.len(), 3);
    }
    #[test]
    fn image_requests_are_limited_to_declared_origins() {
        assert_eq!(
            image_origin("https://boxnovela.b-cdn.net/covers/sample.webp"),
            Some(CDN)
        );
        assert_eq!(
            image_origin("https://novelbin.xyz/covers/sample.webp"),
            Some(SITE)
        );
        assert_eq!(
            image_origin("https://boxnovela.b-cdn.net.evil.test/image"),
            None
        );
        assert_eq!(image_origin("http://boxnovela.b-cdn.net/image"), None);
        assert_eq!(
            image_url(Some("https://api.boxnovela.com/covers/sample.webp".into())).as_deref(),
            Some("https://boxnovela.b-cdn.net/covers/sample.webp")
        );
        assert_eq!(
            image_url(Some("/thumbnail/sample.webp".into())).as_deref(),
            Some("https://boxnovela.b-cdn.net/thumbnail/sample.webp")
        );
    }

    #[test]
    fn adult_metadata_is_not_marked_safe() {
        assert!(matches!(
            content_rating(&serde_json::json!({"is_adult":true})),
            ContentRating::Nsfw
        ));
        assert!(matches!(
            content_rating(&serde_json::json!({"is_adult":false})),
            ContentRating::Safe
        ));
    }
}
