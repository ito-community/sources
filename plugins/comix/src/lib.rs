use ito_rs::Result;
use ito_rs::export_manga_plugin;
use ito_rs::models::manga::{Chapter, Manga, PageResult as MangaPageResult};
use ito_rs::models::{FilterItem, HomeComponent, HomeComponentValue, HomeLayout, Listing, Page};
use ito_rs::provider::MangaProvider;
use std::collections::HashMap;
mod helpers;
mod models;
mod settings;
mod web;

use models::*;
use web::{ComixWebView, Operation};

pub const BASE_URL: &str = "https://comix.to";

const NSFW_GENRE_IDS: &[&str] = &["87264", "8", "87265", "13", "87266", "87268"];

struct Comix;

/// Query fragment that excludes NSFW genres while the setting is enabled.
fn nsfw_query() -> String {
    if settings::hide_nsfw() {
        NSFW_GENRE_IDS
            .iter()
            .map(|id| format!("&genres[]=-{}", id))
            .collect()
    } else {
        String::new()
    }
}

fn paged_result(
    query: &str,
    hidden_types: &[String],
    hidden_terms: &[i32],
) -> Result<MangaPageResult> {
    let page = ComixWebView::fetch::<Paged<ComixManga>>(Operation::List, None, query)?;
    Ok(page.into_page_result(hidden_types, hidden_terms))
}

fn top_result(
    query: &str,
    hidden_types: &[String],
    hidden_terms: &[i32],
) -> Result<MangaPageResult> {
    let items = ComixWebView::fetch::<Vec<ComixManga>>(Operation::Top, None, query)?;
    Ok(MangaPageResult {
        entries: visible(items, hidden_types, hidden_terms),
        has_next_page: false,
    })
}

impl MangaProvider for Comix {
    fn get_home() -> Result<HomeLayout> {
        let extra_qs = nsfw_query();
        let hidden_types = settings::hidden_types();
        let hidden_terms = settings::hidden_terms();

        let popular_mangas = top_result(
            &format!("type=trending&days=1&limit=20{extra_qs}"),
            &hidden_types,
            &hidden_terms,
        )?
        .entries;
        let follows_mangas = top_result(
            &format!("type=follows&days=1&limit=20{extra_qs}"),
            &hidden_types,
            &hidden_terms,
        )?
        .entries;
        let latest_mangas = paged_result(
            &format!("scope=hot&limit=30&order[chapter_updated_at]=desc&page=1{extra_qs}"),
            &hidden_types,
            &hidden_terms,
        )?
        .entries;
        let recent_mangas = paged_result(
            &format!("order[created_at]=desc&limit=10&page=1{extra_qs}"),
            &hidden_types,
            &hidden_terms,
        )?
        .entries;

        Ok(HomeLayout {
            components: vec![
                HomeComponent {
                    title: Some("Most Recent Popular".into()),
                    subtitle: None,
                    value: HomeComponentValue::Scroller(
                        popular_mangas,
                        Some(Listing {
                            id: "Most Recent Popular".into(),
                            name: "Most Recent Popular".into(),
                            kind: 0,
                        }),
                    ),
                },
                HomeComponent {
                    title: Some("Most Follows New Comics".into()),
                    subtitle: None,
                    value: HomeComponentValue::Scroller(
                        follows_mangas,
                        Some(Listing {
                            id: "Most Follows New Comics".into(),
                            name: "Most Follows New Comics".into(),
                            kind: 0,
                        }),
                    ),
                },
                HomeComponent {
                    title: Some("Latest Updates (Hot)".into()),
                    subtitle: None,
                    value: HomeComponentValue::Scroller(
                        latest_mangas,
                        Some(Listing {
                            id: "Latest Updates (Hot)".into(),
                            name: "Latest Updates (Hot)".into(),
                            kind: 0,
                        }),
                    ),
                },
                HomeComponent {
                    title: Some("Recently Added".into()),
                    subtitle: None,
                    value: HomeComponentValue::MangaList(
                        false,
                        None,
                        recent_mangas,
                        Some(Listing {
                            id: "Recently Added".into(),
                            name: "Recently Added".into(),
                            kind: 0,
                        }),
                    ),
                },
            ],
        })
    }

    fn get_manga_list(listing: Listing, page: i32) -> Result<MangaPageResult> {
        let extra_qs = nsfw_query();
        let hidden_types = settings::hidden_types();
        let hidden_terms = settings::hidden_terms();

        match listing.id.as_str() {
            "Trending Webtoon" => {
                // not fully implemented matching exact Aidoku trending
                paged_result(
                    &format!(
                        "types[]=manhua&types[]=manhwa&order[views_90d]=desc&page={page}{extra_qs}"
                    ),
                    &hidden_types,
                    &hidden_terms,
                )
            }
            "Trending Manga" => paged_result(
                &format!("types[]=manga&order[views_90d]=desc&page={page}{extra_qs}"),
                &hidden_types,
                &hidden_terms,
            ),
            "Most Recent Popular" => top_result(
                &format!("type=trending&days=1&limit=50{extra_qs}"),
                &hidden_types,
                &hidden_terms,
            ),
            "Most Follows New Comics" => top_result(
                &format!("type=follows&days=1&limit=50{extra_qs}"),
                &hidden_types,
                &hidden_terms,
            ),
            "Latest Updates (Hot)" => paged_result(
                &format!("scope=hot&limit=30&order[chapter_updated_at]=desc&page={page}{extra_qs}"),
                &hidden_types,
                &hidden_terms,
            ),
            "Recently Added" => paged_result(
                &format!("order[created_at]=desc&limit=30&page={page}{extra_qs}"),
                &hidden_types,
                &hidden_terms,
            ),
            _ => Err(ito_rs::Error::Unsupported),
        }
    }

    fn get_search_manga_list(
        query: &str,
        page: i32,
        _filters: Vec<FilterItem>,
    ) -> Result<MangaPageResult> {
        let mut params = format!("page={page}");
        if !query.is_empty() {
            params.push_str(&format!("&keyword={}", helpers::urlencode(query)));
        }
        params.push_str("&order[relevance]=desc");
        params.push_str(&nsfw_query());

        paged_result(
            &params,
            &settings::hidden_types(),
            &settings::hidden_terms(),
        )
    }

    fn get_manga_update(
        mut manga: Manga,
        needs_details: bool,
        needs_chapters: bool,
    ) -> Result<Manga> {
        if needs_details {
            let updated: Manga =
                ComixWebView::fetch::<ComixManga>(Operation::Detail, Some(manga.key.as_str()), "")?
                    .into();
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

        if needs_chapters {
            let limit = 100;
            let mut page = 1;
            let deduplicate = settings::dedupchapter();
            let mut chapter_map: HashMap<String, ComixChapter> = HashMap::new();
            let mut chapter_list: Vec<ComixChapter> = Vec::new();
            loop {
                let query = format!("limit={limit}&page={page}&order[number]=desc");
                // Any failed page fails the whole update: a partial chapter list is never returned.
                let json = ComixWebView::fetch::<Paged<ComixChapter>>(
                    Operation::Chapters,
                    Some(manga.key.as_str()),
                    &query,
                )?;
                let last_page_reached = json.meta.page >= json.meta.last_page;
                if deduplicate {
                    for item in json.items {
                        helpers::dedup_insert(&mut chapter_map, item);
                    }
                } else {
                    chapter_list.extend(json.items);
                }
                if last_page_reached {
                    break;
                }
                page += 1;
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
        ComixWebView::fetch::<ChapterPages>(Operation::Pages, Some(chapter.key.as_str()), "")?
            .into_pages()
    }

    fn handle_url(_url: &str) -> Result<ito_rs::models::LinkValue> {
        Err(ito_rs::Error::Unsupported)
    }
}

export_manga_plugin!(Comix);
