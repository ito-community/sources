use crate::{BASE_URL, settings};
use ito_rs::catalog::{MAX_ABS_NUMBER, MAX_SCANLATOR_NAME_BYTES, MAX_TITLE_BYTES};
use ito_rs::models::manga::{
    Chapter, ContentRating, Manga, PageResult, Status as MangaStatus, Viewer,
};
use ito_rs::models::{ContentKind, Page, PageContent, Scanlator};
use ito_rs::{Error, Result};
use serde::Deserialize;

/// One page of a site listing, normalized by the bridge from the site's `{t9, zV}` envelope.
#[derive(Deserialize)]
pub struct Paged<T> {
    pub items: Vec<T>,
    pub meta: PageMeta,
}

#[derive(Deserialize)]
pub struct PageMeta {
    pub page: i32,
    pub last_page: i32,
}

impl Paged<ComixManga> {
    pub fn into_page_result(self, hidden_types: &[String]) -> PageResult {
        let has_next_page = self.meta.page < self.meta.last_page;
        PageResult {
            entries: visible(self.items, hidden_types),
            has_next_page,
        }
    }
}

/// Converts manga summaries and drops hidden entries. Top lists carry no page metadata.
pub fn visible(items: Vec<ComixManga>, hidden_types: &[String]) -> Vec<Manga> {
    items
        .into_iter()
        .filter(|manga| !manga.is_hidden(hidden_types))
        .map(Manga::from)
        .collect()
}

#[derive(Deserialize)]
pub struct ComixManga {
    pub hash_id: String,
    pub title: String,
    pub synopsis: Option<String>,
    pub r#type: String,
    pub poster: Option<Poster>,
    pub status: String,
    pub latest_chapter: Option<f32>,
    pub author: Option<Vec<Term>>,
    pub artist: Option<Vec<Term>>,
    pub genre: Option<Vec<Term>>,
}

impl ComixManga {
    pub fn is_hidden(&self, hidden_types: &[String]) -> bool {
        hidden_types.contains(&self.r#type)
    }
}

impl From<ComixManga> for Manga {
    fn from(value: ComixManga) -> Self {
        let url = format!("{}/title/{}", BASE_URL, value.hash_id);
        Self {
            key: value.hash_id,
            title: value.title,
            cover: value
                .poster
                .and_then(|poster| match settings::image_quality().as_str() {
                    "small" => poster.small.or(poster.medium).or(poster.large),
                    "medium" => poster.medium.or(poster.large).or(poster.small),
                    "large" => poster.large.or(poster.medium).or(poster.small),
                    _ => None,
                }),
            artist: value.artist.map(|v| {
                v.into_iter()
                    .map(|t| t.title)
                    .collect::<Vec<_>>()
                    .join(", ")
            }),
            authors: value
                .author
                .map(|v| v.into_iter().map(|t| t.title).collect()),
            description: value.synopsis,
            url: Some(url),
            tags: value
                .genre
                .map(|v| v.into_iter().map(|t| t.title).collect()),
            status: match value.status.as_str() {
                "releasing" => MangaStatus::Ongoing,
                "on_hiatus" => MangaStatus::Hiatus,
                "finished" => MangaStatus::Completed,
                "discontinued" => MangaStatus::Cancelled,
                _ => MangaStatus::Unknown,
            },
            // The site DTO exposes `contentRating` with no verified vocabulary, so no NSFW
            // flag is derived from it. Hidden genres are still excluded server-side.
            content_rating: ContentRating::Safe,
            nsfw: 0,
            viewer: match value.r#type.as_str() {
                "manhwa" => Viewer::Webtoon,
                "manhua" => Viewer::Webtoon,
                "manga" => Viewer::Rtl,
                _ => Viewer::Default,
            },
            chapters: None,
            catalog: None,
        }
    }
}

#[derive(Deserialize, Clone)]
pub struct ComixChapter {
    pub chapter_id: i32,
    pub scanlation_group_id: Option<i32>,
    pub number: f64,
    pub name: Option<String>,
    pub votes: i32,
    pub is_official: bool,
    pub scanlation_group: Option<ScanlationGroup>,
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
            url: Some(format!(
                "{}/title/{}/{}",
                BASE_URL, manga_id, self.chapter_id
            )),
            ..Chapter::new(self.chapter_id.to_string(), ContentKind::Images)
        }
    }

    /// The uploading group by display name only. No group id is emitted: a source-wide group
    /// identifier has not been verified for this site. A chapter without a usable group name is
    /// "Official" when the site flags it as official, and otherwise has no group.
    fn scanlator(&self) -> Option<Scanlator> {
        match self
            .scanlation_group
            .as_ref()
            .map(|group| group.name.as_str())
        {
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

/// Page images of one chapter. Each URL is already absolute: the bridge resolves it
/// against the page, exactly as the site's own image elements would.
#[derive(Deserialize)]
pub struct ChapterPages {
    pub images: Vec<PageImage>,
}

#[derive(Deserialize)]
pub struct PageImage {
    pub url: String,
}

impl ChapterPages {
    pub fn into_pages(self) -> Result<Vec<Page>> {
        self.images
            .into_iter()
            .enumerate()
            .map(|(index, image)| {
                if !(image.url.starts_with("https://") || image.url.starts_with("http://")) {
                    return Err(Error::Host("Comix page URL is not absolute".into()));
                }
                Ok(Page {
                    index: index as i32,
                    content: PageContent::Url(image.url),
                    has_description: false,
                    description: None,
                    headers: None,
                })
            })
            .collect()
    }
}

#[derive(Deserialize)]
pub struct Poster {
    pub small: Option<String>,
    pub medium: Option<String>,
    pub large: Option<String>,
}

#[derive(Deserialize)]
pub struct Term {
    pub title: String,
}

#[derive(Deserialize, Clone)]
pub struct ScanlationGroup {
    pub name: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paged_envelope_reports_next_page_from_page_metadata() {
        let page: Paged<ComixManga> =
            serde_json::from_str(r#"{"items":[],"meta":{"page":2,"last_page":3}}"#)
                .expect("normalized page decodes");
        assert!(page.into_page_result(&[]).has_next_page);
    }

    #[test]
    fn last_page_has_no_next_page() {
        let page: Paged<ComixManga> =
            serde_json::from_str(r#"{"items":[],"meta":{"page":3,"last_page":3}}"#)
                .expect("normalized page decodes");
        assert!(!page.into_page_result(&[]).has_next_page);
    }

    #[test]
    fn manga_summary_decodes_with_absent_detail_fields() {
        let manga: ComixManga = serde_json::from_str(
            r#"{"hash_id":"stable-key","title":"Fixture","type":"manga","status":"releasing","synopsis":null,"latest_chapter":null,"poster":null}"#,
        )
        .expect("summary decodes");
        let manga = Manga::from(manga);
        assert_eq!(manga.key, "stable-key");
        assert!(manga.authors.is_none());
    }

    #[test]
    fn required_manga_fields_are_not_defaulted() {
        assert!(
            serde_json::from_str::<ComixManga>(
                r#"{"hash_id":"stable-key","type":"manga","status":"releasing","poster":null}"#,
            )
            .is_err(),
            "a summary without a title must fail rather than default"
        );
    }

    #[test]
    fn chapter_requires_the_official_flag() {
        assert!(
            serde_json::from_str::<ComixChapter>(
                r#"{"chapter_id":1,"number":1.0,"name":null,"votes":0,"scanlation_group":null}"#,
            )
            .is_err(),
            "a chapter without isOfficial must fail rather than default"
        );
    }

    #[test]
    fn empty_chapter_name_is_not_a_title() {
        let chapter: ComixChapter = serde_json::from_str(
            r#"{"chapter_id":7,"scanlation_group_id":null,"number":2.0,"name":"","votes":0,"is_official":false,"scanlation_group":null}"#,
        )
        .expect("chapter decodes");
        assert!(chapter.into_chapter("hid").title.is_none());
    }

    #[test]
    fn absolute_page_urls_keep_their_order() {
        let pages: ChapterPages = serde_json::from_str(
            r#"{"images":[{"url":"https://cdn.test/1.jpg"},{"url":"https://cdn.test/2.jpg"}]}"#,
        )
        .expect("pages decode");
        let pages = pages.into_pages().expect("absolute URLs are accepted");
        assert_eq!(pages.len(), 2);
        assert!(matches!(
            &pages[1].content,
            PageContent::Url(url) if url == "https://cdn.test/2.jpg"
        ));
    }

    #[test]
    fn relative_page_urls_are_refused_by_the_host_side() {
        let pages: ChapterPages =
            serde_json::from_str(r#"{"images":[{"url":"/page.jpg"}]}"#).expect("pages decode");
        assert!(pages.into_pages().is_err());
    }

    #[test]
    fn null_or_absent_term_lists_decode_as_none() {
        for json in [
            r#"{"hash_id":"stable-key","title":"Fixture","type":"manga","status":"releasing","poster":null,"author":null,"artist":null,"genre":null}"#,
            r#"{"hash_id":"stable-key","title":"Fixture","type":"manga","status":"releasing","poster":null}"#,
        ] {
            let manga: ComixManga = serde_json::from_str(json).expect("detail term lists decode");
            assert!(manga.author.is_none());
            assert!(manga.artist.is_none());
            assert!(manga.genre.is_none());
        }
    }

    /// Representative normalized chapter page. Synthetic: shaped like the bridge's chapter output,
    /// not captured from the site. Pins the keys, titles, numbers, URLs and group names a port keeps.
    const CHAPTER_PAGE: &str = r#"{"items":[
        {"chapter_id":101,"scanlation_group_id":null,"number":1.0,"name":"Opening","votes":12,"is_official":true,"scanlation_group":null},
        {"chapter_id":102,"scanlation_group_id":9,"number":1.5,"name":"","votes":3,"is_official":false,"scanlation_group":{"name":"Group Nine"}},
        {"chapter_id":103,"scanlation_group_id":10702,"number":2.0,"name":null,"votes":0,"is_official":true,"scanlation_group":{"name":"Site Team"}},
        {"chapter_id":104,"scanlation_group_id":null,"number":10.25,"name":"Ten","votes":1,"is_official":false,"scanlation_group":null}
    ],"meta":{"page":1,"last_page":1}}"#;

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
        group: Option<&str>,
        is_official: bool,
    ) -> ComixChapter {
        ComixChapter {
            chapter_id: 7,
            scanlation_group_id: None,
            number,
            name,
            votes: 0,
            is_official,
            scanlation_group: group.map(|name| ScanlationGroup { name: name.into() }),
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
        let empty = chapter(1.0, None, Some(""), false).into_chapter("hid");
        assert!(empty.scanlator.is_none());
        let oversized_name = "g".repeat(MAX_SCANLATOR_NAME_BYTES + 1);
        let oversized = chapter(1.0, None, Some(oversized_name.as_str()), false);
        assert!(oversized.into_chapter("hid").scanlator.is_none());
        let official = chapter(1.0, None, Some(""), true).into_chapter("hid");
        assert_eq!(official.scanlator, Some(Scanlator::named("Official")),);
    }

    #[test]
    fn a_group_is_named_without_a_source_id() {
        let chapter = chapter(1.0, None, Some("Group Nine"), false).into_chapter("hid");
        assert_eq!(chapter.scanlator, Some(Scanlator::named("Group Nine")));
    }

    #[test]
    fn same_number_uploads_keep_distinct_chapter_ids_and_group_relationships() {
        let alpha = ComixChapter {
            chapter_id: 21,
            scanlation_group_id: Some(11),
            number: 4.0,
            name: None,
            votes: 0,
            is_official: false,
            scanlation_group: Some(ScanlationGroup {
                name: "Alpha".into(),
            }),
        };
        let beta = ComixChapter {
            chapter_id: 22,
            scanlation_group_id: Some(12),
            number: 4.0,
            name: None,
            votes: 0,
            is_official: false,
            scanlation_group: Some(ScanlationGroup {
                name: "Beta".into(),
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
            chapter_id: 31,
            scanlation_group_id: Some(11),
            number: 4.0,
            name: None,
            votes: 0,
            is_official: false,
            scanlation_group: Some(ScanlationGroup {
                name: "Alpha".into(),
            }),
        }
        .into_chapter("title");
        let second = ComixChapter {
            chapter_id: 32,
            scanlation_group_id: Some(11),
            number: adjacent,
            name: None,
            votes: 0,
            is_official: false,
            scanlation_group: Some(ScanlationGroup {
                name: "Alpha".into(),
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
