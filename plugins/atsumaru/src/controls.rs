//! Rich settings, structured search and cached taxonomy option pages.
//!
//! Origin types migrate locally once; legacy names remain readable until the corresponding
//! identifier key exists, including an explicitly empty value. Catalog resolution alone
//! fetches taxonomy, and shares the operation-time cache with legacy name resolution.

use crate::models::Taxonomy;
use crate::settings::{self, Preferences, SearchPreferences, Sort};
use ito_rs::controls::{
    ChoiceOption, Control, ControlKind, ControlSection, ControlValue, ControlValues, OptionPage,
    OptionQuery, OptionRequest, OptionSource, SearchCapabilities, SettingsForm,
};
use ito_rs::models::{FilterItem, MediaKind, Setting};
use ito_rs::{Error, Result};
use std::cell::Cell;

thread_local! {
    // Negotiation is scoped to this plugin instance, never persisted into old-host settings.
    static RICH_HOST: Cell<bool> = const { Cell::new(false) };
}

pub fn negotiate_rich_host() {
    RICH_HOST.set(true);
}

pub fn is_rich_host() -> bool {
    RICH_HOST.get()
}

pub const CONTENT_TYPES: &str = "content_types";
pub const BLOCKED_TAG_IDS: &str = "blocked_tag_ids";
pub const SPOTLIGHT_GENRE_ID: &str = "discover_spotlight_genre_id";

fn inline(pairs: &[(&str, &str)]) -> OptionSource {
    OptionSource::Inline(
        pairs
            .iter()
            .map(|(value, label)| ChoiceOption::new(*value, *label))
            .collect(),
    )
}

pub fn form() -> SettingsForm {
    let mut content = ControlSection::new("content", "Content");
    let mut discover = ControlSection::new("discover", "Discover");
    discover.flags = ControlSection::PAGE;
    let mut chapters = ControlSection::new("chapters", "Chapters");
    // Reuse the legacy declarations so reused keys retain exactly the same defaults.
    for setting in settings::schema().settings {
        let control = match setting {
            Setting::Toggle {
                id,
                name,
                summary,
                default_value,
            } => Control::toggle(id, name, default_value).summary(summary.unwrap_or_default()),
            Setting::Text {
                id,
                name,
                summary,
                default_value,
            } => Control::text(id, name, default_value).summary(summary.unwrap_or_default()),
            Setting::Picker {
                id,
                name,
                summary,
                options,
                default_value,
            } => Control::select(
                id,
                name,
                OptionSource::Inline(
                    options
                        .into_iter()
                        .map(|value| ChoiceOption::new(value.clone(), value))
                        .collect(),
                ),
                Some(default_value),
            )
            .summary(summary.unwrap_or_default()),
            _ => continue,
        };
        let mut control = control;
        if let Some((_, title)) = control.title.split_once(" · ") {
            control.title = title.to_owned();
        }
        match control.id.as_str() {
            settings::TYPES => {
                content.controls.push(
                    Control::multi_select(
                        CONTENT_TYPES,
                        "Origin types",
                        inline(&[
                            ("Manga", "Manga"),
                            ("Manwha", "Manhwa"),
                            ("Manhua", "Manhua"),
                            ("OEL", "OEL"),
                            ("Other", "Other"),
                        ]),
                        Vec::new(),
                    )
                    .summary("Empty shows all origin types."),
                );
            }
            settings::INCLUDE_COMICS | settings::INCLUDE_NOVELS | settings::MAX_RATING => {
                content.controls.push(control)
            }
            settings::ADULT_ONLY => content
                .controls
                .push(control.visible_when(settings::MAX_RATING, vec!["Pornographic".into()])),
            settings::SPOTLIGHT_GENRE => discover.controls.push(
                Control::select(
                    SPOTLIGHT_GENRE_ID,
                    "Spotlight genre",
                    OptionSource::searchable_catalog("genres"),
                    None,
                )
                .visible_when("home_spotlight", vec!["true".into()]),
            ),
            // Popular and Novels share this window; neither section alone controls it.
            settings::POPULARITY_PERIOD => discover.controls.push(control),
            settings::BOOKMARK_PERIOD | settings::TALKED_PERIOD => {
                let target = if control.id == settings::BOOKMARK_PERIOD {
                    "home_bookmarked"
                } else {
                    "home_talked"
                };
                discover
                    .controls
                    .push(control.visible_when(target, vec!["true".into()]));
            }
            settings::EXCLUDE_SCANLATORS
            | settings::PREFERRED_SCANLATORS
            | settings::ALTERNATIVES => chapters.controls.push(control),
            id if id.starts_with("home_") => discover.controls.push(control),
            _ => {}
        }
    }
    let mut blocked = Control::multi_select(
        BLOCKED_TAG_IDS,
        "Blocked tags",
        OptionSource::searchable_catalog("tags"),
        Vec::new(),
    )
    .summary("Applies to search and Discover. Editing replaces legacy blocked tag names.");
    if let ControlKind::MultiSelect(value) = &mut blocked.kind {
        value.max_selected = Some(500);
    }
    let mut blocked_section = ControlSection::new("blocked", "Blocked tags").control(blocked);
    blocked_section.flags = ControlSection::PAGE;
    SettingsForm::new(vec![content, blocked_section, discover, chapters])
}

/// Local, idempotent migration. Unknown legacy types are left for legacy validation.
pub fn type_migration(get: impl Fn(&str) -> Option<String>) -> Option<String> {
    if get(CONTENT_TYPES).is_some() {
        return None;
    }
    let parsed = Preferences::parse(&get);
    if parsed
        .issues()
        .iter()
        .any(|issue| issue.key == settings::TYPES)
    {
        return None;
    }
    Some(
        ControlValue::MultiSelect(
            parsed
                .content
                .types
                .iter()
                .map(|value| (*value).to_owned())
                .collect(),
        )
        .canonical(),
    )
}

pub fn settings_form() -> SettingsForm {
    negotiate_rich_host();
    if let Some(value) = type_migration(|key| ito_rs::defaults::get(key).ok().flatten()) {
        ito_rs::defaults::set(CONTENT_TYPES, &value);
    }
    form()
}

pub fn capabilities(kind: Option<MediaKind>) -> Result<SearchCapabilities> {
    if !matches!(kind, None | Some(MediaKind::Manga | MediaKind::Novel)) {
        return Err(Error::Unsupported);
    }
    let mut chapters = Control::integer_range("chapter_count", "Chapter count", 0, 100_000);
    if let ControlKind::IntegerRange(range) = &mut chapters.kind {
        range.unit = Some("chapters".into());
    }
    Ok(SearchCapabilities {
        flags: SearchCapabilities::BROWSE | SearchCapabilities::FILTERS_WITH_QUERY,
        sort: Some(Control::select(
            "sort",
            "Sort",
            inline(&[
                ("popularity", "Popularity"),
                ("trending", "Trending"),
                ("title", "Title"),
                ("recent", "Recently added"),
                ("release", "Release date"),
                ("rating", "Top rated"),
            ]),
            Some("popularity".into()),
        )),
        sections: vec![
            ControlSection::new("filters", "Filters")
                .control(Control::select(
                    "status",
                    "Status",
                    inline(&[
                        ("ongoing", "Ongoing"),
                        ("completed", "Completed"),
                        ("hiatus", "Hiatus"),
                        ("canceled", "Canceled"),
                    ]),
                    None,
                ))
                .control(Control::integer_range(
                    "release_year",
                    "Release year",
                    1900,
                    2100,
                ))
                .control(chapters)
                .control(Control::toggle(
                    "official_only",
                    "Official translations only",
                    false,
                ))
                .control(Control::multi_select(
                    "genres_all",
                    "Required genres",
                    OptionSource::searchable_catalog("genres"),
                    Vec::new(),
                ))
                .control(Control::multi_select(
                    "genres_none",
                    "Excluded genres",
                    OptionSource::searchable_catalog("genres"),
                    Vec::new(),
                ))
                .control(Control::multi_select(
                    "tags_all",
                    "Required tags",
                    OptionSource::searchable_catalog("tags"),
                    Vec::new(),
                )),
        ],
    })
}

/// Only an unnegotiated host's empty filters retain legacy search defaults and validation.
pub fn search_preferences(
    get: impl Fn(&str) -> Option<String>,
    filters: &[FilterItem],
    kind: Option<MediaKind>,
) -> Result<Preferences> {
    let values = ControlValues::search(&capabilities(kind)?, filters)?;
    let mut preferences = Preferences::parse(|key| {
        if (values.is_host_provided() || is_rich_host()) && key.starts_with("search_") {
            None
        } else {
            get(key)
        }
    });
    if values.is_host_provided() {
        let (year_from, year_to) = values.integer_range("release_year")?;
        let (min_chapters, max_chapters) = values.integer_range("chapter_count")?;
        preferences.search = SearchPreferences {
            sort: match values.select("sort")? {
                Some("trending") => Sort::Trending,
                Some("title") => Sort::Title,
                Some("recent") => Sort::RecentlyAdded,
                Some("release") => Sort::ReleaseDate,
                Some("rating") => Sort::TopRated,
                _ => Sort::Popularity,
            },
            status: match values.select("status")? {
                Some("ongoing") => Some("Ongoing"),
                Some("completed") => Some("Completed"),
                Some("hiatus") => Some("Hiatus"),
                Some("canceled") => Some("Canceled"),
                _ => None,
            },
            year_from: year_from.map(|value| value as u16),
            year_to: year_to.map(|value| value as u16),
            min_chapters: min_chapters.map(|value| value as u32),
            max_chapters: max_chapters.map(|value| value as u32),
            official_only: values.toggle("official_only")?,
            include_genres: values.multi_select("genres_all")?.to_vec(),
            exclude_genres: values.multi_select("genres_none")?.to_vec(),
            include_tags: values.multi_select("tags_all")?.to_vec(),
        };
        if preferences
            .search
            .include_genres
            .iter()
            .any(|id| preferences.search.exclude_genres.contains(id))
            || preferences
                .search
                .include_tags
                .iter()
                .any(|id| preferences.content.excluded_tags.contains(id))
        {
            return Err(Error::NeedsSetup(
                "A required genre or tag is also excluded.".into(),
            ));
        }
    }
    if let Some(issue) = preferences.issues().first() {
        return Err(Error::NeedsSetup(issue.message.clone()));
    }
    Ok(preferences)
}

pub fn option_page(taxonomy: &Taxonomy, request: OptionRequest) -> Result<OptionPage> {
    let terms = match request.catalog.as_str() {
        "tags" => &taxonomy.tags,
        "genres" => &taxonomy.genres,
        _ => return Err(Error::Unsupported),
    };
    let mut options: Vec<ChoiceOption> = terms
        .iter()
        .map(|term| ChoiceOption {
            value: term.id.clone(),
            label: term.name.clone(),
            group: term.group.clone(),
        })
        .collect();
    options.sort_by_cached_key(|option| {
        (
            option.group.as_deref().unwrap_or("").to_lowercase(),
            option.label.to_lowercase(),
            option.value.clone(),
        )
    });
    // Explicit byte hashing is deterministic across platforms and Rust versions.
    let mut hash = 0xcbf29ce484222325_u64;
    for option in &options {
        for value in [
            option.value.as_str(),
            option.label.as_str(),
            option.group.as_deref().unwrap_or(""),
        ] {
            for byte in value.bytes().chain(std::iter::once(0)) {
                hash = (hash ^ u64::from(byte)).wrapping_mul(0x100000001b3);
            }
        }
    }
    let revision = Some(format!("{:016x}", hash));
    match request.query {
        OptionQuery::Values(values) => {
            options.retain(|option| values.contains(&option.value));
            Ok(OptionPage {
                total: Some(options.len() as u32),
                options,
                next_offset: None,
                revision,
            })
        }
        OptionQuery::Search { text, offset } => {
            let text = text.trim().to_lowercase();
            options.retain(|option| option.label.to_lowercase().contains(&text));
            let total = options.len() as u32;
            let limit = request.limit.clamp(1, 100);
            let options: Vec<_> = options
                .into_iter()
                .skip(offset as usize)
                .take(limit as usize)
                .collect();
            let end = offset.saturating_add(options.len() as u32);
            Ok(OptionPage {
                options,
                next_offset: (end < total).then_some(end),
                total: Some(total),
                revision,
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ito_rs::models::FilterValue;

    fn taxonomy() -> Taxonomy {
        serde_json::from_str(r#"{"genres":[{"id":"g1","name":"Action"}],"tags":[{"id":"t2","name":"Blue","group":"Themes"},{"id":"t1","name":"Alpha","group":"Characters"},{"id":"t3","name":"BLUE sky","group":"Themes"}]}"#).unwrap()
    }
    fn request(query: OptionQuery, limit: u32) -> OptionRequest {
        OptionRequest {
            catalog: "tags".into(),
            query,
            limit,
        }
    }
    #[test]
    fn catalog_pages_are_grouped_searchable_and_resolve_stable_values() {
        let taxonomy = taxonomy();
        let first = option_page(
            &taxonomy,
            request(
                OptionQuery::Search {
                    text: "".into(),
                    offset: 0,
                },
                1,
            ),
        )
        .unwrap();
        assert_eq!(first.options[0].value, "t1");
        assert_eq!(first.options[0].group.as_deref(), Some("Characters"));
        assert_eq!(first.next_offset, Some(1));
        let rest = option_page(
            &taxonomy,
            request(
                OptionQuery::Search {
                    text: "".into(),
                    offset: 1,
                },
                1000,
            ),
        )
        .unwrap();
        assert_eq!(rest.options.len(), 2);
        assert_eq!(rest.next_offset, None);
        assert_eq!(first.revision, rest.revision);
        let matching = option_page(
            &taxonomy,
            request(
                OptionQuery::Search {
                    text: "bLuE".into(),
                    offset: 0,
                },
                100,
            ),
        )
        .unwrap();
        assert_eq!(matching.total, Some(2));
        let labels = option_page(
            &taxonomy,
            request(OptionQuery::Values(vec!["t2".into(), "missing".into()]), 1),
        )
        .unwrap();
        assert_eq!(labels.options[0].label, "Blue");
        assert_eq!(labels.options[0].value, "t2");
        assert_eq!(labels.revision, first.revision);
        let beyond = option_page(
            &taxonomy,
            request(
                OptionQuery::Search {
                    text: "".into(),
                    offset: 999,
                },
                100,
            ),
        )
        .unwrap();
        assert!(beyond.options.is_empty());
        assert_eq!(beyond.next_offset, None);
    }
    #[test]
    fn catalog_revision_tracks_labels_not_response_order() {
        let mut taxonomy = taxonomy();
        let query = || {
            request(
                OptionQuery::Search {
                    text: "".into(),
                    offset: 0,
                },
                100,
            )
        };
        let before = option_page(&taxonomy, query()).unwrap();
        taxonomy.tags.reverse();
        assert_eq!(before, option_page(&taxonomy, query()).unwrap());
        taxonomy.tags[0].name.push('!');
        assert_ne!(
            before.revision,
            option_page(&taxonomy, query()).unwrap().revision
        );
    }
    #[test]
    fn names_and_ids_resolve_compatibly_without_changing_identity() {
        let taxonomy = taxonomy();
        assert_eq!(
            crate::query::resolve(&["blue".into(), "t2".into()], &taxonomy.tags).unwrap(),
            vec!["t2"]
        );
        assert!(crate::query::resolve(&["missing".into()], &taxonomy.tags).is_err());
        assert!(taxonomy.genres[0].group.is_none());
    }

    #[test]
    fn rich_required_tag_id_conflicts_with_unedited_legacy_blocked_name() {
        let filters = [FilterItem {
            name: "tags_all".into(),
            type_name: "multi_select".into(),
            value: FilterValue::String("[\"t2\"]".into()),
        }];
        let parsed = search_preferences(
            |key| (key == settings::EXCLUDE_TAGS).then(|| "Blue".into()),
            &filters,
            None,
        )
        .unwrap();
        let taxonomy = taxonomy();
        let blocked = crate::query::resolve(&parsed.content.excluded_tags, &taxonomy.tags).unwrap();
        let required = crate::query::SearchIds {
            include_tags: crate::query::resolve(&parsed.search.include_tags, &taxonomy.tags)
                .unwrap(),
            ..Default::default()
        };
        assert!(matches!(
            crate::query::validate_search_ids(&required, &blocked),
            Err(Error::NeedsSetup(_))
        ));
        assert_eq!(parsed.content.excluded_tags, vec!["Blue"]);
    }
    #[test]
    fn local_type_migration_is_canonical_idempotent_and_preserves_invalid_legacy() {
        assert_eq!(
            type_migration(|key| (key == settings::TYPES).then(|| "Manhwa,manga,Manwha".into())),
            Some("[\"Manwha\",\"Manga\"]".into())
        );
        assert_eq!(
            type_migration(|key| (key == CONTENT_TYPES).then(|| "[]".into())),
            None
        );
        assert_eq!(
            type_migration(|key| (key == settings::TYPES).then(|| "unknown".into())),
            None
        );
    }
    #[test]
    fn dense_filters_override_legacy_search_but_empty_filters_preserve_it() {
        let get = |key: &str| match key {
            settings::SORT => Some("Trending".into()),
            settings::INCLUDE_GENRES => Some("Action".into()),
            settings::YEAR_FROM => Some("not a year".into()),
            _ => None,
        };
        assert!(search_preferences(get, &[], None).is_err());
        let dense = vec![FilterItem {
            name: "official_only".into(),
            type_name: "toggle".into(),
            value: FilterValue::String("true".into()),
        }];
        let rich = search_preferences(get, &dense, None).unwrap();
        assert_eq!(rich.search.sort, Sort::Popularity);
        assert!(rich.search.include_genres.is_empty());
        assert!(rich.search.official_only);
        let legacy = search_preferences(
            |key| (key == settings::SORT).then(|| "Trending".into()),
            &[],
            None,
        )
        .unwrap();
        assert_eq!(legacy.search.sort, Sort::Trending);
    }
    #[test]
    fn structured_ranges_reach_query_filters() {
        let filters = vec![
            FilterItem {
                name: "release_year".into(),
                type_name: "integer_range".into(),
                value: FilterValue::String("[2000,2020]".into()),
            },
            FilterItem {
                name: "chapter_count".into(),
                type_name: "integer_range".into(),
                value: FilterValue::String("[10,100]".into()),
            },
        ];
        let preferences = search_preferences(|_| None, &filters, None).unwrap();
        let query = crate::query::search_filters(&preferences.search, &Default::default());
        assert!(query.contains(&"releaseYear:>=2000".into()));
        assert!(query.contains(&"chapterCount:<=100".into()));
    }
}

#[cfg(test)]
mod migration_tests {
    use super::*;

    #[test]
    fn general_operations_ignore_obsolete_search_but_legacy_search_rejects_it() {
        let stored = [
            (settings::YEAR_FROM, "not a year"),
            (settings::SORT, "not a sort"),
            (settings::INCLUDE_GENRES, "Action"),
            (settings::EXCLUDE_GENRES, "Action"),
        ];
        let get = |key: &str| {
            stored
                .iter()
                .find(|(id, _)| *id == key)
                .map(|(_, value)| (*value).to_owned())
        };
        assert!(Preferences::parse_general(get).issues().is_empty());
        assert!(matches!(
            search_preferences(get, &[], None),
            Err(Error::NeedsSetup(_))
        ));
        assert_eq!(get(settings::YEAR_FROM).as_deref(), Some("not a year"));
    }

    #[test]
    fn rich_negotiation_works_before_any_new_key_is_saved() {
        struct ResetRichHost(bool);
        impl Drop for ResetRichHost {
            fn drop(&mut self) {
                RICH_HOST.set(self.0);
            }
        }
        let _reset = ResetRichHost(RICH_HOST.replace(false));
        let get = |key: &str| match key {
            settings::YEAR_FROM => Some("invalid legacy year".into()),
            settings::SORT => Some("Trending".into()),
            settings::ADULT_ONLY => Some("true".into()),
            _ => None,
        };
        assert!(search_preferences(get, &[], None).is_err());
        negotiate_rich_host();
        let general = Preferences::parse_general(get);
        assert!(general.issues().is_empty());
        assert!(!general.content.adult_only);
        let search = search_preferences(get, &[], None).unwrap();
        assert!(search.issues().is_empty());
        assert_eq!(search.search.sort, Sort::Popularity);
        assert_eq!(search.search.year_from, None);
        assert_eq!(get(CONTENT_TYPES), None);
        assert_eq!(get(settings::SORT).as_deref(), Some("Trending"));
    }

    #[test]
    fn novel_only_rich_discover_keeps_the_shared_popularity_window() {
        let parsed = preferences(&[
            (CONTENT_TYPES, "[]"),
            ("home_popular", "false"),
            ("home_novels", "true"),
            (settings::POPULARITY_PERIOD, "Weekly"),
        ]);
        assert!(parsed.issues().is_empty());
        let novels = settings::section("home_novels").unwrap();
        assert!(parsed.section_enabled(novels));
        assert_eq!(parsed.period(novels), Some(settings::Period::Weekly));
    }

    fn preferences(values: &[(&str, &str)]) -> Preferences {
        Preferences::parse(|key| {
            values
                .iter()
                .find(|(id, _)| *id == key)
                .map(|(_, value)| (*value).to_owned())
        })
    }

    #[test]
    fn explicit_empty_ids_replace_all_legacy_fields_without_deleting_them() {
        let legacy = [
            (settings::EXCLUDE_TAGS, "Blue"),
            (settings::TAG_GROUPS[0].0, "Alpha"),
            (settings::SPOTLIGHT_GENRE, "Action"),
            (settings::TYPES, "Manga"),
        ];
        let untouched = preferences(&legacy);
        assert_eq!(untouched.content.excluded_tags, vec!["Blue", "Alpha"]);
        assert_eq!(untouched.spotlight_genre.as_deref(), Some("Action"));
        let mut edited = legacy.to_vec();
        edited.extend([
            (CONTENT_TYPES, "[]"),
            (BLOCKED_TAG_IDS, "[]"),
            (SPOTLIGHT_GENRE_ID, ""),
        ]);
        let cleared = preferences(&edited);
        assert!(cleared.content.types.is_empty());
        assert!(cleared.content.excluded_tags.is_empty());
        assert_eq!(cleared.spotlight_genre, None);
        assert!(cleared.issues().is_empty());
    }

    #[test]
    fn rich_unedited_catalog_fields_retain_legacy_names_until_operation_time() {
        let parsed = preferences(&[
            (CONTENT_TYPES, "[]"),
            (settings::EXCLUDE_TAGS, "Blue"),
            (settings::SPOTLIGHT_GENRE, "Action"),
            ("home_spotlight", "true"),
        ]);
        assert_eq!(parsed.content.excluded_tags, vec!["Blue"]);
        assert_eq!(parsed.spotlight_genre.as_deref(), Some("Action"));
        assert!(parsed.issues().is_empty());
    }

    #[test]
    fn rich_hidden_adult_value_uses_declared_default_but_legacy_keeps_validation() {
        let legacy = preferences(&[(settings::ADULT_ONLY, "true")]);
        assert!(
            legacy
                .issues()
                .iter()
                .any(|issue| issue.key == settings::ADULT_ONLY)
        );
        let hidden = preferences(&[(CONTENT_TYPES, "[]"), (settings::ADULT_ONLY, "true")]);
        assert!(!hidden.content.adult_only);
        assert!(hidden.issues().is_empty());
        let visible = preferences(&[
            (CONTENT_TYPES, "[]"),
            (settings::MAX_RATING, "Pornographic"),
            (settings::ADULT_ONLY, "true"),
        ]);
        assert!(visible.content.adult_only);
    }

    #[test]
    fn invalid_catalog_encoding_and_origin_ids_are_reported_locally() {
        let malformed = preferences(&[(CONTENT_TYPES, "[]"), (BLOCKED_TAG_IDS, "Blue,Alpha")]);
        assert!(
            malformed
                .issues()
                .iter()
                .any(|issue| issue.key == BLOCKED_TAG_IDS)
        );
        let invalid_type = preferences(&[(CONTENT_TYPES, "[\"Manhwa\"]")]);
        assert!(
            invalid_type
                .issues()
                .iter()
                .any(|issue| issue.key == CONTENT_TYPES)
        );
        let selected = preferences(&[
            (CONTENT_TYPES, "[\"Manwha\"]"),
            (BLOCKED_TAG_IDS, "[\"t2\"]"),
        ]);
        assert_eq!(selected.content.types, vec!["Manwha"]);
        assert_eq!(selected.content.excluded_tags, vec!["t2"]);
        assert!(selected.issues().is_empty());
    }

    #[test]
    fn page_size_is_bounded_and_values_mode_returns_every_known_selection() {
        let taxonomy = Taxonomy {
            genres: Vec::new(),
            tags: (0..150)
                .map(|id| crate::models::Term {
                    id: format!("id{id:03}"),
                    name: format!("Tag {id:03}"),
                    group: Some("Group".into()),
                })
                .collect(),
        };
        let page = option_page(
            &taxonomy,
            OptionRequest {
                catalog: "tags".into(),
                query: OptionQuery::Search {
                    text: "".into(),
                    offset: 0,
                },
                limit: 1000,
            },
        )
        .unwrap();
        assert_eq!(page.options.len(), 100);
        assert_eq!(page.next_offset, Some(100));
        let labels = option_page(
            &taxonomy,
            OptionRequest {
                catalog: "tags".into(),
                query: OptionQuery::Values(
                    taxonomy.tags.iter().map(|term| term.id.clone()).collect(),
                ),
                limit: 1,
            },
        )
        .unwrap();
        assert_eq!(labels.options.len(), 150);
        assert_eq!(labels.next_offset, None);
    }
}
