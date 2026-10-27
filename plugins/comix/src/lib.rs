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

fn paged_result(query: &str, hidden_types: &[String]) -> Result<MangaPageResult> {
    let page = ComixWebView::fetch::<Paged<ComixManga>>(Operation::List, None, query)?;
    Ok(page.into_page_result(hidden_types))
}

fn top_result(query: &str, hidden_types: &[String]) -> Result<MangaPageResult> {
    let items = ComixWebView::fetch::<Vec<ComixManga>>(Operation::Top, None, query)?;
    Ok(MangaPageResult {
        entries: visible(items, hidden_types),
        has_next_page: false,
    })
}

/// Decides the chapter loop's next step once `requested` has been answered: `Some(page)` fetches
/// `page`, `None` stops. A response for another page is a host error, so the update fails whole.
fn next_chapter_page(requested: i32, meta: &PageMeta, item_count: usize) -> Result<Option<i32>> {
    if meta.page != requested {
        return Err(ito_rs::Error::Host(format!(
            "Comix chapter page {} answered page {requested}",
            meta.page
        )));
    }
    if item_count == 0 || meta.page >= meta.last_page {
        return Ok(None);
    }
    Ok(Some(requested + 1))
}

impl MangaProvider for Comix {
    fn get_home() -> Result<HomeLayout> {
        let extra_qs = nsfw_query();
        let hidden_types = settings::hidden_types();

        let popular_mangas = top_result(
            &format!("type=trending&days=1&limit=20{extra_qs}"),
            &hidden_types,
        )?
        .entries;
        let follows_mangas = top_result(
            &format!("type=follows&days=1&limit=20{extra_qs}"),
            &hidden_types,
        )?
        .entries;
        let latest_mangas = paged_result(
            &format!("scope=hot&limit=30&order[chapter_updated_at]=desc&page=1{extra_qs}"),
            &hidden_types,
        )?
        .entries;
        let recent_mangas = paged_result(
            &format!("order[created_at]=desc&limit=10&page=1{extra_qs}"),
            &hidden_types,
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

        match listing.id.as_str() {
            "Trending Webtoon" => {
                // comix.to's webtoon listing: manhua and manhwa types, ordered by 90-day views.
                paged_result(
                    &format!(
                        "types[]=manhua&types[]=manhwa&order[views_90d]=desc&page={page}{extra_qs}"
                    ),
                    &hidden_types,
                )
            }
            "Trending Manga" => paged_result(
                &format!("types[]=manga&order[views_90d]=desc&page={page}{extra_qs}"),
                &hidden_types,
            ),
            "Most Recent Popular" => top_result(
                &format!("type=trending&days=1&limit=50{extra_qs}"),
                &hidden_types,
            ),
            "Most Follows New Comics" => top_result(
                &format!("type=follows&days=1&limit=50{extra_qs}"),
                &hidden_types,
            ),
            "Latest Updates (Hot)" => paged_result(
                &format!("scope=hot&limit=30&order[chapter_updated_at]=desc&page={page}{extra_qs}"),
                &hidden_types,
            ),
            "Recently Added" => paged_result(
                &format!("order[created_at]=desc&limit=30&page={page}{extra_qs}"),
                &hidden_types,
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

        paged_result(&params, &settings::hidden_types())
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
                let next = next_chapter_page(page, &json.meta, json.items.len())?;
                if deduplicate {
                    for item in json.items {
                        helpers::dedup_insert(&mut chapter_map, item);
                    }
                } else {
                    chapter_list.extend(json.items);
                }
                let Some(next_page) = next else { break };
                page = next_page;
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

#[cfg(test)]
mod tests {
    use super::*;

    fn meta(page: i32, last_page: i32) -> PageMeta {
        PageMeta { page, last_page }
    }

    #[test]
    fn chapter_loop_fetches_the_following_page_while_pages_remain() {
        assert_eq!(next_chapter_page(1, &meta(1, 3), 100).ok(), Some(Some(2)));
    }

    #[test]
    fn chapter_loop_stops_on_the_last_page() {
        assert_eq!(next_chapter_page(3, &meta(3, 3), 100).ok(), Some(None));
    }

    #[test]
    fn chapter_loop_stops_on_an_empty_page_before_the_last() {
        assert_eq!(next_chapter_page(1, &meta(1, 3), 0).ok(), Some(None));
    }

    #[test]
    fn chapter_response_for_another_page_fails_the_update() {
        assert!(matches!(
            next_chapter_page(2, &meta(1, 3), 100),
            Err(ito_rs::Error::Host(_))
        ));
    }
}
