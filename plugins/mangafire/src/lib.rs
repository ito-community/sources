use ito_rs::export_manga_plugin;
use ito_rs::provider::MangaProvider;
use ito_rs::models::{Listing, FilterItem, Page, PageContent, HomeLayout, HomeComponent, HomeComponentValue, MangaWithChapter, Setting, SettingsSchema, LinkValue};
use ito_rs::models::manga::{Manga, Chapter, PageResult, Status, ContentRating, Viewer};
use ito_rs::net::Request;
use ito_rs::html::Node;
use ito_rs::Result;
use ito_rs::env;
use serde::Deserialize;

mod vrf;
use vrf::VrfGenerator;

const BASE_URL: &str = "https://mangafire.to";

#[derive(Deserialize)]
struct AjaxResponse<T> {
    result: T,
}

#[derive(Deserialize)]
struct AjaxRead {
    html: String,
}

#[derive(Deserialize)]
struct AjaxPageList {
    images: Vec<Vec<serde_json::Value>>,
}

struct MangaFire;

impl MangaFire {
    fn parse_manga_page(html: &Node) -> Result<PageResult> {
        let mut entries = Vec::new();
        if let Ok(nodes) = html.select(".original.card-lg .unit .inner") {
            for el in nodes {
                if let Some(title_el) = el.select(".info > a")?.first() {
                    let title = title_el.text()?;
                    if let Some(href) = title_el.attr("href")? {
                        let key = if let Some(stripped) = href.strip_prefix(BASE_URL) {
                            stripped.to_string()
                        } else {
                            href
                        };
                        let cover = el.select("img")?.first().and_then(|img| img.attr("src").ok().flatten());
                        entries.push(Manga {
                            key: key.clone(),
                            title,
                            cover,
                            authors: None,
                            artist: None,
                            description: None,
                            tags: None,
                            url: Some(format!("{}{}", BASE_URL, key)),
                            status: Status::Unknown,
                            content_rating: ContentRating::Safe,
                            nsfw: 0,
                            viewer: Viewer::Default,
                            chapters: None,
                        });
                    }
                }
            }
        }

        let has_next_page = !html
            .select(".page-item.active + .page-item .page-link")?.is_empty();

        Ok(PageResult { entries, has_next_page })
    }

    fn get_langs() -> Vec<String> {
        let preferred = env::get_preferred_languages().unwrap_or_default();
        if preferred.is_empty() {
            vec!["en".to_string()]
        } else {
            preferred.into_iter().map(|l| match l.as_str() {
                "pt-BR" => "pt-br".into(),
                "es-419" => "es-la".into(),
                _ => l,
            }).collect()
        }
    }
}

impl MangaProvider for MangaFire {
    fn get_settings() -> Option<SettingsSchema> {
        Some(SettingsSchema {
            settings: vec![
                Setting::Picker {
                    id: "languages".to_string(),
                    name: "Content Languages".to_string(),
                    summary: Some("Select languages for manga content".to_string()),
                    options: vec!["en".to_string(), "fr".to_string(), "pt-br".to_string(), "es-la".to_string(), "ja".to_string()],
                    default_value: "en".to_string(),
                }
            ]
        })
    }

    fn handle_url(url: &str) -> Result<LinkValue> {
        if url.contains("/manga/") {
            let key = if let Some(idx) = url.find("/manga/") {
                url[idx..].to_string()
            } else {
                url.to_string()
            };
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
            Err(ito_rs::Error::Host("Unsupported URL".to_string()))
        }
    }

    fn get_home_stream() -> Result<bool> {
        Ok(false)
    }

    fn get_home() -> Result<HomeLayout> {
        let res = Request::get(format!("{}/home", BASE_URL)).send()?;
        if res.status != 200 {
            return Ok(HomeLayout { components: vec![] });
        }
        let html = Node::new(&res.body);
        let mut components = Vec::new();
        
        // Big Scroller (Trending)
        if let Ok(nodes) = html.select(".trending .swiper-wrapper .swiper-slide") {
            let mut entries = Vec::new();
            for el in nodes {
                if let Some(link) = el.select(".info .above a")?.first()
                    && let Some(href) = link.attr("href")? {
                        let key = if let Some(stripped) = href.strip_prefix(BASE_URL) {
                            stripped.to_string()
                        } else {
                            href
                        };
                        entries.push(Manga {
                            key: key.clone(),
                            title: link.text()?,
                            cover: el.select("img")?.first().and_then(|img| img.attr("src").ok().flatten()),
                            authors: None,
                            artist: None,
                            description: el.select(".info .below span")?.first().and_then(|e| e.text().ok()),
                            tags: el.select(".info .below a")?.into_iter().filter_map(|e| e.text().ok()).collect::<Vec<_>>().into(),
                            url: Some(format!("{}{}", BASE_URL, key)),
                            status: Status::Unknown,
                            content_rating: ContentRating::Safe,
                            nsfw: 0,
                            viewer: Viewer::Default,
                            chapters: None,
                        });
                    }
            }
            if !entries.is_empty() {
                components.push(HomeComponent {
                    title: None,
                    subtitle: None,
                    value: HomeComponentValue::BigScroller(entries, Some(10.0)),
                });
            }
        }

        // Most Viewed
        if let Ok(nodes) = html.select("#most-viewed .swiper-wrapper .swiper-slide") {
            let mut entries = Vec::new();
            for el in nodes {
                if let Some(link) = el.select("a")?.first()
                    && let Some(href) = link.attr("href")? {
                        let key = if let Some(stripped) = href.strip_prefix(BASE_URL) {
                            stripped.to_string()
                        } else {
                            href
                        };
                        entries.push(Manga {
                            key: key.clone(),
                            title: link.select("span")?.first().and_then(|e| e.text().ok()).unwrap_or_default(),
                            cover: el.select(".poster img")?.first().and_then(|img| img.attr("src").ok().flatten()),
                            authors: None,
                            artist: None,
                            description: None,
                            tags: None,
                            url: Some(format!("{}{}", BASE_URL, key)),
                            status: Status::Unknown,
                            content_rating: ContentRating::Safe,
                            nsfw: 0,
                            viewer: Viewer::Default,
                            chapters: None,
                        });
                    }
            }
            if !entries.is_empty() {
                components.push(HomeComponent {
                    title: Some("Most Viewed".to_string()),
                    subtitle: None,
                    value: HomeComponentValue::Scroller(entries, None),
                });
            }
        }

        // Recently Updated
        if let Ok(nodes) = html.select("section .tab-content .original .unit") {
            let mut entries = Vec::new();
            for el in nodes {
                if let (Some(link), Some(chapter_el)) = (el.select("a")?.first(), el.select("ul.content li")?.first())
                    && let Some(href) = link.attr("href")? {
                        let key = if let Some(stripped) = href.strip_prefix(BASE_URL) {
                            stripped.to_string()
                        } else {
                            href
                        };
                        let chapter_number = chapter_el.select("a span")?
                            .first()
                            .and_then(|e| e.text().ok())
                            .and_then(|t| t.strip_prefix("Chap ").and_then(|s| s.parse::<f32>().ok()));

                        entries.push(MangaWithChapter {
                            manga: Manga {
                                key: key.clone(),
                                title: el.select(".info a")?.first().and_then(|e| e.text().ok()).unwrap_or_default(),
                                cover: el.select(".poster img")?.first().and_then(|img| img.attr("src").ok().flatten()),
                                authors: None,
                                artist: None,
                                description: None,
                                tags: None,
                                url: Some(format!("{}{}", BASE_URL, key)),
                                status: Status::Unknown,
                                content_rating: ContentRating::Safe,
                                nsfw: 0,
                                viewer: Viewer::Default,
                                chapters: None,
                            },
                            chapter: Chapter {
                                key: key.clone(),
                                title: None,
                                volume: None,
                                chapter: chapter_number,
                                date_updated: None,
                                scanlator: None,
                                url: None,
                                lang: Some("en".to_string()),
                                paywalled: None,
                            },
                        });
                    }
            }
            if !entries.is_empty() {
                components.push(HomeComponent {
                    title: Some("Recently Updated".to_string()),
                    subtitle: None,
                    value: HomeComponentValue::MangaChapterList(Some(12), entries, None),
                });
            }
        }

        Ok(HomeLayout { components })
    }

    fn get_manga_list(listing: Listing, page: i32) -> Result<PageResult> {
        let mut entries = Vec::new();
        let mut has_next_page = false;

        let langs = Self::get_langs();
        for lang in langs {
            let url = match listing.id.as_str() {
                "Newest" => format!("{}/newest?page={}&language%5B%5D={}", BASE_URL, page, lang),
                "Updated" => format!("{}/updated?page={}&language%5B%5D={}", BASE_URL, page, lang),
                "Added" => format!("{}/added?page={}&language%5B%5D={}", BASE_URL, page, lang),
                _ => format!("{}/filter?page={}&language%5B%5D={}", BASE_URL, page, lang),
            };
            let res = Request::get(url).header("Referer", &format!("{}/", BASE_URL)).send()?;
            let html = Node::new(&res.body);
            let res = Self::parse_manga_page(&html)?;
            entries.extend(res.entries);
            has_next_page = has_next_page || res.has_next_page;
        }
        
        // Dedup
        let mut seen = std::collections::HashSet::new();
        entries.retain(|e| seen.insert(e.key.clone()));

        Ok(PageResult { entries, has_next_page })
    }

    fn get_search_manga_list(query: &str, page: i32, _filters: Vec<FilterItem>) -> Result<PageResult> {
        let vrf = VrfGenerator::generate(query);
        let mut entries = Vec::new();
        let mut has_next_page = false;

        let langs = Self::get_langs();
        for lang in langs {
            let url = format!("{}/filter?keyword={}&page={}&vrf={}&language%5B%5D={}", BASE_URL, query, page, vrf, lang);
            let res = Request::get(url).header("Referer", &format!("{}/", BASE_URL)).send()?;
            let html = Node::new(&res.body);
            let res = Self::parse_manga_page(&html)?;
            entries.extend(res.entries);
            has_next_page = has_next_page || res.has_next_page;
        }

        let mut seen = std::collections::HashSet::new();
        entries.retain(|e| seen.insert(e.key.clone()));

        Ok(PageResult { entries, has_next_page })
    }

    fn get_manga_update(mut manga: Manga, needs_details: bool, needs_chapters: bool) -> Result<Manga> {
        let manga_url = format!("{}{}", BASE_URL, manga.key);

        if needs_details {
            let res = Request::get(&manga_url).send()?;
            let html = Node::new(&res.body);

            if let Some(content) = html.select(".main-inner:not(.manga-bottom)")?.first() {
                if let Some(title_el) = content.select("h1")?.first() {
                    manga.title = title_el.text()?;
                }
                manga.cover = content.select(".poster img")?.first().and_then(|e| e.attr("src").ok().flatten());
                
                if let Some(meta) = html.select(".meta")?.first() {
                    manga.authors = meta.select("span:contains(Author:) + span")?.first().and_then(|e| e.text().ok()).map(|t| vec![t]);
                    manga.tags = meta.select("span:contains(Genres:) + span")?.first().and_then(|e| e.text().ok()).map(|t| t.split(',').map(|s| s.trim().to_string()).collect());
                }

                manga.description = html.select("#synopsis .modal-content")?.first().and_then(|e| e.text().ok());
                manga.url = Some(manga_url.clone());
                
                if let Some(status_el) = content.select(".info > p")?.first() {
                    let txt = status_el.text()?.to_lowercase();
                    manga.status = match txt.as_str() {
                        "releasing" => Status::Ongoing,
                        "completed" => Status::Completed,
                        "on_hiatus" => Status::Hiatus,
                        "discontinued" => Status::Cancelled,
                        _ => Status::Unknown,
                    };
                }

                if let Some(viewer_el) = content.select(".info > .min-info > a")?.first() {
                    let txt = viewer_el.text()?;
                    manga.viewer = match txt.as_str() {
                        "Manhua" | "Manhwa" => Viewer::Webtoon,
                        "Manga" => Viewer::Rtl,
                        _ => Viewer::Rtl,
                    };
                }
            }
        }

        if needs_chapters {
            let manga_id = manga.key.rsplit('.').next().unwrap_or_default().to_string();
            let mut all_chapters = Vec::new();

            for lang in Self::get_langs() {
                let ajax_manga_url = format!("{}/ajax/manga/{}/chapter/{}", BASE_URL, manga_id, lang);
                let res_manga = Request::get(&ajax_manga_url).send()?;
                let ajax_manga: AjaxResponse<String> = serde_json::from_slice(&res_manga.body).map_err(|_| ito_rs::Error::Host("AJAX manga error".to_string()))?;
                let html_manga = Node::new(ajax_manga.result.as_bytes());
                let manga_list = html_manga.select("li")?;

                let vrf = VrfGenerator::generate(&format!("{}@chapter@{}", manga_id, lang));
                let ajax_read_url = format!("{}/ajax/read/{}/chapter/{}?vrf={}", BASE_URL, manga_id, lang, vrf);
                let res_read = Request::get(&ajax_read_url).send()?;
                let ajax_read: AjaxResponse<AjaxRead> = serde_json::from_slice(&res_read.body).map_err(|_| ito_rs::Error::Host("AJAX read error".to_string()))?;
                let html_read = Node::new(ajax_read.result.html.as_bytes());
                let read_list = html_read.select("ul a")?;

                for (m, r) in manga_list.into_iter().zip(read_list) {
                    if let (Some(data_id), Some(number)) = (r.attr("data-id")?, m.attr("data-number")?) {
                        let key = format!("chapter/{}", data_id);
                        let href = r.attr("href")?.unwrap_or_default();
                        let title = r.text().ok().map(|t| {
                            let prefix = format!("Chap {}:", number);
                            if t.starts_with(&prefix) {
                                t[prefix.len()..].trim().to_string()
                            } else {
                                t
                            }
                        });

                        all_chapters.push(Chapter {
                            key,
                            title,
                            volume: None,
                            chapter: number.parse::<f32>().ok(),
                            date_updated: None,
                            scanlator: None,
                            url: Some(format!("{}{}", BASE_URL, href)),
                            lang: Some(lang.clone()),
                            paywalled: None,
                        });
                    }
                }
            }
            manga.chapters = Some(all_chapters);
        }

        Ok(manga)
    }

    fn get_page_list(_manga: Manga, chapter: Chapter) -> Result<Vec<Page>> {
        let vrf = VrfGenerator::generate(&chapter.key.replace("/", "@"));
        let ajax_url = format!("{}/ajax/read/{}?vrf={}", BASE_URL, chapter.key, vrf);
        
        let res = Request::get(&ajax_url).send()?;
        let response: AjaxResponse<AjaxPageList> = serde_json::from_slice(&res.body).map_err(|_| ito_rs::Error::Host("AJAX page list error".to_string()))?;
        
        let mut pages = Vec::new();
        for (i, img) in response.result.images.iter().enumerate() {
            if let Some(url) = img.first().and_then(|v| v.as_str()) {
                pages.push(Page {
                    index: i as i32,
                    content: PageContent::Url(url.to_string()),
                    has_description: false,
                    description: None,
                    headers: None,
                });
            }
        }
        Ok(pages)
    }
}

export_manga_plugin!(MangaFire);
