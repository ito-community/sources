use std::collections::HashMap;
use ito_rs::export_manga_plugin;
use ito_rs::provider::MangaProvider;
use ito_rs::models::{Listing, FilterItem, Page, PageContent, HomeLayout, HomeComponent, HomeComponentValue, Setting, SettingsSchema, LinkValue, ContentKind};
use ito_rs::models::manga::{Manga, Chapter, PageResult, Status, ContentRating, Viewer};
use ito_rs::net::{NetResponse, Request};
use ito_rs::Result;
use ito_rs::env;
use serde::Deserialize;

mod vrf;

const BASE_URL: &str = "https://mangafire.to";
const PAGE_LIMIT: i32 = 30;

#[derive(Deserialize)]
struct ApiResponse<T> {
    items: Vec<T>,
    meta: Option<ApiMeta>,
}

#[derive(Deserialize)]
struct ApiMeta {
    #[serde(rename = "hasNext")]
    has_next: bool,
    #[serde(rename = "lastPage")]
    last_page: Option<i32>,
}

#[derive(Deserialize)]
struct ApiManga {
    hid: String,
    title: String,
    poster: Option<ApiPoster>,
}

#[derive(Deserialize)]
struct ApiPoster {
    small: Option<String>,
    medium: Option<String>,
    large: Option<String>,
}

#[derive(Deserialize)]
struct ApiDetailsResponse {
    data: ApiMangaDetails,
}

#[derive(Deserialize)]
struct ApiMangaDetails {
    title: String,
    #[serde(rename = "type")]
    manga_type: Option<String>,
    status: Option<String>,
    poster: Option<ApiPoster>,
    #[serde(rename = "synopsisHtml")]
    synopsis_html: Option<String>,
    authors: Option<Vec<ApiEntity>>,
    genres: Option<Vec<ApiEntity>>,
    themes: Option<Vec<ApiEntity>>,
}

#[derive(Deserialize)]
struct ApiEntity {
    title: String,
}

#[derive(Deserialize)]
struct ApiChapter {
    id: i32,
    number: f64,
    name: Option<String>,
}

#[derive(Deserialize)]
struct ApiPagesResponse {
    data: ApiChapterPages,
}

#[derive(Deserialize)]
struct ApiChapterPages {
    pages: Vec<ApiPage>,
}

#[derive(Deserialize)]
struct ApiPage {
    url: String,
}

struct MangaFire;

/// Percent-encodes everything except RFC 3986 unreserved characters.
fn percent_encode(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for byte in input.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(byte as char),
            _ => out.push_str(&format!("%{:02X}", byte)),
        }
    }
    out
}

/// Builds a signed `/api` URL. Params are sorted by key. The canonical string used for
/// signing indexes `key[]` params (`key[0]`, `key[1]`, ...); the wire query keeps `key[]`.
fn signed_api_url(path: &str, params: &mut [(&str, String)]) -> String {
    params.sort_by(|a, b| a.0.cmp(b.0));

    let mut canonical_query = Vec::new();
    let mut wire_query = Vec::new();
    let mut last_key = String::new();
    let mut index = 0;
    for (key, value) in params.iter() {
        wire_query.push(format!("{}={}", percent_encode(key), percent_encode(value)));
        let canonical_key = match key.strip_suffix("[]") {
            Some(base) => {
                if last_key != *key {
                    index = 0;
                }
                last_key = key.to_string();
                let indexed = format!("{}[{}]", base, index);
                index += 1;
                indexed
            }
            None => key.to_string(),
        };
        canonical_query.push(format!("{}={}", canonical_key, value));
    }

    let mut canonical = path.to_string();
    if !canonical_query.is_empty() {
        canonical.push('?');
        canonical.push_str(&canonical_query.join("&"));
    }

    let mut url = format!("{}/api{}?", BASE_URL, path);
    if !wire_query.is_empty() {
        url.push_str(&wire_query.join("&"));
        url.push('&');
    }
    url.push_str("vrf=");
    url.push_str(&vrf::sign(&canonical));
    url
}

/// Builds a signed `/api` GET request.
fn api_request(path: &str, params: &mut [(&str, String)]) -> Request {
    let url = signed_api_url(path, params);
    let mut request = Request::get(url);
    request
        .header("Accept", "application/json")
        .header("Referer", &format!("{}/", BASE_URL));
    request
}

/// Checks an `/api` response's status and decodes its JSON body.
fn decode_api<T: for<'de> Deserialize<'de>>(response: NetResponse) -> Result<T> {
    if response.status != 200 {
        return Err(ito_rs::Error::Host(format!("MangaFire API returned HTTP {}", response.status)));
    }
    serde_json::from_slice::<T>(&response.body).map_err(|e| ito_rs::Error::Host(format!("JSON error: {}", e)))
}

fn api<T: for<'de> Deserialize<'de>>(path: &str, params: &mut [(&str, String)]) -> Result<T> {
    decode_api(api_request(path, params).send()?)
}

/// Builds the signed request for one page of the trending titles.
fn top_titles_request(days: i32) -> Request {
    api_request(
        "/top-titles",
        &mut [
            ("type", "trending".to_string()),
            ("days", days.to_string()),
            ("limit", "30".to_string()),
        ],
    )
}

/// Builds the signed request for one page of hot updates.
fn hot_updates_request(page: i32) -> Request {
    api_request(
        "/titles",
        &mut [
            ("order[chapter_updated_at]", "desc".to_string()),
            ("hot", "1".to_string()),
            ("page", page.to_string()),
            ("limit", PAGE_LIMIT.to_string()),
        ],
    )
}

fn manga_from_api(item: ApiManga) -> Manga {
    Manga {
        key: item.hid.clone(),
        title: item.title,
        cover: item.poster.and_then(|p| p.large.or(p.medium).or(p.small)),
        authors: None,
        artist: None,
        description: None,
        tags: None,
        url: Some(format!("{}/title/{}", BASE_URL, item.hid)),
        status: Status::Unknown,
        content_rating: ContentRating::Safe,
        nsfw: 0,
        viewer: Viewer::Default,
        chapters: None,
        catalog: None,
    }
}

fn page_from_api(response: ApiResponse<ApiManga>) -> PageResult {
    PageResult {
        has_next_page: response.meta.map(|m| m.has_next).unwrap_or(false),
        entries: response.items.into_iter().map(manga_from_api).collect(),
    }
}

fn viewer_for_type(manga_type: Option<&str>) -> Viewer {
    match manga_type {
        Some("manhua") | Some("manhwa") => Viewer::Webtoon,
        _ => Viewer::Rtl,
    }
}

fn status_for(status: Option<&str>) -> Status {
    match status {
        Some("releasing") => Status::Ongoing,
        Some("completed") => Status::Completed,
        Some("on_hiatus") => Status::Hiatus,
        Some("discontinued") => Status::Cancelled,
        _ => Status::Unknown,
    }
}

/// Removes HTML tags from a synopsis fragment, keeping text content.
fn strip_tags(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut in_tag = false;
    for ch in html.chars() {
        match ch {
            '<' => in_tag = true,
            '>' if in_tag => in_tag = false,
            _ if !in_tag => out.push(ch),
            _ => {}
        }
    }
    out.trim().to_string()
}

/// Maps one language's chapter page into the destination, keeping the site's order.
fn append_chapters_from_api(
    chapters: &mut Vec<Chapter>,
    hid: &str,
    lang: &str,
    items: Vec<ApiChapter>,
) {
    for item in items {
        let title = item.name.filter(|n| !n.is_empty());
        chapters.push(Chapter {
            title,
            chapter: Some(item.number),
            url: Some(format!("{}/title/{}/chapter/{}", BASE_URL, hid, item.id)),
            lang: Some(lang.to_string()),
            ..Chapter::new(item.id.to_string(), ContentKind::Images)
        });
    }
}

/// Reads one language's chapter pages in site order and returns their items. `first` is page 1 and
/// `batch` holds the pages after it that were already fetched, in order; pages past the batch come
/// from `fetch`. Reading stops at the first empty page or the first page without `hasNext`, so
/// pages after that point are never used.
fn read_chapter_pages(
    first: ApiResponse<ApiChapter>,
    batch: Vec<Result<ApiResponse<ApiChapter>>>,
    mut fetch: impl FnMut(i32) -> Result<ApiResponse<ApiChapter>>,
) -> Result<Vec<ApiChapter>> {
    let mut batch = batch.into_iter();
    let mut items = Vec::new();
    let mut page = 1;
    let mut response = first;
    loop {
        let has_next = response.meta.as_ref().map(|m| m.has_next).unwrap_or(false);
        let done = response.items.is_empty() || !has_next;
        items.extend(response.items);
        if done {
            return Ok(items);
        }
        page += 1;
        response = match batch.next() {
            Some(result) => result?,
            None => fetch(page)?,
        };
    }
}

fn chapter_page_request(hid: &str, lang: &str, page: i32) -> Request {
    api_request(
        &format!("/titles/{}/chapters", hid),
        &mut [
            ("language", lang.to_string()),
            ("sort", "number".to_string()),
            ("order", "desc".to_string()),
            ("page", page.to_string()),
            ("limit", "200".to_string()),
        ],
    )
}

fn chapter_page(hid: &str, lang: &str, page: i32) -> Result<ApiResponse<ApiChapter>> {
    decode_api(chapter_page_request(hid, lang, page).send()?)
}

impl MangaFire {
    fn get_langs() -> Vec<String> {
        let preferred = env::get_preferred_languages().unwrap_or_default();
        if preferred.is_empty() {
            vec!["en".to_string()]
        } else {
            preferred
        }
    }

    fn fetch_hot_updates(page: i32) -> Result<PageResult> {
        let response: ApiResponse<ApiManga> = decode_api(hot_updates_request(page).send()?)?;
        Ok(page_from_api(response))
    }
}

impl MangaProvider for MangaFire {
    fn get_settings() -> Option<SettingsSchema> {
        Some(SettingsSchema {
            settings: vec![
                Setting::Picker {
                    id: "languages".to_string(),
                    name: "Content Languages".to_string(),
                    summary: Some("Select languages for manga content".to_string()),
                    options: vec!["en", "es", "es-419", "fr", "ja", "pt", "pt-BR"].into_iter().map(String::from).collect(),
                    default_value: "en".to_string(),
                }
            ]
        })
    }

    fn handle_url(url: &str) -> Result<LinkValue> {
        // Series URLs look like https://mangafire.to/title/{hid}-{slug}; the key is the hid.
        let Some(idx) = url.find("/title/") else {
            return Err(ito_rs::Error::Host("Unsupported URL".to_string()));
        };
        let rest = &url[idx + "/title/".len()..];
        let hid: String = rest.chars().take_while(|c| c.is_ascii_alphanumeric()).collect();
        if hid.is_empty() {
            return Err(ito_rs::Error::Host("Unsupported URL".to_string()));
        }
        Ok(LinkValue::Manga(Manga {
            key: hid,
            title: String::new(),
            authors: None,
            artist: None,
            description: None,
            tags: None,
            cover: None,
            url: Some(url.to_string()),
            status: Status::Unknown,
            content_rating: ContentRating::Safe,
            nsfw: 0,
            viewer: Viewer::Default,
            chapters: None,
            catalog: None,
        }))
    }

    fn get_home() -> Result<HomeLayout> {
        const TRENDING: [(&str, i32); 3] = [
            ("Trending Today", 1),
            ("Trending This Week", 7),
            ("Trending This Month", 30),
        ];
        // The trending sections and the hot updates don't depend on each other, so they go out together.
        let mut requests: Vec<Request> = TRENDING
            .iter()
            .map(|&(_, days)| top_titles_request(days))
            .collect();
        requests.push(hot_updates_request(1));
        let mut responses = Request::send_all(&requests).into_iter();
        let mut next_page = || -> Result<ApiResponse<ApiManga>> {
            decode_api(
                responses
                    .next()
                    .expect("send_all returns one result per request")?,
            )
        };

        let mut components = Vec::new();
        for (title, _) in TRENDING {
            let entries: Vec<Manga> = next_page()?.items.into_iter().map(manga_from_api).collect();
            if !entries.is_empty() {
                components.push(HomeComponent {
                    title: Some(title.to_string()),
                    subtitle: None,
                    value: HomeComponentValue::Scroller(entries, None),
                });
            }
        }

        let hot = page_from_api(next_page()?);
        if !hot.entries.is_empty() {
            components.push(HomeComponent {
                title: Some("Recently Updated".to_string()),
                subtitle: None,
                value: HomeComponentValue::MangaList(
                    false,
                    None,
                    hot.entries,
                    Some(Listing { id: "hot".to_string(), name: "Recently Updated".to_string(), kind: 0 }),
                ),
            });
        }

        Ok(HomeLayout { components })
    }

    fn get_manga_list(listing: Listing, page: i32) -> Result<PageResult> {
        match listing.id.as_str() {
            "hot" => Self::fetch_hot_updates(page),
            other => Err(ito_rs::Error::Host(format!("Unknown MangaFire listing: {}", other))),
        }
    }

    fn get_search_manga_list(query: &str, page: i32, _filters: Vec<FilterItem>) -> Result<PageResult> {
        let mut params = vec![("page", page.to_string()), ("limit", PAGE_LIMIT.to_string())];
        if !query.is_empty() {
            params.push(("keyword", query.to_string()));
        }
        let response: ApiResponse<ApiManga> = api("/titles", &mut params)?;
        Ok(page_from_api(response))
    }

    fn get_manga_update(mut manga: Manga, needs_details: bool, needs_chapters: bool) -> Result<Manga> {
        let hid = manga.key.clone();

        if needs_details {
            let details: ApiDetailsResponse = api(&format!("/titles/{}", hid), &mut [])?;
            let d = details.data;
            manga.title = d.title;
            manga.cover = d.poster.and_then(|p| p.large.or(p.medium).or(p.small));
            manga.description = d.synopsis_html.as_deref().map(strip_tags);
            manga.authors = d
                .authors
                .map(|a| a.into_iter().map(|e| e.title).collect::<Vec<_>>())
                .filter(|a| !a.is_empty());
            let mut tags: Vec<String> = Vec::new();
            for entity in d.genres.unwrap_or_default().into_iter().chain(d.themes.unwrap_or_default()) {
                tags.push(entity.title);
            }
            manga.tags = if tags.is_empty() { None } else { Some(tags) };
            manga.status = status_for(d.status.as_deref());
            manga.viewer = viewer_for_type(d.manga_type.as_deref());
            manga.url = Some(format!("{}/title/{}", BASE_URL, hid));
        }

        if needs_chapters {
            let mut chapters = Vec::new();
            for lang in Self::get_langs() {
                let first = chapter_page(&hid, &lang, 1)?;
                let more =
                    !first.items.is_empty() && first.meta.as_ref().is_some_and(|m| m.has_next);
                // `lastPage` names the last page, so the pages after page 1 go out together. Without
                // it, the pages are read one at a time.
                let batch: Vec<Result<ApiResponse<ApiChapter>>> =
                    match first.meta.as_ref().and_then(|m| m.last_page) {
                        Some(last_page) if more && last_page >= 2 => {
                            let requests: Vec<Request> = (2..=last_page)
                                .map(|page| chapter_page_request(&hid, &lang, page))
                                .collect();
                            Request::send_all(&requests)
                                .into_iter()
                                .map(|result| result.and_then(decode_api))
                                .collect()
                        }
                        _ => Vec::new(),
                    };
                let items =
                    read_chapter_pages(first, batch, |page| chapter_page(&hid, &lang, page))?;
                append_chapters_from_api(&mut chapters, &hid, &lang, items);
            }
            manga.chapters = Some(chapters);
        }

        Ok(manga)
    }

    fn get_page_list(_manga: Manga, chapter: Chapter) -> Result<Vec<Page>> {
        let response: ApiPagesResponse = api(&format!("/chapters/{}", chapter.key), &mut [])?;
        Ok(response
            .data
            .pages
            .into_iter()
            .enumerate()
            .map(|(i, page)| Page {
                index: i as i32,
                content: PageContent::Url(page.url),
                has_description: false,
                description: None,
                headers: Some(HashMap::from([("Referer".to_string(), format!("{}/", BASE_URL))])),
            })
            .collect())
    }
}

export_manga_plugin!(MangaFire);

#[cfg(test)]
mod tests {
    use super::{ApiChapter, ApiResponse, append_chapters_from_api, read_chapter_pages};

    // A chapter list page saved from the site's English listing (first three entries, the
    // paging metadata kept). It holds public chapter data only.
    const CHAPTERS_EN: &str = r#"{"items":[{"id":8537216,"number":3,"name":"","language":"en","type":"unofficial","createdAt":1783487239},{"id":8537215,"number":1.1,"name":"Prologue","language":"en","type":"unofficial","createdAt":1783487239},{"id":8537217,"number":0,"name":"Prologue","language":"en","type":"unofficial","createdAt":1783487239}],"meta":{"total":3,"perPage":20,"page":1,"lastPage":1,"from":1,"to":3,"hasNext":false,"hasPrev":false}}"#;

    #[test]
    fn chapter_keys_are_the_site_ids_in_site_order() {
        let response: ApiResponse<ApiChapter> =
            serde_json::from_str(CHAPTERS_EN).expect("saved chapter page parses");
        let mut chapters = Vec::new();
        append_chapters_from_api(&mut chapters, "pwpz9", "en", response.items);
        let keys: Vec<&str> = chapters.iter().map(|c| c.key.as_str()).collect();
        assert_eq!(keys, ["8537216", "8537215", "8537217"]);
    }

    fn page(json: &str) -> ApiResponse<ApiChapter> {
        serde_json::from_str(json).expect("saved chapter page parses")
    }

    fn ids(chapters: &[ApiChapter]) -> Vec<i32> {
        chapters.iter().map(|c| c.id).collect()
    }

    #[test]
    fn chapter_pages_stop_at_the_last_page_and_ignore_later_batch_results() {
        let first = page(r#"{"items":[{"id":1,"number":3}],"meta":{"hasNext":true,"lastPage":3}}"#);
        let batch = vec![
            Ok(page(
                r#"{"items":[{"id":2,"number":2}],"meta":{"hasNext":false,"lastPage":3}}"#,
            )),
            Err(ito_rs::Error::Host("page 3 was never needed".into())),
        ];
        let items = read_chapter_pages(first, batch, |_| {
            Err(ito_rs::Error::Host("page past the end was fetched".into()))
        })
        .expect("pages read");
        assert_eq!(ids(&items), [1, 2]);
    }

    #[test]
    fn chapter_pages_past_the_batch_are_fetched_one_at_a_time() {
        let first = page(r#"{"items":[{"id":1,"number":3}],"meta":{"hasNext":true}}"#);
        let batch = vec![Ok(page(
            r#"{"items":[{"id":2,"number":2}],"meta":{"hasNext":true}}"#,
        ))];
        let mut fetched = Vec::new();
        let items = read_chapter_pages(first, batch, |n| {
            fetched.push(n);
            Ok(page(
                r#"{"items":[{"id":3,"number":1}],"meta":{"hasNext":false}}"#,
            ))
        })
        .expect("pages read");
        assert_eq!(fetched, [3]);
        assert_eq!(ids(&items), [1, 2, 3]);
    }

    #[test]
    fn a_failed_batch_page_fails_the_read_when_it_is_reached() {
        let first = page(r#"{"items":[{"id":1,"number":3}],"meta":{"hasNext":true,"lastPage":2}}"#);
        let batch = vec![Err(ito_rs::Error::Host("page 2 failed".into()))];
        let result = read_chapter_pages(first, batch, |_| {
            Err(ito_rs::Error::Host("unexpected fetch".into()))
        });
        assert!(result.is_err());
    }
}
