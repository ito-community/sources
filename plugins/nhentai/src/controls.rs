use crate::api;
use crate::gallery::TitlePreference;
use crate::images;
use crate::query::{self, SearchParts, Sort, is_blocked_name, is_meta_language, quoted_clause};
use ito_rs::controls::{
    ChoiceOption, Control, ControlKind, ControlSection, ControlValues, OptionPage, OptionQuery,
    OptionRequest, OptionSource, SearchCapabilities, SearchSuggestion, SettingsForm,
};
use ito_rs::models::{FilterItem, Setting, SettingIssue, SettingsSchema};
use ito_rs::{Error, Result};

pub struct Preferences {
    pub title: TitlePreference,
    pub server: u8,
    pub languages: Vec<String>,
    pub blocked: Vec<String>,
    pub extra: String,
}

pub fn settings_form() -> SettingsForm {
    SettingsForm::new(vec![
        ControlSection::new("display", "Display")
            .control(Control::select(
                "title_preference",
                "Title",
                inline(&[
                    ("english", "English"),
                    ("japanese", "Japanese"),
                    ("pretty", "Pretty"),
                ]),
                Some("english".into()),
            ))
            .control(
                Control::select(
                    "image_server",
                    "Image server",
                    inline(&[
                        ("1", "Server 1"),
                        ("2", "Server 2"),
                        ("3", "Server 3"),
                        ("4", "Server 4"),
                    ]),
                    Some("1".into()),
                )
                .summary("Used for covers and pages. Change it if images fail to load."),
            ),
        ControlSection::new("content", "Content")
            .summary("Lolicon and shotacon are always excluded.")
            .control(
                Control::multi_select(
                    "default_languages",
                    "Languages",
                    OptionSource::searchable_catalog("languages"),
                    Vec::new(),
                )
                .summary("Used by Home, listings, and search unless a search sets its own."),
            )
            .control(
                Control::multi_select(
                    "blocked_tags",
                    "Blocked tags",
                    OptionSource::searchable_catalog("tags"),
                    Vec::new(),
                )
                .summary("Excluded from every search, in addition to the tags this source always excludes."),
            ),
        ControlSection::new("account", "Account")
            .summary("Browse does not need an account.")
            .control(
                Control::secret("api_key", "API key").summary(
                    "Optional. Unlocks favorites and a higher rate limit. Create one in your nhentai account settings.",
                ),
            ),
        ControlSection::new("advanced", "Advanced").control(extra_query()),
    ])
}

pub fn capabilities() -> SearchCapabilities {
    let mut pages = Control::integer_range("pages", "Page count", 1, 2_000);
    if let ControlKind::IntegerRange(range) = &mut pages.kind {
        range.unit = Some("pages".into());
    }
    SearchCapabilities {
        flags: SearchCapabilities::BROWSE
            | SearchCapabilities::SORT_WITH_QUERY
            | SearchCapabilities::FILTERS_WITH_QUERY
            | SearchCapabilities::SUGGESTIONS,
        sort: Some(Control::select(
            "sort",
            "Sort",
            inline(&[
                ("date", "Latest"),
                ("popular-today", "Popular today"),
                ("popular-week", "Popular this week"),
                ("popular-month", "Popular this month"),
                ("popular", "Popular all time"),
            ]),
            Some("date".into()),
        )),
        sections: vec![
            ControlSection::new("filters", "Filters")
                .control(Control::multi_select(
                    "languages",
                    "Languages",
                    OptionSource::searchable_catalog("languages"),
                    Vec::new(),
                ))
                .control(Control::multi_select(
                    "categories",
                    "Categories",
                    OptionSource::searchable_catalog("categories"),
                    Vec::new(),
                ))
                .control(pages),
            ControlSection::new("tags", "Tags")
                .control(Control::multi_select(
                    "tags",
                    "Required tags",
                    OptionSource::searchable_catalog("tags"),
                    Vec::new(),
                ))
                .control(Control::multi_select(
                    "excluded_tags",
                    "Excluded tags",
                    OptionSource::searchable_catalog("tags"),
                    Vec::new(),
                )),
            ControlSection::new("credits", "Credits")
                .control(Control::multi_select(
                    "artists",
                    "Artists",
                    OptionSource::searchable_catalog("artists"),
                    Vec::new(),
                ))
                .control(Control::multi_select(
                    "groups",
                    "Groups",
                    OptionSource::searchable_catalog("groups"),
                    Vec::new(),
                ))
                .control(Control::multi_select(
                    "parodies",
                    "Parodies",
                    OptionSource::searchable_catalog("parodies"),
                    Vec::new(),
                ))
                .control(Control::multi_select(
                    "characters",
                    "Characters",
                    OptionSource::searchable_catalog("characters"),
                    Vec::new(),
                )),
        ],
    }
}

pub fn legacy_settings() -> SettingsSchema {
    SettingsSchema {
        settings: vec![
            Setting::Picker {
                id: "title_preference".into(),
                name: "Title".into(),
                summary: None,
                options: vec!["english".into(), "japanese".into(), "pretty".into()],
                default_value: "english".into(),
            },
            Setting::Picker {
                id: "image_server".into(),
                name: "Image server".into(),
                summary: Some("1 to 4. Change it if images fail to load.".into()),
                options: vec!["1".into(), "2".into(), "3".into(), "4".into()],
                default_value: "1".into(),
            },
            Setting::Text {
                id: "extra_query".into(),
                name: "Extra query".into(),
                summary: Some("Appended to every search. Some tags are always excluded.".into()),
                default_value: String::new(),
            },
        ],
    }
}

pub fn preferences() -> Result<Preferences> {
    from_settings(&ControlValues::settings(&settings_form()))
}

pub fn search_parts(query: &str, filters: &[FilterItem]) -> Result<(SearchParts, Sort)> {
    let prefs = preferences()?;
    let values = ControlValues::search(&capabilities(), filters)?;
    let languages = nonempty(values.multi_select("languages")?).unwrap_or(&prefs.languages);
    let mut excluded = prefs.blocked.clone();
    excluded.extend(values.multi_select("excluded_tags")?.iter().cloned());
    Ok((
        SearchParts {
            text: query.to_owned(),
            languages: languages.to_vec(),
            categories: values.multi_select("categories")?.to_vec(),
            tags: values.multi_select("tags")?.to_vec(),
            excluded_tags: excluded,
            artists: values.multi_select("artists")?.to_vec(),
            characters: values.multi_select("characters")?.to_vec(),
            parodies: values.multi_select("parodies")?.to_vec(),
            groups: values.multi_select("groups")?.to_vec(),
            pages_min: values.integer_range("pages")?.0,
            pages_max: values.integer_range("pages")?.1,
            extra: prefs.extra,
        },
        Sort::from_control(values.select("sort")?),
    ))
}

pub fn browse_parts() -> Result<SearchParts> {
    let prefs = preferences()?;
    Ok(SearchParts::browse(
        &prefs.languages,
        &prefs.blocked,
        &prefs.extra,
    ))
}

pub fn view() -> Result<crate::gallery::View> {
    let prefs = preferences()?;
    Ok(crate::gallery::View {
        title: prefs.title,
        server: prefs.server,
    })
}

pub fn validate() -> Result<Vec<SettingIssue>> {
    let values = ControlValues::settings(&settings_form());
    let mut issues = values.issues().to_vec();
    let extra = values.text("extra_query").unwrap_or("");
    if query::mentions_blocked(extra) {
        issues.push(SettingIssue {
            key: "extra_query".into(),
            message: "Part of the extra query is ignored. Some tags are always excluded.".into(),
        });
    }
    Ok(issues)
}

pub fn option_page(request: OptionRequest) -> Result<OptionPage> {
    let kind = catalog_kind(&request.catalog).ok_or(Error::Unsupported)?;
    match request.query {
        OptionQuery::Values(values) => {
            let options = values
                .into_iter()
                .filter(|value| !value.is_empty() && !is_blocked_name(value))
                .take(500)
                .map(|value| ChoiceOption::new(value.clone(), value))
                .collect::<Vec<_>>();
            let total = options.len() as u32;
            Ok(OptionPage {
                options,
                next_offset: None,
                total: Some(total),
                revision: None,
            })
        }
        OptionQuery::Search { text, offset } => {
            let text = text.trim().to_owned();
            let limit = request.limit.clamp(1, 50);
            if text.is_empty() {
                browse_catalog(kind, offset, limit)
            } else if offset > 0 {
                Ok(empty_page())
            } else {
                let tags = api::search_tags(kind, &text, limit)?;
                Ok(page_from(tags, kind, None, None))
            }
        }
    }
}

pub fn suggestions(query: &str) -> Result<Vec<SearchSuggestion>> {
    let query = query.trim();
    if query.is_empty() || query.len() > 64 {
        return Ok(Vec::new());
    }
    let Some((kind, prefix, text)) = query::suggestion_prefix(query) else {
        return Ok(Vec::new());
    };
    let tags = api::search_tags(kind, text, 8)?;
    let mut out = Vec::new();
    for tag in tags {
        if is_blocked_name(&tag.name) || (kind == "language" && is_meta_language(&tag.name)) {
            continue;
        }
        out.push(SearchSuggestion {
            query: quoted_clause(prefix, &tag.name),
            subtitle: Some(format!("{} · {}", type_label(&tag.kind), tag.count)),
        });
        if out.len() == 8 {
            break;
        }
    }
    Ok(out)
}

fn browse_catalog(kind: &str, offset: u32, limit: u32) -> Result<OptionPage> {
    let page_no = offset / limit + 1;
    let response = api::tags_by_type(kind, page_no as i32, limit as i32)?;
    let more = page_no < response.num_pages.max(0) as u32;
    Ok(page_from(
        response.result,
        kind,
        more.then_some(offset.saturating_add(limit)),
        response.total.map(|total| total as u32),
    ))
}

fn page_from(
    tags: Vec<api::Tag>,
    kind: &str,
    next_offset: Option<u32>,
    total: Option<u32>,
) -> OptionPage {
    let options = tags
        .into_iter()
        .filter(|tag| !is_blocked_name(&tag.name))
        .filter(|tag| kind != "language" || !is_meta_language(&tag.name))
        .map(|tag| {
            let name = tag.name;
            ChoiceOption::new(name.clone(), name).grouped(type_label(&tag.kind))
        })
        .collect();
    OptionPage {
        options,
        next_offset,
        total,
        revision: None,
    }
}

fn empty_page() -> OptionPage {
    OptionPage {
        options: Vec::new(),
        next_offset: None,
        total: Some(0),
        revision: None,
    }
}

fn catalog_kind(id: &str) -> Option<&'static str> {
    match id {
        "tags" => Some("tag"),
        "artists" => Some("artist"),
        "characters" => Some("character"),
        "parodies" => Some("parody"),
        "groups" => Some("group"),
        "languages" => Some("language"),
        "categories" => Some("category"),
        _ => None,
    }
}

fn type_label(kind: &str) -> String {
    match kind {
        "tag" => "Tag",
        "artist" => "Artist",
        "character" => "Character",
        "parody" => "Parody",
        "group" => "Group",
        "language" => "Language",
        "category" => "Category",
        _ => "Tag",
    }
    .to_owned()
}

fn from_settings(values: &ControlValues) -> Result<Preferences> {
    Ok(Preferences {
        title: TitlePreference::parse(values.select("title_preference")?),
        server: images::server_index(values.select("image_server")?),
        languages: values.multi_select("default_languages")?.to_vec(),
        blocked: values.multi_select("blocked_tags")?.to_vec(),
        extra: values.text("extra_query")?.to_owned(),
    })
}

fn nonempty(values: &[String]) -> Option<&[String]> {
    (!values.is_empty()).then_some(values)
}

fn extra_query() -> Control {
    let mut control = Control::text("extra_query", "Extra query", "");
    if let ControlKind::Text(text) = &mut control.kind {
        text.placeholder = Some("pages:>20 artist:\"name\"".into());
    }
    control.summary("Appended to every search. Some tags are always excluded.")
}

fn inline(pairs: &[(&str, &str)]) -> OptionSource {
    OptionSource::inline(
        pairs
            .iter()
            .map(|(value, label)| ChoiceOption::new(*value, *label))
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use ito_rs::controls::{validate_capabilities, validate_form};
    use ito_rs::models::FilterValue;

    #[test]
    fn forms_satisfy_the_sdk_contract() {
        validate_form(&settings_form()).expect("settings form");
        validate_capabilities(&capabilities()).expect("search capabilities");
    }

    #[test]
    fn search_filters_override_empty_language_defaults() {
        let form = settings_form();
        let values = ControlValues::settings_with(&form, |key| {
            (key == "default_languages").then(|| "[\"english\"]".into())
        });
        let prefs = from_settings(&values).expect("settings");
        assert_eq!(prefs.languages, ["english"]);
        assert_eq!(prefs.server, 1);
        let filters = vec![FilterItem {
            type_name: "multi_select".into(),
            name: "languages".into(),
            value: FilterValue::String("[\"japanese\"]".into()),
        }];
        let values = ControlValues::search(&capabilities(), &filters).expect("filters");
        assert_eq!(values.multi_select("languages").unwrap(), ["japanese"]);
        assert_eq!(
            Sort::from_control(values.select("sort").unwrap()),
            Sort::Date
        );
    }
}
