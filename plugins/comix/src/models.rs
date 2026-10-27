use crate::{BASE_URL, settings};
use ito_rs::models::manga::{
    Chapter, ContentRating, Manga, PageResult, Status as MangaStatus, Viewer,
};
use ito_rs::models::{Page, PageContent};
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
        }
    }
}

#[derive(Deserialize, Clone)]
pub struct ComixChapter {
    pub chapter_id: i32,
    pub scanlation_group_id: Option<i32>,
    pub number: f32,
    pub name: Option<String>,
    pub votes: i32,
    pub is_official: bool,
    pub scanlation_group: Option<ScanlationGroup>,
}

impl ComixChapter {
    pub fn into_chapter(self, manga_id: &str) -> Chapter {
        Chapter {
            key: self.chapter_id.to_string(),
            title: self.name.filter(|name| !name.is_empty()),
            chapter: Some(self.number),
            volume: None,
            date_updated: None,
            scanlator: if let Some(scanlation_group) = self.scanlation_group {
                Some(scanlation_group.name)
            } else if self.is_official {
                Some("Official".into())
            } else {
                None
            },
            url: Some(format!(
                "{}/title/{}/{}",
                BASE_URL, manga_id, self.chapter_id
            )),
            lang: None,
            paywalled: None,
        }
    }
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
}
