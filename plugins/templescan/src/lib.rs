//! Temple Scan (templetoons.com): browse, search and read the site's public manhwa.
//!
//! The site is a Next.js application behind a Turnstile verification page, so every page is
//! loaded through Ito's web view (see `web`). Its catalog is delivered whole and browsed
//! locally (see `catalog`); its text search is the endpoint its own search box uses (see
//! `search`). Early-access chapters carry the site's own lock mark and are listed as locked;
//! their pages are never requested.

mod catalog;
mod controls;
mod detail;
mod dom;
mod flight;
mod home;
mod images;
mod links;
mod models;
mod numbers;
mod reader;
mod search;
mod site;
mod time;
mod url;
mod web;

use catalog::{Browse, Item, Order};
use ito_rs::controls::SearchCapabilities;
use ito_rs::models::manga::{ContentRating, Manga, Status, Viewer};
use ito_rs::models::{
    Chapter, FilterItem, HomeComponent, HomeComponentValue, HomeLayout, LinkValue, Listing,
    MangaWithChapter, MediaEntry, MediaKind, MediaPageResult, Page, PluginImageRequest,
};
use ito_rs::provider::SourceProvider;
use ito_rs::{Error, Result};
use models::{Listed, nsfw_flag, severity};
use site::{Site, Web};
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

const LATEST: &str = "latest";
const NEW_SERIES: &str = "new";

/// Catalog snapshots by the listing or search they were loaded for, and the browse.
type Catalogs = HashMap<(Entry, Browse), Rc<Vec<Item>>>;
thread_local! {
    /// The catalog each listing or search loaded on its first page, per browse. Later pages of the
    /// same entry point and browse read the same snapshot, so pages never shift under the reader
    /// and cost no new load. Entry points keep their own snapshots, so interleaved browses never
    /// read another's.
    static CATALOGS: RefCell<Catalogs> = RefCell::new(HashMap::new());
}

/// Where a browse came from. The latest listing and the default search browse the same way, but
/// they are separate browses with separate snapshots.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum Entry {
    Listing(&'static str),
    Search,
}

fn snapshot<S: Site>(entry: &Entry, browse: &Browse, fresh: bool) -> Result<Rc<Vec<Item>>> {
    let key = (entry.clone(), browse.clone());
    if !fresh && let Some(items) = CATALOGS.with_borrow(|catalogs| catalogs.get(&key).cloned()) {
        return Ok(items);
    }
    let items = Rc::new(catalog::parse(&S::page("/comics")?)?);
    CATALOGS.with_borrow_mut(|catalogs| catalogs.insert(key, Rc::clone(&items)));
    Ok(items)
}

fn entries(listed: Vec<Listed>) -> Vec<MediaEntry> {
    listed
        .into_iter()
        .map(|listed| MediaEntry::Manga(listed.into_manga()))
        .collect()
}

fn empty_page() -> MediaPageResult {
    MediaPageResult {
        entries: Vec::new(),
        has_next_page: false,
    }
}

fn browse_page<S: Site>(entry: Entry, browse: &Browse, page: i32) -> Result<MediaPageResult> {
    let items = snapshot::<S>(&entry, browse, page <= 1)?;
    let (listed, has_next_page) = catalog::page(&items, browse, page);
    Ok(MediaPageResult {
        entries: entries(listed),
        has_next_page,
    })
}

fn list(id: &str, name: &str) -> Option<Listing> {
    Some(Listing {
        id: id.into(),
        name: name.into(),
        kind: 0,
    })
}

fn home_layout<S: Site>() -> Result<HomeLayout> {
    let page = home::parse(&S::page("/")?)?;
    let mut components = Vec::new();
    if !page.new_series.is_empty() {
        components.push(HomeComponent {
            title: Some("New Series".into()),
            subtitle: None,
            value: HomeComponentValue::Scroller(
                page.new_series
                    .into_iter()
                    .map(Listed::into_manga)
                    .collect(),
                list(NEW_SERIES, "New Series"),
            ),
        });
    }
    if !page.trending.is_empty() {
        components.push(HomeComponent {
            title: Some("Trending".into()),
            subtitle: Some("Today".into()),
            value: HomeComponentValue::Scroller(
                page.trending.into_iter().map(Listed::into_manga).collect(),
                None,
            ),
        });
    }
    if !page.latest.is_empty() {
        components.push(HomeComponent {
            title: Some("Latest updates".into()),
            subtitle: None,
            value: HomeComponentValue::MangaChapterList(
                None,
                page.latest
                    .into_iter()
                    .map(|update| MangaWithChapter {
                        manga: update.listed.into_manga(),
                        chapter: update.chapter,
                    })
                    .collect(),
                list(LATEST, "Latest updates"),
            ),
        });
    }
    Ok(HomeLayout { components })
}

fn listing<S: Site>(listing: &Listing, page: i32) -> Result<MediaPageResult> {
    let (id, browse) = match listing.id.as_str() {
        LATEST => (LATEST, Browse::default()),
        NEW_SERIES => (
            NEW_SERIES,
            Browse {
                order: Order::Created,
                ..Browse::default()
            },
        ),
        _ => return Err(Error::NotFound("Temple Scan has no such list".into())),
    };
    browse_page::<S>(Entry::Listing(id), &browse, page)
}

fn run_search<S: Site>(
    query: &str,
    page: i32,
    kind: Option<MediaKind>,
    filters: &[FilterItem],
) -> Result<MediaPageResult> {
    if kind.is_some_and(|kind| kind != MediaKind::Manga) {
        return Ok(empty_page());
    }
    if query.trim().is_empty() {
        return browse_page::<S>(Entry::Search, &controls::browse(filters)?, page);
    }
    // The site does not search one character, and its endpoint takes no filters or ordering.
    let Some(text) = search::normalize(query) else {
        return Ok(empty_page());
    };
    let reply = S::reply(&search::path(&text, page))?;
    if reply.status != 200 {
        return Err(search::failure(reply.status));
    }
    let results = search::parse(&reply.body, page)?;
    Ok(MediaPageResult {
        entries: entries(results.entries),
        has_next_page: results.has_next_page,
    })
}

/// Loads the series page of `manga` and fills in what was asked for. Details the page does not
/// give stay as they were.
fn update<S: Site>(entry: MediaEntry, details: bool, units: bool) -> Result<MediaEntry> {
    let MediaEntry::Manga(mut manga) = entry else {
        return Err(Error::Unsupported);
    };
    if !details && !units {
        return Ok(MediaEntry::Manga(manga));
    }
    if !links::valid_slug(&manga.key) {
        return Err(Error::NotFound("Temple Scan has no such series".into()));
    }
    let doc = S::page(&format!("/comic/{}", url::encode(&manga.key)))?;
    let series = detail::parse(&doc, &manga.key)?;
    if details {
        manga.title = series.title;
        manga.cover = series.cover.or(manga.cover);
        manga.description = series.description.or(manga.description);
        manga.authors = series.authors.or(manga.authors);
        manga.tags = series.tags.or(manga.tags);
        if !matches!(series.status, Status::Unknown) {
            manga.status = series.status;
        }
        // A series page can raise the rating a listing showed, never lower it.
        if severity(&series.rating) > severity(&manga.content_rating) {
            manga.content_rating = series.rating;
        }
        manga.nsfw = nsfw_flag(&manga.content_rating);
        manga.url = Some(links::series_url(&manga.key));
        manga.viewer = Viewer::Webtoon;
    }
    if units {
        manga.chapters = Some(series.chapters);
    }
    Ok(MediaEntry::Manga(manga))
}

fn chapter_pages<S: Site>(entry: MediaEntry, chapter: Chapter) -> Result<Vec<Page>> {
    let MediaEntry::Manga(manga) = entry else {
        return Err(Error::Unsupported);
    };
    // A chapter the site marks early access is never requested.
    if chapter.paywalled == Some(true) {
        return Err(reader::early_access());
    }
    if !links::valid_slug(&manga.key) || !links::valid_slug(&chapter.key) {
        return Err(Error::NotFound("Temple Scan has no such chapter".into()));
    }
    let doc = S::page(&format!(
        "/comic/{}/{}",
        url::encode(&manga.key),
        url::encode(&chapter.key)
    ))?;
    reader::pages(&doc)
}

/// A pasted site address opens the series it belongs to, whether it names the series or one of
/// its chapters.
fn open_link<S: Site>(address: &str) -> Result<LinkValue> {
    let link = links::parse(address).ok_or(Error::Unsupported)?;
    let seed = Manga {
        key: link.series().to_owned(),
        title: String::new(),
        authors: None,
        artist: None,
        description: None,
        tags: None,
        cover: None,
        url: None,
        status: Status::Unknown,
        content_rating: ContentRating::Safe,
        nsfw: 0,
        viewer: Viewer::Webtoon,
        chapters: None,
    };
    match update::<S>(MediaEntry::Manga(seed), true, true)? {
        MediaEntry::Manga(manga) => Ok(LinkValue::Manga(manga)),
        _ => Err(Error::Unsupported),
    }
}

pub struct TempleScan;

impl SourceProvider for TempleScan {
    const KINDS: &'static [MediaKind] = &[MediaKind::Manga];

    fn get_home() -> Result<HomeLayout> {
        home_layout::<Web>()
    }

    fn get_search_capabilities(kind: Option<MediaKind>) -> Result<SearchCapabilities> {
        if kind.is_some_and(|kind| kind != MediaKind::Manga) {
            return Err(Error::Unsupported);
        }
        Ok(controls::capabilities())
    }

    fn get_listing(list: Listing, page: i32) -> Result<MediaPageResult> {
        listing::<Web>(&list, page)
    }

    fn search(
        query: &str,
        page: i32,
        kind: Option<MediaKind>,
        filters: Vec<FilterItem>,
    ) -> Result<MediaPageResult> {
        run_search::<Web>(query, page, kind, &filters)
    }

    fn get_media_update(
        entry: MediaEntry,
        needs_details: bool,
        needs_units: bool,
    ) -> Result<MediaEntry> {
        update::<Web>(entry, needs_details, needs_units)
    }

    fn get_chapter_pages(entry: MediaEntry, chapter: Chapter) -> Result<Vec<Page>> {
        chapter_pages::<Web>(entry, chapter)
    }

    fn handle_url(url: &str) -> Result<LinkValue> {
        open_link::<Web>(url)
    }

    fn get_image_origins() -> Result<Vec<String>> {
        Ok(images::origins())
    }

    fn get_image_request(url: &str) -> Result<PluginImageRequest> {
        images::request(url).ok_or_else(|| {
            Error::Host("Temple Scan images come only from the site's own image hosts".into())
        })
    }
}

ito_rs::export_plugin!(TempleScan);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::tests::{page_html, record};
    use crate::detail::tests::{page as series_page, row};
    use crate::home::tests::home_page;
    use crate::reader::tests::{chapter_page, url as image_url};
    use crate::search::tests::body as search_body;
    use ito_rs::models::{ContentKind, PageContent};
    use scraper::Html;
    use std::collections::HashMap;

    /// The site as recorded pages and answers; every request is logged.
    struct Recorded;

    thread_local! {
        static PAGES: RefCell<HashMap<String, String>> = RefCell::new(HashMap::new());
        static ANSWERS: RefCell<HashMap<String, (u16, String)>> = RefCell::new(HashMap::new());
        static REQUESTS: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
    }

    impl Site for Recorded {
        fn page(path: &str) -> Result<Html> {
            REQUESTS.with_borrow_mut(|requests| requests.push(path.to_owned()));
            PAGES
                .with_borrow(|pages| pages.get(path).map(|html| dom::parse(html)))
                .ok_or_else(|| Error::Net(format!("no recorded page {path}")))
        }

        fn reply(path: &str) -> Result<web::Reply> {
            REQUESTS.with_borrow_mut(|requests| requests.push(path.to_owned()));
            ANSWERS
                .with_borrow(|answers| answers.get(path).cloned())
                .map(|(status, body)| web::Reply {
                    status,
                    url: String::new(),
                    body,
                })
                .ok_or_else(|| Error::Net(format!("no recorded answer {path}")))
        }
    }

    fn serve(path: &str, html: String) {
        PAGES.with_borrow_mut(|pages| {
            pages.insert(path.to_owned(), html);
        });
    }

    fn answer(path: &str, status: u16, body: String) {
        ANSWERS.with_borrow_mut(|answers| {
            answers.insert(path.to_owned(), (status, body));
        });
    }

    fn requests() -> Vec<String> {
        REQUESTS.with_borrow_mut(std::mem::take)
    }

    /// A valid, strictly increasing date for the `n`th series.
    fn day(month: usize, n: usize) -> String {
        format!("2026-{:02}-{:02}T00:00:00Z", month + n / 28, 1 + n % 28)
    }

    fn catalog_of(count: usize) -> String {
        let records: Vec<String> = (0..count)
            .map(|n| {
                record(
                    &format!("Series {n:02}"),
                    &format!("series-{n:02}"),
                    if n % 10 == 9 { "Completed" } else { "Ongoing" },
                    &day(1, n),
                    Some(&day(4, n)),
                    n as u64 * 10,
                )
            })
            .collect();
        page_html(&records)
    }

    fn keys(page: &MediaPageResult) -> Vec<String> {
        page.entries
            .iter()
            .map(|entry| entry.key().to_owned())
            .collect()
    }

    fn listing_of(id: &str) -> Listing {
        Listing {
            id: id.into(),
            name: id.into(),
            kind: 0,
        }
    }

    fn seed(key: &str) -> MediaEntry {
        MediaEntry::Manga(Manga {
            key: key.into(),
            title: "Seed".into(),
            authors: None,
            artist: None,
            description: None,
            tags: None,
            cover: Some("https://media.templetoons.com/file/b/covers/seed.webp".into()),
            url: None,
            status: Status::Unknown,
            content_rating: ContentRating::Safe,
            nsfw: 0,
            viewer: Viewer::Webtoon,
            chapters: None,
        })
    }

    fn chapter(key: &str, paywalled: Option<bool>) -> Chapter {
        Chapter {
            key: key.into(),
            title: None,
            volume: None,
            chapter: None,
            date_updated: None,
            scanlator: None,
            url: None,
            lang: None,
            paywalled,
            kind: ContentKind::Images,
        }
    }

    fn manga_of(entry: MediaEntry) -> Manga {
        match entry {
            MediaEntry::Manga(manga) => manga,
            _ => panic!("manga expected"),
        }
    }

    #[test]
    fn home_lists_the_sites_sections_with_lists_to_open_them() {
        serve("/", home_page());
        let layout = home_layout::<Recorded>().expect("home");
        assert_eq!(requests(), ["/"]);
        let titles: Vec<_> = layout
            .components
            .iter()
            .map(|c| c.title.as_deref().unwrap_or(""))
            .collect();
        assert_eq!(titles, ["New Series", "Trending", "Latest updates"]);
        let HomeComponentValue::Scroller(new, Some(new_list)) = &layout.components[0].value else {
            panic!("new series scroller expected");
        };
        assert_eq!(new_list.id, NEW_SERIES);
        assert_eq!(
            new.iter().map(|m| m.key.as_str()).collect::<Vec<_>>(),
            ["a-suckers-love-story", "switch-boyfriend"]
        );
        assert!(matches!(
            &layout.components[1].value,
            HomeComponentValue::Scroller(_, None)
        ));
        let HomeComponentValue::MangaChapterList(_, updates, Some(latest)) =
            &layout.components[2].value
        else {
            panic!("latest updates expected");
        };
        assert_eq!(latest.id, LATEST);
        assert_eq!(updates.len(), 2, "the sponsored card is not an update");
        assert_eq!(updates[1].chapter.paywalled, Some(true));
        assert_eq!(updates[0].chapter.paywalled, None);
    }

    #[test]
    fn home_that_the_site_changed_is_an_error() {
        serve(
            "/",
            "<html><body><main><h2>Nothing known</h2></main></body></html>".into(),
        );
        assert!(matches!(home_layout::<Recorded>(), Err(Error::Host(_))));
        requests();
    }

    #[test]
    fn browsing_pages_through_one_snapshot_and_reloads_on_a_new_first_page() {
        serve("/comics", catalog_of(45));
        let first = listing::<Recorded>(&listing_of(LATEST), 1).expect("page 1");
        assert_eq!(first.entries.len(), 20);
        assert!(first.has_next_page);
        // Newest update first: the highest numbered series was updated last.
        assert_eq!(keys(&first)[0], "series-44");
        assert_eq!(requests(), ["/comics"]);
        let second = listing::<Recorded>(&listing_of(LATEST), 2).expect("page 2");
        let third = listing::<Recorded>(&listing_of(LATEST), 3).expect("page 3");
        assert!(requests().is_empty(), "later pages read the snapshot");
        assert_eq!(second.entries.len(), 20);
        assert_eq!(third.entries.len(), 5);
        assert!(second.has_next_page && !third.has_next_page);
        let mut all = keys(&first);
        all.extend(keys(&second));
        all.extend(keys(&third));
        let unique: std::collections::HashSet<_> = all.iter().collect();
        assert_eq!(
            unique.len(),
            45,
            "every series appears once across the pages"
        );
        // A new first page is a new browse.
        let again = listing::<Recorded>(&listing_of(LATEST), 1).expect("page 1");
        assert_eq!(requests(), ["/comics"]);
        assert_eq!(keys(&again), keys(&first));
    }

    #[test]
    fn interleaved_browses_each_read_the_catalog_they_loaded() {
        serve("/comics", catalog_of(45));
        let latest = listing::<Recorded>(&listing_of(LATEST), 1).expect("latest page 1");
        // The site's catalog changes before the second browse loads its first page.
        serve("/comics", catalog_of(3));
        let new = listing::<Recorded>(&listing_of(NEW_SERIES), 1).expect("new page 1");
        assert_eq!(new.entries.len(), 3);
        let latest_second = listing::<Recorded>(&listing_of(LATEST), 2).expect("latest page 2");
        assert_eq!(
            latest_second.entries.len(),
            20,
            "page 2 reads the snapshot its own page 1 loaded"
        );
        assert!(latest_second.has_next_page);
        assert_eq!(
            requests(),
            ["/comics", "/comics"],
            "only the two first pages load the catalog"
        );
        let mut all = keys(&latest);
        all.extend(keys(&latest_second));
        let unique: std::collections::HashSet<_> = all.iter().collect();
        assert_eq!(unique.len(), 40);
    }

    #[test]
    fn the_latest_listing_and_the_default_search_keep_their_own_catalogs() {
        serve("/comics", catalog_of(45));
        let latest = listing::<Recorded>(&listing_of(LATEST), 1).expect("latest page 1");
        // The site's catalog changes before the default search loads its first page.
        serve("/comics", catalog_of(3));
        let search = run_search::<Recorded>("", 1, None, &[]).expect("search page 1");
        assert_eq!(
            search.entries.len(),
            3,
            "the default search loads its own catalog"
        );
        let latest_second = listing::<Recorded>(&listing_of(LATEST), 2).expect("latest page 2");
        assert_eq!(
            latest_second.entries.len(),
            20,
            "latest page 2 reads the catalog its own page 1 loaded"
        );
        assert!(latest_second.has_next_page);
        assert_eq!(requests(), ["/comics", "/comics"]);
        let search_second = run_search::<Recorded>("", 2, None, &[]).expect("search page 2");
        assert!(search_second.entries.is_empty());
        assert!(
            requests().is_empty(),
            "search page 2 reads the search snapshot"
        );
        let mut all = keys(&latest);
        all.extend(keys(&latest_second));
        let unique: std::collections::HashSet<_> = all.iter().collect();
        assert_eq!(unique.len(), 40);
    }

    #[test]
    fn new_series_are_ordered_by_creation_and_unknown_lists_are_refused() {
        serve("/comics", catalog_of(3));
        let new = listing::<Recorded>(&listing_of(NEW_SERIES), 1).expect("new");
        assert_eq!(keys(&new), ["series-02", "series-01", "series-00"]);
        requests();
        let refused = listing::<Recorded>(&listing_of("popular"), 1);
        assert!(matches!(refused, Err(Error::NotFound(_))));
        assert!(requests().is_empty());
    }

    #[test]
    fn empty_search_browses_with_the_chosen_controls() {
        serve("/comics", catalog_of(30));
        let capabilities = controls::capabilities();
        let values: HashMap<String, ito_rs::controls::ControlValue> = HashMap::from([
            (
                "status".to_owned(),
                ito_rs::controls::ControlValue::Select(Some("Completed".into())),
            ),
            (
                "order".to_owned(),
                ito_rs::controls::ControlValue::Select(Some("title".into())),
            ),
            (
                "direction".to_owned(),
                ito_rs::controls::ControlValue::Select(Some("asc".into())),
            ),
        ]);
        let filters = capabilities.filter_items(&values);
        let page =
            run_search::<Recorded>("  ", 1, Some(MediaKind::Manga), &filters).expect("browse");
        assert_eq!(keys(&page), ["series-09", "series-19", "series-29"]);
        assert!(!page.has_next_page);
        // A status nobody has lists nothing and says so.
        let values = HashMap::from([(
            "status".to_owned(),
            ito_rs::controls::ControlValue::Select(Some("Hiatus".into())),
        )]);
        let none = run_search::<Recorded>("", 1, None, &capabilities.filter_items(&values))
            .expect("browse");
        assert!(none.entries.is_empty() && !none.has_next_page);
        requests();
    }

    #[test]
    fn text_search_uses_the_sites_endpoint_and_its_totals() {
        answer(
            "/api/search?q=orchid&page=1&limit=20",
            200,
            search_body(45, 20, 20, ""),
        );
        answer(
            "/api/search?q=orchid&page=3&limit=20",
            200,
            search_body(45, 20, 5, ""),
        );
        let first = run_search::<Recorded>("  orchid ", 1, None, &[]).expect("search");
        assert_eq!(first.entries.len(), 20);
        assert!(first.has_next_page);
        let last = run_search::<Recorded>("orchid", 3, None, &[]).expect("search");
        assert_eq!(last.entries.len(), 5);
        assert!(!last.has_next_page);
        assert_eq!(
            requests(),
            [
                "/api/search?q=orchid&page=1&limit=20",
                "/api/search?q=orchid&page=3&limit=20"
            ]
        );
        // Filters do not change what the site's search answers, so they are not sent.
        let capabilities = controls::capabilities();
        let filters = capabilities.filter_items(&HashMap::new());
        run_search::<Recorded>("orchid", 1, None, &filters).expect("search");
        assert_eq!(requests(), ["/api/search?q=orchid&page=1&limit=20"]);
    }

    #[test]
    fn search_failures_are_errors_not_empty_results() {
        answer(
            "/api/search?q=blocked&page=1&limit=20",
            403,
            "{\"error\":\"forbidden\"}".into(),
        );
        answer("/api/search?q=limited&page=1&limit=20", 429, String::new());
        answer(
            "/api/search?q=broken&page=1&limit=20",
            200,
            "<html>not json</html>".into(),
        );
        assert!(matches!(
            run_search::<Recorded>("blocked", 1, None, &[]),
            Err(Error::Server(_))
        ));
        assert!(matches!(
            run_search::<Recorded>("limited", 1, None, &[]),
            Err(Error::Server(_))
        ));
        assert!(matches!(
            run_search::<Recorded>("broken", 1, None, &[]),
            Err(Error::Host(_))
        ));
        assert!(matches!(
            run_search::<Recorded>("unrecorded", 1, None, &[]),
            Err(Error::Net(_))
        ));
        requests();
    }

    #[test]
    fn text_the_site_does_not_search_and_other_kinds_make_no_request() {
        for kind in [MediaKind::Anime, MediaKind::Novel] {
            let page = run_search::<Recorded>("orchid", 1, Some(kind), &[]).expect("empty");
            assert!(page.entries.is_empty() && !page.has_next_page);
            assert!(matches!(
                TempleScan::get_search_capabilities(Some(kind)),
                Err(Error::Unsupported)
            ));
        }
        let page = run_search::<Recorded>(" a ", 1, None, &[]).expect("empty");
        assert!(page.entries.is_empty() && !page.has_next_page);
        assert!(requests().is_empty());
    }

    #[test]
    fn a_series_update_fills_details_and_chapters_without_lowering_what_it_knew() {
        let mut locked = row("chapter-2", "Chapter 2", "2025-01-02T00:00:00Z");
        locked.premium = true;
        serve(
            "/comic/evil-husband",
            series_page(
                "evil-husband",
                &[
                    locked,
                    row("chapter-1", "Chapter 1", "2025-01-01T00:00:00Z"),
                ],
                "",
            ),
        );
        let entry = update::<Recorded>(seed("evil-husband"), true, true).expect("update");
        assert_eq!(requests(), ["/comic/evil-husband"]);
        let manga = manga_of(entry);
        assert_eq!(manga.title, "Evil Husband");
        assert_eq!(
            manga.url.as_deref(),
            Some("https://templetoons.com/comic/evil-husband")
        );
        assert_eq!(
            manga.cover.as_deref(),
            Some("https://media.templetoons.com/file/b/covers/c.webp")
        );
        assert!(matches!(manga.status, Status::Completed));
        assert_eq!((severity(&manga.content_rating), manga.nsfw), (2, 1));
        let chapters = manga.chapters.expect("chapters");
        assert_eq!(chapters.len(), 2);
        assert_eq!(chapters[0].paywalled, Some(true));
        assert_eq!(chapters[1].paywalled, None);
    }

    #[test]
    fn asking_for_only_chapters_leaves_details_alone_and_a_known_adult_rating_stays() {
        serve(
            "/comic/evil-husband",
            series_page(
                "evil-husband",
                &[row("chapter-1", "Chapter 1", "2025-01-01T00:00:00Z")],
                "",
            ),
        );
        let MediaEntry::Manga(mut known) = seed("evil-husband") else {
            panic!("manga")
        };
        known.content_rating = ContentRating::Nsfw;
        known.title = "Kept".into();
        let entry = update::<Recorded>(MediaEntry::Manga(known), false, true).expect("update");
        let manga = manga_of(entry);
        assert_eq!(manga.title, "Kept");
        assert_eq!(severity(&manga.content_rating), 2);
        assert_eq!(manga.chapters.expect("chapters").len(), 1);
        // Details only: chapters are not touched.
        let manga =
            manga_of(update::<Recorded>(seed("evil-husband"), true, false).expect("update"));
        assert!(manga.chapters.is_none());
        requests();
    }

    #[test]
    fn a_series_the_site_does_not_have_is_not_found() {
        serve(
            "/comic/gone",
            "<html><body><h1>404</h1></body></html>".into(),
        );
        assert!(matches!(
            update::<Recorded>(seed("gone"), true, true),
            Err(Error::NotFound(_))
        ));
        requests();
    }

    #[test]
    fn a_locked_chapter_is_refused_without_asking_the_site() {
        let refused =
            chapter_pages::<Recorded>(seed("evil-husband"), chapter("chapter-52", Some(true)));
        assert!(matches!(refused, Err(Error::NeedsLogin(_))));
        assert!(requests().is_empty());
    }

    #[test]
    fn an_open_chapter_gives_its_pages_and_an_early_access_notice_is_refused() {
        let first = image_url(1);
        let second = image_url(2);
        serve(
            "/comic/evil-husband/chapter-1",
            chapter_page(&[&first, &second], "k"),
        );
        let pages = chapter_pages::<Recorded>(seed("evil-husband"), chapter("chapter-1", None))
            .expect("pages");
        assert_eq!(requests(), ["/comic/evil-husband/chapter-1"]);
        assert_eq!(pages.len(), 2);
        assert!(matches!(&pages[1].content, PageContent::Url(url) if *url == second));
        // A chapter that turns out to be early access, whatever the list said.
        serve(
            "/comic/evil-husband/chapter-9",
            "<html><body><main><p>This chapter is early access!</p></main></body></html>".into(),
        );
        let notice = chapter_pages::<Recorded>(seed("evil-husband"), chapter("chapter-9", None));
        assert!(matches!(notice, Err(Error::NeedsLogin(_))));
        requests();
    }

    #[test]
    fn chapter_keys_are_encoded_into_the_page_path() {
        serve(
            "/comic/evil-husband/a%20b",
            chapter_page(&[&image_url(1)], "k"),
        );
        chapter_pages::<Recorded>(seed("evil-husband"), chapter("a b", None)).expect("pages");
        assert_eq!(requests(), ["/comic/evil-husband/a%20b"]);
    }

    #[test]
    fn keys_that_cannot_name_a_page_are_not_found_without_a_request() {
        for key in ["", "..", "a/b"] {
            assert!(
                matches!(
                    chapter_pages::<Recorded>(seed(key), chapter("chapter-1", None)),
                    Err(Error::NotFound(_))
                ),
                "{key:?}"
            );
            assert!(
                matches!(
                    chapter_pages::<Recorded>(seed("evil-husband"), chapter(key, None)),
                    Err(Error::NotFound(_))
                ),
                "{key:?}"
            );
            assert!(
                matches!(
                    update::<Recorded>(seed(key), true, true),
                    Err(Error::NotFound(_))
                ),
                "{key:?}"
            );
        }
        assert!(requests().is_empty());
    }

    #[test]
    fn an_update_that_asks_for_nothing_changes_nothing_and_loads_nothing() {
        let manga =
            manga_of(update::<Recorded>(seed("evil-husband"), false, false).expect("entry"));
        assert_eq!(manga.title, "Seed");
        assert!(requests().is_empty());
    }

    #[test]
    fn other_media_kinds_are_not_ours() {
        let anime = MediaEntry::Anime(ito_rs::models::anime::Anime {
            key: "a".into(),
            title: "A".into(),
            studios: None,
            description: None,
            tags: None,
            cover: None,
            url: None,
            status: ito_rs::models::anime::Status::Unknown,
            content_rating: ito_rs::models::anime::ContentRating::Safe,
            nsfw: 0,
            episodes: None,
            seasons: None,
        });
        assert!(matches!(
            update::<Recorded>(anime.clone(), true, true),
            Err(Error::Unsupported)
        ));
        assert!(matches!(
            chapter_pages::<Recorded>(anime, chapter("c", None)),
            Err(Error::Unsupported)
        ));
        assert!(requests().is_empty());
    }

    #[test]
    fn pasted_links_open_the_series_they_belong_to() {
        serve(
            "/comic/evil-husband",
            series_page(
                "evil-husband",
                &[row("chapter-1", "Chapter 1", "2025-01-01T00:00:00Z")],
                "",
            ),
        );
        for link in [
            "https://templetoons.com/comic/evil-husband",
            "https://www.templetoons.com/comic/evil-husband/chapter-1?x=1",
            "https://templescan.net/comic/evil-husband/",
        ] {
            let LinkValue::Manga(manga) = open_link::<Recorded>(link).expect("link") else {
                panic!("manga expected");
            };
            assert_eq!(manga.key, "evil-husband", "{link}");
            assert_eq!(manga.title, "Evil Husband");
            assert_eq!(manga.chapters.expect("chapters").len(), 1);
        }
        assert_eq!(requests(), ["/comic/evil-husband"; 3]);
        for link in [
            "https://example.com/comic/evil-husband",
            "https://templetoons.com/comics",
            "not a url",
        ] {
            assert!(
                matches!(open_link::<Recorded>(link), Err(Error::Unsupported)),
                "{link}"
            );
        }
        assert!(
            requests().is_empty(),
            "links that are not ours make no request"
        );
    }

    #[test]
    fn image_requests_are_limited_to_the_sites_hosts_whatever_the_origins_say() {
        assert_eq!(
            TempleScan::get_image_origins().expect("origins"),
            [
                "https://media.templetoons.com",
                "https://akira.templetoons.com"
            ]
        );
        let request =
            TempleScan::get_image_request("https://media.templetoons.com/a.jpg").expect("request");
        assert_eq!(request.url, "https://media.templetoons.com/a.jpg");
        assert_eq!(
            request
                .headers
                .as_ref()
                .and_then(|h| h.get("Referer"))
                .map(String::as_str),
            Some("https://templetoons.com/")
        );
        assert!(TempleScan::get_image_request("https://example.com/a.jpg").is_err());
    }

    #[test]
    fn the_source_serves_manga_only() {
        assert_eq!(MediaKind::mask(TempleScan::KINDS), 0b001);
    }
}
