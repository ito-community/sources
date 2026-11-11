use ito_rs::export_manga_plugin;
use ito_rs::provider::MangaProvider;
use ito_rs::models::{ContentKind, Listing, FilterItem, Page, PageContent, HomeLayout, HomeComponent, HomeComponentValue};
use ito_rs::models::manga::{Manga, Chapter, PageResult, Status, ContentRating, Viewer};
use ito_rs::net::Request;
use ito_rs::html::Node;
use ito_rs::Result;
use ito_rs::catalog::{MAX_KEY_BYTES, MAX_TITLE_BYTES};
use std::collections::HashSet;

const BASE_URL: &str = "https://violetmanga.com";

/// One row of the site's chapter list as read from the page, newest first.
struct ChapterRow {
    href: String,
    title: Option<String>,
}

/// Maps the chapter list into chapters. A chapter's number counts down from the number of rows,
/// so it depends only on its place in the list. A row whose key would be empty, longer than the
/// SDK allows or already listed is left out, since the SDK rejects a whole list that carries one
/// of those; the rows left out still count, so the other numbers do not move.
fn chapters_from_rows(rows: Vec<ChapterRow>) -> Vec<Chapter> {
    let mut chapter_num = rows.len() as f64;
    let mut listed = HashSet::with_capacity(rows.len());
    let mut chapters = Vec::with_capacity(rows.len());
    for row in rows {
        let key = row.href.replace(BASE_URL, "").replace("/", "");
        if !key.is_empty() && key.len() <= MAX_KEY_BYTES && listed.insert(key.clone()) {
            chapters.push(Chapter {
                title: row.title.filter(|title| title.len() <= MAX_TITLE_BYTES),
                url: Some(row.href),
                chapter: Some(chapter_num),
                lang: Some("en".to_string()),
                ..Chapter::new(key, ContentKind::Images)
            });
        }
        chapter_num -= 1.0;
    }
    chapters
}

struct VioletScans;

impl VioletScans {
    fn urlencode(s: &str) -> String {
        let mut encoded = String::new();
        for b in s.as_bytes() {
            match *b as char {
                'A'..='Z' | 'a'..='z' | '0'..='9' | '-' | '_' | '.' | '~' => encoded.push(*b as char),
                ' ' => encoded.push('+'),
                _ => encoded.push_str(&format!("%{:02X}", b)),
            }
        }
        encoded
    }

    fn parse_manga_list(html: &Node) -> Result<PageResult> {
        let mut entries = Vec::new();
        
        let has_next_page = html.select("a.next").ok().and_then(|nodes| nodes.first().map(|_| true)).unwrap_or(false);

        if let Ok(items) = html.select(".bsx") {
            for item in items {
                if let Ok(a_nodes) = item.select("a")
                    && let Some(a_node) = a_nodes.first() {
                        let url = a_node.attr("href")?.unwrap_or_default();
                        let mut title = a_node.attr("title")?.unwrap_or_default();
                        
                        if title.is_empty()
                            && let Ok(tt_nodes) = item.select(".tt")
                                && let Some(tt) = tt_nodes.first() {
                                    title = tt.text()?.trim().to_string();
                                }
                        
                        let mut cover = String::new();
                        if let Ok(img_nodes) = item.select("img")
                            && let Some(img) = img_nodes.first() {
                                cover = img.attr("src")?.unwrap_or_default();
                                if cover.is_empty() {
                                    cover = img.attr("data-src")?.unwrap_or_default();
                                }
                            }

                        if !url.is_empty() {
                            let key = url.replace(BASE_URL, "").replace("/comics/", "").replace("/manga/", "").replace("/", "");
                            entries.push(Manga {
                                key,
                                title,
                                cover: if cover.is_empty() { None } else { Some(cover) },
                                url: Some(url),
                                authors: None,
                                artist: None,
                                description: None,
                                tags: None,
                                status: Status::Unknown,
                                content_rating: ContentRating::Safe,
                                nsfw: 0,
                                viewer: Viewer::Default,
                                chapters: None,
                                catalog: None,
                            });
                        }
                    }
            }
        }

        Ok(PageResult { entries, has_next_page })
    }
}

impl MangaProvider for VioletScans {
    fn get_search_manga_list(
        query: &str,
        page: i32,
        _filters: Vec<FilterItem>,
    ) -> Result<PageResult> {
        if !query.is_empty() {
            let url = format!("{}/?s={}", BASE_URL, Self::urlencode(query));
            let res = Request::get(&url).send()?;
            let html = Node::new(&res.body);
            Self::parse_manga_list(&html)
        } else {
            let url = format!("{}/comics/?page={}&order=update", BASE_URL, page);
            let res = Request::get(&url).send()?;
            let html = Node::new(&res.body);
            Self::parse_manga_list(&html)
        }
    }

    fn get_manga_list(listing: Listing, page: i32) -> Result<PageResult> {
        let order = match listing.name.as_str() {
            "Latest Update" | "Latest Updates" => "update",
            "Editor's Choice" | "Trending Daily" | "Trending Weekly" | "Trending All Time" | "Popular Today" => "popular",
            _ => "update",
        };
        let url = format!("{}/comics/?page={}&order={}", BASE_URL, page, order);
        let res = Request::get(&url).send()?;
        let html = Node::new(&res.body);
        Self::parse_manga_list(&html)
    }

    fn get_manga_update(
        mut manga: Manga,
        needs_details: bool,
        needs_chapters: bool,
    ) -> Result<Manga> {
        let url = format!("{}/comics/{}/", BASE_URL, manga.key);
        let res = Request::get(&url).send()?;
        let html = Node::new(&res.body);
        
        if needs_details {
            if let Ok(nodes) = html.select("h1.entry-title")
                && let Some(node) = nodes.first() {
                    manga.title = node.text()?;
                }
            
            if let Ok(nodes) = html.select(".thumb img")
                && let Some(node) = nodes.first()
                    && let Ok(Some(src)) = node.attr("src") {
                        manga.cover = Some(src);
                    }
            
            if let Ok(nodes) = html.select(".entry-content p")
                && let Some(node) = nodes.first() {
                    manga.description = Some(node.text()?);
                }
            
            manga.url = Some(url.clone());
            manga.viewer = Viewer::Webtoon;
            
            if let Ok(items) = html.select(".imptdt") {
                for item in items {
                    let text = item.text().unwrap_or_default();
                    if text.contains("Author") {
                        if let Ok(i_nodes) = item.select("i")
                            && let Some(i_node) = i_nodes.first() {
                                manga.authors = Some(vec![i_node.text()?]);
                            }
                    } else if text.contains("Status")
                        && let Ok(i_nodes) = item.select("i")
                            && let Some(i_node) = i_nodes.first() {
                                let s = i_node.text()?;
                                let s_lower = s.to_lowercase();
                                manga.status = if s_lower.contains("ongoing") {
                                    Status::Ongoing
                                } else if s_lower.contains("completed") {
                                    Status::Completed
                                } else if s_lower.contains("hiatus") {
                                    Status::Hiatus
                                } else if s_lower.contains("cancelled") || s_lower.contains("dropped") {
                                    Status::Cancelled
                                } else {
                                    Status::Unknown
                                };
                            }
                }
            }
            
            let mut tags = Vec::new();
            if let Ok(nodes) = html.select(".mgen a") {
                for genre in nodes {
                    tags.push(genre.text()?);
                }
            }
            if !tags.is_empty() {
                manga.tags = Some(tags);
            }
        }

        if needs_chapters {
            let mut rows = Vec::new();

            if let Ok(chbox_nodes) = html.select("#chapterlist li") {
                for item in chbox_nodes {
                    let mut href = String::new();

                    if let Ok(a_nodes) = item.select("a")
                        && let Some(a_tag) = a_nodes.first() {
                            href = a_tag.attr("href")?.unwrap_or_default();
                        }

                    let mut title = None;
                    if let Ok(nodes) = item.select(".chapternum")
                        && let Some(node) = nodes.first() {
                            title = Some(node.text()?);
                        }

                    rows.push(ChapterRow { href, title });
                }
            }

            manga.chapters = Some(chapters_from_rows(rows));
        }

        Ok(manga)
    }

    fn get_page_list(_manga: Manga, chapter: Chapter) -> Result<Vec<Page>> {
        let url = format!("{}/{}/", BASE_URL, chapter.key);
        let res = Request::get(&url).send()?;
        let html_str = String::from_utf8_lossy(&res.body).into_owned();
        
        let mut pages = Vec::new();
        
        let marker = r#""images":["#;
        if let Some(start_idx) = html_str.find(marker) {
            let array_start = start_idx + marker.len();
            if let Some(array_end) = html_str[array_start..].find("]") {
                let json_array = &html_str[array_start..array_start + array_end];
                for s in json_array.split(',') {
                    let clean = s.trim().replace("\\\"", "").replace("\"", "").replace("\\", "");
                    if clean.is_empty() { continue; }
                    pages.push(Page {
                        index: pages.len() as i32,
                        content: PageContent::Url(clean),
                        has_description: false,
                        description: None,
                        headers: None,
                    });
                }
            }
        }
        
        Ok(pages)
    }

    fn get_home() -> Result<HomeLayout> {
        let mut components = Vec::new();
        
        let res = Request::get(BASE_URL).send()?;
        let html = Node::new(&res.body);
        
        // 1. Featured (Slider)
        let mut featured_mangas = Vec::new();
        if let Ok(slides) = html.select(".slidernew .swiper-slide") {
            for item in slides {
                if let Ok(a_nodes) = item.select("a")
                    && let Some(a) = a_nodes.first() {
                        let url = a.attr("href")?.unwrap_or_default();
                        if let Ok(img_nodes) = a.select("img")
                            && let Some(img) = img_nodes.first() {
                                let cover = img.attr("src")?.unwrap_or_default();
                                let title = img.attr("alt")?.unwrap_or_default();
                                if !url.is_empty() {
                                    let key = url.replace(BASE_URL, "").replace("/comics/", "").replace("/manga/", "").replace("/", "");
                                    featured_mangas.push(Manga {
                                        key,
                                        cover: Some(cover),
                                        title,
                                        url: Some(url),
                                        authors: None,
                                        artist: None,
                                        description: None,
                                        tags: None,
                                        status: Status::Unknown,
                                        content_rating: ContentRating::Safe,
                                        nsfw: 0,
                                        viewer: Viewer::Default,
                                        chapters: None,
                                        catalog: None,
                                    });
                                }
                            }
                    }
            }
        }
        
        if !featured_mangas.is_empty() {
            components.push(HomeComponent {
                title: Some("Featured".to_string()),
                subtitle: None,
                value: HomeComponentValue::BigScroller(featured_mangas, None),
            });
        }

        // Parse bixbox sections (Popular Today, New Series, Latest Update)
        if let Ok(bixboxes) = html.select(".bixbox") {
            for bixbox in bixboxes {
                if let Ok(h2_nodes) = bixbox.select("h2")
                    && let Some(h2) = h2_nodes.first()
                        && let Ok(title_text) = h2.text() {
                            let mut entries = Vec::new();
                            if let Ok(items) = bixbox.select(".bsx") {
                                for item in items {
                                    if let Ok(a_nodes) = item.select("a")
                                        && let Some(a) = a_nodes.first() {
                                            let url = a.attr("href")?.unwrap_or_default();
                                            let mut manga_title = a.attr("title")?.unwrap_or_default();
                                            if manga_title.is_empty()
                                                && let Ok(tt_nodes) = item.select(".tt")
                                                    && let Some(tt) = tt_nodes.first() {
                                                        manga_title = tt.text()?.trim().to_string();
                                                    }
                                            
                                            let mut cover = String::new();
                                            if let Ok(img_nodes) = item.select("img")
                                                && let Some(img) = img_nodes.first() {
                                                    cover = img.attr("src")?.unwrap_or_default();
                                                    if cover.is_empty() {
                                                        cover = img.attr("data-src")?.unwrap_or_default();
                                                    }
                                                }

                                            if !url.is_empty() {
                                                let key = url.replace(BASE_URL, "").replace("/comics/", "").replace("/manga/", "").replace("/", "");
                                                entries.push(Manga {
                                                    key,
                                                    cover: Some(cover),
                                                    title: manga_title,
                                                    url: Some(url),
                                                    authors: None,
                                                    artist: None,
                                                    description: None,
                                                    tags: None,
                                                    status: Status::Unknown,
                                                    content_rating: ContentRating::Safe,
                                                    nsfw: 0,
                                                    viewer: Viewer::Default,
                                                    chapters: None,
                                                    catalog: None,
                                                });
                                            }
                                        }
                                }
                            }
                            
                            if !entries.is_empty() {
                                let listing = Listing {
                                    id: title_text.replace(" ", ""),
                                    name: title_text.clone(),
                                    kind: 0,
                                };
                                
                                components.push(HomeComponent {
                                    title: Some(title_text),
                                    subtitle: None,
                                    value: HomeComponentValue::Scroller(entries, Some(listing)),
                                });
                            }
                        }
            }
        }

        Ok(HomeLayout { components })
    }
}

export_manga_plugin!(VioletScans);

#[cfg(test)]
mod tests {
    use super::*;

    fn row(href: &str, title: Option<&str>) -> ChapterRow {
        ChapterRow {
            href: href.to_string(),
            title: title.map(str::to_string),
        }
    }

    /// Key stability over a trimmed chapter list (representative rows, not a saved page). A key
    /// is the URL path segment without slashes, and a chapter's number counts down from the list
    /// length, so the numbers shown for existing chapters do not move.
    #[test]
    fn keys_are_the_path_segments_and_numbers_count_down_from_the_list_length() {
        let chapters = chapters_from_rows(vec![
            row(
                "https://violetmanga.com/hunter-chapter-12/",
                Some("Chapter 12"),
            ),
            row(
                "https://violetmanga.com/hunter-chapter-11/",
                Some("Chapter 11"),
            ),
            row("https://violetmanga.com/hunter-chapter-0/", None),
        ]);
        let keys: Vec<_> = chapters.iter().map(|c| c.key.as_str()).collect();
        assert_eq!(
            keys,
            ["hunter-chapter-12", "hunter-chapter-11", "hunter-chapter-0"]
        );
        let numbers: Vec<_> = chapters.iter().map(|c| c.chapter).collect();
        assert_eq!(numbers, [Some(3.0), Some(2.0), Some(1.0)]);
        assert_eq!(
            chapters[0].url.as_deref(),
            Some("https://violetmanga.com/hunter-chapter-12/")
        );
        assert_eq!(chapters[0].title.as_deref(), Some("Chapter 12"));
    }

    /// The SDK rejects a whole list that repeats a key, carries an empty or over-long key, or a
    /// title over 1 KiB. Such rows are left out or untitled, and the other numbers do not move.
    #[test]
    fn rows_the_sdk_would_reject_are_left_out_and_the_numbers_stay() {
        let long_title = "x".repeat(MAX_TITLE_BYTES + 1);
        let long_key = format!("https://violetmanga.com/{}/", "k".repeat(MAX_KEY_BYTES + 1));
        let chapters = chapters_from_rows(vec![
            row("https://violetmanga.com/hunter-chapter-5/", None),
            row("", Some("No link")),
            row("https://violetmanga.com/hunter-chapter-5/", Some("Again")),
            row(&long_key, None),
            row(
                "https://violetmanga.com/hunter-chapter-1/",
                Some(long_title.as_str()),
            ),
        ]);
        let keys: Vec<_> = chapters.iter().map(|c| c.key.as_str()).collect();
        assert_eq!(keys, ["hunter-chapter-5", "hunter-chapter-1"]);
        let numbers: Vec<_> = chapters.iter().map(|c| c.chapter).collect();
        assert_eq!(numbers, [Some(5.0), Some(1.0)]);
        assert_eq!(chapters[1].title, None);
        assert!(ito_rs::catalog::validate_chapters(&chapters).is_ok());
    }
}
