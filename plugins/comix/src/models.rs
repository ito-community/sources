use crate::{BASE_URL, settings};
use ito_rs::catalog::{MAX_ABS_NUMBER, MAX_SCANLATOR_NAME_BYTES, MAX_TITLE_BYTES};
use ito_rs::models::manga::{
    Chapter, ContentRating, Manga, PageResult, Status as MangaStatus, Viewer,
};
use ito_rs::models::{ContentKind, Page, PageContent, Scanlator};
use ito_rs::{Error, Result};
use serde::Deserialize;

/// One page of a listing: `{"items": [...], "meta": {...}}` after the envelope is unwrapped.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Paged<T> {
    pub items: Vec<T>,
    pub meta: PageMeta,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PageMeta {
    pub page: i32,
    pub last_page: i32,
}

impl Paged<Series> {
    pub fn into_page_result(self, hidden_types: &[String]) -> PageResult {
        let has_next_page = self.meta.page < self.meta.last_page;
        PageResult {
            entries: visible(self.items, hidden_types),
            has_next_page,
        }
    }
}

/// Converts series summaries and drops hidden entries. Top lists carry no page metadata.
pub fn visible(items: Vec<Series>, hidden_types: &[String]) -> Vec<Manga> {
    items
        .into_iter()
        .filter(|series| !hidden_types.contains(&series.r#type))
        .map(Manga::from)
        .collect()
}

/// A series as listings and detail return it. Detail-only lists are absent from listings.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Series {
    pub hid: String,
    pub title: String,
    pub r#type: String,
    pub status: String,
    /// Missing or null is read as `None`; an unrecognised rating as `Unknown`. Both are flagged
    /// NSFW, so a rating the plugin does not know never shows as safe.
    #[serde(default)]
    pub content_rating: Option<Rating>,
    pub synopsis: Option<String>,
    pub poster: Option<Poster>,
    pub authors: Option<Vec<Term>>,
    pub artists: Option<Vec<Term>>,
    pub genres: Option<Vec<Term>>,
}

/// The site's content ratings (spec section 5.1). Anything other than the four known values is
/// `Unknown`, which is flagged NSFW rather than failing the item.
#[derive(Deserialize, Clone, Copy)]
#[serde(rename_all = "lowercase")]
pub enum Rating {
    Safe,
    Suggestive,
    Erotica,
    Pornographic,
    #[serde(other)]
    Unknown,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Poster {
    pub medium: Option<String>,
    pub large: Option<String>,
}

#[derive(Deserialize)]
pub struct Term {
    pub title: String,
}

/// Site paths are relative to the site origin; anything already absolute is kept.
fn absolute_url(path: &str) -> String {
    if path.starts_with("https://") || path.starts_with("http://") {
        path.to_owned()
    } else {
        format!("{BASE_URL}{path}")
    }
}

fn cover(poster: Option<Poster>) -> Option<String> {
    let poster = poster?;
    let path = match settings::image_quality().as_str() {
        "medium" => poster.medium.or(poster.large),
        "large" => poster.large.or(poster.medium),
        _ => None,
    };
    path.map(|path| absolute_url(&path))
}

fn status(value: &str) -> MangaStatus {
    match value {
        "releasing" => MangaStatus::Ongoing,
        "on_hiatus" => MangaStatus::Hiatus,
        "finished" => MangaStatus::Completed,
        "discontinued" => MangaStatus::Cancelled,
        _ => MangaStatus::Unknown,
    }
}

fn viewer(series_type: &str) -> Viewer {
    match series_type {
        "manhwa" | "manhua" => Viewer::Webtoon,
        "manga" => Viewer::Rtl,
        _ => Viewer::Default,
    }
}

/// Erotica and pornographic series are the NSFW rating. A missing, null or unknown rating is
/// flagged NSFW as well: the plugin fails closed on what it cannot classify, and keeps the item.
fn rating(value: Option<Rating>) -> (ContentRating, i32) {
    match value {
        Some(Rating::Safe) => (ContentRating::Safe, 0),
        Some(Rating::Suggestive) => (ContentRating::Suggestive, 0),
        Some(Rating::Erotica | Rating::Pornographic | Rating::Unknown) | None => {
            (ContentRating::Nsfw, 1)
        }
    }
}

impl From<Series> for Manga {
    fn from(value: Series) -> Self {
        let url = format!("{BASE_URL}/title/{}", value.hid);
        let (content_rating, nsfw) = rating(value.content_rating);
        let status = status(&value.status);
        let viewer = viewer(&value.r#type);
        Self {
            key: value.hid,
            title: value.title,
            cover: cover(value.poster),
            artist: value.artists.map(|v| {
                v.into_iter()
                    .map(|t| t.title)
                    .collect::<Vec<_>>()
                    .join(", ")
            }),
            authors: value
                .authors
                .map(|v| v.into_iter().map(|t| t.title).collect()),
            description: value.synopsis,
            url: Some(url),
            tags: value
                .genres
                .map(|v| v.into_iter().map(|t| t.title).collect()),
            status,
            content_rating,
            nsfw,
            viewer,
            chapters: None,
            catalog: None,
        }
    }
}

/// One upload of a chapter. Several uploads can share a number.
#[derive(Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ComixChapter {
    pub id: i32,
    pub number: f64,
    pub name: Option<String>,
    pub votes: i32,
    pub is_official: bool,
    /// Zero means the site does not know the group.
    pub group_id: Option<i32>,
    pub group: Option<ScanlationGroup>,
}

#[derive(Deserialize, Clone)]
pub struct ScanlationGroup {
    pub name: Option<String>,
}

impl ComixChapter {
    pub fn into_chapter(self, manga_id: &str) -> Chapter {
        let scanlator = self.scanlator();
        let title = self
            .name
            .filter(|name| !name.is_empty() && name.len() <= MAX_TITLE_BYTES);
        Chapter {
            title,
            chapter: chapter_number(self.number),
            scanlator,
            url: Some(format!("{BASE_URL}/title/{manga_id}/{}", self.id)),
            ..Chapter::new(self.id.to_string(), ContentKind::Images)
        }
    }

    /// The uploading group by display name only. No group id is emitted: a source-wide group
    /// identifier has not been verified for this site. A chapter without a usable group name is
    /// "Official" when the site flags it as official, and otherwise has no group.
    fn scanlator(&self) -> Option<Scanlator> {
        match self.group.as_ref().and_then(|group| group.name.as_deref()) {
            Some(name) if is_valid_scanlator_name(name) => Some(Scanlator::named(name)),
            _ if self.is_official => Some(Scanlator::named("Official")),
            _ => None,
        }
    }
}

/// Chapter numbers the SDK accepts: finite and at most 1e9 in magnitude. Anything else is unknown.
fn chapter_number(number: f64) -> Option<f64> {
    (number.is_finite() && number.abs() <= MAX_ABS_NUMBER).then_some(number)
}

/// Group names the SDK accepts: one to 256 bytes. An empty or longer name is not used.
fn is_valid_scanlator_name(name: &str) -> bool {
    !name.is_empty() && name.len() <= MAX_SCANLATOR_NAME_BYTES
}

/// The `result` of a sealed chapter response; only its page list is read here.
#[derive(Deserialize)]
pub struct ChapterPages {
    pub pages: PageList,
}

/// Page images of one chapter, either as an object with an optional URL prefix or as a bare list.
#[derive(Deserialize)]
#[serde(untagged)]
pub enum PageList {
    #[serde(rename_all = "camelCase")]
    Object {
        base_url: Option<String>,
        items: Vec<PageItem>,
    },
    Items(Vec<PageItem>),
}

#[derive(Deserialize)]
pub struct PageItem {
    pub url: String,
}

impl ChapterPages {
    pub fn into_pages(self) -> Result<Vec<Page>> {
        let (prefix, items) = match self.pages {
            PageList::Object { base_url, items } => (base_url.unwrap_or_default(), items),
            PageList::Items(items) => (String::new(), items),
        };
        items
            .into_iter()
            .enumerate()
            .map(|(index, item)| {
                let url = format!("{prefix}{}", item.url);
                if !url.starts_with("https://") {
                    return Err(Error::Server(
                        "Comix returned a page address that is not https".into(),
                    ));
                }
                Ok(Page {
                    index: index as i32,
                    content: PageContent::Url(url),
                    has_description: false,
                    description: None,
                    headers: None,
                })
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paged_envelope_reports_next_page_from_page_metadata() {
        let page: Paged<Series> =
            serde_json::from_str(r#"{"items":[],"meta":{"page":2,"lastPage":3}}"#)
                .expect("page decodes");
        assert!(page.into_page_result(&[]).has_next_page);
    }

    #[test]
    fn last_page_has_no_next_page() {
        let page: Paged<Series> =
            serde_json::from_str(r#"{"items":[],"meta":{"page":3,"lastPage":3}}"#)
                .expect("page decodes");
        assert!(!page.into_page_result(&[]).has_next_page);
    }

    #[test]
    fn series_summary_decodes_with_absent_detail_fields() {
        let series: Series = serde_json::from_str(
            r#"{"hid":"stable-key","title":"Fixture","type":"manga","status":"releasing","contentRating":"safe","synopsis":null,"poster":null}"#,
        )
        .expect("summary decodes");
        let manga = Manga::from(series);
        assert_eq!(manga.key, "stable-key");
        assert_eq!(
            manga.url.as_deref(),
            Some("https://comix.to/title/stable-key")
        );
        assert!(manga.authors.is_none());
        assert!(manga.tags.is_none());
    }

    #[test]
    fn required_series_fields_are_not_defaulted() {
        assert!(
            serde_json::from_str::<Series>(
                r#"{"hid":"stable-key","type":"manga","status":"releasing","contentRating":"safe"}"#,
            )
            .is_err(),
            "a series without a title must fail rather than default"
        );
    }

    #[test]
    fn unknown_missing_or_null_content_rating_is_flagged_nsfw() {
        for json in [
            r#"{"hid":"h","title":"T","type":"manga","status":"releasing","contentRating":"gore"}"#,
            r#"{"hid":"h","title":"T","type":"manga","status":"releasing","contentRating":null}"#,
            r#"{"hid":"h","title":"T","type":"manga","status":"releasing"}"#,
        ] {
            let series: Series = serde_json::from_str(json).expect("the item still decodes");
            let manga = Manga::from(series);
            assert!(matches!(manga.content_rating, ContentRating::Nsfw));
            assert_eq!(manga.nsfw, 1);
        }
    }

    #[test]
    fn one_unknown_rating_does_not_fail_the_page() {
        let page: Paged<Series> = serde_json::from_str(
            r#"{"items":[{"hid":"a","title":"A","type":"manga","status":"releasing","contentRating":"gore"},{"hid":"b","title":"B","type":"manga","status":"releasing","contentRating":"safe"}],"meta":{"page":1,"lastPage":1}}"#,
        )
        .expect("the page decodes");
        let entries = page.into_page_result(&[]).entries;
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].nsfw, 1);
        assert_eq!(entries[1].nsfw, 0);
    }

    #[test]
    fn erotica_and_pornographic_are_nsfw_and_suggestive_is_not() {
        let series = |rating: &str| -> Manga {
            let json = format!(
                r#"{{"hid":"h","title":"T","type":"manga","status":"releasing","contentRating":"{rating}"}}"#
            );
            Manga::from(serde_json::from_str::<Series>(&json).expect("series decodes"))
        };
        let erotica = series("erotica");
        assert!(matches!(erotica.content_rating, ContentRating::Nsfw));
        assert_eq!(erotica.nsfw, 1);
        let pornographic = series("pornographic");
        assert!(matches!(pornographic.content_rating, ContentRating::Nsfw));
        let suggestive = series("suggestive");
        assert!(matches!(
            suggestive.content_rating,
            ContentRating::Suggestive
        ));
        assert_eq!(suggestive.nsfw, 0);
    }

    #[test]
    fn relative_cover_paths_become_site_urls() {
        let series: Series = serde_json::from_str(
            r#"{"hid":"h","title":"T","type":"manga","status":"releasing","contentRating":"safe","poster":{"medium":"/images/covers/a.jpg","large":"/images/covers/b.jpg"}}"#,
        )
        .expect("series decodes");
        assert_eq!(
            Manga::from(series).cover.as_deref(),
            Some("https://comix.to/images/covers/a.jpg")
        );
    }

    #[test]
    fn chapter_requires_the_official_flag() {
        assert!(
            serde_json::from_str::<ComixChapter>(
                r#"{"id":1,"number":1.0,"name":null,"votes":0,"groupId":null,"group":null}"#,
            )
            .is_err(),
            "a chapter without isOfficial must fail rather than default"
        );
    }

    #[test]
    fn empty_chapter_name_is_not_a_title() {
        let chapter: ComixChapter = serde_json::from_str(
            r#"{"id":7,"groupId":null,"number":2.0,"name":"","votes":0,"isOfficial":false,"group":null}"#,
        )
        .expect("chapter decodes");
        assert!(chapter.into_chapter("hid").title.is_none());
    }

    #[test]
    fn absolute_page_urls_keep_their_order_in_both_shapes() {
        let object: ChapterPages = serde_json::from_str(
            r#"{"pages":{"alt":["x.test"],"items":[{"url":"https://cdn.test/1.jpg","width":800,"height":1200},{"url":"https://cdn.test/2.jpg"}]}}"#,
        )
        .expect("object pages decode");
        let pages = object.into_pages().expect("absolute URLs are accepted");
        assert_eq!(pages.len(), 2);
        assert!(matches!(
            &pages[1].content,
            PageContent::Url(url) if url == "https://cdn.test/2.jpg"
        ));

        let bare: ChapterPages =
            serde_json::from_str(r#"{"pages":[{"url":"https://cdn.test/a.jpg"}]}"#)
                .expect("bare pages decode");
        assert_eq!(bare.into_pages().expect("bare pages").len(), 1);
    }

    #[test]
    fn base_url_prefixes_page_urls() {
        let pages: ChapterPages = serde_json::from_str(
            r#"{"pages":{"baseUrl":"https://cdn.test/","items":[{"url":"1.jpg"}]}}"#,
        )
        .expect("pages decode");
        let pages = pages.into_pages().expect("prefixed URL is absolute");
        assert!(matches!(
            &pages[0].content,
            PageContent::Url(url) if url == "https://cdn.test/1.jpg"
        ));
    }

    #[test]
    fn relative_page_urls_are_refused() {
        let pages: ChapterPages =
            serde_json::from_str(r#"{"pages":[{"url":"/page.jpg"}]}"#).expect("pages decode");
        assert!(pages.into_pages().is_err());
    }

    #[test]
    fn http_page_addresses_are_refused_because_the_site_serves_https_only() {
        let pages: ChapterPages =
            serde_json::from_str(r#"{"pages":[{"url":"http://cdn.test/1.jpg"}]}"#)
                .expect("pages decode");
        assert!(pages.into_pages().is_err());
    }

    #[test]
    fn null_or_absent_term_lists_decode_as_none() {
        for json in [
            r#"{"hid":"h","title":"T","type":"manga","status":"releasing","contentRating":"safe","authors":null,"artists":null,"genres":null}"#,
            r#"{"hid":"h","title":"T","type":"manga","status":"releasing","contentRating":"safe"}"#,
        ] {
            let series: Series = serde_json::from_str(json).expect("detail term lists decode");
            assert!(series.authors.is_none());
            assert!(series.artists.is_none());
            assert!(series.genres.is_none());
        }
    }

    /// Representative chapter page. Synthetic: shaped like the site's chapter list, not captured
    /// from it. Pins the keys, titles, numbers, URLs and group names a port keeps.
    const CHAPTER_PAGE: &str = r#"{"items":[
        {"id":101,"groupId":null,"number":1.0,"name":"Opening","votes":12,"isOfficial":true,"group":null},
        {"id":102,"groupId":9,"number":1.5,"name":"","votes":3,"isOfficial":false,"group":{"name":"Group Nine"}},
        {"id":103,"groupId":10702,"number":2.0,"name":null,"votes":0,"isOfficial":true,"group":{"name":"Site Team"}},
        {"id":104,"groupId":null,"number":10.25,"name":"Ten","votes":1,"isOfficial":false,"group":null}
    ],"meta":{"page":1,"lastPage":1}}"#;

    /// The group name a serialized chapter reports, whichever shape its scanlator is written in.
    fn group_name(chapter: &serde_json::Value) -> Option<&str> {
        match &chapter["scanlator"] {
            serde_json::Value::String(name) => Some(name),
            serde_json::Value::Object(scanlator) => scanlator["name"].as_str(),
            _ => None,
        }
    }

    #[test]
    fn chapter_keys_titles_numbers_and_groups_stay_stable() {
        let page: Paged<ComixChapter> = serde_json::from_str(CHAPTER_PAGE).expect("page decodes");
        let values: Vec<serde_json::Value> = page
            .items
            .into_iter()
            .map(|item| {
                serde_json::to_value(item.into_chapter("hid-1")).expect("chapter serializes")
            })
            .collect();

        assert_eq!(values[0]["key"], "101");
        assert_eq!(values[0]["title"], "Opening");
        assert_eq!(values[0]["chapter"], 1.0);
        assert_eq!(values[0]["url"], "https://comix.to/title/hid-1/101");
        assert_eq!(group_name(&values[0]), Some("Official"));

        assert_eq!(values[1]["key"], "102");
        assert!(values[1]["title"].is_null(), "an empty name is not a title");
        assert_eq!(values[1]["chapter"], 1.5);
        assert_eq!(group_name(&values[1]), Some("Group Nine"));

        assert_eq!(values[2]["key"], "103");
        assert_eq!(values[2]["chapter"], 2.0);
        assert_eq!(group_name(&values[2]), Some("Site Team"));
        assert!(
            values[2]["scanlator"]["id"].is_null(),
            "a source id is never emitted for a group the page did not supply one for"
        );

        assert_eq!(values[3]["key"], "104");
        assert_eq!(values[3]["chapter"], 10.25);
        assert!(group_name(&values[3]).is_none());
    }

    fn chapter(
        number: f64,
        name: Option<String>,
        group: Option<Option<&str>>,
        is_official: bool,
    ) -> ComixChapter {
        ComixChapter {
            id: 7,
            group_id: None,
            number,
            name,
            votes: 0,
            is_official,
            group: group.map(|name| ScanlationGroup {
                name: name.map(str::to_owned),
            }),
        }
    }

    #[test]
    fn oversized_titles_are_unknown_and_the_limit_is_inclusive() {
        let at_limit = chapter(1.0, Some("t".repeat(MAX_TITLE_BYTES)), None, false);
        let over_limit = chapter(1.0, Some("t".repeat(MAX_TITLE_BYTES + 1)), None, false);
        assert_eq!(
            at_limit.into_chapter("hid").title.map(|title| title.len()),
            Some(MAX_TITLE_BYTES)
        );
        assert!(over_limit.into_chapter("hid").title.is_none());
    }

    #[test]
    fn numbers_beyond_the_sdk_limit_are_unknown() {
        assert_eq!(chapter_number(1e9), Some(1e9));
        assert_eq!(chapter_number(-1e9), Some(-1e9));
        assert!(chapter_number(1e9 + 1.0).is_none());
        assert!(chapter_number(f64::NAN).is_none());
        assert!(chapter_number(f64::INFINITY).is_none());
    }

    #[test]
    fn empty_or_oversized_group_names_are_not_used() {
        let empty = chapter(1.0, None, Some(Some("")), false).into_chapter("hid");
        assert!(empty.scanlator.is_none());
        let oversized_name = "g".repeat(MAX_SCANLATOR_NAME_BYTES + 1);
        let oversized = chapter(1.0, None, Some(Some(oversized_name.as_str())), false);
        assert!(oversized.into_chapter("hid").scanlator.is_none());
        let official = chapter(1.0, None, Some(Some("")), true).into_chapter("hid");
        assert_eq!(official.scanlator, Some(Scanlator::named("Official")));
        let unnamed = chapter(1.0, None, Some(None), false).into_chapter("hid");
        assert!(unnamed.scanlator.is_none());
    }

    #[test]
    fn a_group_is_named_without_a_source_id() {
        let chapter = chapter(1.0, None, Some(Some("Group Nine")), false).into_chapter("hid");
        assert_eq!(chapter.scanlator, Some(Scanlator::named("Group Nine")));
    }

    #[test]
    fn same_number_uploads_keep_distinct_chapter_ids_and_group_relationships() {
        let alpha = ComixChapter {
            id: 21,
            group_id: Some(11),
            number: 4.0,
            name: None,
            votes: 0,
            is_official: false,
            group: Some(ScanlationGroup {
                name: Some("Alpha".into()),
            }),
        };
        let beta = ComixChapter {
            id: 22,
            group_id: Some(12),
            number: 4.0,
            name: None,
            votes: 0,
            is_official: false,
            group: Some(ScanlationGroup {
                name: Some("Beta".into()),
            }),
        };

        let alpha = alpha.into_chapter("title");
        let beta = beta.into_chapter("title");
        assert_eq!(alpha.key, "21");
        assert_eq!(beta.key, "22");
        assert_eq!(alpha.chapter, Some(4.0));
        assert_eq!(beta.chapter, Some(4.0));
        assert_eq!(alpha.scanlator, Some(Scanlator::named("Alpha")));
        assert_eq!(beta.scanlator, Some(Scanlator::named("Beta")));
    }

    #[test]
    fn adjacent_float_numbers_keep_their_exact_values_and_chapter_ids() {
        let adjacent = f64::from_bits(4.0f64.to_bits() + 1);
        let first = ComixChapter {
            id: 31,
            group_id: Some(11),
            number: 4.0,
            name: None,
            votes: 0,
            is_official: false,
            group: Some(ScanlationGroup {
                name: Some("Alpha".into()),
            }),
        }
        .into_chapter("title");
        let second = ComixChapter {
            id: 32,
            group_id: Some(11),
            number: adjacent,
            name: None,
            votes: 0,
            is_official: false,
            group: Some(ScanlationGroup {
                name: Some("Alpha".into()),
            }),
        }
        .into_chapter("title");

        assert_eq!(first.key, "31");
        assert_eq!(second.key, "32");
        assert_eq!(first.chapter, Some(4.0));
        assert_eq!(second.chapter, Some(adjacent));
        assert_ne!(first.chapter, second.chapter);
    }
}
