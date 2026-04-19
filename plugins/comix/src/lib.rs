use ito_rs::export_manga_plugin;
use ito_rs::provider::MangaProvider;
use ito_rs::models::{FilterItem, HomeLayout, HomeComponent, HomeComponentValue, Listing, Page};
use ito_rs::models::manga::{Chapter, Manga, PageResult as MangaPageResult};
use ito_rs::net::Request;
use ito_rs::Result;
use std::collections::HashMap;

mod hash;
mod helpers;
mod models;
mod settings;

use models::*;

pub const BASE_URL: &str = "https://comix.to";
pub const API_URL: &str = "https://comix.to/api/v2";

const NSFW_GENRE_IDS: &[&str] = &["87264", "8", "87265", "13", "87266", "87268"];

struct Comix;

impl MangaProvider for Comix {
    fn get_home() -> Result<HomeLayout> {
        let extra_qs = if settings::hide_nsfw() {
            NSFW_GENRE_IDS
                .iter()
                .map(|id| format!("&genres[]=-{}", id))
                .collect::<String>()
        } else {
            String::new()
        };

        let hidden_types = settings::hidden_types();
        let hidden_terms = settings::hidden_terms();

        let popular_url = format!("{}/top?type=trending&days=1&limit=20{}", API_URL, extra_qs);
        let popular_res = Request::get(&popular_url).send()?;
        let popular_mangas: Vec<Manga> = if let Ok(json) = serde_json::from_slice::<SearchResponse>(&popular_res.body) {
            json.result.items.into_iter().filter(|m| !m.is_hidden(&hidden_types, &hidden_terms)).map(Manga::from).collect()
        } else { vec![] };

        let follows_url = format!("{}/top?type=follows&days=1&limit=20{}", API_URL, extra_qs);
        let follows_res = Request::get(&follows_url).send()?;
        let follows_mangas: Vec<Manga> = if let Ok(json) = serde_json::from_slice::<SearchResponse>(&follows_res.body) {
            json.result.items.into_iter().filter(|m| !m.is_hidden(&hidden_types, &hidden_terms)).map(Manga::from).collect()
        } else { vec![] };

        let latest_url = format!("{}/manga?scope=hot&limit=30&order[chapter_updated_at]=desc&page=1{}", API_URL, extra_qs);
        let latest_res = Request::get(&latest_url).send()?;
        let latest_mangas: Vec<Manga> = if let Ok(json) = serde_json::from_slice::<SearchResponse>(&latest_res.body) {
            json.result.items.into_iter().filter(|m| !m.is_hidden(&hidden_types, &hidden_terms)).map(Manga::from).collect()
        } else { vec![] };

        let recent_url = format!("{}/manga?order[created_at]=desc&limit=10&page=1{}", API_URL, extra_qs);
        let recent_res = Request::get(&recent_url).send()?;
        let recent_mangas: Vec<Manga> = if let Ok(json) = serde_json::from_slice::<SearchResponse>(&recent_res.body) {
            json.result.items.into_iter().filter(|m| !m.is_hidden(&hidden_types, &hidden_terms)).map(Manga::from).collect()
        } else { vec![] };

        Ok(HomeLayout {
            components: vec![
                HomeComponent {
                    title: Some("Most Recent Popular".into()),
                    subtitle: None,
                    value: HomeComponentValue::Scroller(
                        popular_mangas,
                        Some(Listing { id: "Most Recent Popular".into(), name: "Most Recent Popular".into(), kind: 0 })
                    ),
                },
                HomeComponent {
                    title: Some("Most Follows New Comics".into()),
                    subtitle: None,
                    value: HomeComponentValue::Scroller(
                        follows_mangas,
                        Some(Listing { id: "Most Follows New Comics".into(), name: "Most Follows New Comics".into(), kind: 0 })
                    ),
                },
                HomeComponent {
                    title: Some("Latest Updates (Hot)".into()),
                    subtitle: None,
                    value: HomeComponentValue::Scroller(
                        latest_mangas,
                        Some(Listing { id: "Latest Updates (Hot)".into(), name: "Latest Updates (Hot)".into(), kind: 0 })
                    ),
                },
                HomeComponent {
                    title: Some("Recently Added".into()),
                    subtitle: None,
                    value: HomeComponentValue::MangaList(
                        false, None,
                        recent_mangas,
                        Some(Listing { id: "Recently Added".into(), name: "Recently Added".into(), kind: 0 })
                    ),
                },
            ],
        })
    }

    fn get_manga_list(listing: Listing, page: i32) -> Result<MangaPageResult> {
        let extra_qs = if settings::hide_nsfw() {
            NSFW_GENRE_IDS
                .iter()
                .map(|id| format!("&genres[]=-{}", id))
                .collect::<String>()
        } else {
            String::new()
        };
        let hidden_types = settings::hidden_types();
        let hidden_terms = settings::hidden_terms();

        let url = match listing.id.as_str() {
            "Trending Webtoon" => {
                // not fully implemented matching exact Aidoku trending
                format!("{}/manga?types[]=manhua&types[]=manhwa&order[views_90d]=desc&page={}{}", API_URL, page, extra_qs)
            },
            "Trending Manga" => {
                format!("{}/manga?types[]=manga&order[views_90d]=desc&page={}{}", API_URL, page, extra_qs)
            },
            "Most Recent Popular" => {
                format!("{}/top?type=trending&days=1&limit=50{}", API_URL, extra_qs)
            }
            "Most Follows New Comics" => {
                format!("{}/top?type=follows&days=1&limit=50{}", API_URL, extra_qs)
            }
            "Latest Updates (Hot)" => {
                format!("{}/manga?scope=hot&limit=30&order[chapter_updated_at]=desc&page={}{}", API_URL, page, extra_qs)
            }
            "Recently Added" => {
                format!("{}/manga?order[created_at]=desc&limit=30&page={}{}", API_URL, page, extra_qs)
            }
            _ => return Ok(MangaPageResult { entries: vec![], has_next_page: false }),
        };

        if !url.is_empty() {
            let res = Request::get(&url).send()?;
            if let Ok(json) = serde_json::from_slice::<SearchResponse>(&res.body) {
                return Ok(json.result.into_filtered(&hidden_types, &hidden_terms));
            }
        }
        Ok(MangaPageResult { entries: vec![], has_next_page: false })
    }

    fn get_search_manga_list(
        query: String,
        page: i32,
        _filters: Vec<FilterItem>,
    ) -> Result<MangaPageResult> {
        let mut url = format!("{}/manga?page={}", API_URL, page);
        if !query.is_empty() {
            url.push_str(&format!("&keyword={}", helpers::urlencode(&query)));
        }

        url.push_str("&order[relevance]=desc");

        let hidden_types = settings::hidden_types();
        let hidden_terms = settings::hidden_terms();

        let extra_qs = if settings::hide_nsfw() {
            NSFW_GENRE_IDS
                .iter()
                .map(|id| format!("&genres[]=-{}", id))
                .collect::<String>()
        } else {
            String::new()
        };

        url.push_str(&extra_qs);

        let res = Request::get(&url).send()?;
        if let Ok(json) = serde_json::from_slice::<SearchResponse>(&res.body) {
            Ok(json.result.into_filtered(&hidden_types, &hidden_terms))
        } else {
            Ok(MangaPageResult { entries: vec![], has_next_page: false })
        }
    }

    fn get_manga_update(mut manga: Manga, needs_details: bool, needs_chapters: bool) -> Result<Manga> {
        if needs_details {
            let url = format!(
                "{}/manga/{}/?includes[]=demographic\
                                    &includes[]=genre\
                                    &includes[]=theme\
                                    &includes[]=author\
                                    &includes[]=artist\
                                    &includes[]=publisher",
                API_URL, manga.key
            );
            let res = Request::get(&url).send()?;
            if let Ok(json) = serde_json::from_slice::<SingleMangaResponse>(&res.body) {
                let updated: Manga = json.result.into();
                manga.title = updated.title;
                manga.cover = updated.cover;
                manga.authors = updated.authors;
                manga.artist = updated.artist;
                manga.description = updated.description;
                manga.tags = updated.tags;
                manga.status = updated.status;
                manga.content_rating = updated.content_rating;
                manga.viewer = updated.viewer;
                manga.nsfw = updated.nsfw;
            }
        }

        if needs_chapters {
            let limit = 100;
            let mut page = 1;
            let deduplicate = settings::dedupchapter();
            let mut chapter_map: HashMap<String, ComixChapter> = HashMap::new();
            let mut chapter_list: Vec<ComixChapter> = Vec::new();
            loop {
                let path = format!("/manga/{}/chapters", manga.key);
                let time = 1;
                let token = hash::generate_hash(&path, 0, time);
                let url = format!(
                    "{}/manga/{}/chapters?limit={}&page={}&order[number]=desc&time={}&_={}",
                    API_URL, manga.key, limit, page, time, token
                );

                let res = Request::get(&url).send()?;
                if let Ok(json) = serde_json::from_slice::<ChapterDetailsResponse>(&res.body) {
                    let items = json.result.items;

                    if deduplicate {
                        for item in items {
                            helpers::dedup_insert(&mut chapter_map, item);
                        }
                    } else {
                        chapter_list.extend(items);
                    }

                    if json.result.pagination.current_page >= json.result.pagination.last_page {
                        break;
                    }
                    page += 1;
                } else {
                    break;
                }
            }

            let mut chapters: Vec<Chapter> = if deduplicate {
                chapter_map
                    .into_values()
                    .map(|item| item.into_chapter(&manga.key))
                    .collect()
            } else {
                chapter_list
                    .into_iter()
                    .map(|item| item.into_chapter(&manga.key))
                    .collect()
            };

            if deduplicate {
                chapters.sort_by(|a, b| {
                    let a_num = a.chapter.unwrap_or(0.0);
                    let b_num = b.chapter.unwrap_or(0.0);
                    b_num
                        .partial_cmp(&a_num)
                        .unwrap_or(core::cmp::Ordering::Equal)
                });
            }

            manga.chapters = Some(chapters);
        }

        Ok(manga)
    }

    fn get_page_list(_manga: Manga, chapter: Chapter) -> Result<Vec<Page>> {
        let url = format!("{}/chapters/{}", API_URL, chapter.key);
        let res = Request::get(&url).send()?;
        if let Ok(json) = serde_json::from_slice::<ChapterResponse>(&res.body) {
            if let Some(result) = json.result {
                return Ok(result.images.into_iter().enumerate().map(|(i, img)| img.into_page(i as i32)).collect());
            }
        }
        Ok(vec![])
    }
}

export_manga_plugin!(Comix);
