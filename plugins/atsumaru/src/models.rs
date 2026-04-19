use serde::Deserialize;

#[derive(Deserialize, Debug, Clone)]
pub struct SearchResponse {
    pub hits: Vec<SearchHit>,
    pub found: i32,
    pub page: i32,
}

#[derive(Deserialize, Debug, Clone)]
pub struct SearchHit {
    pub document: SearchDocument,
}

#[derive(Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct SearchDocument {
    pub id: Option<String>,
    pub title: Option<String>,
    pub poster: Option<String>, // the Typesense search uses string URL
    pub status: Option<String>,
    pub synopsis: Option<String>,
    pub tags: Option<Vec<String>>,
    pub authors: Option<Vec<String>>,
}

#[derive(Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ChapterPageResponse {
    pub read_chapter: ReadChapter,
}

#[allow(dead_code)]
#[derive(Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ReadChapter {
    pub id: String,
    pub title: Option<String>,
    pub pages: Vec<ChapterPage>,
}

#[allow(dead_code)]
#[derive(Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ChapterPage {
    pub id: String,
    pub image: String,
    pub number: i32,
}

#[derive(Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct MangaPageWrapper {
    pub manga_page: MangaPageDetail,
}

#[allow(dead_code)]
#[derive(Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct MangaPageDetail {
    pub id: Option<String>,
    pub title: Option<String>,
    pub english_title: Option<String>,
    pub poster: Option<ImageAsset>,     
    pub banner: Option<ImageAsset>,
    pub status: Option<String>,
    pub synopsis: Option<String>,
    pub scanlators: Option<Vec<Scanlator>>,
    pub genres: Option<Vec<Entity>>,
    pub authors: Option<Vec<Entity>>,
}

#[derive(Deserialize, Debug, Clone)]
pub struct ImageAsset {
    pub image: Option<String>,
}

#[allow(dead_code)]
#[derive(Deserialize, Debug, Clone)]
pub struct Entity {
    pub id: Option<String>,
    pub name: Option<String>,
}

#[derive(Deserialize, Debug, Clone)]
pub struct Scanlator {
    pub id: String,
    pub name: String,
}

#[derive(Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ChapterListResponse {
    pub chapters: Vec<ChapterItem>,
}

#[derive(Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ChapterItem {
    pub id: String,
    pub title: Option<String>,
    pub number: f32,
    pub created_at: i64,
    pub scanlation_manga_id: Option<String>,
}

#[derive(serde::Deserialize, Debug, Clone)]
pub struct InfiniteResponse {
    pub items: Vec<InfiniteItem>,
}

#[derive(serde::Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct InfiniteItem {
    pub id: String,
    pub title: String,
    pub image: Option<String>,
    pub small_image: Option<String>,
    pub medium_image: Option<String>,
    pub large_image: Option<String>,
    pub is_adult: Option<bool>,
    pub r#type: Option<String>,
}
