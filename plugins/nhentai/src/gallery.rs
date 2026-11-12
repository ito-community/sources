use crate::api::{Gallery, ListItem, PageImage, Tag};
use crate::images::{self, ImageKind};
use crate::query::{self, is_blocked_id, is_blocked_name};
use ito_rs::models::manga::{ContentRating, Manga, Status, Viewer};
use ito_rs::models::{
    CatalogCompleteness, CatalogScope, Chapter, ChapterCatalog, ContentKind, Page, PageContent,
    Scanlator,
};
use ito_rs::{Error, Result};

const BASE: &str = "https://nhentai.net";
const MAX_TITLE: usize = 1_024;
const MAX_TAGS: usize = 40;
const MAX_NAMES: usize = 16;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TitlePreference {
    English,
    Japanese,
    Pretty,
}

impl TitlePreference {
    pub fn parse(value: Option<&str>) -> Self {
        match value {
            Some("japanese") => Self::Japanese,
            Some("pretty") => Self::Pretty,
            _ => Self::English,
        }
    }
}

pub struct View {
    pub title: TitlePreference,
    pub server: u8,
}

pub fn list_item(item: &ListItem, view: &View) -> Option<Manga> {
    if item.blacklisted || item.id <= 0 || item.tag_ids.iter().copied().any(is_blocked_id) {
        return None;
    }
    let mut manga = card(
        item.id,
        &list_title(item, view.title),
        images::url(&item.thumbnail, view.server, ImageKind::Thumb),
        None,
        None,
        None,
        Viewer::Rtl,
    );
    manga.description = list_summary(item);
    Some(manga)
}

pub fn detail(gallery: &Gallery, view: &View) -> Result<Manga> {
    if blocked(gallery) {
        return Err(unavailable());
    }
    let title = gallery_title(gallery, view.title);
    let mut manga = card(
        gallery.id,
        &title,
        images::url(&gallery.cover.path, view.server, ImageKind::Thumb),
        Some(authors(gallery)),
        artist(gallery),
        Some(tags(gallery)),
        viewer(gallery),
    );
    manga.description = Some(describe(gallery, &title));
    let chapter = chapter(gallery);
    manga.chapters = Some(vec![chapter]);
    manga.catalog = Some(ChapterCatalog {
        scope: CatalogScope::Visible,
        completeness: CatalogCompleteness::Complete,
    });
    Ok(manga)
}

pub fn pages(gallery: &Gallery, server: u8) -> Result<Vec<Page>> {
    if blocked(gallery) {
        return Err(unavailable());
    }
    let mut images = gallery.pages.clone();
    images.sort_by_key(|page| page.number);
    let pages = images
        .iter()
        .filter_map(|page| page_url(page, server))
        .enumerate()
        .map(|(index, url)| Page {
            index: index as i32,
            content: PageContent::Url(url),
            has_description: false,
            description: None,
            headers: Some(images::headers()),
        })
        .collect::<Vec<_>>();
    if pages.is_empty() {
        return Err(Error::Server("This gallery has no pages.".into()));
    }
    Ok(pages)
}

fn blocked(gallery: &Gallery) -> bool {
    gallery
        .tags
        .iter()
        .any(|tag| is_blocked_name(&tag.name) || is_blocked_id(tag.id))
}

fn unavailable() -> Error {
    Error::NotFound("This gallery is not available in this source.".into())
}

fn card(
    id: i64,
    title: &str,
    cover: Option<String>,
    authors: Option<Vec<String>>,
    artist: Option<String>,
    tags: Option<Vec<String>>,
    viewer: Viewer,
) -> Manga {
    Manga {
        key: id.to_string(),
        title: bound(title, MAX_TITLE),
        authors,
        artist,
        description: None,
        tags,
        cover,
        url: Some(gallery_url(id)),
        status: Status::Completed,
        content_rating: ContentRating::Nsfw,
        nsfw: 1,
        viewer,
        chapters: None,
        catalog: None,
    }
}

fn list_title(item: &ListItem, preference: TitlePreference) -> String {
    let english = item.english_title.trim();
    let japanese = item.japanese_title.as_deref().unwrap_or("").trim();
    let chosen = match preference {
        TitlePreference::Japanese if !japanese.is_empty() => japanese,
        _ if !english.is_empty() => english,
        _ if !japanese.is_empty() => japanese,
        _ => return format!("#{}", item.id),
    };
    chosen.to_owned()
}

fn list_summary(item: &ListItem) -> Option<String> {
    let mut parts = Vec::new();
    if item.num_pages > 0 {
        let label = if item.num_pages == 1 { "page" } else { "pages" };
        parts.push(format!("{} {label}", item.num_pages));
    }
    if item.num_favorites > 0 {
        parts.push(format!("{} favorites", item.num_favorites));
    }
    (!parts.is_empty()).then(|| parts.join(" · "))
}

fn gallery_title(gallery: &Gallery, preference: TitlePreference) -> String {
    let english = gallery.title.english.trim();
    let japanese = gallery.title.japanese.as_deref().unwrap_or("").trim();
    let pretty = gallery.title.pretty.trim();
    let chosen = match preference {
        TitlePreference::Japanese if !japanese.is_empty() => japanese,
        TitlePreference::Pretty if !pretty.is_empty() => pretty,
        _ if !english.is_empty() => english,
        _ if !pretty.is_empty() => pretty,
        _ if !japanese.is_empty() => japanese,
        _ => return format!("#{}", gallery.id),
    };
    chosen.to_owned()
}

fn chapter(gallery: &Gallery) -> Chapter {
    let title = match gallery.num_pages {
        1 => Some("1 page".into()),
        n if n > 1 => Some(format!("{n} pages")),
        _ => None,
    };
    Chapter {
        title,
        chapter: Some(1.0),
        date_updated: date(gallery.upload_date),
        scanlator: scanlator(&gallery.scanlator),
        url: Some(gallery_url(gallery.id)),
        lang: language_code(&gallery.tags),
        reading_order: Some(0),
        ..Chapter::new(gallery.id.to_string(), ContentKind::Images)
    }
}

fn page_url(page: &PageImage, server: u8) -> Option<String> {
    images::url(&page.path, server, ImageKind::Page)
}

fn gallery_url(id: i64) -> String {
    format!("{BASE}/g/{id}")
}

fn date(upload: i64) -> Option<f64> {
    let date = upload as f64;
    (date.is_finite() && date.abs() <= 1e11).then_some(date)
}

fn scanlator(name: &str) -> Option<Scanlator> {
    let name = name.trim();
    (!name.is_empty()).then(|| Scanlator::named(name))
}

fn language_code(tags: &[Tag]) -> Option<String> {
    let mut languages: Vec<&Tag> = tags
        .iter()
        .filter(|tag| tag.kind == "language" && !query::is_meta_language(&tag.name))
        .collect();
    languages.sort_by_key(|tag| std::cmp::Reverse(tag.count));
    let name = languages.first()?.name.as_str();
    let code = match name {
        "english" => "en",
        "japanese" => "ja",
        "chinese" => "zh",
        "korean" => "ko",
        "spanish" => "es",
        "french" => "fr",
        "russian" => "ru",
        "portuguese" => "pt",
        "german" => "de",
        "italian" => "it",
        "thai" => "th",
        "vietnamese" => "vi",
        "indonesian" => "id",
        "polish" => "pl",
        "hungarian" => "hu",
        "arabic" => "ar",
        "czech" => "cs",
        "turkish" => "tr",
        "ukrainian" => "uk",
        other if other.len() <= 35 && !other.contains('|') => other,
        _ => return None,
    };
    Some(code.to_owned())
}

fn viewer(gallery: &Gallery) -> Viewer {
    if gallery
        .tags
        .iter()
        .any(|tag| tag.name.eq_ignore_ascii_case("webtoon"))
    {
        Viewer::Webtoon
    } else {
        Viewer::Rtl
    }
}

fn authors(gallery: &Gallery) -> Vec<String> {
    named(gallery, &["group", "artist"])
}

fn artist(gallery: &Gallery) -> Option<String> {
    gallery
        .tags
        .iter()
        .filter(|tag| tag.kind == "artist")
        .max_by_key(|tag| tag.count)
        .map(|tag| tag.name.clone())
}

fn tags(gallery: &Gallery) -> Vec<String> {
    let mut tags: Vec<&Tag> = gallery
        .tags
        .iter()
        .filter(|tag| tag.kind == "tag" && !is_blocked_name(&tag.name))
        .collect();
    tags.sort_by_key(|tag| std::cmp::Reverse(tag.count));
    tags.into_iter()
        .take(MAX_TAGS)
        .map(|tag| tag.name.clone())
        .collect()
}

fn named(gallery: &Gallery, kinds: &[&str]) -> Vec<String> {
    let mut names = Vec::new();
    for kind in kinds {
        let mut tags: Vec<&Tag> = gallery
            .tags
            .iter()
            .filter(|tag| tag.kind == *kind && !is_blocked_name(&tag.name))
            .collect();
        tags.sort_by_key(|tag| std::cmp::Reverse(tag.count));
        for tag in tags {
            if names.len() == MAX_NAMES {
                return names;
            }
            if names
                .iter()
                .any(|name: &String| name.eq_ignore_ascii_case(&tag.name))
            {
                continue;
            }
            names.push(tag.name.clone());
        }
    }
    names
}

fn describe(gallery: &Gallery, title: &str) -> String {
    let mut lines = Vec::new();
    push_alt(&mut lines, "English", gallery.title.english.trim(), title);
    push_alt(
        &mut lines,
        "Japanese",
        gallery.title.japanese.as_deref().unwrap_or("").trim(),
        title,
    );
    push_alt(&mut lines, "Pretty", gallery.title.pretty.trim(), title);
    if gallery.num_pages > 0 {
        lines.push(format!("Pages: {}", gallery.num_pages));
    }
    if gallery.num_favorites > 0 {
        lines.push(format!("Favorites: {}", gallery.num_favorites));
    }
    push_line(&mut lines, "Parodies", &named(gallery, &["parody"]), 8);
    push_line(
        &mut lines,
        "Characters",
        &named(gallery, &["character"]),
        12,
    );
    push_line(&mut lines, "Categories", &named(gallery, &["category"]), 4);
    push_line(&mut lines, "Languages", &language_names(gallery), 4);
    lines.join("\n")
}

fn language_names(gallery: &Gallery) -> Vec<String> {
    let mut tags: Vec<&Tag> = gallery
        .tags
        .iter()
        .filter(|tag| tag.kind == "language" && !query::is_meta_language(&tag.name))
        .collect();
    tags.sort_by_key(|tag| std::cmp::Reverse(tag.count));
    tags.into_iter().map(|tag| tag.name.clone()).collect()
}

fn push_alt(lines: &mut Vec<String>, label: &str, value: &str, title: &str) {
    if !value.is_empty() && value != title {
        lines.push(format!("{label}: {value}"));
    }
}

fn push_line(lines: &mut Vec<String>, label: &str, names: &[String], max: usize) {
    if names.is_empty() {
        return;
    }
    let shown = names
        .iter()
        .take(max)
        .cloned()
        .collect::<Vec<_>>()
        .join(", ");
    lines.push(format!("{label}: {shown}"));
}

fn bound(value: &str, max: usize) -> String {
    if value.len() <= max {
        return value.to_owned();
    }
    let mut end = max;
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    value[..end].to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn view() -> View {
        View {
            title: TitlePreference::English,
            server: 1,
        }
    }

    fn gallery() -> Gallery {
        serde_json::from_value(json!({
            "id": 42,
            "title": {"english": "English title", "japanese": "日本語", "pretty": "Pretty"},
            "cover": {"path": "galleries/42/cover.webp"},
            "scanlator": "Circle",
            "upload_date": 1_700_000_000,
            "num_pages": 3,
            "num_favorites": 9,
            "tags": [
                {"id": 1, "type": "language", "name": "english", "count": 10},
                {"id": 2, "type": "language", "name": "translated", "count": 99},
                {"id": 3, "type": "artist", "name": "Artist", "count": 4},
                {"id": 4, "type": "group", "name": "Group", "count": 5},
                {"id": 5, "type": "parody", "name": "original", "count": 1},
                {"id": 6, "type": "tag", "name": "webtoon", "count": 2},
                {"id": 7, "type": "tag", "name": "vanilla", "count": 8},
                {"id": 8, "type": "category", "name": "doujinshi", "count": 3}
            ],
            "pages": [
                {"number": 2, "path": "galleries/42/2.webp"},
                {"number": 1, "path": "galleries/42/1.webp"}
            ]
        }))
        .expect("fixture")
    }

    #[test]
    fn detail_uses_the_preferred_title_and_a_complete_single_chapter() {
        let mut view = view();
        view.title = TitlePreference::Pretty;
        let manga = detail(&gallery(), &view).expect("gallery");
        assert_eq!(manga.title, "Pretty");
        assert_eq!(manga.key, "42");
        assert!(matches!(manga.content_rating, ContentRating::Nsfw));
        assert_eq!(manga.nsfw, 1);
        assert!(matches!(manga.viewer, Viewer::Webtoon));
        assert_eq!(manga.artist.as_deref(), Some("Artist"));
        assert_eq!(
            manga.authors.as_deref(),
            Some(["Group".to_string(), "Artist".into()].as_slice())
        );
        assert_eq!(
            manga.tags.as_deref(),
            Some(["vanilla".to_string(), "webtoon".into()].as_slice())
        );
        assert!(
            manga
                .description
                .as_deref()
                .unwrap()
                .contains("English: English title")
        );
        let chapters = manga.chapters.expect("chapter");
        assert_eq!(chapters.len(), 1);
        assert_eq!(chapters[0].key, "42");
        assert_eq!(chapters[0].chapter, Some(1.0));
        assert_eq!(chapters[0].lang.as_deref(), Some("en"));
        assert_eq!(
            chapters[0]
                .scanlator
                .as_ref()
                .map(|group| group.name.as_str()),
            Some("Circle")
        );
        assert_eq!(
            manga.catalog.map(|catalog| catalog.completeness),
            Some(CatalogCompleteness::Complete)
        );
        let pages = pages(&gallery(), 1).expect("pages");
        assert_eq!(pages.len(), 2);
        assert!(matches!(
            &pages[0].content,
            PageContent::Url(url) if url == "https://i1.nhentai.net/galleries/42/1.webp"
        ));
    }

    #[test]
    fn blocked_galleries_and_list_rows_are_dropped() {
        let mut gallery = gallery();
        gallery.tags.push(Tag {
            id: 19_440,
            kind: "tag".into(),
            name: "lolicon".into(),
            count: 1,
        });
        assert!(detail(&gallery, &view()).is_err());
        assert!(pages(&gallery, 1).is_err());
        let item = ListItem {
            id: 7,
            english_title: "Listed".into(),
            japanese_title: None,
            thumbnail: "galleries/7/thumb.webp".into(),
            num_pages: 1,
            num_favorites: 0,
            tag_ids: vec![32_341],
            blacklisted: false,
        };
        assert!(list_item(&item, &view()).is_none());
    }
}
