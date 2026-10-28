//! Request URLs for catalog search and the ranked feeds, built from typed preferences.
//!
//! Values from settings or the user only ever appear as quoted filter literals or
//! percent-encoded query values; field names and operators are fixed here.

use crate::links::SITE;
use crate::models::Term;
use crate::settings::{ContentPreferences, Period, Rating, SearchPreferences, Sort};
use ito_rs::models::MediaKind;

pub const PAGE_SIZE: usize = 24;
const SEARCH_URL: &str = "https://atsu.moe/collections/manga/documents/search";
const QUERY_FIELDS: &str = "title,englishTitle,otherNames,authors,acronyms";
const RESULT_FIELDS: &str =
    "id,title,englishTitle,authors,poster,type,medium,isAdult,mbContentRating,status";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Medium {
    Comic,
    Novel,
}

impl Medium {
    fn name(self) -> &'static str {
        match self {
            Medium::Comic => "Comic",
            Medium::Novel => "Novel",
        }
    }
}

/// The media a request may return: the requested kind, if the settings show it.
pub fn mediums(content: &ContentPreferences, kind: Option<MediaKind>) -> Vec<Medium> {
    let comics = content.include_comics && matches!(kind, None | Some(MediaKind::Manga));
    let novels = content.include_novels && matches!(kind, None | Some(MediaKind::Novel));
    [(comics, Medium::Comic), (novels, Medium::Novel)]
        .into_iter()
        .filter_map(|(shown, medium)| shown.then_some(medium))
        .collect()
}

/// Genre and tag ids for the search-only filters.
#[derive(Debug, Default, PartialEq)]
pub struct SearchIds {
    pub include_genres: Vec<String>,
    pub exclude_genres: Vec<String>,
    pub include_tags: Vec<String>,
}

/// Maps names (case-insensitive) or ids to ids. Returns the first unknown entry as the error.
pub fn resolve<'a>(entries: &'a [String], terms: &[Term]) -> Result<Vec<String>, &'a str> {
    let mut ids = Vec::with_capacity(entries.len());
    for entry in entries {
        let term = terms
            .iter()
            .find(|term| term.id == *entry || crate::settings::same_name(&term.name, entry))
            .ok_or(entry.as_str())?;
        if !ids.contains(&term.id) {
            ids.push(term.id.clone());
        }
    }
    Ok(ids)
}

/// Filters every catalog request carries: visibility, media, rating, origin and blocked tags.
pub fn catalog_filters(
    content: &ContentPreferences,
    mediums: &[Medium],
    blocked_tag_ids: &[String],
) -> Vec<String> {
    let mut filters = vec!["hidden:!=true".to_owned()];
    filters.push(any_of("medium", mediums.iter().map(|medium| medium.name())));
    if content.adult_only {
        filters.push("isAdult:=true".into());
    } else if content.max_rating < Rating::Pornographic {
        filters.push("isAdult:!=true".into());
        filters.push(none_of(
            "mbContentRating",
            Rating::ALL
                .into_iter()
                .filter(|rating| *rating > content.max_rating)
                .map(Rating::name),
        ));
    }
    if !content.types.is_empty() {
        filters.push(any_of("type", content.types.iter().copied()));
    }
    if !blocked_tag_ids.is_empty() {
        filters.push(none_of(
            "tagIds",
            blocked_tag_ids.iter().map(String::as_str),
        ));
    }
    filters
}

/// Filters applied to searches only.
pub fn search_filters(search: &SearchPreferences, ids: &SearchIds) -> Vec<String> {
    let mut filters = Vec::new();
    if let Some(status) = search.status {
        filters.push(any_of("status", [status]));
    }
    if let Some(year) = search.year_from {
        filters.push(format!("releaseYear:>={year}"));
    }
    if let Some(year) = search.year_to {
        filters.push(format!("releaseYear:<={year}"));
    }
    if let Some(count) = search.min_chapters {
        filters.push(format!("chapterCount:>={count}"));
    }
    if let Some(count) = search.max_chapters {
        filters.push(format!("chapterCount:<={count}"));
    }
    if search.official_only {
        filters.push("officialTranslation:=true".into());
    }
    filters.extend(ids.include_genres.iter().map(|id| has("genreIds", id)));
    if !ids.exclude_genres.is_empty() {
        filters.push(none_of(
            "genreIds",
            ids.exclude_genres.iter().map(String::as_str),
        ));
    }
    filters.extend(ids.include_tags.iter().map(|id| has("tagIds", id)));
    filters
}

/// A filter matching titles in a genre.
pub fn genre_filter(id: &str) -> String {
    has("genreIds", id)
}

/// A filter matching the given series ids.
pub fn id_filter<'a>(ids: impl IntoIterator<Item = &'a str>) -> String {
    any_of("id", ids)
}

/// A catalog search. Without text, results follow `sort`; with text, relevance.
pub fn search_url(text: &str, page: i32, mut filters: Vec<String>, sort: Sort) -> String {
    let text = text.trim();
    let mut pairs = vec![
        ("q", if text.is_empty() { "*" } else { text }.to_owned()),
        ("query_by", QUERY_FIELDS.to_owned()),
        ("include_fields", RESULT_FIELDS.to_owned()),
        ("page", page.max(1).to_string()),
        ("per_page", PAGE_SIZE.to_string()),
    ];
    if text.is_empty() {
        let (field, guard) = match sort {
            Sort::Popularity => ("views:desc", Some("views:>0")),
            Sort::Trending => ("trending:desc", None),
            Sort::Title => ("title:asc", None),
            Sort::RecentlyAdded => ("dateAdded:desc", None),
            Sort::ReleaseDate => ("releaseDate:desc", None),
            Sort::TopRated => ("mbRating:desc", Some("mbRating:>0")),
        };
        filters.extend(guard.map(str::to_owned));
        pairs.push(("sort_by", field.to_owned()));
    }
    pairs.push(("filter_by", filters.join(" && ")));
    with_query(SEARCH_URL, &pairs)
}

/// A lookup of specific series, for ratings or a pasted link.
pub fn lookup_url(ids: &[&str]) -> String {
    with_query(
        SEARCH_URL,
        &[
            ("q", "*".to_owned()),
            ("query_by", "title".to_owned()),
            ("include_fields", RESULT_FIELDS.to_owned()),
            ("per_page", ids.len().max(1).to_string()),
            ("filter_by", id_filter(ids.iter().copied())),
        ],
    )
}

/// A page of a ranked feed such as `popular` or `recentlyUpdated`.
pub fn feed_url(
    endpoint: &str,
    offset: usize,
    mediums: &[Medium],
    content: &ContentPreferences,
    blocked_tag_ids: &[String],
    period: Option<Period>,
) -> String {
    let mut pairs = vec![
        ("offset", offset.to_string()),
        ("limit", PAGE_SIZE.to_string()),
        ("mediums", join(mediums.iter().map(|medium| medium.name()))),
        (
            "contentRatings",
            join(
                Rating::ALL
                    .into_iter()
                    .filter(|rating| *rating <= content.max_rating)
                    .map(Rating::name),
            ),
        ),
    ];
    if content.adult_only {
        pairs.push(("adult", "1".to_owned()));
    }
    if !content.types.is_empty() {
        pairs.push(("types", join(content.types.iter().copied())));
    }
    if !blocked_tag_ids.is_empty() {
        pairs.push((
            "excludedTags",
            join(blocked_tag_ids.iter().map(String::as_str)),
        ));
    }
    if let Some(period) = period {
        pairs.push(("timeframe", period.api_value().to_owned()));
    }
    with_query(&format!("{SITE}/api/home2/{endpoint}"), &pairs)
}

pub fn with_query(base: &str, pairs: &[(&str, String)]) -> String {
    let mut url = base.to_owned();
    for (index, (key, value)) in pairs.iter().enumerate() {
        url.push(if index == 0 { '?' } else { '&' });
        url.push_str(key);
        url.push('=');
        url.push_str(&encode(value));
    }
    url
}

/// Percent-encodes everything except RFC 3986 unreserved characters.
pub fn encode(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
            out.push(char::from(byte));
        } else {
            const HEX: &[u8; 16] = b"0123456789ABCDEF";
            out.push('%');
            out.push(char::from(HEX[usize::from(byte >> 4)]));
            out.push(char::from(HEX[usize::from(byte & 0x0F)]));
        }
    }
    out
}

fn join<'a>(values: impl IntoIterator<Item = &'a str>) -> String {
    values.into_iter().collect::<Vec<_>>().join(",")
}

/// A backtick-quoted filter literal.
fn literal(value: &str) -> String {
    format!("`{}`", value.replace('`', "\\`"))
}

fn has(field: &str, value: &str) -> String {
    format!("{field}:={}", literal(value))
}

fn any_of<'a>(field: &str, values: impl IntoIterator<Item = &'a str>) -> String {
    format!("{field}:=[{}]", literals(values))
}

fn none_of<'a>(field: &str, values: impl IntoIterator<Item = &'a str>) -> String {
    format!("{field}:!=[{}]", literals(values))
}

fn literals<'a>(values: impl IntoIterator<Item = &'a str>) -> String {
    values
        .into_iter()
        .map(literal)
        .collect::<Vec<_>>()
        .join(",")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::Preferences;

    fn content(values: &[(&str, &str)]) -> ContentPreferences {
        let values: Vec<(String, String)> = values
            .iter()
            .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
            .collect();
        Preferences::parse(|key| {
            values
                .iter()
                .find(|(stored, _)| stored == key)
                .map(|(_, value)| value.clone())
        })
        .content
    }

    fn decoded_filter(url: &str) -> String {
        let encoded = url
            .split("filter_by=")
            .nth(1)
            .and_then(|rest| rest.split('&').next())
            .expect("filter present");
        let bytes = encoded.as_bytes();
        let mut out = Vec::new();
        let mut index = 0;
        while index < bytes.len() {
            if bytes[index] == b'%' {
                let hex = std::str::from_utf8(&bytes[index + 1..index + 3]).expect("ascii");
                out.push(u8::from_str_radix(hex, 16).expect("hex"));
                index += 3;
            } else {
                out.push(bytes[index]);
                index += 1;
            }
        }
        String::from_utf8(out).expect("utf8")
    }

    #[test]
    fn encoding_leaves_only_unreserved_bytes() {
        assert_eq!(encode("a b&c=d/é~"), "a%20b%26c%3Dd%2F%C3%A9~");
    }

    #[test]
    fn requested_kind_is_limited_by_the_content_toggles() {
        let both = content(&[]);
        assert_eq!(mediums(&both, None), [Medium::Comic, Medium::Novel]);
        assert_eq!(mediums(&both, Some(MediaKind::Novel)), [Medium::Novel]);
        assert!(mediums(&both, Some(MediaKind::Anime)).is_empty());
        let comics = content(&[("include_novels", "false")]);
        assert!(mediums(&comics, Some(MediaKind::Novel)).is_empty());
    }

    #[test]
    fn rating_ceiling_hides_adult_and_higher_ratings() {
        let safe = catalog_filters(&content(&[]), &[Medium::Comic], &[]);
        assert_eq!(
            safe,
            [
                "hidden:!=true",
                "medium:=[`Comic`]",
                "isAdult:!=true",
                "mbContentRating:!=[`Suggestive`,`Erotica`,`Pornographic`]",
            ]
        );
        let explicit = content(&[("maximum_rating", "Pornographic")]);
        assert_eq!(catalog_filters(&explicit, &[Medium::Novel], &[]).len(), 2);
        let adult = content(&[("maximum_rating", "Pornographic"), ("adult_only", "true")]);
        assert!(
            catalog_filters(&adult, &[Medium::Comic], &[]).contains(&"isAdult:=true".to_owned())
        );
    }

    #[test]
    fn literals_cannot_escape_their_quotes() {
        let ids = SearchIds {
            include_genres: vec!["a` || hidden:=true".into()],
            ..SearchIds::default()
        };
        let filters = search_filters(&Preferences::parse(|_| None).search, &ids);
        assert_eq!(filters, ["genreIds:=`a\\` || hidden:=true`"]);
    }

    #[test]
    fn browsing_sorts_and_text_search_ranks_by_relevance() {
        let browse = search_url("", 0, vec!["hidden:!=true".into()], Sort::TopRated);
        assert!(browse.contains("sort_by=mbRating%3Adesc"));
        assert!(browse.contains("page=1&"));
        assert_eq!(decoded_filter(&browse), "hidden:!=true && mbRating:>0");
        let text = search_url(
            " solo & co ",
            2,
            vec!["hidden:!=true".into()],
            Sort::TopRated,
        );
        assert!(text.contains("q=solo%20%26%20co&"));
        assert!(!text.contains("sort_by"));
        assert_eq!(decoded_filter(&text), "hidden:!=true");
    }

    #[test]
    fn feeds_carry_content_preferences() {
        let preferences = content(&[("maximum_rating", "Suggestive"), ("types", "Manhwa")]);
        let url = feed_url(
            "popular",
            48,
            &[Medium::Comic, Medium::Novel],
            &preferences,
            &["t1".into(), "t2".into()],
            Some(Period::Weekly),
        );
        assert_eq!(
            url,
            "https://atsu.moe/api/home2/popular?offset=48&limit=24&mediums=Comic%2CNovel\
             &contentRatings=Safe%2CSuggestive&types=Manwha&excludedTags=t1%2Ct2&timeframe=weekly"
        );
    }

    #[test]
    fn names_resolve_to_ids_and_unknown_names_are_reported() {
        let terms = [
            Term {
                id: "g1".into(),
                name: "Slice of Life".into(),
                group: None,
            },
            Term {
                id: "g2".into(),
                name: "Action".into(),
                group: None,
            },
        ];
        let entries = vec![
            "slice of life".to_owned(),
            "g2".to_owned(),
            "ACTION".to_owned(),
        ];
        assert_eq!(
            resolve(&entries, &terms),
            Ok(vec!["g1".to_owned(), "g2".to_owned()])
        );
        let unknown = vec!["Action".to_owned(), "Actoin".to_owned()];
        assert_eq!(resolve(&unknown, &terms), Err("Actoin"));
    }
}
