use ito_rs::export_manga_plugin;
use ito_rs::provider::MangaProvider;
use ito_rs::models::{Listing, FilterItem, Page, PageContent, HomeLayout, HomeComponent, HomeComponentValue, MangaWithChapter, Setting, SettingsSchema, LinkValue};
use ito_rs::models::manga::{Manga, Chapter, PageResult, Status, ContentRating, Viewer};
use ito_rs::net::Request;
use ito_rs::html::Node;
use ito_rs::Result;
use ito_rs::defaults;

mod helpers;
use helpers::*;

struct AsuraScans;

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
            // If it's a chapter link, we still return the Manga for now as per Aidoku logic
            // or we could return something else if supported.
            // But usually handle_url returns the model to open.
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
            }))
        } else {
            Err(ito_rs::Error::Host("Invalid URL".to_string()))
        }
    }

    fn get_home() -> Result<HomeLayout> {
        let res = Request::get(BASE_URL).send()?;
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
                        let chapter_link = el.select("div.col-span-8 > div.flex > a")?.into_iter().next();
                        
                        if let (Some(link), Some(chapter_link)) = (link, chapter_link)
                            && let (Some(manga_href), Some(chapter_href)) = (link.attr("href")?, chapter_link.attr("href")?)
                                && let (Some(manga_key), Some(chapter_key)) = (get_manga_key(&manga_href), get_chapter_key(&chapter_href)) {
                                    let chapter_number = chapter_link.select("span.font-medium")?
                                        .first()
                                        .and_then(|s| s.text().ok())
                                        .and_then(|s| s.strip_prefix("Chapter").map(|s| s.trim().to_string()))
                                        .and_then(|s| s.parse::<f32>().ok());

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
                                        },
                                        chapter: Chapter {
                                            key: chapter_key,
                                            title: None,
                                            volume: None,
                                            chapter: chapter_number,
                                            date_updated: None,
                                            scanlator: None,
                                            url: Some(chapter_href),
                                            lang: Some("en".to_string()),
                                            paywalled: None,
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

        let res = Request::get(url).send()?;
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
        let res = Request::get(&url).send()?;
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

        if needs_chapters
            && let Some(island) = html.select("astro-island[component-url*=ChapterListReact], astro-island[opts*=ChapterListReact]")?.first()
                && let Some(props) = island.attr("props")? {
                    let json: serde_json::Value = serde_json::from_str(&props).map_err(|_| ito_rs::Error::Host("JSON parse error".to_string()))?;
                    
                    if let Some(chapters_arr) = json["chapters"][1].as_array() {
                        let mut chapters = Vec::new();
                        let show_locked = defaults::get("show_locked")?.unwrap_or_else(|| "true".to_string()) == "true";

                        for obj in chapters_arr {
                            if let Some(obj) = obj[1].as_object() {
                                let locked = obj["is_locked"][1].as_bool().unwrap_or_default();
                                if !show_locked && locked {
                                    continue;
                                }

                                let chapter_number = obj["number"][1].as_f64().map(|f| f as f32);
                                let key = chapter_number.map(|f| f.to_string()).unwrap_or_default();
                                
                                chapters.push(Chapter {
                                    key: key.clone(),
                                    title: None,
                                    volume: None,
                                    chapter: chapter_number,
                                    date_updated: None, 
                                    scanlator: None,
                                    url: Some(get_chapter_url(&key, &manga.key)),
                                    lang: Some("en".to_string()),
                                    paywalled: Some(locked),
                                });
                            }
                        }
                        manga.chapters = Some(chapters);
                    }
                }

        Ok(manga)
    }

    fn get_page_list(manga: Manga, chapter: Chapter) -> Result<Vec<Page>> {
        let url = get_chapter_url(&chapter.key, &manga.key);
        let res = Request::get(url).send()?;
        let html = Node::new(&res.body);

        if let Some(island) = html.select("astro-island[component-url*=ChapterReader], astro-island[opts*=ChapterReader]")?.first()
            && let Some(props) = island.attr("props")? {
                let json: serde_json::Value = serde_json::from_str(&props).map_err(|_| ito_rs::Error::Host("JSON parse error".to_string()))?;
                if let Some(page_arr) = json["pages"][1].as_array() {
                    let mut pages = Vec::new();
                    for (i, obj) in page_arr.iter().enumerate() {
                        if let Some(url) = obj[1]["url"][1].as_str() {
                            pages.push(Page {
                                index: i as i32,
                                content: PageContent::Url(url.to_string()),
                                has_description: false,
                                description: None,
                                headers: None,
                            });
                        }
                    }
                    return Ok(pages);
                }
            }

        Ok(vec![])
    }
}

export_manga_plugin!(AsuraScans);
