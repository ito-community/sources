use ito_rs::Error;
use ito_rs::Result;
use ito_rs::export_manga_plugin;
use ito_rs::models::manga::{Chapter, Manga, PageResult as MangaPageResult};
use ito_rs::models::{FilterItem, HomeComponent, HomeComponentValue, HomeLayout, Listing, Page};
use ito_rs::provider::MangaProvider;
use std::collections::HashMap;
mod api;
mod decode;
mod helpers;
mod key;
mod models;
mod query;
mod settings;
mod sign;

use models::*;
use query::Query;

pub const BASE_URL: &str = "https://comix.to";

/// Genres the site labels as adult: Adult (87264), Hentai (87266) and Smut (87268). Their series
/// are almost all rated erotica or pornographic. Other genres are left to the content rating.
const NSFW_GENRE_IDS: &[&str] = &["87264", "87266", "87268"];

/// Chapters per page: the largest page the site accepts.
const CHAPTER_PAGE_LIMIT: i32 = 100;
const _: () = assert!(CHAPTER_PAGE_LIMIT <= 100);

struct Comix;

/// Narrows a listing to the content the user has not hidden: safe and suggestive series without
/// the NSFW genres. The site reads these as ordinary request parameters.
fn with_content_filter(query: Query) -> Query {
    if settings::hide_nsfw() {
        query
            .list("content_rating", &["safe", "suggestive"])
            .list("genres_ex", NSFW_GENRE_IDS)
    } else {
        query
    }
}

fn paged_result(query: &Query, hidden_types: &[String]) -> Result<MangaPageResult> {
    let page: Paged<Series> = api::get("/manga", query)?;
    Ok(page.into_page_result(hidden_types))
}

fn top_result(query: &Query, hidden_types: &[String]) -> Result<MangaPageResult> {
    let items: Vec<Series> = api::get("/manga/top", query)?;
    Ok(MangaPageResult {
        entries: visible(items, hidden_types),
        has_next_page: false,
    })
}

/// A top list of `kind` (`trending` or `follows`) over one day.
fn top_query(kind: &'static str, limit: i32) -> Query {
    with_content_filter(
        Query::new()
            .text("type", kind)
            .int("days", 1)
            .int("limit", limit),
    )
}

/// The "hot" latest-updates scope, ordered by the newest chapter.
fn hot_query(page: i32) -> Query {
    with_content_filter(
        Query::new()
            .text("scope", "hot")
            .int("limit", 30)
            .order("chapter_updated_at", "desc")
            .int("page", page),
    )
}

/// The site path of a series. The key is the site's alphanumeric short id; anything else cannot
/// name a series.
fn series_path(hid: &str) -> Result<String> {
    if hid.is_empty() || !hid.bytes().all(|byte| byte.is_ascii_alphanumeric()) {
        return Err(Error::NotFound(UNKNOWN_KEY.into()));
    }
    Ok(format!("/manga/{hid}"))
}

/// The site path of a chapter's pages. Chapter keys are the site's numeric ids.
fn chapter_path(id: &str) -> Result<String> {
    if id.is_empty() || !id.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(Error::NotFound(UNKNOWN_KEY.into()));
    }
    Ok(format!("/chapters/{id}"))
}

const UNKNOWN_KEY: &str = "Comix has no series or chapter with that id.";

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
        let hidden_types = settings::hidden_types();

        let popular_mangas = top_result(&top_query("trending", 20), &hidden_types)?.entries;
        let follows_mangas = top_result(&top_query("follows", 20), &hidden_types)?.entries;
        let latest_mangas = paged_result(&hot_query(1), &hidden_types)?.entries;
        let recent_mangas = paged_result(
            &with_content_filter(
                Query::new()
                    .order("created_at", "desc")
                    .int("limit", 10)
                    .int("page", 1),
            ),
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
        let hidden_types = settings::hidden_types();

        match listing.id.as_str() {
            "Trending Webtoon" => {
                // comix.to's webtoon listing: manhua and manhwa types, ordered by 90-day views.
                let query = with_content_filter(
                    Query::new()
                        .list("types", &["manhua", "manhwa"])
                        .order("views_90d", "desc")
                        .int("page", page),
                );
                paged_result(&query, &hidden_types)
            }
            "Trending Manga" => {
                let query = with_content_filter(
                    Query::new()
                        .list("types", &["manga"])
                        .order("views_90d", "desc")
                        .int("page", page),
                );
                paged_result(&query, &hidden_types)
            }
            "Most Recent Popular" => top_result(&top_query("trending", 50), &hidden_types),
            "Most Follows New Comics" => top_result(&top_query("follows", 50), &hidden_types),
            "Latest Updates (Hot)" => paged_result(&hot_query(page), &hidden_types),
            "Recently Added" => {
                let query = with_content_filter(
                    Query::new()
                        .order("created_at", "desc")
                        .int("limit", 30)
                        .int("page", page),
                );
                paged_result(&query, &hidden_types)
            }
            _ => Err(ito_rs::Error::Unsupported),
        }
    }

    fn get_search_manga_list(
        query: &str,
        page: i32,
        _filters: Vec<FilterItem>,
    ) -> Result<MangaPageResult> {
        let mut params = Query::new().int("page", page).order("relevance", "desc");
        if !query.is_empty() {
            params = params.text("keyword", query);
        }
        paged_result(&with_content_filter(params), &settings::hidden_types())
    }

    fn get_manga_update(
        mut manga: Manga,
        needs_details: bool,
        needs_chapters: bool,
    ) -> Result<Manga> {
        let series = series_path(&manga.key)?;

        if needs_details {
            let detail: Series = api::get(&series, &Query::new())?;
            let updated: Manga = detail.into();
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
            let chapters_path = format!("{series}/chapters");
            let deduplicate = settings::dedupchapter();
            let mut page = 1;
            let mut chapter_map: HashMap<String, ComixChapter> = HashMap::new();
            let mut chapter_list: Vec<ComixChapter> = Vec::new();
            // Pages are requested one after another and never past the last page. The SDK offers
            // no delay between requests, so the loop cannot space them further.
            loop {
                let query = Query::new()
                    .int("limit", CHAPTER_PAGE_LIMIT)
                    .order("number", "desc")
                    .int("page", page);
                // Any failed page fails the whole update: a partial chapter list is never returned.
                let json: Paged<ComixChapter> = api::get(&chapters_path, &query)?;
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

            helpers::retain_first_keys(&mut chapters);

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
        let path = chapter_path(chapter.key.as_str())?;
        let pages: ChapterPages = api::get(&path, &Query::new())?;
        pages.into_pages()
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

    #[test]
    fn hidden_content_is_excluded_by_rating_and_genre_parameters() {
        assert_eq!(
            with_content_filter(Query::new()).message("/manga"),
            "/manga?content_rating[0]=safe&content_rating[1]=suggestive&genres_ex[0]=87264&genres_ex[1]=87266&genres_ex[2]=87268"
        );
    }

    #[test]
    fn series_keys_must_be_site_short_ids() {
        assert_eq!(series_path("8v88").ok().as_deref(), Some("/manga/8v88"));
        assert!(series_path("").is_err());
        assert!(series_path("8v88/../x").is_err());
        assert!(series_path("3756?x=1").is_err());
    }

    #[test]
    fn chapter_keys_must_be_numeric_ids() {
        assert_eq!(
            chapter_path("11734794").ok().as_deref(),
            Some("/chapters/11734794")
        );
        assert!(chapter_path("").is_err());
        assert!(chapter_path("12a").is_err());
    }
}
