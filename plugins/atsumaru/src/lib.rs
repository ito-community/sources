//! Atsumaru (atsu.moe): comics and novels, with discovery and filtering driven by settings.

use std::cell::RefCell;
use std::rc::Rc;

use ito_rs::models::manga::{self, ContentRating, Manga, Viewer};
use ito_rs::models::novel::{self, Novel};
use ito_rs::models::{
    Chapter, ContentKind, FilterItem, HomeComponent, HomeComponentValue, HomeLayout, LinkValue,
    Listing, MediaEntry, MediaKind, MediaPageResult, Page, PageContent, SettingIssue,
    SettingsAction, SettingsSchema,
};
use ito_rs::net::Request;
use ito_rs::provider::SourceProvider;
use ito_rs::{Error, Result};
use serde::de::DeserializeOwned;

mod chapters;
mod links;
mod models;
mod query;
mod settings;

use models::*;
use query::{Medium, PAGE_SIZE, SearchIds};
use settings::{ContentPreferences, Preferences, Rating, Section, SectionSource, Sort};

const API: &str = "https://atsu.moe/api";
const CHECK_FILTERS: &str = "check_filter_names";

pub struct Atsumaru;

fn get_json<T: DeserializeOwned>(url: &str) -> Result<T> {
    let response = Request::get(url)
        .header("Referer", "https://atsu.moe/")
        .send()?;
    decode(response.status, &response.body)
}

/// Checks the status before decoding; bodies never appear in errors.
fn decode<T: DeserializeOwned>(status: i32, body: &[u8]) -> Result<T> {
    match status {
        200..=299 => serde_json::from_slice(body).map_err(|_| {
            Error::Host("Atsumaru sent a response the plugin does not understand".into())
        }),
        404 => Err(Error::NotFound(
            "Atsumaru couldn't find what was requested".into(),
        )),
        _ => Err(Error::Server(format!(
            "Atsumaru answered with HTTP {status}"
        ))),
    }
}

thread_local! {
    static TAXONOMY: RefCell<Option<Rc<Taxonomy>>> = const { RefCell::new(None) };
}

/// Genre and tag names, fetched once per plugin instance and only when a setting names one.
fn taxonomy() -> Result<Rc<Taxonomy>> {
    if let Some(cached) = TAXONOMY.with_borrow(Clone::clone) {
        return Ok(cached);
    }
    let fetched = Rc::new(get_json::<Taxonomy>(&format!(
        "{API}/explore/availableFilters"
    ))?);
    TAXONOMY.set(Some(Rc::clone(&fetched)));
    Ok(fetched)
}

fn unknown_filter_name() -> Error {
    Error::NeedsSetup(
        "A genre or tag name in Atsumaru's settings doesn't exist on the site. \
         Use Check filter names in the source settings to find it."
            .into(),
    )
}

/// Ids for names entered in settings; resolving needs the taxonomy only when one is set.
fn ids(names: &[String], terms: impl FnOnce(&Taxonomy) -> &[Term]) -> Result<Vec<String>> {
    if names.is_empty() {
        return Ok(Vec::new());
    }
    let taxonomy = taxonomy()?;
    query::resolve(names, terms(&taxonomy)).map_err(|_| unknown_filter_name())
}

/// Loads the settings, refusing to run with settings that can't be honoured.
fn preferences() -> Result<Preferences> {
    let preferences = Preferences::load();
    match preferences.issues().first() {
        Some(issue) => Err(Error::NeedsSetup(format!(
            "Atsumaru's settings need attention: {}",
            issue.message
        ))),
        None => Ok(preferences),
    }
}

/// Converts the site rating to the app's three levels. Without a site rating, adult titles
/// count as explicit and others take the most the settings allow, capped at Suggestive.
fn content_rating(site: Option<&str>, adult: bool, ceiling: Rating) -> ContentRating {
    if adult {
        return ContentRating::Nsfw;
    }
    match site
        .and_then(Rating::parse)
        .unwrap_or(ceiling.min(Rating::Suggestive))
    {
        Rating::Safe => ContentRating::Safe,
        Rating::Suggestive => ContentRating::Suggestive,
        Rating::Erotica | Rating::Pornographic => ContentRating::Nsfw,
    }
}

fn manga_status(status: Option<&str>) -> manga::Status {
    match status {
        Some("Ongoing") => manga::Status::Ongoing,
        Some("Completed") => manga::Status::Completed,
        Some("Hiatus") => manga::Status::Hiatus,
        Some("Canceled" | "Cancelled") => manga::Status::Cancelled,
        _ => manga::Status::Unknown,
    }
}

fn novel_status(status: Option<&str>) -> novel::Status {
    match status {
        Some("Ongoing") => novel::Status::Ongoing,
        Some("Completed") => novel::Status::Completed,
        Some("Hiatus") => novel::Status::Hiatus,
        Some("Canceled" | "Cancelled") => novel::Status::Cancelled,
        _ => novel::Status::Unknown,
    }
}

fn novel_rating(rating: ContentRating) -> novel::ContentRating {
    match rating {
        ContentRating::Safe => novel::ContentRating::Safe,
        ContentRating::Suggestive => novel::ContentRating::Suggestive,
        ContentRating::Nsfw => novel::ContentRating::Nsfw,
    }
}

/// Default reading mode from the title's origin. Japanese manga reads right to left; everything
/// else keeps the vertical scroll this source has always used.
fn viewer(origin: Option<&str>) -> Viewer {
    match origin {
        Some("Manga") => Viewer::Rtl,
        _ => Viewer::Webtoon,
    }
}

fn is_novel(medium: Option<&str>) -> bool {
    medium == Some("Novel")
}

fn nsfw(rating: &ContentRating) -> i32 {
    i32::from(matches!(rating, ContentRating::Nsfw))
}

/// A catalog entry before it becomes a manga or a novel.
struct Listed {
    id: String,
    title: String,
    cover: Option<String>,
    medium: Option<String>,
    origin: Option<String>,
    status: Option<String>,
    authors: Option<Vec<String>>,
    rating: ContentRating,
}

impl Listed {
    fn from_document(document: SearchDocument, ceiling: Rating) -> Self {
        let rating = content_rating(
            document.mb_content_rating.as_deref(),
            document.is_adult.unwrap_or(false),
            ceiling,
        );
        Listed {
            title: display_title(document.title, document.english_title, &document.id),
            cover: document.poster.as_deref().and_then(links::image_url),
            id: document.id,
            medium: document.medium,
            origin: document.origin,
            status: document.status,
            authors: document.authors.filter(|authors| !authors.is_empty()),
            rating,
        }
    }

    fn into_entry(self) -> MediaEntry {
        let novel = is_novel(self.medium.as_deref());
        let url = Some(links::series_url(&self.id, novel));
        if novel {
            MediaEntry::Novel(Novel {
                key: self.id,
                title: self.title,
                authors: self.authors,
                artist: None,
                description: None,
                tags: None,
                cover: self.cover,
                url,
                status: novel_status(self.status.as_deref()),
                nsfw: nsfw(&self.rating),
                content_rating: novel_rating(self.rating),
                chapters: None,
            })
        } else {
            MediaEntry::Manga(Manga {
                key: self.id,
                title: self.title,
                authors: self.authors,
                artist: None,
                description: None,
                tags: None,
                cover: self.cover,
                url,
                status: manga_status(self.status.as_deref()),
                nsfw: nsfw(&self.rating),
                content_rating: self.rating,
                viewer: viewer(self.origin.as_deref()),
                chapters: None,
            })
        }
    }
}

/// The site's title, then its English title, then the id, so an entry is never untitled.
fn display_title(title: String, english: Option<String>, id: &str) -> String {
    [Some(title), english]
        .into_iter()
        .flatten()
        .find(|title| !title.trim().is_empty())
        .unwrap_or_else(|| id.to_owned())
}

fn search_page(url: &str, page: i32, ceiling: Rating) -> Result<MediaPageResult> {
    let response: SearchResponse = get_json(url)?;
    let shown = usize::try_from(page.max(1))
        .unwrap_or(1)
        .saturating_mul(PAGE_SIZE);
    Ok(MediaPageResult {
        has_next_page: response.found > shown,
        entries: response
            .hits
            .into_iter()
            .map(|hit| Listed::from_document(hit.document, ceiling).into_entry())
            .collect(),
    })
}

/// Site documents for specific series, in no particular order.
fn lookup(ids: &[&str]) -> Result<Vec<SearchDocument>> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    let response: SearchResponse = get_json(&query::lookup_url(ids))?;
    Ok(response.hits.into_iter().map(|hit| hit.document).collect())
}

fn blocked_tag_ids(preferences: &Preferences) -> Result<Vec<String>> {
    ids(&preferences.content.excluded_tags, |taxonomy| {
        &taxonomy.tags
    })
}

fn search_catalog(query: &str, page: i32, kind: Option<MediaKind>) -> Result<MediaPageResult> {
    let preferences = preferences()?;
    let ceiling = preferences.content.max_rating;
    let mediums = query::mediums(&preferences.content, kind);
    if mediums.is_empty() {
        return Ok(empty_page());
    }
    if let Some(id) = links::series_id(query) {
        return pasted_link(&id, &mediums, ceiling);
    }
    let search = &preferences.search;
    let search_ids = SearchIds {
        include_genres: ids(&search.include_genres, |taxonomy| &taxonomy.genres)?,
        exclude_genres: ids(&search.exclude_genres, |taxonomy| &taxonomy.genres)?,
        include_tags: ids(&search.include_tags, |taxonomy| &taxonomy.tags)?,
    };
    let mut filters = query::catalog_filters(
        &preferences.content,
        &mediums,
        &blocked_tag_ids(&preferences)?,
    );
    filters.extend(query::search_filters(search, &search_ids));
    search_page(
        &query::search_url(query, page, filters, search.sort),
        page,
        ceiling,
    )
}

/// A pasted site link finds its series directly, ignoring search filters but not the media
/// shown.
fn pasted_link(id: &str, mediums: &[Medium], ceiling: Rating) -> Result<MediaPageResult> {
    let shown = |document: &SearchDocument| {
        let medium = if is_novel(document.medium.as_deref()) {
            Medium::Novel
        } else {
            Medium::Comic
        };
        document.id == id && mediums.contains(&medium)
    };
    let entries = lookup(&[id])?
        .into_iter()
        .filter(shown)
        .map(|document| Listed::from_document(document, ceiling).into_entry())
        .collect();
    Ok(MediaPageResult {
        entries,
        has_next_page: false,
    })
}

fn empty_page() -> MediaPageResult {
    MediaPageResult {
        entries: Vec::new(),
        has_next_page: false,
    }
}

/// The media a section shows under the current settings.
fn section_mediums(section: &Section, preferences: &Preferences) -> Vec<Medium> {
    match section.source {
        SectionSource::Feed {
            novels_only: true, ..
        } => query::mediums(&preferences.content, Some(MediaKind::Novel)),
        _ => query::mediums(&preferences.content, None),
    }
}

/// The spotlight genre's id and display name.
fn spotlight(preferences: &Preferences) -> Result<Option<Term>> {
    let Some(name) = preferences.spotlight_genre.as_deref() else {
        return Ok(None);
    };
    let taxonomy = taxonomy()?;
    taxonomy
        .genres
        .iter()
        .find(|genre| genre.id == name || settings::same_name(&genre.name, name))
        .map(|genre| {
            Some(Term {
                id: genre.id.clone(),
                name: genre.name.clone(),
            })
        })
        .ok_or_else(unknown_filter_name)
}

fn section_page(
    section: &Section,
    preferences: &Preferences,
    page: i32,
) -> Result<MediaPageResult> {
    let mediums = section_mediums(section, preferences);
    if mediums.is_empty() {
        return Ok(empty_page());
    }
    let content = &preferences.content;
    let blocked = blocked_tag_ids(preferences)?;
    match section.source {
        SectionSource::Feed { endpoint, .. } => {
            let offset = usize::try_from(page.max(1) - 1).unwrap_or(0) * PAGE_SIZE;
            let url = query::feed_url(
                endpoint,
                offset,
                &mediums,
                content,
                &blocked,
                preferences.period(section),
            );
            let feed: FeedResponse = get_json(&url)?;
            Ok(MediaPageResult {
                has_next_page: feed.items.len() == PAGE_SIZE,
                entries: rate_feed(feed.items, content)?,
            })
        }
        SectionSource::Trending => {
            let filters = query::catalog_filters(content, &mediums, &blocked);
            search_page(
                &query::search_url("", page, filters, Sort::Trending),
                page,
                content.max_rating,
            )
        }
        SectionSource::GenreSpotlight => {
            let genre = spotlight(preferences)?.ok_or_else(|| {
                Error::NeedsSetup("Set a spotlight genre in Atsumaru's settings.".into())
            })?;
            let mut filters = query::catalog_filters(content, &mediums, &blocked);
            filters.push(query::genre_filter(&genre.id));
            search_page(
                &query::search_url("", page, filters, Sort::Popularity),
                page,
                content.max_rating,
            )
        }
    }
}

/// Feeds carry no content rating, so it comes from the catalog in one lookup per page. The
/// site already filters feeds by rating; entries outside the settings are dropped regardless.
fn rate_feed(items: Vec<FeedItem>, content: &ContentPreferences) -> Result<Vec<MediaEntry>> {
    let ids: Vec<&str> = items.iter().map(|item| item.id.as_str()).collect();
    let documents = lookup(&ids)?;
    Ok(items
        .into_iter()
        .filter_map(|item| {
            let rated = documents.iter().find(|document| document.id == item.id);
            let site_rating = rated.and_then(|document| document.mb_content_rating.as_deref());
            let adult = item.is_adult.unwrap_or(false)
                || rated.and_then(|document| document.is_adult) == Some(true);
            if !feed_allows(site_rating.and_then(Rating::parse), adult, content) {
                return None;
            }
            let rating = content_rating(site_rating, adult, content.max_rating);
            let listed = Listed {
                cover: item.image.as_deref().and_then(links::image_url),
                status: rated.and_then(|document| document.status.clone()),
                authors: rated
                    .and_then(|document| document.authors.clone())
                    .filter(|authors| !authors.is_empty()),
                id: item.id,
                title: item.title,
                medium: item.medium,
                origin: item.origin,
                rating,
            };
            Some(listed.into_entry())
        })
        .collect())
}

/// Whether a feed entry fits the content settings, as the search filters require.
fn feed_allows(rating: Option<Rating>, adult: bool, content: &ContentPreferences) -> bool {
    if content.adult_only {
        return adult;
    }
    if adult && content.max_rating < Rating::Pornographic {
        return false;
    }
    rating.is_none_or(|rating| rating <= content.max_rating)
}

fn home() -> Result<HomeLayout> {
    let preferences = preferences()?;
    let mut components = Vec::new();
    for section in settings::SECTIONS {
        if !preferences.section_enabled(section) {
            continue;
        }
        let mediums = section_mediums(section, &preferences);
        if mediums.is_empty() {
            continue;
        }
        let title = match section.source {
            SectionSource::GenreSpotlight => match spotlight(&preferences)? {
                Some(genre) => format!("Spotlight: {}", genre.name),
                None => continue,
            },
            _ => section.title.to_owned(),
        };
        let entries = section_page(section, &preferences, 1)
            .map_err(|error| in_section(error, &title))?
            .entries;
        let listing = Some(Listing {
            id: section.id.into(),
            name: title.clone(),
            kind: 0,
        });
        let value = match mediums.as_slice() {
            [Medium::Comic] => HomeComponentValue::Scroller(
                entries
                    .into_iter()
                    .filter_map(|entry| match entry {
                        MediaEntry::Manga(manga) => Some(manga),
                        _ => None,
                    })
                    .collect(),
                listing,
            ),
            [Medium::Novel] => HomeComponentValue::NovelScroller(
                entries
                    .into_iter()
                    .filter_map(|entry| match entry {
                        MediaEntry::Novel(novel) => Some(novel),
                        _ => None,
                    })
                    .collect(),
                listing,
            ),
            _ => HomeComponentValue::MediaScroller(entries, listing),
        };
        components.push(HomeComponent {
            title: Some(title),
            subtitle: None,
            value,
        });
    }
    Ok(HomeLayout { components })
}

fn listing(listing: &Listing, page: i32) -> Result<MediaPageResult> {
    let section = settings::section(&listing.id)
        .ok_or_else(|| Error::NotFound("Atsumaru has no such list".into()))?;
    section_page(section, &preferences()?, page)
}

/// Refreshes an entry, keeping its kind: entries saved before novels were supported stay
/// manga, and their refreshed chapters are marked as text.
fn update(mut entry: MediaEntry, details: bool, units: bool) -> Result<MediaEntry> {
    if !details && !units {
        return Ok(entry);
    }
    if matches!(entry, MediaEntry::Anime(_)) {
        return Err(Error::Unsupported);
    }
    let key = entry.key().to_owned();
    let series =
        get_json::<SeriesResponse>(&format!("{API}/manga/page?id={}", query::encode(&key)))?
            .manga_page;
    let novel = is_novel(series.medium.as_deref());
    let preferences = Preferences::load();

    let chapters = if units {
        let list: ChapterListResponse = get_json(&format!(
            "{API}/manga/allChapters?mangaId={}",
            query::encode(&key)
        ))?;
        Some(
            chapters::select(list.chapters, &series.scanlators, &preferences.chapters)
                .into_iter()
                .map(|item| chapter(item, &key, &series.scanlators, novel))
                .collect::<Vec<_>>(),
        )
    } else {
        None
    };

    let details = if details {
        let catalog = lookup(&[key.as_str()])?
            .into_iter()
            .find(|document| document.id == key);
        Some(Details::new(
            series,
            catalog,
            preferences.content.max_rating,
        ))
    } else {
        None
    };

    match &mut entry {
        MediaEntry::Manga(manga) => {
            if let Some(details) = details {
                manga.viewer = viewer(details.origin.as_deref());
                manga.status = manga_status(details.status.as_deref());
                manga.nsfw = nsfw(&details.rating);
                manga.content_rating = details.rating;
                manga.title = details.title;
                manga.description = details.description;
                manga.cover = details.cover;
                manga.url = Some(details.url);
                manga.authors = details.authors;
                manga.artist = details.artist;
                manga.tags = details.tags;
            }
            if chapters.is_some() {
                manga.chapters = chapters;
            }
        }
        MediaEntry::Novel(novel) => {
            if let Some(details) = details {
                novel.status = novel_status(details.status.as_deref());
                novel.nsfw = nsfw(&details.rating);
                novel.content_rating = novel_rating(details.rating);
                novel.title = details.title;
                novel.description = details.description;
                novel.cover = details.cover;
                novel.url = Some(details.url);
                novel.authors = details.authors;
                novel.artist = details.artist;
                novel.tags = details.tags;
            }
            if chapters.is_some() {
                novel.chapters = chapters;
            }
        }
        MediaEntry::Anime(_) => return Err(Error::Unsupported),
    }
    Ok(entry)
}

struct Details {
    title: String,
    description: Option<String>,
    cover: Option<String>,
    url: String,
    status: Option<String>,
    origin: Option<String>,
    authors: Option<Vec<String>>,
    artist: Option<String>,
    tags: Option<Vec<String>>,
    rating: ContentRating,
}

impl Details {
    fn new(series: Series, catalog: Option<SearchDocument>, ceiling: Rating) -> Self {
        let adult = series.is_adult.unwrap_or(false)
            || catalog.as_ref().and_then(|document| document.is_adult) == Some(true);
        let rating = content_rating(
            catalog
                .as_ref()
                .and_then(|document| document.mb_content_rating.as_deref()),
            adult,
            ceiling,
        );
        let (artists, authors): (Vec<Creator>, Vec<Creator>) = series
            .authors
            .into_iter()
            .partition(|creator| creator.role.as_deref() == Some("Artist"));
        let names = |creators: Vec<Creator>| -> Vec<String> {
            creators.into_iter().map(|creator| creator.name).collect()
        };
        let artists = names(artists);
        let authors = names(authors);
        let tags: Vec<String> = series.genres.into_iter().map(|genre| genre.name).collect();
        Details {
            url: links::series_url(&series.id, is_novel(series.medium.as_deref())),
            title: display_title(series.title, series.english_title, &series.id),
            description: series.synopsis.filter(|text| !text.trim().is_empty()),
            cover: series
                .poster
                .and_then(|poster| poster.image)
                .as_deref()
                .and_then(links::image_url),
            status: series.status,
            origin: series.origin,
            authors: (!authors.is_empty()).then_some(authors),
            artist: (!artists.is_empty()).then(|| artists.join(", ")),
            tags: (!tags.is_empty()).then_some(tags),
            rating,
        }
    }
}

fn chapter(item: ChapterItem, series: &str, groups: &[Scanlator], novel: bool) -> Chapter {
    let scanlator = item.scanlation_manga_id.as_deref().and_then(|id| {
        groups
            .iter()
            .find(|group| group.id == id)
            .map(|group| group.name.clone())
    });
    Chapter {
        url: Some(links::chapter_url(series, &item.id, novel)),
        key: item.id,
        title: item.title.filter(|title| !title.trim().is_empty()),
        volume: None,
        // The app stores chapter numbers as f32.
        chapter: item.number.map(|number| number as f32),
        date_updated: item.created_at.map(|millis| millis as f64 / 1000.0),
        scanlator,
        lang: Some("en".into()),
        paywalled: None,
        kind: if novel {
            ContentKind::Text
        } else {
            ContentKind::Images
        },
    }
}

fn pages(entry: &MediaEntry, chapter: &Chapter) -> Result<Vec<Page>> {
    let pairs = [
        ("mangaId", entry.key().to_owned()),
        ("chapterId", chapter.key.clone()),
    ];
    let text = matches!(entry, MediaEntry::Novel(_)) || chapter.kind == ContentKind::Text;
    if !text {
        let read: ReadChapterResponse =
            get_json(&query::with_query(&format!("{API}/read/chapter"), &pairs))?;
        let mut images = read.read_chapter.pages;
        // Chapters of novels saved before novels were supported say images; the site answers
        // those with no pages, so they are read as text.
        if !images.is_empty() {
            images.sort_by_key(|page| page.number);
            return images
                .into_iter()
                .enumerate()
                .map(|(index, page)| {
                    let url = links::image_url(&page.image).ok_or_else(|| {
                        Error::Host("Atsumaru returned an unexpected image location".into())
                    })?;
                    Ok(page_at(index, PageContent::Url(url)))
                })
                .collect();
        }
    }
    let read: NovelChapterResponse = get_json(&query::with_query(
        &format!("{API}/read/novelChapter"),
        &pairs,
    ))
    .map_err(|error| match error {
        // A comic chapter without pages is not a novel chapter either.
        Error::NotFound(_) | Error::Host(_) if !text => no_content(),
        other => other,
    })?;
    if read.read_novel_chapter.paragraphs.is_empty() {
        return Err(no_content());
    }
    Ok(read
        .read_novel_chapter
        .paragraphs
        .into_iter()
        .enumerate()
        .map(|(index, paragraph)| page_at(index, PageContent::Text(paragraph)))
        .collect())
}

/// Names the Discover section a failure came from, so it can be turned off in settings.
fn in_section(error: Error, title: &str) -> Error {
    match error {
        Error::NotFound(_) => Error::NotFound(format!(
            "Atsumaru's {title} section is unavailable; turn it off in the source settings"
        )),
        Error::Server(message) => Error::Server(format!("{title}: {message}")),
        other => other,
    }
}

fn no_content() -> Error {
    Error::NotFound("This Atsumaru chapter has no pages".into())
}

fn page_at(index: usize, content: PageContent) -> Page {
    Page {
        index: i32::try_from(index).unwrap_or(i32::MAX),
        content,
        has_description: false,
        description: None,
        headers: None,
    }
}

fn open_link(url: &str) -> Result<LinkValue> {
    let id = links::series_id(url).ok_or(Error::Unsupported)?;
    let ceiling = Preferences::load().content.max_rating;
    let document = lookup(&[id.as_str()])?
        .into_iter()
        .find(|document| document.id == id)
        .ok_or_else(|| Error::NotFound("Atsumaru has no such title".into()))?;
    match Listed::from_document(document, ceiling).into_entry() {
        MediaEntry::Novel(novel) => Ok(LinkValue::Novel(novel)),
        MediaEntry::Manga(manga) => Ok(LinkValue::Manga(manga)),
        MediaEntry::Anime(_) => Err(Error::Unsupported),
    }
}

/// Every genre and tag field, with the names entered that the site doesn't know.
fn check_filter_names() -> Result<String> {
    let taxonomy = taxonomy()?;
    let read = |key: &str| {
        settings::split_list(
            &ito_rs::defaults::get(key)
                .ok()
                .flatten()
                .unwrap_or_default(),
        )
    };
    let mut fields: Vec<(String, Vec<String>, &[Term])> = vec![
        (
            "Blocked tags · Any group".into(),
            read(settings::EXCLUDE_TAGS),
            &taxonomy.tags,
        ),
        (
            "Search · Required genres".into(),
            read(settings::INCLUDE_GENRES),
            &taxonomy.genres,
        ),
        (
            "Search · Excluded genres".into(),
            read(settings::EXCLUDE_GENRES),
            &taxonomy.genres,
        ),
        (
            "Search · Required tags".into(),
            read(settings::INCLUDE_TAGS),
            &taxonomy.tags,
        ),
        (
            "Discover · Spotlight genre".into(),
            read(settings::SPOTLIGHT_GENRE),
            &taxonomy.genres,
        ),
    ];
    fields.extend(settings::TAG_GROUPS.iter().map(|(id, group)| {
        (
            format!("Blocked tags · {group}"),
            read(id),
            taxonomy.tags.as_slice(),
        )
    }));
    Ok(filter_report(&fields))
}

fn filter_report(fields: &[(String, Vec<String>, &[Term])]) -> String {
    let mut checked = 0;
    let mut problems = Vec::new();
    for (label, names, terms) in fields {
        checked += names.len();
        let unknown: Vec<&str> = names
            .iter()
            .filter(|name| query::resolve(std::slice::from_ref(*name), terms).is_err())
            .map(String::as_str)
            .collect();
        if !unknown.is_empty() {
            problems.push(format!("{label}: {}", unknown.join(", ")));
        }
    }
    if problems.is_empty() {
        format!("All {checked} genre and tag names match the site.")
    } else {
        format!("Not found on the site — {}", problems.join("; "))
    }
}

impl SourceProvider for Atsumaru {
    const KINDS: &'static [MediaKind] = &[MediaKind::Manga, MediaKind::Novel];

    fn get_home() -> Result<HomeLayout> {
        home()
    }

    fn get_settings() -> Option<SettingsSchema> {
        Some(settings::schema())
    }

    fn get_settings_actions() -> Result<Vec<SettingsAction>> {
        Ok(vec![SettingsAction {
            id: CHECK_FILTERS.into(),
            title: "Check filter names".into(),
            summary: Some("Look up every genre and tag name entered above on Atsumaru.".into()),
        }])
    }

    fn run_settings_action(action_id: &str) -> Result<String> {
        match action_id {
            CHECK_FILTERS => check_filter_names(),
            _ => Err(Error::Unsupported),
        }
    }

    fn validate_settings() -> Result<Vec<SettingIssue>> {
        Ok(Preferences::load().issues().to_vec())
    }

    fn get_listing(listing_ref: Listing, page: i32) -> Result<MediaPageResult> {
        listing(&listing_ref, page)
    }

    fn search(
        query: &str,
        page: i32,
        kind: Option<MediaKind>,
        filters: Vec<FilterItem>,
    ) -> Result<MediaPageResult> {
        // Filtering is configured in settings; the host sends no filters of its own.
        if !filters.is_empty() {
            return Err(Error::Unsupported);
        }
        search_catalog(query, page, kind)
    }

    fn get_media_update(
        entry: MediaEntry,
        needs_details: bool,
        needs_units: bool,
    ) -> Result<MediaEntry> {
        update(entry, needs_details, needs_units)
    }

    fn get_chapter_pages(entry: MediaEntry, chapter: Chapter) -> Result<Vec<Page>> {
        pages(&entry, &chapter)
    }

    fn handle_url(url: &str) -> Result<LinkValue> {
        open_link(url)
    }
}

ito_rs::export_plugin!(Atsumaru);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ratings_never_understate_adult_titles() {
        assert!(matches!(
            content_rating(Some("Safe"), true, Rating::Safe),
            ContentRating::Nsfw
        ));
        assert!(matches!(
            content_rating(Some("Erotica"), false, Rating::Safe),
            ContentRating::Nsfw
        ));
        assert!(matches!(
            content_rating(Some("Suggestive"), false, Rating::Pornographic),
            ContentRating::Suggestive
        ));
        // Unrated titles take what the settings allow, at most Suggestive.
        assert!(matches!(
            content_rating(None, false, Rating::Safe),
            ContentRating::Safe
        ));
        assert!(matches!(
            content_rating(None, false, Rating::Pornographic),
            ContentRating::Suggestive
        ));
    }

    #[test]
    fn responses_are_checked_before_decoding() {
        assert!(matches!(
            decode::<ChapterListResponse>(404, b"{}"),
            Err(Error::NotFound(_))
        ));
        assert!(matches!(
            decode::<ChapterListResponse>(503, b"{\"chapters\":[]}"),
            Err(Error::Server(_))
        ));
        assert!(matches!(
            decode::<ChapterListResponse>(200, b"{}"),
            Err(Error::Host(_))
        ));
        assert!(matches!(
            decode::<ChapterListResponse>(200, b"<html>"),
            Err(Error::Host(_))
        ));
        let list = decode::<ChapterListResponse>(200, br#"{"chapters":[{"id":"c","number":0}]}"#)
            .expect("valid list");
        assert_eq!(list.chapters[0].number, Some(0.0));
    }

    #[test]
    fn chapters_follow_the_series_medium() {
        let item = ChapterItem {
            id: "c 2".into(),
            scanlation_manga_id: Some("g".into()),
            number: Some(10.5),
            title: Some(" ".into()),
            created_at: Some(1_700_000_000_000),
            page_count: None,
        };
        let groups = [Scanlator {
            id: "g".into(),
            name: "Group".into(),
            score: None,
        }];
        let novel = chapter(item.clone(), "s", &groups, true);
        assert_eq!(novel.kind, ContentKind::Text);
        assert_eq!(novel.url.as_deref(), Some("https://atsu.moe/novel/s/c%202"));
        assert_eq!(novel.chapter, Some(10.5));
        assert_eq!(novel.title, None);
        assert_eq!(novel.scanlator.as_deref(), Some("Group"));
        assert_eq!(novel.date_updated, Some(1_700_000_000.0));
        let comic = chapter(item, "s", &groups, false);
        assert_eq!(comic.kind, ContentKind::Images);
        assert_eq!(comic.url.as_deref(), Some("https://atsu.moe/read/s/c%202"));
    }

    #[test]
    fn filter_report_names_each_unknown_entry() {
        let genres = [Term {
            id: "g1".into(),
            name: "Action".into(),
        }];
        let fields: Vec<(String, Vec<String>, &[Term])> = vec![
            (
                "Required".into(),
                vec!["action".into(), "Actoin".into()],
                &genres,
            ),
            ("Excluded".into(), vec!["g1".into()], &genres),
        ];
        assert_eq!(
            filter_report(&fields),
            "Not found on the site — Required: Actoin"
        );
        let valid: Vec<(String, Vec<String>, &[Term])> =
            vec![("Required".into(), vec!["Action".into()], &genres)];
        assert_eq!(
            filter_report(&valid),
            "All 1 genre and tag names match the site."
        );
    }

    #[test]
    fn feed_entries_keep_their_medium() {
        let listed = |medium: &str| Listed {
            id: "s".into(),
            title: "T".into(),
            cover: None,
            medium: Some(medium.into()),
            origin: Some("Manga".into()),
            status: Some("Canceled".into()),
            authors: None,
            rating: ContentRating::Safe,
        };
        assert!(
            matches!(listed("Novel").into_entry(), MediaEntry::Novel(novel) if novel.url.as_deref() == Some("https://atsu.moe/novel/s"))
        );
        match listed("Comic").into_entry() {
            MediaEntry::Manga(manga) => {
                assert!(matches!(manga.viewer, Viewer::Rtl));
                assert!(matches!(manga.status, manga::Status::Cancelled));
            }
            _ => panic!("comic became another kind"),
        }
    }

    #[test]
    fn reading_mode_follows_origin() {
        assert!(matches!(viewer(Some("Manga")), Viewer::Rtl));
        for origin in [
            Some("Manwha"),
            Some("Manhua"),
            Some("OEL"),
            Some("Other"),
            None,
        ] {
            assert!(matches!(viewer(origin), Viewer::Webtoon), "{origin:?}");
        }
    }

    #[test]
    fn feeds_drop_entries_outside_the_content_settings() {
        let content = |rating: &str, adult_only: &str| {
            let values = [("maximum_rating", rating), ("adult_only", adult_only)];
            Preferences::parse(|key| {
                values
                    .iter()
                    .find(|(stored, _)| *stored == key)
                    .map(|(_, value)| (*value).to_owned())
            })
            .content
        };
        let safe = content("Safe", "false");
        assert!(feed_allows(Some(Rating::Safe), false, &safe));
        assert!(feed_allows(None, false, &safe));
        assert!(!feed_allows(Some(Rating::Suggestive), false, &safe));
        assert!(!feed_allows(Some(Rating::Safe), true, &safe));
        let explicit = content("Pornographic", "false");
        assert!(feed_allows(Some(Rating::Pornographic), true, &explicit));
        let adult_only = content("Pornographic", "true");
        assert!(!feed_allows(Some(Rating::Erotica), false, &adult_only));
        assert!(feed_allows(None, true, &adult_only));
    }

    #[test]
    fn titles_fall_back_to_english_then_id() {
        assert_eq!(
            display_title(" ".into(), Some("English".into()), "id"),
            "English"
        );
        assert_eq!(
            display_title(String::new(), Some(String::new()), "id"),
            "id"
        );
        assert_eq!(display_title("Title".into(), None, "id"), "Title");
    }
}
