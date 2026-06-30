use ito_rs::export_manga_plugin;
use ito_rs::provider::MangaProvider;
use ito_rs::models::{FilterItem, HomeLayout, HomeComponent, HomeComponentValue, Listing, Page, PageContent};
use ito_rs::models::manga::{Chapter, Manga, PageResult as MangaPageResult};
use ito_rs::net::Request;
use ito_rs::Result;
use std::collections::HashMap;
use base64::Engine;

mod hash;
mod helpers;
mod models;
mod settings;
mod web;

use models::*;

pub const BASE_URL: &str = "https://comix.to";
pub const API_URL: &str = "https://comix.to/api/v1";

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

        let popular_url = format!("{}/manga/top?type=trending&days=1&limit=20{}", API_URL, extra_qs);
        let popular_mangas: Vec<Manga> = if let Ok(json) = web::ComixWebView::fetch_json::<SearchResponse>(&popular_url) {
            json.result.into_items().into_iter().filter(|m| !m.is_hidden(&hidden_types, &hidden_terms)).map(Manga::from).collect()
        } else {
            vec![]
        };

        let follows_url = format!("{}/manga/top?type=follows&days=1&limit=20{}", API_URL, extra_qs);
        let follows_mangas: Vec<Manga> = if let Ok(json) = web::ComixWebView::fetch_json::<SearchResponse>(&follows_url) {
            json.result.into_items().into_iter().filter(|m| !m.is_hidden(&hidden_types, &hidden_terms)).map(Manga::from).collect()
        } else {
            vec![]
        };

        let latest_url = format!("{}/manga?scope=hot&limit=30&order[chapter_updated_at]=desc&page=1{}", API_URL, extra_qs);
        let latest_mangas: Vec<Manga> = if let Ok(json) = web::ComixWebView::fetch_json::<SearchResponse>(&latest_url) {
            json.result.into_items().into_iter().filter(|m| !m.is_hidden(&hidden_types, &hidden_terms)).map(Manga::from).collect()
        } else {
            vec![]
        };

        let recent_url = format!("{}/manga?order[created_at]=desc&limit=10&page=1{}", API_URL, extra_qs);
        let recent_mangas: Vec<Manga> = if let Ok(json) = web::ComixWebView::fetch_json::<SearchResponse>(&recent_url) {
            json.result.into_items().into_iter().filter(|m| !m.is_hidden(&hidden_types, &hidden_terms)).map(Manga::from).collect()
        } else {
            vec![]
        };

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
                format!("{}/manga/top?type=trending&days=1&limit=50{}", API_URL, extra_qs)
            }
            "Most Follows New Comics" => {
                format!("{}/manga/top?type=follows&days=1&limit=50{}", API_URL, extra_qs)
            }
            "Latest Updates (Hot)" => {
                format!("{}/manga?scope=hot&limit=30&order[chapter_updated_at]=desc&page={}{}", API_URL, page, extra_qs)
            }
            "Recently Added" => {
                format!("{}/manga?order[created_at]=desc&limit=30&page={}{}", API_URL, page, extra_qs)
            }
            _ => return Ok(MangaPageResult { entries: vec![], has_next_page: false }),
        };

        if !url.is_empty()
            && let Ok(json) = web::ComixWebView::fetch_json::<SearchResponse>(&url) {
                return Ok(json.result.into_filtered(&hidden_types, &hidden_terms));
            }
        Ok(MangaPageResult { entries: vec![], has_next_page: false })
    }

    fn get_search_manga_list(
        query: &str,
        page: i32,
        _filters: Vec<FilterItem>,
    ) -> Result<MangaPageResult> {
        let mut url = format!("{}/manga?page={}", API_URL, page);
        if !query.is_empty() {
            url.push_str(&format!("&keyword={}", helpers::urlencode(query)));
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

        if let Ok(json) = web::ComixWebView::fetch_json::<SearchResponse>(&url) {
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
            if let Ok(json) = web::ComixWebView::fetch_json::<SingleMangaResponse>(&url) {
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

                if let Ok(json) = web::ComixWebView::fetch_json::<ChapterDetailsResponse>(&url) {
                    let items = json.result.items;

                    if deduplicate {
                        for item in items {
                            helpers::dedup_insert(&mut chapter_map, item);
                        }
                    } else {
                        chapter_list.extend(items);
                    }

                    if json.result.meta.page >= json.result.meta.last_page {
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
        if let Ok(json) = web::ComixWebView::fetch_json::<ChapterResponse>(&url)
            && let Some(result) = json.result {
                return Ok(result.get_images().into_iter().enumerate().map(|(i, img)| {
                    let is_scrambled = img.s == Some(1);
                    let img_w = img.width;
                    let img_h = img.height;
                    let mut page = img.into_page(i as i32);
                    // prefix with ito://com.kunihir0.comix/ so ItoRunner intercepts it via handle_image
                    if let PageContent::Url(u) = &page.content {
                        let separator = if u.contains('?') { "&" } else { "?" };
                        let scrambled_flag = if is_scrambled { format!("{}ito_scrambled=1&ito_w={}&ito_h={}", separator, img_w, img_h) } else { "".to_string() };
                        let intercept_url = format!("ito://com.kunihir0.comix/{}{}", u, scrambled_flag);
                        page.content = PageContent::Url(intercept_url);
                    }
                    page
                }).collect());
            }
        Ok(vec![])
    }

    fn handle_url(_url: &str) -> Result<ito_rs::models::LinkValue> {
        Err(ito_rs::Error::Unsupported)
    }

    fn handle_image(url: &str) -> Result<Vec<u8>> {
        let mut actual_url = url.replace("ito://com.kunihir0.comix/", "");
        let is_scrambled = actual_url.contains("ito_scrambled=1");
        
        let mut width = 1000.0;
        let mut height = 1000.0;

        if is_scrambled {
            if let Some(w_start) = actual_url.find("ito_w=") {
                let w_str = actual_url[w_start + 6..].split('&').next().unwrap_or("");
                width = w_str.parse().unwrap_or(1000.0);
                actual_url = actual_url.replace(&format!("ito_w={}", w_str), "");
            }
            if let Some(h_start) = actual_url.find("ito_h=") {
                let h_str = actual_url[h_start + 6..].split('&').next().unwrap_or("");
                height = h_str.parse().unwrap_or(1000.0);
                actual_url = actual_url.replace(&format!("ito_h={}", h_str), "");
            }
            actual_url = actual_url.replace("ito_scrambled=1", "").replace("?&", "?").replace("&&", "&");
            actual_url = actual_url.trim_end_matches('?').trim_end_matches('&').to_string();

            // Execute descrambler directly
            if let Ok(data_uri) = web::ComixWebView::descramble_image(width, height, &actual_url)
                && let Some((_, base64_data)) = data_uri.split_once(',')
                    && let Ok(bytes) = base64::prelude::BASE64_STANDARD.decode(base64_data) {
                        return Ok(bytes);
                    }
            return Err(ito_rs::Error::Unsupported);
        }

        let res = Request::get(&actual_url).send()?;
        let headers = res.headers;
        if let Some(x_enc) = headers.get("x-enc")
            && x_enc == "1" {
                // Execute descrambler
                if let Ok(data_uri) = web::ComixWebView::descramble_image(width, height, &actual_url)
                    && let Some((_, base64_data)) = data_uri.split_once(',')
                        && let Ok(bytes) = base64::prelude::BASE64_STANDARD.decode(base64_data) {
                            return Ok(bytes);
                        }
            }
        Ok(res.body)
    }
}

export_manga_plugin!(Comix);
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn test_hash() {
        let token = hash::generate_hash("/api/v1/manga/w0wj9/chapters", 0, 1);
        println!("TOKEN: {}", token);
        assert_eq!(token, "8DseLn4JBMOTpz9Uvea0EKFQ1kaphg7rIsmQzZcaHL4y0vSZ8VXvcCHijle644TzLa011_n3H2s");
    }
}
