use ito_rs::export_manga_plugin;
use ito_rs::provider::MangaProvider;
use ito_rs::models::{Listing, FilterItem, Page, PageContent, HomeLayout, HomeComponent, HomeComponentValue, MangaWithChapter, Setting, SettingsSchema, LinkValue, ContentKind};
use ito_rs::models::manga::{Manga, Chapter, PageResult, Status, ContentRating, Viewer};
use ito_rs::html::Node;
use ito_rs::Result;
use ito_rs::defaults;

mod helpers;
mod chapters;
use helpers::*;

struct AsuraScans;

fn chapter_list_missing() -> ito_rs::Error {
    ito_rs::Error::Host("Asura Scans returned a series page without its chapter list".into())
}

fn pages_missing() -> ito_rs::Error {
    ito_rs::Error::Host("Asura Scans returned a chapter page without its page list".into())
}

/// The error for a reader without a page list. A locked chapter asks for sign-in on the website;
/// any other chapter means the site's reader data changed.
fn missing_pages(chapter: &Chapter) -> ito_rs::Error {
    if chapter.paywalled == Some(true) {
        ito_rs::Error::NeedsLogin(
            "This chapter is locked on Asura Scans. Open it in the browser to sign in or unlock it."
                .into(),
        )
    } else {
        pages_missing()
    }
}

/// The pages of the reader island, each a `[tag, value]` pair like the chapter rows. `None` means
/// the reader carries no page list. An entry without an image address is an error, so a chapter
/// with holes is never returned as if it were complete.
fn reader_pages(props: &str) -> Result<Option<Vec<Page>>> {
    let json: serde_json::Value = serde_json::from_str(props)
        .map_err(|_| ito_rs::Error::Host("JSON parse error".to_string()))?;
    let Some(page_arr) = json["pages"][1]
        .as_array()
        .filter(|rows| !rows.is_empty())
    else {
        return Ok(None);
    };
    let pages = page_arr
        .iter()
        .enumerate()
        .map(|(i, obj)| -> Result<Page> {
            let url = obj[1]["url"][1].as_str().ok_or_else(|| {
                ito_rs::Error::Host(
                    "Asura Scans returned a chapter page without an image address".into(),
                )
            })?;
            Ok(Page {
                index: i as i32,
                content: PageContent::Url(url.to_string()),
                has_description: false,
                description: None,
                headers: None,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(Some(pages))
}

/// The chapter rows of the series page's chapter-list island. Astro serialises each prop as a
/// `[tag, value]` pair, so the list is the value at index 1.
fn chapter_rows(props: &str) -> ito_rs::Result<Vec<serde_json::Value>> {
    let mut json: serde_json::Value = serde_json::from_str(props)
        .map_err(|_| ito_rs::Error::Host("JSON parse error".to_string()))?;
    match json
        .get_mut("chapters")
        .and_then(|chapters| chapters.get_mut(1))
        .map(serde_json::Value::take)
    {
        Some(serde_json::Value::Array(rows)) => Ok(rows),
        _ => Err(chapter_list_missing()),
    }
}

impl MangaProvider for AsuraScans {
    fn get_settings() -> Option<SettingsSchema> {
        Some(SettingsSchema {
            settings: vec![
                Setting::Toggle {
                    id: "show_locked".to_string(),
                    name: "Show Locked Chapters".to_string(),
                    summary: Some("Show chapters that are locked or behind a paywall".to_string()),
                    default_value: true,
                }
            ]
        })
    }

    fn handle_url(url: &str) -> Result<LinkValue> {
        if let Some(key) = get_manga_key(url) {
            // Handles both series and chapter links: the series key is taken from the
            // path segment after `comics`, so a chapter link resolves to its series. A
            // placeholder Manga is returned for the host to open; the title and chapter
            // list are loaded afterwards.
            Ok(LinkValue::Manga(Manga {
                key,
                title: String::new(),
                authors: None,
                artist: None,
                description: None,
                tags: None,
                cover: None,
                url: Some(url.to_string()),
                status: Status::Unknown,
                content_rating: ContentRating::Safe,
                nsfw: 0,
                viewer: Viewer::Default,
                chapters: None,
                catalog: None,
            }))
        } else {
            Err(ito_rs::Error::Host("Invalid URL".to_string()))
        }
    }

    fn get_home() -> Result<HomeLayout> {
        let res = page_request(BASE_URL).send()?;
        let html = Node::new(&res.body);
        
        let mut components = Vec::new();

        // Trending
        if let Ok(nodes) = html.select("astro-island[opts*=TrendingSection] > section")
            && let Some(trending_today) = nodes.first() {
                let title = trending_today
                    .select("h2")?
                    .first()
                    .and_then(|el| el.text().ok())
                    .unwrap_or_else(|| "Trending Today".to_string());
                
                let mut entries = Vec::new();
                if let Ok(nodes) = trending_today.select("div.embla-trending > div > div > a") {
                    for el in nodes {
                        if let Some(href) = el.attr("href")?
                            && let Some(key) = get_manga_key(&href)
                                && let Some(title_node) = el.select("span.block")?.first() {
                                    entries.push(Manga {
                                        key,
                                        title: title_node.text()?,
                                        cover: el.select("img")?.first().and_then(|img| img.attr("src").ok().flatten()),
                                        authors: None,
                                        artist: None,
                                        description: None,
                                        tags: None,
                                        url: Some(href),
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
                
                if !entries.is_empty() {
                    components.push(HomeComponent {
                        title: Some(title),
                        subtitle: None,
                        value: HomeComponentValue::Scroller(entries, None),
                    });
                }
            }

        // Latest Updates
        if let Ok(nodes) = html.select("astro-island[opts*=LatestUpdates] > section")
            && let Some(latest_updates) = nodes.first() {
                let title = latest_updates
                    .select("h2")?
                    .first()
                    .and_then(|el| el.text().ok())
                    .unwrap_or_else(|| "Latest Updates".to_string());

                let mut entries = Vec::new();
                if let Ok(nodes) = latest_updates.select("div.grid > div.grid") {
                    for el in nodes {
                        let link = el.select("a.font-bold")?.into_iter().next();
                        let chapter_link = el.select("div.col-span-8 a[href*='/chapter/']")?.into_iter().next();
                        
                        if let (Some(link), Some(chapter_link)) = (link, chapter_link)
                            && let (Some(manga_href), Some(chapter_href)) = (link.attr("href")?, chapter_link.attr("href")?)
                                && let (Some(manga_key), Some(chapter_key)) = (get_manga_key(&manga_href), get_chapter_key(&chapter_href))
                                    && !chapter_key.is_empty() {
                                    let chapter_number = chapter_link.select("span.font-medium")?
                                        .first()
                                        .and_then(|s| s.text().ok())
                                        .and_then(|s| s.strip_prefix("Chapter").map(|s| s.trim().to_string()))
                                        .and_then(|s| s.parse::<f64>().ok())
                                        .and_then(chapters::chapter_number);

                                    entries.push(MangaWithChapter {
                                        manga: Manga {
                                            key: manga_key,
                                            title: link.text()?,
                                            cover: el.select("img")?.first().and_then(|img| img.attr("src").ok().flatten()),
                                            authors: None,
                                            artist: None,
                                            description: None,
                                            tags: None,
                                            url: Some(manga_href),
                                            status: Status::Unknown,
                                            content_rating: ContentRating::Safe,
                                            nsfw: 0,
                                            viewer: Viewer::Default,
                                            chapters: None,
                                            catalog: None,
                                        },
                                        chapter: Chapter {
                                            title: None,
                                            chapter: chapter_number,
                                            url: Some(chapter_href),
                                            lang: Some("en".to_string()),
                                            paywalled: None,
                                            ..Chapter::new(chapter_key, ContentKind::Images)
                                        },
                                    });
                                }
                    }
                }

                if !entries.is_empty() {
                    components.push(HomeComponent {
                        title: Some(title),
                        subtitle: None,
                        value: HomeComponentValue::MangaChapterList(None, entries, None),
                    });
                }
            }

        Ok(HomeLayout { components })
    }

    fn get_manga_list(_listing: Listing, page: i32) -> Result<PageResult> {
        Self::get_search_manga_list("", page, Vec::new())
    }

    fn get_search_manga_list(query: &str, page: i32, _filters: Vec<FilterItem>) -> Result<PageResult> {
        let mut url = format!("{}/browse?page={}", BASE_URL, page);
        if !query.is_empty() {
            url.push_str(&format!("&q={}", query));
        }

        let res = page_request(&url).send()?;
        let html = Node::new(&res.body);

        let mut entries = Vec::new();
        if let Ok(nodes) = html.select("#series-grid > .series-card") {
            for el in nodes {
                if let Some(link) = el.select("a")?.first()
                    && let Some(href) = link.attr("href")?
                        && let Some(key) = get_manga_key(&href)
                            && let Some(title_node) = el.select("h3")?.first() {
                                entries.push(Manga {
                                    key: key.clone(),
                                    title: title_node.text()?,
                                    cover: el.select("img")?.first().and_then(|img| img.attr("src").ok().flatten()),
                                    authors: None,
                                    artist: None,
                                    description: None,
                                    tags: None,
                                    url: Some(href),
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

        let has_next_page = !html
            .select("a:contains(Next page), div.flex > a.flex.bg-themecolor:contains(Next)")?.is_empty();

        Ok(PageResult { entries, has_next_page })
    }

    fn get_manga_update(mut manga: Manga, needs_details: bool, needs_chapters: bool) -> Result<Manga> {
        let url = get_manga_url(&manga.key);
        let res = page_request(&url).send()?;
        if needs_details || needs_chapters {
            check_response(res.status, &res.body)?;
        }
        let html = Node::new(&res.body);

        if needs_details {
            if let Some(title_node) = html.select("h1.text-xl.font-semibold")?.first() {
                manga.title = title_node.text()?;
            }
            manga.cover = html.select("div#desktop-cover-container img")?.first().and_then(|el| el.attr("src").ok().flatten());
            
            let mut authors = Vec::new();
            if let Ok(nodes) = html.select("a[href^=/browse?artist]") {
                for el in nodes {
                    if let Ok(text) = el.text()
                        && text != "_" { authors.push(text); }
                }
            }
            if !authors.is_empty() {
                manga.authors = Some(authors);
            }

            // Description
            if let Some(desc_node) = html.select("div#description-text")?.first() {
                manga.description = Some(desc_node.text()?);
            }

            manga.url = Some(url);

            // Tags
            let mut tags = Vec::new();
            if let Ok(nodes) = html.select("a[href^=/browse?genres=]") {
                for el in nodes {
                    if let Ok(text) = el.text() {
                        tags.push(text);
                    }
                }
            }
            if !tags.is_empty() {
                manga.tags = Some(tags.clone());
                if tags.iter().any(|t| t == "Adult" || t == "Ecchi") {
                    manga.content_rating = ContentRating::Suggestive;
                }
            }

            // Status & Viewer
            if let Some(status_node) = html.select("div.flex.gap-3.pt-4.border-t > div:nth-child(2) > div > span.text-base")?.first() {
                let s = status_node.text()?;
                manga.status = match s.to_lowercase().as_str() {
                    "ongoing" => Status::Ongoing,
                    "hiatus" => Status::Hiatus,
                    "completed" => Status::Completed,
                    "dropped" => Status::Cancelled,
                    _ => Status::Unknown,
                };
                manga.viewer = match s.to_lowercase().as_str() {
                    "manhwa" | "manhua" => Viewer::Webtoon,
                    "mangatoon" => Viewer::Rtl,
                    _ => Viewer::Webtoon,
                };
            }
        }

        if needs_chapters {
            let islands = html.select("astro-island[component-url*=ChapterListReact], astro-island[opts*=ChapterListReact]")?;
            let props = match islands.first() {
                Some(island) => island.attr("props")?,
                None => None,
            }
            .ok_or_else(chapter_list_missing)?;
            let rows = chapter_rows(&props)?;
            let show_locked =
                defaults::get("show_locked")?.unwrap_or_else(|| "true".to_string()) == "true";
            manga.chapters = Some(chapters::parse_chapter_list(&rows, &manga.key, show_locked));
            manga.catalog = chapters::list_catalog(show_locked);
        }

        Ok(manga)
    }

    fn get_page_list(manga: Manga, chapter: Chapter) -> Result<Vec<Page>> {
        let url = get_chapter_url(&chapter.key, &manga.key);
        let res = page_request(&url).send()?;
        check_response(res.status, &res.body)?;
        let html = Node::new(&res.body);

        let islands = html.select("astro-island[component-url*=ChapterReader], astro-island[opts*=ChapterReader]")?;
        let props = match islands.first() {
            Some(island) => island.attr("props")?,
            None => None,
        };
        let pages = match props {
            Some(props) => reader_pages(&props)?,
            None => None,
        };
        pages.ok_or_else(|| missing_pages(&chapter))
    }
}

export_manga_plugin!(AsuraScans);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_series_page_without_its_chapter_list_is_an_error() {
        let rows = chapter_rows(r#"{"chapters":[0,[[0,{"number":[0,1]}]]]}"#)
            .expect("chapter list is present");
        assert_eq!(rows.len(), 1);
        assert!(matches!(
            chapter_rows(r#"{"title":[0,"x"]}"#),
            Err(ito_rs::Error::Host(_))
        ));
        assert!(matches!(
            chapter_rows(r#"{"chapters":[0,{}]}"#),
            Err(ito_rs::Error::Host(_))
        ));
        assert!(matches!(
            chapter_rows("not json"),
            Err(ito_rs::Error::Host(_))
        ));
    }

    #[test]
    fn reader_pages_are_all_returned_or_the_read_fails() {
        let pages = reader_pages(
            r#"{"pages":[0,[[0,{"url":[0,"https://cdn.example/1.webp"]}],[0,{"url":[0,"https://cdn.example/2.webp"]}]]]}"#,
        )
        .expect("page list parses")
        .expect("page list is present");
        assert_eq!(pages.len(), 2);
        assert_eq!(pages[1].index, 1);
        assert!(matches!(
            reader_pages(
                r#"{"pages":[0,[[0,{"url":[0,"https://cdn.example/1.webp"]}],[0,{"url":[0,null]}]]]}"#
            ),
            Err(ito_rs::Error::Host(_))
        ));
        assert!(matches!(
            reader_pages("not json"),
            Err(ito_rs::Error::Host(_))
        ));
    }

    #[test]
    fn a_reader_without_a_page_list_carries_none() {
        assert!(matches!(reader_pages(r#"{"chapter":[0,{}]}"#), Ok(None)));
        assert!(matches!(reader_pages(r#"{"pages":[0,[]]}"#), Ok(None)));
    }

    #[test]
    fn a_locked_chapter_without_pages_asks_for_sign_in() {
        let locked = Chapter {
            paywalled: Some(true),
            ..Chapter::new("1".to_string(), ContentKind::Images)
        };
        assert!(matches!(
            missing_pages(&locked),
            ito_rs::Error::NeedsLogin(_)
        ));
        let open = Chapter {
            paywalled: Some(false),
            ..Chapter::new("1".to_string(), ContentKind::Images)
        };
        let unknown = Chapter::new("1".to_string(), ContentKind::Images);
        for chapter in [open, unknown] {
            assert!(matches!(missing_pages(&chapter), ito_rs::Error::Host(_)));
        }
    }
}
