use std::collections::HashMap;
use ito_rs::export_manga_plugin;
use ito_rs::provider::MangaProvider;
use ito_rs::models::{Listing, FilterItem, Page, PageContent, HomeLayout, HomeComponent, HomeComponentValue, Setting, SettingsSchema, LinkValue};
use ito_rs::models::manga::{Manga, Chapter, PageResult, Status, ContentRating, Viewer};
use ito_rs::net::Request;
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
    number: f32,
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

fn api<T: for<'de> Deserialize<'de>>(path: &str, params: &mut [(&str, String)]) -> Result<T> {
    let url = signed_api_url(path, params);
    let res = Request::get(url)
        .header("Accept", "application/json")
        .header("Referer", &format!("{}/", BASE_URL))
        .send()?;
    if res.status != 200 {
        return Err(ito_rs::Error::Host(format!("MangaFire API returned HTTP {}", res.status)));
    }
    serde_json::from_slice::<T>(&res.body).map_err(|e| ito_rs::Error::Host(format!("JSON error: {}", e)))
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
        let response: ApiResponse<ApiManga> = api(
            "/titles",
            &mut [
                ("order[chapter_updated_at]", "desc".to_string()),
                ("hot", "1".to_string()),
                ("page", page.to_string()),
                ("limit", PAGE_LIMIT.to_string()),
            ],
        )?;
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
        }))
    }

    fn get_home_stream() -> Result<bool> {
        Ok(false)
    }

    fn get_home() -> Result<HomeLayout> {
        let mut components = Vec::new();

        for (title, days) in [("Trending Today", 1), ("Trending This Week", 7), ("Trending This Month", 30)] {
            let response: ApiResponse<ApiManga> = api(
                "/top-titles",
                &mut [
                    ("type", "trending".to_string()),
                    ("days", days.to_string()),
                    ("limit", "30".to_string()),
                ],
            )?;
            let entries: Vec<Manga> = response.items.into_iter().map(manga_from_api).collect();
            if !entries.is_empty() {
                components.push(HomeComponent {
                    title: Some(title.to_string()),
                    subtitle: None,
                    value: HomeComponentValue::Scroller(entries, None),
                });
            }
        }

        let hot = Self::fetch_hot_updates(1)?;
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
                let mut page = 1;
                loop {
                    let response: ApiResponse<ApiChapter> = api(
                        &format!("/titles/{}/chapters", hid),
                        &mut [
                            ("language", lang.clone()),
                            ("sort", "number".to_string()),
                            ("order", "desc".to_string()),
                            ("page", page.to_string()),
                            ("limit", "200".to_string()),
                        ],
                    )?;
                    let has_next = response.meta.as_ref().map(|m| m.has_next).unwrap_or(false);
                    let done = response.items.is_empty() || !has_next;
                    for item in response.items {
                        let title = item.name.filter(|n| !n.is_empty());
                        chapters.push(Chapter {
                            key: item.id.to_string(),
                            title,
                            volume: None,
                            chapter: Some(item.number),
                            date_updated: None,
                            scanlator: None,
                            url: Some(format!("{}/title/{}/chapter/{}", BASE_URL, hid, item.id)),
                            lang: Some(lang.clone()),
                            paywalled: None,
                        });
                    }
                    if done {
                        break;
                    }
                    page += 1;
                }
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
