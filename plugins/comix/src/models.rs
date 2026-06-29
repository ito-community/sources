use crate::{BASE_URL, settings};
use ito_rs::models::manga::{Chapter, Manga, PageResult, Status as MangaStatus, Viewer, ContentRating};
use ito_rs::models::{Page, PageContent};
use serde::{Deserialize, Deserializer, de};

#[derive(Deserialize)]
pub struct SearchResponse {
	pub result: MangaItemsResult,
}

#[derive(Deserialize)]
#[serde(untagged)]
pub enum MangaItemsResult {
	Object(MangaItems),
	Array(Vec<ComixManga>),
}

impl MangaItemsResult {
	pub fn into_items(self) -> Vec<ComixManga> {
		match self {
			Self::Object(o) => o.items,
			Self::Array(a) => a,
		}
	}

	pub fn into_filtered(self, content_types: &[String], hidden_terms: &[i32]) -> PageResult {
		match self {
			Self::Object(o) => o.into_filtered(content_types, hidden_terms),
			Self::Array(a) => PageResult {
				entries: a.into_iter().filter(|m| !m.is_hidden(content_types, hidden_terms)).map(Into::into).collect(),
				has_next_page: false,
			},
		}
	}
}

#[derive(Deserialize)]
pub struct SingleMangaResponse {
	pub result: ComixManga,
}

#[derive(Deserialize)]
pub struct ChapterDetailsResponse {
	pub result: ChapterItems,
}

#[derive(Deserialize)]
pub struct ChapterResponse {
	pub result: Option<ComixChapterWithImages>,
}

#[derive(Deserialize)]
pub struct TermResponse {
	pub result: TermItems,
}

#[derive(Deserialize)]
pub struct Pagination {
	pub current_page: i32,
	pub last_page: i32,
}

#[derive(Deserialize)]
pub struct MangaItems {
	pub items: Vec<ComixManga>,
	pub pagination: Option<Pagination>,
}

impl MangaItems {
	pub fn into_filtered(self, content_types: &[String], hidden_terms: &[i32]) -> PageResult {
		PageResult {
			entries: self
				.items
				.into_iter()
				.filter(|m| !m.is_hidden(content_types, hidden_terms))
				.map(Into::into)
				.collect(),
			has_next_page: self
				.pagination
				.map(|p| p.current_page < p.last_page)
				.unwrap_or_default(),
		}
	}
}

impl From<MangaItems> for PageResult {
	fn from(value: MangaItems) -> Self {
		PageResult {
			entries: value.items.into_iter().map(Into::into).collect(),
			has_next_page: value
				.pagination
				.map(|p| p.current_page < p.last_page)
				.unwrap_or_default(),
		}
	}
}

#[derive(Deserialize)]
pub struct ChapterItems {
	pub items: Vec<ComixChapter>,
	#[serde(alias = "pagination")]
	pub meta: ChapterMeta,
}

#[derive(Deserialize)]
pub struct ChapterMeta {
	pub page: i32,
	#[serde(rename = "lastPage", alias = "last_page")]
	pub last_page: i32,
}

#[derive(Deserialize)]
pub struct TermItems {
	pub items: Vec<Term>,
}

#[derive(Deserialize)]
pub struct ComixManga {
	#[serde(alias = "hid")]
	pub hash_id: String,
	pub title: String,
	pub synopsis: Option<String>,
	#[serde(default)]
	pub r#type: String,
	pub poster: Poster,
	#[serde(default)]
	pub status: String,
	#[serde(default)]
	pub is_nsfw: bool,
	pub author: Option<Vec<Term>>,
	pub artist: Option<Vec<Term>>,
	pub genre: Option<Vec<Term>>,
	pub latest_chapter: Option<f32>,
	pub chapter_updated_at: Option<i64>,
	pub term_ids: Option<Vec<i32>>,
}

impl ComixManga {
	pub fn is_hidden(&self, hidden_types: &[String], hidden_terms: &[i32]) -> bool {
		if hidden_types.contains(&self.r#type) {
			return true;
		}

		if !hidden_terms.is_empty() {
			self.term_ids
				.as_ref()
				.map(|ids| ids.iter().any(|id| hidden_terms.contains(id)))
				.unwrap_or_default()
		} else {
			false
		}
	}
}

impl From<ComixManga> for Manga {
	fn from(value: ComixManga) -> Self {
		let url = format!("{}/title/{}", BASE_URL, value.hash_id);
		Self {
			key: value.hash_id,
			title: value.title,
			cover: match settings::image_quality().as_str() {
				"small" => value.poster.small.clone().or_else(|| value.poster.medium.clone()).or_else(|| value.poster.large.clone()),
				"medium" => value.poster.medium.clone().or_else(|| value.poster.large.clone()).or_else(|| value.poster.small.clone()),
				"large" => value.poster.large.clone().or_else(|| value.poster.medium.clone()).or_else(|| value.poster.small.clone()),
				_ => None,
			},
			artist: value
				.artist
				.map(|v| v.into_iter().map(|t| t.title).collect::<Vec<_>>().join(", ")),
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
			content_rating: if value.is_nsfw {
				ContentRating::Nsfw
			} else {
				ContentRating::Safe
			},
            nsfw: if value.is_nsfw { 1 } else { 0 },
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
	#[serde(alias = "id")]
	pub chapter_id: i32,
	#[serde(alias = "groupId")]
	pub scanlation_group_id: Option<i32>,
	pub number: f32,
	#[serde(default)]
	pub name: String,
	#[serde(default)]
	pub votes: i32,
	#[serde(default)]
	pub updated_at: i64,
	#[serde(alias = "group")]
	pub scanlation_group: Option<ScanlationGroup>,
	#[serde(alias = "isOfficial", deserialize_with = "bool_from_any")]
	pub is_official: bool,
}

impl ComixChapter {
	pub fn into_chapter(self, manga_id: &str) -> Chapter {
		Chapter {
			key: self.chapter_id.to_string(),
			title: (!self.name.is_empty()).then_some(self.name),
			chapter: Some(self.number),
            volume: None,
			date_updated: Some(self.updated_at as f64),
			scanlator: if let Some(scanlation_group) = self.scanlation_group {
				Some(scanlation_group.name)
			} else if self.is_official {
				Some("Official".into())
			} else {
				None
			},
			url: Some(format!("{}/title/{}/{}", BASE_URL, manga_id, self.chapter_id)),
			lang: None,
            paywalled: None,
		}
	}
}

#[derive(Deserialize)]
pub struct ComixChapterWithImages {
	pub images: Option<Vec<Image>>,
	#[serde(alias = "pages")]
	pub pages: Option<PageItems>,
}

#[derive(Deserialize)]
pub struct PageItems {
	pub items: Vec<Image>,
}

impl ComixChapterWithImages {
	pub fn get_images(self) -> Vec<Image> {
		if let Some(p) = self.pages {
			p.items
		} else {
			self.images.unwrap_or_default()
		}
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
	pub term_id: i32,
	pub title: String,
}

#[derive(Deserialize, Clone)]
pub struct ScanlationGroup {
	pub name: String,
}

#[derive(Deserialize)]
pub struct Image {
	pub url: String,
	pub s: Option<i32>,
	pub width: f32,
	pub height: f32,
}

impl Image {
	pub fn into_page(self, index: i32) -> Page {
		Page {
            index,
			content: PageContent::Url(self.url),
			has_description: false,
            description: None,
            headers: None,
		}
	}
}

// deserialize a bool from a json bool, number, or string
fn bool_from_any<'de, D: Deserializer<'de>>(deserializer: D) -> Result<bool, D::Error> {
	struct BoolVisitor;

	impl<'de> de::Visitor<'de> for BoolVisitor {
		type Value = bool;

		fn expecting(&self, formatter: &mut core::fmt::Formatter) -> core::fmt::Result {
			formatter.write_str("a boolean or 0/1")
		}

		fn visit_bool<E>(self, v: bool) -> Result<bool, E> {
			Ok(v)
		}

		fn visit_u64<E>(self, v: u64) -> Result<bool, E> {
			match v {
				0 => Ok(false),
				_ => Ok(true),
			}
		}

		fn visit_i64<E>(self, v: i64) -> Result<bool, E> {
			match v {
				0 => Ok(false),
				_ => Ok(true),
			}
		}

		fn visit_str<E: de::Error>(self, v: &str) -> Result<bool, E> {
			match v.to_ascii_lowercase().as_str() {
				"true" => Ok(true),
				"false" => Ok(false),
				"1" => Ok(true),
				"0" => Ok(false),
				_ => Err(E::custom(format!("invalid string for bool: {v}"))),
			}
		}

		fn visit_none<E>(self) -> Result<bool, E> {
			Ok(false)
		}
	}

	deserializer.deserialize_any(BoolVisitor)
}
