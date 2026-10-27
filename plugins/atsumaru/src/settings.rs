//! Source settings: the schema shown by the host and the typed preferences parsed from it.
//!
//! Parsing is pure (it reads through a lookup function) so policy can be tested natively.
//! Settings are flat in the current host contract, so each name starts with the area it
//! applies to.

use ito_rs::models::{Setting, SettingIssue, SettingsSchema};

pub const INCLUDE_COMICS: &str = "include_comics";
pub const INCLUDE_NOVELS: &str = "include_novels";
pub const TYPES: &str = "types";
pub const MAX_RATING: &str = "maximum_rating";
pub const ADULT_ONLY: &str = "adult_only";
pub const EXCLUDE_TAGS: &str = "exclude_tags";
pub const SORT: &str = "search_sort";
pub const STATUS: &str = "search_status";
pub const YEAR_FROM: &str = "search_year_from";
pub const YEAR_TO: &str = "search_year_to";
pub const MIN_CHAPTERS: &str = "search_min_chapters";
pub const MAX_CHAPTERS: &str = "search_max_chapters";
pub const OFFICIAL_ONLY: &str = "search_official_only";
pub const INCLUDE_GENRES: &str = "search_include_genres";
pub const EXCLUDE_GENRES: &str = "search_exclude_genres";
pub const INCLUDE_TAGS: &str = "search_include_tags";
pub const POPULARITY_PERIOD: &str = "discover_popularity_period";
pub const BOOKMARK_PERIOD: &str = "discover_bookmark_period";
pub const TALKED_PERIOD: &str = "discover_talked_period";
pub const SPOTLIGHT_GENRE: &str = "discover_spotlight_genre";
pub const EXCLUDE_SCANLATORS: &str = "chapters_exclude_scanlators";
pub const PREFERRED_SCANLATORS: &str = "chapters_preferred_scanlators";
pub const ALTERNATIVES: &str = "chapters_alternatives";

/// Tag groups the site's taxonomy uses, each with its own exclusion field. Tag names are
/// unique across groups, so a name resolves wherever it is entered; tags from groups the site
/// adds later go in the general field.
pub const TAG_GROUPS: &[(&str, &str)] = &[
    ("exclude_tags.sexual_content", "Sexual Content"),
    ("exclude_tags.themes", "Themes"),
    ("exclude_tags.narrative_tropes", "Narrative Tropes"),
    ("exclude_tags.relationship", "Relationship"),
    ("exclude_tags.tone", "Tone"),
    ("exclude_tags.character_types", "Character Types"),
    ("exclude_tags.character_traits", "Character Traits"),
    ("exclude_tags.character_archetype", "Character Archetype"),
    ("exclude_tags.occupations", "Occupations"),
    ("exclude_tags.species_creatures", "Species & Creatures"),
    ("exclude_tags.activities", "Activities"),
    ("exclude_tags.settings", "Settings"),
    ("exclude_tags.locations", "Locations"),
    ("exclude_tags.time_period", "Time Period"),
    ("exclude_tags.world_building", "World Building"),
    ("exclude_tags.objects", "Objects"),
    (
        "exclude_tags.audience_demographics",
        "Audience Demographics",
    ),
    ("exclude_tags.derivative_work", "Derivative Work"),
    ("exclude_tags.work_info", "Work Info"),
];

/// Site content ratings, from mildest to most explicit.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Rating {
    Safe,
    Suggestive,
    Erotica,
    Pornographic,
}

impl Rating {
    pub const ALL: [Rating; 4] = [
        Rating::Safe,
        Rating::Suggestive,
        Rating::Erotica,
        Rating::Pornographic,
    ];

    /// The label shown in settings, which is also the site's value.
    pub fn name(self) -> &'static str {
        match self {
            Rating::Safe => "Safe",
            Rating::Suggestive => "Suggestive",
            Rating::Erotica => "Erotica",
            Rating::Pornographic => "Pornographic",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|rating| rating.name() == value)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sort {
    Popularity,
    Trending,
    Title,
    RecentlyAdded,
    ReleaseDate,
    TopRated,
}

impl Sort {
    const ALL: [Sort; 6] = [
        Sort::Popularity,
        Sort::Trending,
        Sort::Title,
        Sort::RecentlyAdded,
        Sort::ReleaseDate,
        Sort::TopRated,
    ];

    fn name(self) -> &'static str {
        match self {
            Sort::Popularity => "Popularity",
            Sort::Trending => "Trending",
            Sort::Title => "Title",
            Sort::RecentlyAdded => "Recently added",
            Sort::ReleaseDate => "Release date",
            Sort::TopRated => "Top rated",
        }
    }
}

/// Ranking window for the Discover sections that support one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Period {
    Daily,
    Weekly,
    Monthly,
    AllTime,
}

impl Period {
    const ALL: [Period; 4] = [
        Period::Daily,
        Period::Weekly,
        Period::Monthly,
        Period::AllTime,
    ];

    fn name(self) -> &'static str {
        match self {
            Period::Daily => "Daily",
            Period::Weekly => "Weekly",
            Period::Monthly => "Monthly",
            Period::AllTime => "All time",
        }
    }

    /// The site's `timeframe` value.
    pub fn api_value(self) -> &'static str {
        match self {
            Period::Daily => "daily",
            Period::Weekly => "weekly",
            Period::Monthly => "monthly",
            Period::AllTime => "all",
        }
    }
}

/// What to do when several groups translated the same chapter number.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Alternatives {
    ShowAll,
    HighestVoted,
    NewestUpload,
    MostPages,
}

impl Alternatives {
    const ALL: [Alternatives; 4] = [
        Alternatives::ShowAll,
        Alternatives::HighestVoted,
        Alternatives::NewestUpload,
        Alternatives::MostPages,
    ];

    fn name(self) -> &'static str {
        match self {
            Alternatives::ShowAll => "Show all",
            Alternatives::HighestVoted => "One per chapter: highest-voted group",
            Alternatives::NewestUpload => "One per chapter: newest upload",
            Alternatives::MostPages => "One per chapter: most pages",
        }
    }
}

const STATUSES: [&str; 4] = ["Ongoing", "Completed", "Hiatus", "Canceled"];
const ANY_STATUS: &str = "Any";
/// Labels accepted in the types field and the site value each one stands for.
const TYPE_NAMES: [(&str, &str); 6] = [
    ("manga", "Manga"),
    ("manhwa", "Manwha"),
    ("manwha", "Manwha"),
    ("manhua", "Manhua"),
    ("oel", "OEL"),
    ("other", "Other"),
];

/// Where a Discover section's entries come from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SectionSource {
    /// A ranked site feed, optionally limited to novels.
    Feed {
        endpoint: &'static str,
        novels_only: bool,
    },
    /// The catalog sorted by trend.
    Trending,
    /// The most popular titles in the configured genre.
    GenreSpotlight,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Section {
    pub id: &'static str,
    pub title: &'static str,
    pub enabled_by_default: bool,
    pub source: SectionSource,
    /// The setting holding this section's ranking window, with its default.
    pub period: Option<(&'static str, Period)>,
}

const fn feed(
    id: &'static str,
    title: &'static str,
    endpoint: &'static str,
    period: Option<(&'static str, Period)>,
) -> Section {
    Section {
        id,
        title,
        enabled_by_default: false,
        source: SectionSource::Feed {
            endpoint,
            novels_only: false,
        },
        period,
    }
}

/// Discover sections in display order. Ids double as listing ids.
pub const SECTIONS: &[Section] = &[
    Section {
        enabled_by_default: true,
        ..feed(
            "home_bookmarked",
            "Most Bookmarked",
            "mostBookmarked",
            Some((BOOKMARK_PERIOD, Period::Monthly)),
        )
    },
    Section {
        id: "home_trending",
        title: "Trending",
        enabled_by_default: true,
        source: SectionSource::Trending,
        period: None,
    },
    Section {
        enabled_by_default: true,
        ..feed("home_updated", "Recently Updated", "recentlyUpdated", None)
    },
    feed(
        "home_popular",
        "Popular",
        "popular",
        Some((POPULARITY_PERIOD, Period::Daily)),
    ),
    Section {
        id: "home_novels",
        title: "Popular Novels",
        enabled_by_default: false,
        source: SectionSource::Feed {
            endpoint: "popular",
            novels_only: true,
        },
        period: Some((POPULARITY_PERIOD, Period::Daily)),
    },
    Section {
        id: "home_spotlight",
        title: "Genre Spotlight",
        enabled_by_default: false,
        source: SectionSource::GenreSpotlight,
        period: None,
    },
    feed("home_rising", "Rising", "rising", None),
    feed("home_hot_updates", "Hot Updates", "hotUpdates", None),
    feed("home_hot_arrivals", "Hot Arrivals", "hotArrivals", None),
    feed(
        "home_recently_added",
        "Recently Added",
        "recentlyAdded",
        None,
    ),
    feed(
        "home_talked",
        "Most Talked About",
        "mostTalkedAbout",
        Some((TALKED_PERIOD, Period::Weekly)),
    ),
    feed("home_top_rated", "Top Rated", "topRated", None),
    feed("home_binge", "Binge-Worthy", "bingeWorthy", None),
    feed("home_hidden_gems", "Hidden Gems", "hiddenGems", None),
    feed("home_polarizing", "Most Polarizing", "mostPolarizing", None),
];

/// Listing ids published before sections were configurable.
pub fn section(listing_id: &str) -> Option<&'static Section> {
    let id = match listing_id {
        "mostBookmarked" => "home_bookmarked",
        "trending" => "home_trending",
        "recentlyUpdated" => "home_updated",
        other => other,
    };
    SECTIONS.iter().find(|section| section.id == id)
}

pub fn schema() -> SettingsSchema {
    let mut settings = vec![
        toggle(
            INCLUDE_COMICS,
            "Content · Comics",
            "Show comics in search and Discover.",
            true,
        ),
        toggle(
            INCLUDE_NOVELS,
            "Content · Novels",
            "Show novels in search and Discover.",
            true,
        ),
        text(
            TYPES,
            "Content · Origin types",
            "Comma-separated: Manga, Manhwa, Manhua, OEL, Other. Blank shows all.",
        ),
        picker(
            MAX_RATING,
            "Content · Maximum rating",
            "Titles rated above this are hidden. Titles the site has not rated are shown.",
            Rating::ALL.iter().map(|rating| rating.name()),
            Rating::Safe.name(),
        ),
        toggle(
            ADULT_ONLY,
            "Content · Adult titles only",
            "Show only titles the site marks adult. Requires the Pornographic maximum rating.",
            false,
        ),
        text(
            EXCLUDE_TAGS,
            "Blocked tags · Any group",
            "Comma-separated exact tag names or ids. Applies to search and Discover.",
        ),
    ];
    settings.extend(TAG_GROUPS.iter().map(|(id, group)| {
        text(
            id,
            &format!("Blocked tags · {group}"),
            &format!("Comma-separated exact {group} tag names."),
        )
    }));
    settings.extend([
        picker(
            SORT,
            "Search · Sort when browsing",
            "Order of results without a search term. Searches rank by relevance.",
            Sort::ALL.iter().map(|sort| sort.name()),
            Sort::Popularity.name(),
        ),
        picker(
            STATUS,
            "Search · Status",
            "Only show titles with this publication status.",
            std::iter::once(ANY_STATUS).chain(STATUSES),
            ANY_STATUS,
        ),
        text(
            YEAR_FROM,
            "Search · Released from year",
            "Optional, for example 2015.",
        ),
        text(
            YEAR_TO,
            "Search · Released to year",
            "Optional, for example 2024.",
        ),
        text(
            MIN_CHAPTERS,
            "Search · Minimum chapters",
            "Optional whole number.",
        ),
        text(
            MAX_CHAPTERS,
            "Search · Maximum chapters",
            "Optional whole number.",
        ),
        toggle(
            OFFICIAL_ONLY,
            "Search · Official translations only",
            "Only show titles with an official translation.",
            false,
        ),
        text(
            INCLUDE_GENRES,
            "Search · Required genres",
            "Comma-separated exact genre names; titles must have all of them.",
        ),
        text(
            EXCLUDE_GENRES,
            "Search · Excluded genres",
            "Comma-separated exact genre names.",
        ),
        text(
            INCLUDE_TAGS,
            "Search · Required tags",
            "Comma-separated exact tag names; titles must have all of them.",
        ),
    ]);
    for section in SECTIONS {
        settings.push(toggle(
            section.id,
            &format!("Discover · {}", section.title),
            match section.source {
                SectionSource::GenreSpotlight => "Shown when a spotlight genre is set below.",
                _ => "Show this section on the source's home page.",
            },
            section.enabled_by_default,
        ));
    }
    settings.extend([
        period_picker(POPULARITY_PERIOD, "Popular", Period::Daily),
        period_picker(BOOKMARK_PERIOD, "Most Bookmarked", Period::Monthly),
        period_picker(TALKED_PERIOD, "Most Talked About", Period::Weekly),
        text(
            SPOTLIGHT_GENRE,
            "Discover · Spotlight genre",
            "Exact genre name for the Genre Spotlight section.",
        ),
        text(
            EXCLUDE_SCANLATORS,
            "Chapters · Hidden scanlators",
            "Comma-separated scanlator names; their chapters are not listed.",
        ),
        picker(
            ALTERNATIVES,
            "Chapters · Alternative versions",
            "When several groups translated the same chapter number. Saved progress is kept.",
            Alternatives::ALL.iter().map(|mode| mode.name()),
            Alternatives::ShowAll.name(),
        ),
        text(
            PREFERRED_SCANLATORS,
            "Chapters · Preferred scanlators",
            "Comma-separated, most preferred first. Used when showing one version per chapter.",
        ),
    ]);
    SettingsSchema { settings }
}

fn toggle(id: &str, name: &str, summary: &str, default_value: bool) -> Setting {
    Setting::Toggle {
        id: id.into(),
        name: name.into(),
        summary: Some(summary.into()),
        default_value,
    }
}

fn text(id: &str, name: &str, summary: &str) -> Setting {
    Setting::Text {
        id: id.into(),
        name: name.into(),
        summary: Some(summary.into()),
        default_value: String::new(),
    }
}

fn picker<'a>(
    id: &str,
    name: &str,
    summary: &str,
    options: impl Iterator<Item = &'a str>,
    default_value: &str,
) -> Setting {
    Setting::Picker {
        id: id.into(),
        name: name.into(),
        summary: Some(summary.into()),
        options: options.map(str::to_owned).collect(),
        default_value: default_value.into(),
    }
}

fn period_picker(id: &str, section: &str, default: Period) -> Setting {
    picker(
        id,
        &format!("Discover · {section} period"),
        "Ranking window for this section and its full list.",
        Period::ALL.iter().map(|period| period.name()),
        default.name(),
    )
}

/// Preferences that narrow results everywhere.
#[derive(Clone, Debug, PartialEq)]
pub struct ContentPreferences {
    pub include_comics: bool,
    pub include_novels: bool,
    /// Site type values, deduplicated.
    pub types: Vec<&'static str>,
    pub max_rating: Rating,
    pub adult_only: bool,
    /// Blocked tag names or ids from every blocked-tags field.
    pub excluded_tags: Vec<String>,
}

/// Preferences applied only to search results.
#[derive(Clone, Debug, PartialEq)]
pub struct SearchPreferences {
    pub sort: Sort,
    pub status: Option<&'static str>,
    pub year_from: Option<u16>,
    pub year_to: Option<u16>,
    pub min_chapters: Option<u32>,
    pub max_chapters: Option<u32>,
    pub official_only: bool,
    pub include_genres: Vec<String>,
    pub exclude_genres: Vec<String>,
    pub include_tags: Vec<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ChapterPreferences {
    pub hidden_scanlators: Vec<String>,
    pub preferred_scanlators: Vec<String>,
    pub alternatives: Alternatives,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Preferences {
    pub content: ContentPreferences,
    pub search: SearchPreferences,
    pub chapters: ChapterPreferences,
    pub spotlight_genre: Option<String>,
    enabled_sections: Vec<&'static str>,
    periods: Vec<(&'static str, Period)>,
    issues: Vec<SettingIssue>,
}

impl Preferences {
    /// Reads the saved settings. Absent values use the schema defaults.
    pub fn load() -> Self {
        Self::parse(|key| ito_rs::defaults::get(key).ok().flatten())
    }

    pub fn parse(get: impl Fn(&str) -> Option<String>) -> Self {
        let mut parser = Parser {
            get,
            issues: Vec::new(),
        };
        let mut excluded_tags = parser.list(EXCLUDE_TAGS);
        for (id, _) in TAG_GROUPS {
            excluded_tags.extend(parser.list(id));
        }
        dedupe_case_insensitive(&mut excluded_tags);

        let content = ContentPreferences {
            include_comics: parser.toggle(INCLUDE_COMICS, true),
            include_novels: parser.toggle(INCLUDE_NOVELS, true),
            types: parser.types(),
            max_rating: parser.choice(MAX_RATING, &Rating::ALL, Rating::name, Rating::Safe),
            adult_only: parser.toggle(ADULT_ONLY, false),
            excluded_tags,
        };
        let status = parser.choice(
            STATUS,
            &[
                Some("Ongoing"),
                Some("Completed"),
                Some("Hiatus"),
                Some("Canceled"),
                None,
            ],
            |status| status.unwrap_or(ANY_STATUS),
            None,
        );
        let search = SearchPreferences {
            sort: parser.choice(SORT, &Sort::ALL, Sort::name, Sort::Popularity),
            status,
            year_from: parser.number(YEAR_FROM, "Enter a year such as 2015, or leave it blank."),
            year_to: parser.number(YEAR_TO, "Enter a year such as 2024, or leave it blank."),
            min_chapters: parser.number(MIN_CHAPTERS, "Enter a whole number, or leave it blank."),
            max_chapters: parser.number(MAX_CHAPTERS, "Enter a whole number, or leave it blank."),
            official_only: parser.toggle(OFFICIAL_ONLY, false),
            include_genres: parser.list(INCLUDE_GENRES),
            exclude_genres: parser.list(EXCLUDE_GENRES),
            include_tags: parser.list(INCLUDE_TAGS),
        };
        let chapters = ChapterPreferences {
            hidden_scanlators: parser.list(EXCLUDE_SCANLATORS),
            preferred_scanlators: parser.list(PREFERRED_SCANLATORS),
            alternatives: parser.choice(
                ALTERNATIVES,
                &Alternatives::ALL,
                Alternatives::name,
                Alternatives::ShowAll,
            ),
        };
        let enabled_sections = SECTIONS
            .iter()
            .filter(|section| parser.toggle(section.id, section.enabled_by_default))
            .map(|section| section.id)
            .collect();
        let periods = [
            (POPULARITY_PERIOD, Period::Daily),
            (BOOKMARK_PERIOD, Period::Monthly),
            (TALKED_PERIOD, Period::Weekly),
        ]
        .into_iter()
        .map(|(id, default)| (id, parser.choice(id, &Period::ALL, Period::name, default)))
        .collect();
        let spotlight_genre = parser.list(SPOTLIGHT_GENRE);
        if spotlight_genre.len() > 1 {
            parser.issue(SPOTLIGHT_GENRE, "Enter one genre.");
        }

        let mut preferences = Preferences {
            content,
            search,
            chapters,
            spotlight_genre: spotlight_genre.into_iter().next(),
            enabled_sections,
            periods,
            issues: parser.issues,
        };
        preferences.check_combinations();
        preferences
    }

    fn check_combinations(&mut self) {
        let content = &self.content;
        let search = &self.search;
        let mut issues = Vec::new();
        if !content.include_comics && !content.include_novels {
            issues.push(issue(INCLUDE_NOVELS, "Turn on comics, novels or both."));
        }
        if content.adult_only && content.max_rating != Rating::Pornographic {
            issues.push(issue(
                ADULT_ONLY,
                "Adult titles only needs the maximum rating set to Pornographic.",
            ));
        }
        if let (Some(from), Some(to)) = (search.year_from, search.year_to)
            && from > to
        {
            issues.push(issue(YEAR_TO, "The end year is before the start year."));
        }
        if let (Some(min), Some(max)) = (search.min_chapters, search.max_chapters)
            && min > max
        {
            issues.push(issue(
                MAX_CHAPTERS,
                "The maximum chapter count is below the minimum.",
            ));
        }
        if overlaps(&search.include_genres, &search.exclude_genres) {
            issues.push(issue(
                EXCLUDE_GENRES,
                "A genre is both required and excluded.",
            ));
        }
        if overlaps(&search.include_tags, &content.excluded_tags) {
            issues.push(issue(INCLUDE_TAGS, "A required tag is also blocked."));
        }
        self.issues.extend(issues);
    }

    /// Problems found without contacting the site, keyed by setting id.
    pub fn issues(&self) -> &[SettingIssue] {
        &self.issues
    }

    pub fn section_enabled(&self, section: &Section) -> bool {
        self.enabled_sections.contains(&section.id)
    }

    /// The ranking window a section uses, if it has one.
    pub fn period(&self, section: &Section) -> Option<Period> {
        let (id, default) = section.period?;
        Some(
            self.periods
                .iter()
                .find(|(key, _)| *key == id)
                .map_or(default, |(_, period)| *period),
        )
    }
}

struct Parser<F> {
    get: F,
    issues: Vec<SettingIssue>,
}

impl<F: Fn(&str) -> Option<String>> Parser<F> {
    fn raw(&self, key: &str) -> Option<String> {
        (self.get)(key)
    }

    fn issue(&mut self, key: &str, message: &str) {
        self.issues.push(issue(key, message));
    }

    fn toggle(&mut self, key: &str, default: bool) -> bool {
        match self.raw(key).as_deref().map(str::trim) {
            None | Some("") => default,
            Some("true" | "1") => true,
            Some("false" | "0") => false,
            Some(_) => {
                self.issue(key, "Unrecognised on/off value.");
                default
            }
        }
    }

    fn choice<T: Copy>(
        &mut self,
        key: &str,
        options: &[T],
        name: impl Fn(T) -> &'static str,
        default: T,
    ) -> T {
        let Some(value) = self.raw(key) else {
            return default;
        };
        match options
            .iter()
            .copied()
            .find(|option| name(*option) == value)
        {
            Some(option) => option,
            None => {
                self.issue(key, "Choose one of the listed options.");
                default
            }
        }
    }

    fn list(&mut self, key: &str) -> Vec<String> {
        let mut values = split_list(&self.raw(key).unwrap_or_default());
        dedupe_case_insensitive(&mut values);
        values
    }

    fn number<T: std::str::FromStr>(&mut self, key: &str, message: &str) -> Option<T> {
        let raw = self.raw(key).unwrap_or_default();
        let raw = raw.trim();
        if raw.is_empty() {
            return None;
        }
        let parsed = raw.parse().ok();
        if parsed.is_none() {
            self.issue(key, message);
        }
        parsed
    }

    fn types(&mut self) -> Vec<&'static str> {
        let mut types = Vec::new();
        for name in self.list(TYPES) {
            let lower = name.to_lowercase();
            match TYPE_NAMES.iter().find(|(label, _)| *label == lower) {
                Some((_, value)) if !types.contains(value) => types.push(*value),
                Some(_) => {}
                None => self.issue(TYPES, "Use Manga, Manhwa, Manhua, OEL or Other."),
            }
        }
        types
    }
}

/// Splits a comma-separated field. Site genre, tag and type names contain no commas.
pub fn split_list(raw: &str) -> Vec<String> {
    raw.split(',')
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .collect()
}

fn dedupe_case_insensitive(values: &mut Vec<String>) {
    let mut seen = Vec::<String>::with_capacity(values.len());
    values.retain(|value| {
        let lower = value.to_lowercase();
        let fresh = !seen.contains(&lower);
        if fresh {
            seen.push(lower);
        }
        fresh
    });
}

fn overlaps(left: &[String], right: &[String]) -> bool {
    left.iter()
        .any(|a| right.iter().any(|b| a.eq_ignore_ascii_case(b)))
}

fn issue(key: &str, message: &str) -> SettingIssue {
    SettingIssue {
        key: key.into(),
        message: message.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn parse(values: &[(&str, &str)]) -> Preferences {
        let map: HashMap<String, String> = values
            .iter()
            .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
            .collect();
        Preferences::parse(|key| map.get(key).cloned())
    }

    fn issue_keys(preferences: &Preferences) -> Vec<&str> {
        preferences
            .issues()
            .iter()
            .map(|issue| issue.key.as_str())
            .collect()
    }

    #[test]
    fn absent_settings_use_conservative_defaults() {
        let preferences = parse(&[]);
        assert!(preferences.issues().is_empty());
        assert_eq!(preferences.content.max_rating, Rating::Safe);
        assert!(!preferences.content.adult_only);
        assert_eq!(preferences.chapters.alternatives, Alternatives::ShowAll);
        let enabled: Vec<&str> = SECTIONS
            .iter()
            .filter(|section| preferences.section_enabled(section))
            .map(|section| section.id)
            .collect();
        assert_eq!(
            enabled,
            ["home_bookmarked", "home_trending", "home_updated"]
        );
    }

    #[test]
    fn adult_only_requires_the_most_explicit_rating() {
        let blocked = parse(&[(ADULT_ONLY, "true"), (MAX_RATING, "Erotica")]);
        assert_eq!(issue_keys(&blocked), [ADULT_ONLY]);
        let allowed = parse(&[(ADULT_ONLY, "true"), (MAX_RATING, "Pornographic")]);
        assert!(allowed.issues().is_empty());
    }

    #[test]
    fn invalid_and_inverted_ranges_are_reported_not_guessed() {
        let preferences = parse(&[
            (YEAR_FROM, "2020"),
            (YEAR_TO, "2010"),
            (MIN_CHAPTERS, "ten"),
            (MAX_CHAPTERS, "-3"),
        ]);
        assert_eq!(preferences.search.min_chapters, None);
        assert_eq!(preferences.search.max_chapters, None);
        assert_eq!(
            issue_keys(&preferences),
            [MIN_CHAPTERS, MAX_CHAPTERS, YEAR_TO]
        );
    }

    #[test]
    fn types_accept_friendly_spelling_and_reject_unknown_names() {
        let preferences = parse(&[(TYPES, "manhwa, Manga, Manwha, comic")]);
        assert_eq!(preferences.content.types, ["Manwha", "Manga"]);
        assert_eq!(issue_keys(&preferences), [TYPES]);
    }

    #[test]
    fn blocked_tags_merge_every_group_without_duplicates() {
        let preferences = parse(&[
            (EXCLUDE_TAGS, "Gore, gore"),
            ("exclude_tags.themes", " Revenge ,GORE"),
            ("exclude_tags.tone", "Dark"),
        ]);
        assert_eq!(
            preferences.content.excluded_tags,
            ["Gore", "Revenge", "Dark"]
        );
    }

    #[test]
    fn conflicting_filters_and_unknown_choices_are_reported() {
        let preferences = parse(&[
            (INCLUDE_COMICS, "false"),
            (INCLUDE_NOVELS, "false"),
            (INCLUDE_GENRES, "Action"),
            (EXCLUDE_GENRES, "action"),
            (SORT, "Newest"),
            (SPOTLIGHT_GENRE, "Action, Drama"),
        ]);
        assert_eq!(preferences.search.sort, Sort::Popularity);
        assert_eq!(
            issue_keys(&preferences),
            [SORT, SPOTLIGHT_GENRE, INCLUDE_NOVELS, EXCLUDE_GENRES]
        );
    }

    #[test]
    fn periods_follow_their_own_setting() {
        let preferences = parse(&[(POPULARITY_PERIOD, "Weekly")]);
        let popular = section("home_popular").expect("popular section");
        let novels = section("home_novels").expect("novel section");
        let bookmarked = section("mostBookmarked").expect("legacy listing id");
        assert_eq!(preferences.period(popular), Some(Period::Weekly));
        assert_eq!(preferences.period(novels), Some(Period::Weekly));
        assert_eq!(preferences.period(bookmarked), Some(Period::Monthly));
        assert_eq!(
            preferences.period(section("home_rising").expect("rising")),
            None
        );
    }

    #[test]
    fn every_setting_id_is_unique() {
        let schema = schema();
        let mut ids: Vec<String> = schema
            .settings
            .iter()
            .map(|setting| match setting {
                Setting::Toggle { id, .. }
                | Setting::Text { id, .. }
                | Setting::Picker { id, .. }
                | Setting::Secret { id, .. } => id.clone(),
                _ => String::new(),
            })
            .collect();
        let count = ids.len();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), count);
    }
}
