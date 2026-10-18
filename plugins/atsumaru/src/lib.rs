use ito_rs::models::{FilterItem, Listing, Page, PageContent, HomeLayout, HomeComponent, HomeComponentValue};
use ito_rs::models::manga::{Chapter, Manga, PageResult as MangaPageResult, Status as MangaStatus, Viewer as MangaViewer, ContentRating};
use ito_rs::net::Request;
use ito_rs::provider::MangaProvider;
use ito_rs::export_manga_plugin;
use ito_rs::{Error, Result};

mod models;
use models::*;

const BASE_URL: &str = "https://atsu.moe";
const CDN_URL: &str = "https://cdn.atsu.moe";
const API_BASE: &str = "https://atsu.moe/api";
const SEARCH_URL: &str = "https://atsu.moe/collections/manga/documents/search";
const HOME_API: &str = "https://atsu.moe/api/home2";
const HOME_PAGE_SIZE: i32 = 24;
const COMIC_FILTER: &str = "medium:Comic";
pub struct Atsumaru;

fn resolve_image_url(path: String) -> String {
    if path.starts_with("http") {
        path
    } else if path.starts_with("/static/") || path.starts_with("static/") {
        format!("{}/{}", CDN_URL, path.trim_start_matches('/'))
    } else if path.starts_with('/') {
        format!("{}{}", CDN_URL, path)
    } else {
        format!("{}/static/{}", CDN_URL, path)
    }
}

fn build_manga_from_doc(doc: &SearchDocument) -> Manga {
    Manga {
        key: doc.id.clone().unwrap_or_default(),
        title: doc.title.clone().unwrap_or_default(),
        cover: doc.poster.clone().map(resolve_image_url),
        url: Some(format!("{}/manga/{}", BASE_URL, doc.id.clone().unwrap_or_default())),
        status: match doc.status.as_deref() {
            Some("Ongoing") => MangaStatus::Ongoing,
            Some("Completed") => MangaStatus::Completed,
            Some("Hiatus") => MangaStatus::Hiatus,
            Some("Dropped") | Some("Cancelled") => MangaStatus::Cancelled,
            _ => MangaStatus::Unknown,
        },
        description: doc.synopsis.clone(),
        tags: doc.tags.clone(),
        authors: doc.authors.clone(),
        viewer: MangaViewer::Webtoon,
        artist: None,
        content_rating: ContentRating::Safe,
        nsfw: 0,
        chapters: None,
    }
}

fn build_manga_from_infinite(item: &InfiniteItem) -> Manga {
    Manga {
        key: item.id.clone(),
        title: item.title.clone(),
        cover: item.image.clone().map(resolve_image_url),
        url: Some(format!("{}/manga/{}", BASE_URL, item.id)),
        status: MangaStatus::Unknown,
        description: None,
        tags: None,
        authors: None,
        viewer: MangaViewer::Webtoon,
        artist: None,
        content_rating: ContentRating::Safe,
        nsfw: 0,
        chapters: None,
    }
}

impl Atsumaru {
    fn fetch_search(
        q: &str,
        page: i32,
        per_page: i32,
        sort_by: &str,
        filter_by: Option<&str>,
    ) -> Result<MangaPageResult> {
        let q_escaped = q.replace(" ", "%20");
        let mut url = format!(
            "{}?q={}&page={}&per_page={}&query_by=title,englishTitle,otherNames,authors,tags&include_fields=id,title,englishTitle,poster,posterSmall,posterMedium,type,isAdult,status,year,synopsis,tags,authors",
            SEARCH_URL,
            q_escaped,
            page,
            per_page
        );

        if !sort_by.is_empty() {
            url.push_str(&format!("&sort_by={}", sort_by));
        }
        if let Some(filter) = filter_by {
            url.push_str(&format!("&filter_by={}", filter));
        }

        let res = Request::get(&url).send()?;
        if res.status != 200 {
            return Err(ito_rs::Error::Host(format!("Atsumaru search returned HTTP {}", res.status)));
        }

        let json = serde_json::from_slice::<SearchResponse>(&res.body)
            .map_err(|e| ito_rs::Error::Host(format!("JSON error: {}", e)))?;

        let entries: Vec<Manga> = json.hits.into_iter().map(|hit| build_manga_from_doc(&hit.document)).collect();
        let has_next_page = json.found > (json.page * per_page);

        Ok(MangaPageResult {
            entries,
            has_next_page,
        })
    }

    fn fetch_home2(endpoint: &str, offset: i32, timeframe: Option<&str>) -> Result<MangaPageResult> {
        let mut url = format!(
            "{}/{}?offset={}&limit={}&mediums=Comic",
            HOME_API, endpoint, offset, HOME_PAGE_SIZE
        );
        if let Some(t) = timeframe {
            url.push_str(&format!("&timeframe={}", t));
        }

        let res = Request::get(&url).send()?;
        if res.status != 200 {
            return Err(ito_rs::Error::Host(format!("Atsumaru {} returned HTTP {}", endpoint, res.status)));
        }

        let json = serde_json::from_slice::<InfiniteResponse>(&res.body)
            .map_err(|e| ito_rs::Error::Host(format!("JSON error: {}", e)))?;

        let entries: Vec<Manga> = json.items.into_iter().map(|item| build_manga_from_infinite(&item)).collect();
        Ok(MangaPageResult {
            has_next_page: entries.len() as i32 >= HOME_PAGE_SIZE,
            entries,
        })
    }
}

impl MangaProvider for Atsumaru {
    fn get_home() -> Result<HomeLayout> {
        let hot_updates = Self::fetch_home2("recentlyUpdated", 0, None)?.entries;
        let trending = Self::fetch_search("*", 1, HOME_PAGE_SIZE, "trending:desc", Some(COMIC_FILTER))?.entries;
        let most_bookmarked = Self::fetch_home2("mostBookmarked", 0, Some("monthly"))?.entries;

        Ok(HomeLayout {
            components: vec![
                HomeComponent {
                    title: Some("Most Bookmarked".to_string()),
                    subtitle: None,
                    value: HomeComponentValue::Scroller(
                        most_bookmarked,
                        Some(Listing { id: "mostBookmarked".to_string(), name: "Most Bookmarked".to_string(), kind: 0 })
                    ),
                },
                HomeComponent {
                    title: Some("Trending".to_string()),
                    subtitle: None,
                    value: HomeComponentValue::Scroller(
                        trending,
                        Some(Listing { id: "trending".to_string(), name: "Trending".to_string(), kind: 0 })
                    ),
                },
                HomeComponent {
                    title: Some("Hot Updates".to_string()),
                    subtitle: None,
                    value: HomeComponentValue::MangaList(
                        false,
                        None,
                        hot_updates,
                        Some(Listing { id: "recentlyUpdated".to_string(), name: "Hot Updates".to_string(), kind: 0 })
                    ),
                },
            ],
        })
    }

    fn get_search_manga_list(
        query: &str,
        page: i32,
        _filters: Vec<FilterItem>,
    ) -> Result<MangaPageResult> {
        let q = if query.is_empty() { "*".to_string() } else { query.to_string() };
        Self::fetch_search(&q, page, 24, "views:desc", None) // hardcode sort for demo
    }

    fn get_manga_list(listing: Listing, page: i32) -> Result<MangaPageResult> {
        // Ito pages are 1-indexed; home2 uses offset paging.
        let offset = (page - 1).max(0) * HOME_PAGE_SIZE;
        match listing.id.as_str() {
            "mostBookmarked" => Self::fetch_home2("mostBookmarked", offset, Some("monthly")),
            "trending" => Self::fetch_search("*", page, HOME_PAGE_SIZE, "trending:desc", Some(COMIC_FILTER)),
            "recentlyUpdated" => Self::fetch_home2("recentlyUpdated", offset, None),
            _ => Self::fetch_search("*", page, 24, "views:desc", None),
        }
    }

    fn get_manga_update(
        manga: Manga,
        needs_details: bool,
        needs_chapters: bool,
    ) -> Result<Manga> {
        let mut updated_manga = manga.clone();
        let mut scanlator_map = std::collections::BTreeMap::new();

        if needs_details || needs_chapters {
            let url = format!("{}/manga/page?id={}", API_BASE, manga.key);
            let res = Request::get(&url).send()?;
            
            if let Ok(json) = serde_json::from_slice::<MangaPageWrapper>(&res.body) {
                let detail = json.manga_page;
                
                if needs_details {
                    updated_manga.title = detail.title.unwrap_or_default();
                    updated_manga.description = detail.synopsis;
                    updated_manga.cover = detail.poster.and_then(|p| p.image).map(resolve_image_url);
                    updated_manga.url = Some(format!("{}/manga/{}", BASE_URL, detail.id.clone().unwrap_or_default()));
                    updated_manga.status = match detail.status.as_deref() {
                        Some("Ongoing") => MangaStatus::Ongoing,
                        Some("Completed") => MangaStatus::Completed,
                        Some("Hiatus") => MangaStatus::Hiatus,
                        Some("Dropped") | Some("Cancelled") => MangaStatus::Cancelled,
                        _ => MangaStatus::Unknown,
                    };
                    let authors_vec: Vec<String> = detail.authors.unwrap_or_default().into_iter().filter_map(|e| e.name).collect();
                    updated_manga.authors = if authors_vec.is_empty() { None } else { Some(authors_vec) };
                    
                    let tags_vec: Vec<String> = detail.genres.unwrap_or_default().into_iter().filter_map(|e| e.name).collect();
                    updated_manga.tags = if tags_vec.is_empty() { None } else { Some(tags_vec) };
                }

                if let Some(scanlators) = detail.scanlators {
                    for s in scanlators {
                        scanlator_map.insert(s.id, s.name);
                    }
                }
            }
        }

        if needs_chapters {
            let url = format!("{}/manga/allChapters?mangaId={}", API_BASE, manga.key);
            let res = Request::get(&url).send()?;
            
            {
                let msg = format!("Fetched chapter list len: {}", res.body.len());
                ito_rs::host::print(&msg);
            }
            
            match serde_json::from_slice::<ChapterListResponse>(&res.body) {
                Ok(json) => {
                    {
                        let msg = format!("Parsed Chapter List JSON! Total Chapters: {}", json.chapters.len());
                        ito_rs::host::print(&msg);
                    }
                    let mut chapters = Vec::new();
                    for chap in json.chapters {
                        let chapter_url = format!("{}/read/{}?chapterId={}", BASE_URL, manga.key, chap.id);
                        let scanlator_name = chap.scanlation_manga_id
                            .as_ref()
                            .and_then(|id| scanlator_map.get(id)).cloned();
                        
                        chapters.push(Chapter {
                            key: chap.id.clone(),
                            title: chap.title.clone(),
                            chapter: Some(chap.number),
                            volume: None,
                            date_updated: Some((chap.created_at / 1000) as f64), // ms to seconds
                            url: Some(chapter_url),
                            scanlator: scanlator_name,
                            lang: None,
                            paywalled: None,
                        });
                    }
                    
                    // Sort chapters descending by chapter number to fix hierarchy for multiple scanlators
                    chapters.sort_by(|a, b| {
                        let a_num = a.chapter.unwrap_or(0.0);
                        let b_num = b.chapter.unwrap_or(0.0);
                        b_num.partial_cmp(&a_num).unwrap_or(core::cmp::Ordering::Equal)
                    });
                    
                    updated_manga.chapters = Some(chapters);
                },
                Err(e) => {
                    {
                        let msg = format!("Failed to parse JSON chapters: {}", e);
                        ito_rs::host::print(&msg);
                    }
                    let partial_body = String::from_utf8_lossy(&res.body).chars().take(200).collect::<String>();
                    {
                        let msg = format!("Response snippet: {}", partial_body);
                        ito_rs::host::print(&msg);
                    }
                }
            }
        }

        Ok(updated_manga)
    }

    fn get_page_list(manga: Manga, chapter: Chapter) -> Result<Vec<Page>> {
        let url = format!("{}/read/chapter?mangaId={}&chapterId={}", API_BASE, manga.key, chapter.key);
        let res = Request::get(&url).send()?;
        
        let json: ChapterPageResponse = serde_json::from_slice(&res.body).map_err(|_| Error::Host("Failed to parse page list".into()))?;

        Ok(json.read_chapter.pages.into_iter().enumerate().map(|(idx, p)| {
            let img_url = resolve_image_url(p.image);
            Page {
                index: idx as i32,
                content: PageContent::Url(img_url),
                has_description: false,
                description: None,
                headers: None,
            }
        }).collect())
    }
}

export_manga_plugin!(Atsumaru);
