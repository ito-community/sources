//! nhentai galleries: home, search, listings, and the page reader.
//!
//! Lolicon and shotacon are never searched or opened.

mod api;
mod controls;
mod gallery;
mod images;
mod query;

use gallery::View;
use ito_rs::controls::{
    OptionPage, OptionRequest, SearchCapabilities, SearchSuggestion, SettingsForm,
};
use ito_rs::models::manga::{Manga, PageResult};
use ito_rs::models::{
    FilterItem, HomeComponent, HomeComponentValue, HomeLayout, Link, LinkValue, Listing, Page,
    PluginImageRequest, SettingIssue, SettingsAction, SettingsSchema,
};
use ito_rs::provider::MangaProvider;
use ito_rs::{Error, Result};
use query::{SearchParts, Sort};

const RANDOM_ATTEMPTS: u8 = 3;

struct NHentai;

impl MangaProvider for NHentai {
    fn get_home() -> Result<HomeLayout> {
        let parts = controls::browse_parts()?;
        let view = controls::view()?;
        let mut components = Vec::new();
        let mut failure = None;
        for (title, sort, kind) in [
            ("Popular today", Sort::PopularToday, Section::Featured),
            ("Popular this week", Sort::PopularWeek, Section::Ranked),
            ("Popular this month", Sort::PopularMonth, Section::List),
            ("Latest", Sort::Date, Section::Scroller),
        ] {
            match section(title, sort, kind, &parts, &view) {
                Ok(Some(component)) => components.push(component),
                Ok(None) => {}
                Err(err) => {
                    failure.get_or_insert(err);
                }
            }
        }
        components.push(browse_links());
        if components.len() == 1 {
            return Err(
                failure.unwrap_or_else(|| Error::Server("nhentai returned no galleries".into()))
            );
        }
        Ok(HomeLayout { components })
    }

    fn get_settings() -> Option<SettingsSchema> {
        Some(controls::legacy_settings())
    }

    fn get_settings_actions() -> Result<Vec<SettingsAction>> {
        Ok(vec![SettingsAction {
            id: "test_api_key".into(),
            title: "Test API key".into(),
            summary: Some("Checks the saved key without changing the account.".into()),
        }])
    }

    fn run_settings_action(action_id: &str) -> Result<String> {
        if action_id != "test_api_key" {
            return Err(Error::Unsupported);
        }
        if api::api_key().is_none() {
            return Ok(
                "No API key is saved. Browse works without one. Favorites need a key.".into(),
            );
        }
        match api::account_accepted() {
            Ok(true) => Ok("The API key was accepted.".into()),
            Ok(false) => Ok("The API key was rejected.".into()),
            Err(Error::Server(message)) if message.contains("rate limit") => Ok(message),
            Err(Error::Net(_)) => Ok("nhentai could not be reached.".into()),
            Err(_) => Ok("nhentai could not check the API key.".into()),
        }
    }

    fn validate_settings() -> Result<Vec<SettingIssue>> {
        controls::validate()
    }

    fn get_settings_form() -> Result<SettingsForm> {
        Ok(controls::settings_form())
    }

    fn get_search_capabilities() -> Result<SearchCapabilities> {
        Ok(controls::capabilities())
    }

    fn get_option_page(request: OptionRequest) -> Result<OptionPage> {
        controls::option_page(request)
    }

    fn get_search_suggestions(query: &str) -> Result<Vec<SearchSuggestion>> {
        controls::suggestions(query)
    }

    fn get_manga_list(listing: Listing, page: i32) -> Result<PageResult> {
        let view = controls::view()?;
        match listing.id.as_str() {
            "random" => random_page(&view),
            "favorites" => {
                let response = api::favorites(page)?;
                Ok(page_result(response, page, &view))
            }
            id => {
                let sort = Sort::from_listing(id)
                    .ok_or_else(|| Error::NotFound("Unknown listing".into()))?;
                let response = api::search(&controls::browse_parts()?.build(), page, sort)?;
                Ok(page_result(response, page, &view))
            }
        }
    }

    fn get_search_manga_list(
        query: &str,
        page: i32,
        filters: Vec<FilterItem>,
    ) -> Result<PageResult> {
        let view = controls::view()?;
        if let Some(id) = query::lookup_id(query) {
            return id_page(id, &view);
        }
        let (parts, sort) = controls::search_parts(query, &filters)?;
        let response = api::search(&parts.build(), page, sort)?;
        Ok(page_result(response, page, &view))
    }

    fn get_manga_update(manga: Manga, needs_details: bool, needs_chapters: bool) -> Result<Manga> {
        if !needs_details && !needs_chapters {
            return Ok(manga);
        }
        let id = manga
            .key
            .parse()
            .map_err(|_| Error::NotFound("This gallery is not available in this source.".into()))?;
        gallery::detail(&api::gallery(id)?, &controls::view()?)
    }

    fn get_page_list(manga: Manga, chapter: ito_rs::models::Chapter) -> Result<Vec<Page>> {
        let id = chapter
            .key
            .parse()
            .or_else(|_| manga.key.parse())
            .map_err(|_| Error::NotFound("This gallery is not available in this source.".into()))?;
        gallery::pages(&api::gallery(id)?, controls::view()?.server)
    }

    fn handle_url(url: &str) -> Result<LinkValue> {
        let id = query::gallery_id_from_url(url).ok_or(Error::Unsupported)?;
        Ok(LinkValue::Manga(gallery::detail(
            &api::gallery(id)?,
            &controls::view()?,
        )?))
    }

    fn get_image_origins() -> Result<Vec<String>> {
        Ok(images::origins())
    }

    fn get_image_request(url: &str) -> Result<PluginImageRequest> {
        images::request(url)
            .ok_or_else(|| Error::Host("nhentai images come only from the site image hosts".into()))
    }
}

ito_rs::export_manga_plugin!(NHentai);

enum Section {
    Featured,
    Ranked,
    List,
    Scroller,
}

fn section(
    title: &str,
    sort: Sort,
    kind: Section,
    parts: &SearchParts,
    view: &View,
) -> Result<Option<HomeComponent>> {
    let response = api::search(&parts.build(), 1, sort)?;
    let mut entries = map_list(&response.result, view);
    if entries.is_empty() {
        return Ok(None);
    }
    if matches!(kind, Section::Featured) {
        entries.truncate(12);
    }
    let listing = listing_for(sort, title);
    let value = match kind {
        Section::Featured => HomeComponentValue::BigScroller(entries, Some(8.0)),
        Section::Ranked => HomeComponentValue::MangaList(true, Some(5), entries, Some(listing)),
        Section::List => HomeComponentValue::MangaList(false, None, entries, Some(listing)),
        Section::Scroller => HomeComponentValue::Scroller(entries, Some(listing)),
    };
    Ok(Some(HomeComponent {
        title: Some(title.into()),
        subtitle: None,
        value,
    }))
}

fn browse_links() -> HomeComponent {
    HomeComponent {
        title: Some("Browse".into()),
        subtitle: None,
        value: HomeComponentValue::Links(vec![
            link("Popular all time", "popular"),
            link("Random", "random"),
            link("Favorites", "favorites"),
        ]),
    }
}

fn link(title: &str, id: &str) -> Link {
    Link {
        title: title.into(),
        value: Some(LinkValue::Listing(Listing {
            id: id.into(),
            name: title.into(),
            kind: 0,
        })),
    }
}

fn listing_for(sort: Sort, title: &str) -> Listing {
    let id = match sort {
        Sort::Date => "latest",
        Sort::PopularToday => "popular-today",
        Sort::PopularWeek => "popular-week",
        Sort::PopularMonth => "popular-month",
        Sort::Popular => "popular",
    };
    Listing {
        id: id.into(),
        name: title.into(),
        kind: 0,
    }
}

fn id_page(id: i64, view: &View) -> Result<PageResult> {
    match api::gallery(id) {
        Ok(item) => match gallery::detail(&item, view) {
            Ok(manga) => {
                let mut manga = manga;
                manga.chapters = None;
                manga.catalog = None;
                Ok(PageResult {
                    entries: vec![manga],
                    has_next_page: false,
                })
            }
            Err(Error::NotFound(_)) => Ok(empty_page()),
            Err(err) => Err(err),
        },
        Err(Error::NotFound(_)) => Ok(empty_page()),
        Err(err) => Err(err),
    }
}

fn random_page(view: &View) -> Result<PageResult> {
    for _ in 0..RANDOM_ATTEMPTS {
        let id = api::random_id()?;
        match api::gallery(id) {
            Ok(item) => match gallery::detail(&item, view) {
                Ok(manga) => {
                    let mut manga = manga;
                    manga.chapters = None;
                    manga.catalog = None;
                    return Ok(PageResult {
                        entries: vec![manga],
                        has_next_page: true,
                    });
                }
                Err(Error::NotFound(_)) => continue,
                Err(err) => return Err(err),
            },
            Err(Error::NotFound(_)) => continue,
            Err(err) => return Err(err),
        }
    }
    Ok(PageResult {
        entries: Vec::new(),
        has_next_page: true,
    })
}

fn page_result(page: api::Page, requested: i32, view: &View) -> PageResult {
    PageResult {
        entries: map_list(&page.result, view),
        has_next_page: requested < page.num_pages,
    }
}

fn map_list(items: &[api::ListItem], view: &View) -> Vec<Manga> {
    items
        .iter()
        .filter_map(|item| gallery::list_item(item, view))
        .collect()
}

fn empty_page() -> PageResult {
    PageResult {
        entries: Vec::new(),
        has_next_page: false,
    }
}
