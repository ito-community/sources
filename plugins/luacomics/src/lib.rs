use ito_rs::export_manga_plugin;
use ito_rs::provider::MangaProvider;
use ito_rs::models::{Listing, FilterItem, Page, PageContent, HomeLayout, HomeComponent, HomeComponentValue, LinkValue};
use ito_rs::models::manga::{Manga, Chapter, PageResult, Status, ContentRating, Viewer};
use ito_rs::net::Request;
use ito_rs::html::Node;
use ito_rs::Result;

mod models;
use models::{ApiChapter, SeriesResponse, ApiSeries};

const BASE_URL: &str = "https://luacomic.org";
const API_URL: &str = "https://api.luacomic.org";

struct LuaComic;

impl LuaComic {
    fn urlencode(s: &str) -> String {
        let mut encoded = String::new();
        for b in s.as_bytes() {
            match *b as char {
                'A'..='Z' | 'a'..='z' | '0'..='9' | '-' | '_' | '.' | '~' => encoded.push(*b as char),
                ' ' => encoded.push_str("%20"),
                _ => encoded.push_str(&format!("%{:02X}", b)),
            }
        }
        encoded
    }

    fn get_clean_cover_url(encoded: String) -> String {
        if let Some(start) = encoded.find("url=") {
            let rest = &encoded[start + 4..];
            let end = rest.find('&').unwrap_or(rest.len());
            let url_part = &rest[..end];
            
            let decoded = url_part
                .replace("%3A", ":")
                .replace("%2F", "/")
                .replace("%3F", "?")
                .replace("%3D", "=")
                .replace("%26", "&")
                .replace("%20", " ")
                .replace("%2D", "-");
            
            return decoded;
        }
        encoded
    }

    fn slug_to_title(slug: &str) -> String {
        slug.replace('-', " ")
            .split_whitespace()
            .map(|word| {
                let mut chars = word.chars();
                match chars.next() {
                    None => String::new(),
                    Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
                }
            })
            .collect::<Vec<String>>()
            .join(" ")
    }

    fn api_series_to_manga(series: ApiSeries) -> Manga {
        Manga {
            key: format!("/series/{}", series.series_slug),
            title: series.title,
            cover: Some(Self::get_clean_cover_url(series.thumbnail)),
            url: Some(format!("{}/series/{}", BASE_URL, series.series_slug)),
            status: match series.status.as_str() {
                "Ongoing" => Status::Ongoing,
                "Completed" => Status::Completed,
                "Hiatus" => Status::Hiatus,
                "Dropped" => Status::Cancelled,
                _ => Status::Unknown,
            },
            description: series.description,
            tags: series.tags,
            authors: None,
            artist: None,
            content_rating: ContentRating::Safe,
            nsfw: 0,
            viewer: Viewer::Default,
            chapters: None,
        }
    }
}

impl MangaProvider for LuaComic {
    fn get_search_manga_list(
        query: String,
        page: i32,
        _filters: Vec<FilterItem>,
    ) -> Result<PageResult> {
        let order_by = "latest";
        let status = "All";

        let mut url = format!(
            "{}/query?page={}&perPage=20&series_type=Comic&orderBy={}&adult=true&status={}&tags_ids=[]",
            API_URL, page, order_by, status
        );

        if !query.is_empty() {
            url.push_str(&format!("&query_string={}", Self::urlencode(&query)));
        }

        let res = Request::get(&url)
            .header("Referer", "https://luacomic.org/")
            .header("Origin", "https://luacomic.org")
            .send()?;
        
        let json: SeriesResponse = serde_json::from_slice(&res.body)
            .map_err(|_| ito_rs::Error::Host("Failed to parse API response".to_string()))?;

        let entries: Vec<Manga> = json.data
            .into_iter()
            .map(Self::api_series_to_manga)
            .collect();

        let has_next_page = json.meta.current_page < json.meta.last_page;

        Ok(PageResult {
            entries,
            has_next_page,
        })
    }

    fn get_manga_list(listing: Listing, page: i32) -> Result<PageResult> {
        if listing.id.starts_with("Trending") {
            let type_param = match listing.id.as_str() {
                "TrendingDaily" => "daily",
                "TrendingWeekly" => "weekly",
                "TrendingAll" => "all",
                _ => "weekly",
            };
            let url = format!("{}/trending?type={}", API_URL, type_param);
            
            let res = Request::get(&url)
                .header("Referer", "https://luacomic.org/")
                .header("Origin", "https://luacomic.org")
                .send()?;
            
            if let Ok(series_list) = serde_json::from_slice::<Vec<ApiSeries>>(&res.body) {
                let entries: Vec<Manga> = series_list
                    .into_iter()
                    .map(Self::api_series_to_manga)
                    .collect();
                
                return Ok(PageResult {
                    entries,
                    has_next_page: false,
                });
            } else {
                return Ok(PageResult { entries: vec![], has_next_page: false });
            }
        }

        let sort_index = match listing.id.as_str() {
            "Latest" => 0,
            "EditorsChoice" => 3,
            _ => 0,
        };
        
        let order_by = if sort_index == 3 { "total_views" } else { "latest" };
        let url = format!(
            "{}/query?page={}&perPage=20&series_type=Comic&orderBy={}&adult=true&status=All&tags_ids=[]",
            API_URL, page, order_by
        );

        let res = Request::get(&url)
            .header("Referer", "https://luacomic.org/")
            .header("Origin", "https://luacomic.org")
            .send()?;
        
        if let Ok(json) = serde_json::from_slice::<SeriesResponse>(&res.body) {
            let entries: Vec<Manga> = json.data
                .into_iter()
                .map(Self::api_series_to_manga)
                .collect();

            let has_next_page = json.meta.current_page < json.meta.last_page;

            return Ok(PageResult {
                entries,
                has_next_page,
            });
        }
        
        Ok(PageResult { entries: vec![], has_next_page: false })
    }

    fn get_manga_update(
        mut manga: Manga,
        needs_details: bool,
        needs_chapters: bool,
    ) -> Result<Manga> {
        let url = format!("{}{}", BASE_URL, manga.key);
        
        if needs_details {
            if let Ok(res) = Request::get(&url).send() {
                let html = Node::new(&res.body);
                
                if let Ok(nodes) = html.select("h1.text-foreground") {
                    if let Some(n) = nodes.first() {
                        if let Ok(t) = n.text() { manga.title = t; }
                    }
                }
                
                if let Ok(nodes) = html.select("div.rounded.overflow-hidden img") {
                    if let Some(n) = nodes.first() {
                        if let Ok(Some(src)) = n.attr("src") {
                            let abs_src = if src.starts_with("http") { src } else { format!("{}{}", BASE_URL, src) };
                            manga.cover = Some(Self::get_clean_cover_url(abs_src));
                        }
                    }
                }
                
                if let Ok(nodes) = html.select("meta[name='description']") {
                    if let Some(n) = nodes.first() {
                        if let Ok(Some(content)) = n.attr("content") {
                            manga.description = Some(content);
                        }
                    }
                }
                
                if let Ok(nodes) = html.select("span.uppercase") {
                    if let Some(n) = nodes.first() {
                        if let Ok(s) = n.text() {
                            manga.status = match s.to_lowercase().as_str() {
                                "ongoing" => Status::Ongoing,
                                "completed" => Status::Completed,
                                "hiatus" => Status::Hiatus,
                                _ => Status::Unknown,
                            };
                        }
                    }
                }
                manga.viewer = Viewer::Webtoon;

                if let Ok(nodes) = html.select("div.space-y-2.rounded.p-5") {
                    if let Some(details_box) = nodes.first() {
                        if let Ok(divs) = details_box.select("div.flex.justify-between") {
                            for div in divs {
                                if let Ok(spans) = div.select("span.text-muted-foreground:first-child") {
                                    if let Some(label_span) = spans.first() {
                                        if let Ok(label) = label_span.text() {
                                            if label.contains("Author") {
                                                if let Ok(last_spans) = div.select("span:last-child") {
                                                    if let Some(last_span) = last_spans.first() {
                                                        if let Ok(auth) = last_span.text() {
                                                            manga.authors = Some(vec![auth]);
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        if needs_chapters {
            let slug = manga.key.replace("/series/", "");
            let api_url = format!("{}/chapter/all/{}", API_URL, slug);
            
            if let Ok(res) = Request::get(&api_url).send() {
                if let Ok(response) = serde_json::from_slice::<Vec<ApiChapter>>(&res.body) {
                    let mut chapters = Vec::new();
                    for ch in response {
                        let key = format!("{}/{}", manga.key, ch.chapter_slug);
                        let url = format!("{}{}", BASE_URL, key);
                        
                        let chapter_number = ch.chapter_name
                            .to_lowercase()
                            .replace("chapter ", "").trim()
                            .trim()
                            .parse::<f32>()
                            .unwrap_or_default();

                        let title = if let Some(t) = ch.chapter_title {
                            if t.is_empty() { 
                                Some(ch.chapter_name.clone())
                            } else {
                                Some(t)
                            }
                        } else {
                            Some(ch.chapter_name.clone())
                        };

                        chapters.push(Chapter {
                            key,
                            title,
                            chapter: Some(chapter_number),
                            date_updated: None,
                            url: Some(url),
                            lang: Some("en".to_string()),
                            paywalled: Some(ch.price > 0),
                            volume: None,
                            scanlator: None,
                        });
                    }
                    
                    manga.chapters = Some(chapters);
                }
            }
        }

        Ok(manga)
    }

    fn get_page_list(_manga: Manga, chapter: Chapter) -> Result<Vec<Page>> {
        let url = format!("{}{}", BASE_URL, chapter.key);
        let res = Request::get(&url).send()?;
        let response = String::from_utf8_lossy(&res.body).into_owned();
        
        let marker_escaped = r#"\"chapter_data\":{\"images\":["#;
        if let Some(start_idx) = response.find(marker_escaped) {
            let array_start = start_idx + marker_escaped.len();
            if let Some(array_end) = response[array_start..].find("]") {
                let json_array = &response[array_start..array_start + array_end];
                let pages: Vec<Page> = json_array.split(',')
                    .filter_map(|s: &str| {
                        let clean = s.trim().replace("\\\"", "").replace("\"", "").replace("\\", "");
                        if clean.is_empty() { return None; }
                        Some(Page {
                            index: 0,
                            content: PageContent::Url(Self::get_clean_cover_url(clean)),
                            has_description: false,
                            description: None,
                            headers: None,
                        })
                    })
                    .enumerate()
                    .map(|(i, mut p)| {
                        p.index = i as i32;
                        p
                    })
                    .collect();
                if !pages.is_empty() {
                    return Ok(pages);
                }
            }
        }

        let marker_unescaped = r#""chapter_data":{"images":["#;
        if let Some(start_idx) = response.find(marker_unescaped) {
            let array_start = start_idx + marker_unescaped.len();
            if let Some(array_end) = response[array_start..].find("]") {
                let json_array = &response[array_start..array_start + array_end];
                let pages: Vec<Page> = json_array.split(',')
                    .filter_map(|s: &str| {
                        let clean = s.trim().trim_matches('"').replace("\\", "");
                        if clean.is_empty() { return None; }
                        Some(Page {
                            index: 0,
                            content: PageContent::Url(Self::get_clean_cover_url(clean)),
                            has_description: false,
                            description: None,
                            headers: None,
                        })
                    })
                    .enumerate()
                    .map(|(i, mut p)| {
                        p.index = i as i32;
                        p
                    })
                    .collect();
                if !pages.is_empty() {
                    return Ok(pages);
                }
            }
        }

        let html = Node::new(&res.body);
        
        let mut pages: Vec<Page> = Vec::new();
        if let Ok(nodes) = html.select("link[rel='preload'][as='image']") {
            for el in nodes {
                if let Ok(Some(url)) = el.attr("href") {
                    if url.is_empty() { continue; }
                    if url.contains("/uploads/series/") || url.contains("media.luacomic.org") {
                        pages.push(Page {
                            index: pages.len() as i32,
                            content: PageContent::Url(Self::get_clean_cover_url(url)),
                            has_description: false,
                            description: None,
                            headers: None,
                        });
                    }
                }
            }
        }

        if !pages.is_empty() {
            return Ok(pages);
        }

        if let Ok(nodes) = html.select("div.flex.flex-col.justify-center.items-center > img") {
            for el in nodes {
                let url = el.attr("src").unwrap_or_default().unwrap_or_default();
                let data_src = el.attr("data-src").unwrap_or_default().unwrap_or_default();
                
                let final_url = if url.is_empty() || url.starts_with("data:") {
                    data_src
                } else {
                    url
                };
                
                if final_url.is_empty() { continue; }
                
                pages.push(Page {
                    index: pages.len() as i32,
                    content: PageContent::Url(Self::get_clean_cover_url(final_url)),
                    has_description: false,
                    description: None,
                    headers: None,
                });
            }
        }

        Ok(pages)
    }

    fn get_home() -> Result<HomeLayout> {
        let mut components = Vec::new();
        let mut featured_mangas: Vec<Manga> = Vec::new();
        let mut recommended_mangas: Vec<Manga> = Vec::new();

        if let Ok(res) = Request::get(BASE_URL).send() {
            let html = Node::new(&res.body);

            // 1. Featured
            if let Ok(nodes) = html.select("div[role='region'][aria-roledescription='carousel']") {
                if let Some(featured_carousel) = nodes.first() {
                    if let Ok(links) = featured_carousel.select("a[href^='/series/']") {
                        for el in links {
                            let url = el.attr("href").unwrap_or_default().unwrap_or_default();
                            let abs_url = if url.starts_with("http") { url.clone() } else { format!("{}{}", BASE_URL, url) };
                            
                            let raw_cover = el.select("img").ok()
                                .and_then(|n| n.first().map(|img| img.attr("src").ok().flatten().unwrap_or_default()))
                                .unwrap_or_default();
                            let abs_cover = if raw_cover.starts_with("http") { raw_cover.clone() } else if !raw_cover.is_empty() { format!("{}{}", BASE_URL, raw_cover) } else { raw_cover };
                            
                            let cover = LuaComic::get_clean_cover_url(abs_cover);
                            let slug = abs_url.split("/series/").nth(1).unwrap_or_default();
                            if slug.is_empty() { continue; }
                            
                            let title = LuaComic::slug_to_title(slug);
                            let key = abs_url.strip_prefix(BASE_URL).unwrap_or(&abs_url).to_string();
                            featured_mangas.push(Manga {
                                key,
                                title,
                                cover: Some(cover),
                                url: Some(abs_url),
                                authors: None,
                                artist: None,
                                description: None,
                                tags: None,
                                status: Status::Unknown,
                                content_rating: ContentRating::Safe,
                                nsfw: 0,
                                viewer: Viewer::Default,
                                chapters: None,
                            });
                        }
                    }
                }
            }

            // 2. Recommended
            if let Ok(nodes) = html.select("div.embla") {
                if let Some(recommended_section) = nodes.first() {
                    if let Ok(slides) = recommended_section.select("div.embla__slide") {
                        for el in slides {
                            if let Ok(links) = el.select("a") {
                                if let Some(link) = links.first() {
                                    let url = link.attr("href").unwrap_or_default().unwrap_or_default();
                                    let abs_url = if url.starts_with("http") { url.clone() } else { format!("{}{}", BASE_URL, url) };
                                    
                                    let title = el.select("h5").ok()
                                        .and_then(|n| n.first().map(|h| h.text().unwrap_or_default()))
                                        .unwrap_or_default();
                                        
                                    let raw_cover = el.select("img").ok()
                                        .and_then(|n| n.first().map(|img| img.attr("src").ok().flatten().unwrap_or_default()))
                                        .unwrap_or_default();
                                    let abs_cover = if raw_cover.starts_with("http") { raw_cover.clone() } else if !raw_cover.is_empty() { format!("{}{}", BASE_URL, raw_cover) } else { raw_cover };
                                        
                                    let cover = LuaComic::get_clean_cover_url(abs_cover);
                                    let key = abs_url.strip_prefix(BASE_URL).unwrap_or(&abs_url).to_string();
                                    recommended_mangas.push(Manga {
                                        key,
                                        title,
                                        cover: Some(cover),
                                        url: Some(abs_url),
                                        authors: None,
                                        artist: None,
                                        description: None,
                                        tags: None,
                                        status: Status::Unknown,
                                        content_rating: ContentRating::Safe,
                                        nsfw: 0,
                                        viewer: Viewer::Default,
                                        chapters: None,
                                    });
                                }
                            }
                        }
                    }
                }
            }

            // 3. Editor's Choice
            if let Ok(sections) = html.select("div.container") {
                for section in sections {
                    let header = section.select("h1").ok()
                        .and_then(|n| n.first().map(|h| h.text().unwrap_or_default()))
                        .unwrap_or_default();
                        
                    if header.to_lowercase().contains("editor") && header.to_lowercase().contains("choice") {
                        let mut entries = Vec::new();
                        if let Ok(grid_items) = section.select("div.grid > div") {
                            for el in grid_items {
                                if let Ok(links) = el.select("a") {
                                    if let Some(link) = links.first() {
                                        let url = link.attr("href").unwrap_or_default().unwrap_or_default();
                                        let abs_url = if url.starts_with("http") { url.clone() } else { format!("{}{}", BASE_URL, url) };
                                        
                                        let title = el.select("h5").ok()
                                            .and_then(|n| n.first().map(|h| h.text().unwrap_or_default()))
                                            .unwrap_or_default();
                                            
                                        let raw_cover = el.select("img").ok()
                                            .and_then(|n| n.first().map(|img| img.attr("src").ok().flatten().unwrap_or_default()))
                                            .unwrap_or_default();
                                        let abs_cover = if raw_cover.starts_with("http") { raw_cover.clone() } else if !raw_cover.is_empty() { format!("{}{}", BASE_URL, raw_cover) } else { raw_cover };
                                            
                                        let cover = LuaComic::get_clean_cover_url(abs_cover);
                                        let key = abs_url.strip_prefix(BASE_URL).unwrap_or(&abs_url).to_string();
                                        entries.push(Manga {
                                            key,
                                            title,
                                            cover: Some(cover),
                                            url: Some(abs_url),
                                            authors: None,
                                            artist: None,
                                            description: None,
                                            tags: None,
                                            status: Status::Unknown,
                                            content_rating: ContentRating::Safe,
                                            nsfw: 0,
                                            viewer: Viewer::Default,
                                            chapters: None,
                                        });
                                    }
                                }
                            }
                        }
                        if !entries.is_empty() {
                            components.push(HomeComponent {
                                title: Some("Editor's Choice".into()),
                                subtitle: None,
                                value: HomeComponentValue::Scroller(entries, Some(Listing {
                                    id: "EditorsChoice".into(),
                                    name: "Editor's Choice".into(), kind: 0,
                                })),
                            });
                        }
                    }
                }
            }
        }
        
        if !featured_mangas.is_empty() {
            components.insert(0, HomeComponent {
                title: Some("Featured".into()),
                subtitle: None,
                value: HomeComponentValue::BigScroller(featured_mangas, None),
            });
        }

        if !recommended_mangas.is_empty() {
            let idx = if !components.is_empty() && components[0].title.as_deref() == Some("Featured") { 1 } else { 0 };
            components.insert(idx, HomeComponent {
                title: Some("Recommended".into()),
                subtitle: None,
                value: HomeComponentValue::BigScroller(recommended_mangas, None),
            });
        }

        // Fetch Trending Daily
        if let Ok(res) = Request::get(format!("{}/trending?type=daily", API_URL))
            .header("Referer", "https://luacomic.org/")
            .send() {
            if let Ok(series_list) = serde_json::from_slice::<Vec<ApiSeries>>(&res.body) {
                let entries: Vec<Manga> = series_list.into_iter().map(LuaComic::api_series_to_manga).collect();
                if !entries.is_empty() {
                    components.push(HomeComponent {
                        title: Some("Trending Today".into()),
                        subtitle: None,
                        value: HomeComponentValue::Scroller(entries, Some(Listing { id: "TrendingDaily".into(), name: "Trending Today".into(), kind: 0 })),
                    });
                }
            }
        }

        // Fetch Trending Weekly
        if let Ok(res) = Request::get(format!("{}/trending?type=weekly", API_URL))
            .header("Referer", "https://luacomic.org/")
            .send() {
            if let Ok(series_list) = serde_json::from_slice::<Vec<ApiSeries>>(&res.body) {
                let entries: Vec<Manga> = series_list.into_iter().map(LuaComic::api_series_to_manga).collect();
                if !entries.is_empty() {
                    components.push(HomeComponent {
                        title: Some("Trending This Week".into()),
                        subtitle: None,
                        value: HomeComponentValue::Scroller(entries, Some(Listing { id: "TrendingWeekly".into(), name: "Trending This Week".into(), kind: 0 })),
                    });
                }
            }
        }

        // Fetch Trending All Time
        if let Ok(res) = Request::get(format!("{}/trending?type=all", API_URL))
            .header("Referer", "https://luacomic.org/")
            .send() {
            if let Ok(series_list) = serde_json::from_slice::<Vec<ApiSeries>>(&res.body) {
                let entries: Vec<Manga> = series_list.into_iter().map(LuaComic::api_series_to_manga).collect();
                if !entries.is_empty() {
                    components.push(HomeComponent {
                        title: Some("Trending All Time".into()),
                        subtitle: None,
                        value: HomeComponentValue::Scroller(entries, Some(Listing { id: "TrendingAll".into(), name: "Trending All Time".into(), kind: 0 })),
                    });
                }
            }
        }

        // Fetch Latest Updates
        if let Ok(res) = Request::get(format!("{}/query?page=1&perPage=20&series_type=Comic&orderBy=latest&adult=true&status=All&tags_ids=[]", API_URL))
            .header("Referer", "https://luacomic.org/")
            .send() {
            if let Ok(json) = serde_json::from_slice::<SeriesResponse>(&res.body) {
                let entries: Vec<Manga> = json.data.into_iter().map(LuaComic::api_series_to_manga).collect();
                if !entries.is_empty() {
                    components.push(HomeComponent {
                        title: Some("Latest Updates".into()),
                        subtitle: None,
                        value: HomeComponentValue::Scroller(entries, Some(Listing {
                            id: "Latest".into(),
                            name: "Latest Updates".into(), kind: 0,
                        })),
                    });
                }
            }
        }

        Ok(HomeLayout { components })
    }

    fn handle_url(url: String) -> Result<LinkValue> {
        let key = url.strip_prefix(BASE_URL).unwrap_or(&url);
        
        if key.contains("/chapter-") || key.split('/').count() > 3 {
            let parts: Vec<&str> = key.split('/').collect();
            if parts.len() >= 4 && parts[1] == "series" {
                let manga_key = format!("/series/{}", parts[2]);
                
                return Ok(LinkValue::Manga(Manga {
                    key: manga_key,
                    title: String::new(),
                    authors: None,
                    artist: None,
                    description: None,
                    tags: None,
                    cover: None,
                    url: Some(url),
                    status: Status::Unknown,
                    content_rating: ContentRating::Safe,
                    nsfw: 0,
                    viewer: Viewer::Default,
                    chapters: None,
                }));
            }
        } else if key.starts_with("/series/") {
            return Ok(LinkValue::Manga(Manga {
                key: key.to_string(),
                title: String::new(),
                authors: None,
                artist: None,
                description: None,
                tags: None,
                cover: None,
                url: Some(url),
                status: Status::Unknown,
                content_rating: ContentRating::Safe,
                nsfw: 0,
                viewer: Viewer::Default,
                chapters: None,
            }));
        }

        Err(ito_rs::Error::Host("Invalid URL".to_string()))
    }
}

export_manga_plugin!(LuaComic);
