//! Response shapes of the site's public API. Fields the plugin doesn't use are not decoded.

use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct SearchResponse {
    pub hits: Vec<SearchHit>,
    pub found: usize,
}

#[derive(Debug, Deserialize)]
pub struct SearchHit {
    pub document: SearchDocument,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchDocument {
    pub id: String,
    #[serde(default)]
    pub title: String,
    pub english_title: Option<String>,
    pub poster: Option<String>,
    pub medium: Option<String>,
    #[serde(rename = "type")]
    pub origin: Option<String>,
    pub is_adult: Option<bool>,
    pub mb_content_rating: Option<String>,
    pub status: Option<String>,
    pub authors: Option<Vec<String>>,
}

#[derive(Debug, Deserialize)]
pub struct FeedResponse {
    pub items: Vec<FeedItem>,
}

/// An entry of a ranked feed; feeds carry no rating or status.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FeedItem {
    pub id: String,
    pub title: String,
    pub image: Option<String>,
    pub medium: Option<String>,
    #[serde(rename = "type")]
    pub origin: Option<String>,
    pub is_adult: Option<bool>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SeriesResponse {
    pub manga_page: Series,
}

/// A series page; it has no content rating, only the adult flag.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Series {
    pub id: String,
    #[serde(default)]
    pub title: String,
    pub english_title: Option<String>,
    pub medium: Option<String>,
    #[serde(rename = "type")]
    pub origin: Option<String>,
    pub is_adult: Option<bool>,
    pub status: Option<String>,
    pub synopsis: Option<String>,
    pub poster: Option<Poster>,
    #[serde(default)]
    pub genres: Vec<Named>,
    #[serde(default)]
    pub authors: Vec<Creator>,
    #[serde(default)]
    pub scanlators: Vec<Scanlator>,
}

#[derive(Debug, Deserialize)]
pub struct Poster {
    pub image: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct Named {
    pub name: String,
}

#[derive(Debug, Deserialize)]
pub struct Creator {
    pub name: String,
    #[serde(rename = "type")]
    pub role: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct Scanlator {
    pub id: String,
    pub name: String,
    /// Net community votes for the group on this series.
    pub score: Option<f64>,
}

#[derive(Debug, Deserialize)]
pub struct ChapterListResponse {
    pub chapters: Vec<ChapterItem>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChapterItem {
    pub id: String,
    pub scanlation_manga_id: Option<String>,
    pub number: Option<f64>,
    pub title: Option<String>,
    /// Upload time in milliseconds since the epoch.
    pub created_at: Option<i64>,
    pub page_count: Option<u32>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReadChapterResponse {
    pub read_chapter: ReadChapter,
}

#[derive(Debug, Deserialize)]
pub struct ReadChapter {
    pub pages: Vec<ChapterPage>,
}

#[derive(Debug, Deserialize)]
pub struct ChapterPage {
    pub image: String,
    pub number: i64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NovelChapterResponse {
    pub read_novel_chapter: NovelChapter,
}

#[derive(Debug, Deserialize)]
pub struct NovelChapter {
    pub paragraphs: Vec<String>,
}

/// Genre and tag names with their ids, from `/api/explore/availableFilters`.
#[derive(Debug, Deserialize)]
pub struct Taxonomy {
    pub genres: Vec<Term>,
    pub tags: Vec<Term>,
}

#[derive(Debug, Deserialize)]
pub struct Term {
    pub id: String,
    pub name: String,
    pub group: Option<String>,
}
