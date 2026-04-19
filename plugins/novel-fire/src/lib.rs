use ito_rs::export_novel_plugin;
use ito_rs::provider::NovelProvider;
use ito_rs::models::{FilterItem, Listing, Page, PageContent, HomeLayout, HomeComponent, HomeComponentValue};
use ito_rs::models::novel::{Novel, Chapter, PageResult, Status as NovelStatus, ContentRating};
use ito_rs::net::Request;
use ito_rs::html::Node;
use ito_rs::Result;

const BASE_URL: &str = "https://novelfire.net";

pub struct NovelFire;

impl NovelProvider for NovelFire {
    fn get_home_stream() -> Result<bool> {
        Ok(false)
    }

    fn get_home() -> Result<HomeLayout> {
        let listing1 = Listing { id: "overall-ranking".to_string(), name: "Newly Hotted Updates".to_string(), kind: 0 };
        let listing2 = Listing { id: "ratings".to_string(), name: "User Ratings".to_string(), kind: 0 };
        let listing3 = Listing { id: "most-lib".to_string(), name: "Most Bookmarked".to_string(), kind: 0 };
        let listing4 = Listing { id: "most-read".to_string(), name: "Most Read".to_string(), kind: 0 };
        let listing5 = Listing { id: "most-review".to_string(), name: "Most Reviewed".to_string(), kind: 0 };
        let listing6 = Listing { id: "most-comment".to_string(), name: "Most Commented".to_string(), kind: 0 };

        Ok(HomeLayout {
            components: vec![
                HomeComponent {
                    title: Some("Newly Hotted Updates".to_string()),
                    subtitle: Some("Hot updates from across the source!".to_string()),
                    value: HomeComponentValue::NovelScroller(
                        Self::get_novel_list(listing1.clone(), 1)?.entries,
                        Some(listing1),
                    ),
                },
                HomeComponent {
                    title: Some("User Ratings".to_string()),
                    subtitle: Some("Novels based on user ratings!".to_string()),
                    value: HomeComponentValue::NovelScroller(
                        Self::get_novel_list(listing2.clone(), 1)?.entries,
                        Some(listing2),
                    ),
                },
                HomeComponent {
                    title: Some("Most Bookmarked".to_string()),
                    subtitle: Some("Highest number of libraries".to_string()),
                    value: HomeComponentValue::NovelScroller(
                        Self::get_novel_list(listing3.clone(), 1)?.entries,
                        Some(listing3),
                    ),
                },
                HomeComponent {
                    title: Some("Most Read".to_string()),
                    subtitle: Some("Novels that are read the most".to_string()),
                    value: HomeComponentValue::NovelScroller(
                        Self::get_novel_list(listing4.clone(), 1)?.entries,
                        Some(listing4),
                    ),
                },
                HomeComponent {
                    title: Some("Most Reviewed".to_string()),
                    subtitle: Some("Novels with the most reviews".to_string()),
                    value: HomeComponentValue::NovelScroller(
                        Self::get_novel_list(listing5.clone(), 1)?.entries,
                        Some(listing5),
                    ),
                },
                HomeComponent {
                    title: Some("Most Commented".to_string()),
                    subtitle: Some("Novels with the most comments".to_string()),
                    value: HomeComponentValue::NovelScroller(
                        Self::get_novel_list(listing6.clone(), 1)?.entries,
                        Some(listing6),
                    ),
                },
            ],
        })
    }

    fn get_novel_list(listing: Listing, _page: i32) -> Result<PageResult> {
        let url = match listing.id.as_str() {
            "overall-ranking" => format!("{}/ranking", BASE_URL),
            "most-review" => format!("{}/ranking/most-review", BASE_URL),
            "most-lib" => format!("{}/ranking/most-lib", BASE_URL),
            "ratings" => format!("{}/ranking/ratings", BASE_URL),
            "most-read" => format!("{}/ranking/most-read", BASE_URL),
            "most-comment" => format!("{}/ranking/most-comment", BASE_URL),
            _ => format!("{}/ranking", BASE_URL),
        };

        let res = Request::get(&url).send()?;
        let html = Node::new(&res.body);
        let mut entries = Vec::new();
        
        let novel_nodes = html.select(".rank-novels .novel-item")?;
        for novel_node in novel_nodes {
            let a_nodes = novel_node.select("a")?;
            if let Some(a_first) = a_nodes.first() {
                let href = a_first.attr("href")?.unwrap_or_default();
                let key = href.replace("/book/", "");
                let url = format!("{}{}", BASE_URL, href);

                let cover = novel_node.select("img")?.into_iter().next()
                    .and_then(|img| img.attr("data-src").ok().flatten())
                    .map(|src| format!("{}{}", BASE_URL, src));

                let mut title = String::new();
                for a_node in a_nodes {
                    let txt = a_node.text()?.trim().to_string();
                    if !txt.is_empty() {
                        title = txt;
                    }
                    if title.is_empty() {
                        if let Some(t) = a_node.attr("title")? {
                            title = t;
                        }
                    }
                    if !title.is_empty() {
                        break;
                    }
                }

                entries.push(Novel {
                    key,
                    title,
                    authors: None,
                    artist: None,
                    description: None,
                    tags: None,
                    cover,
                    url: Some(url),
                    status: NovelStatus::Ongoing,
                    content_rating: ContentRating::Safe,
                    nsfw: 0,
                    chapters: None,
                });
            }
        }

        Ok(PageResult {
            entries,
            has_next_page: false,
        })
    }

    fn get_search_novel_list(query: String, page: i32, _filters: Vec<FilterItem>) -> Result<PageResult> {
        let url = format!(
            "{}/search?keyword={}&page={}",
            BASE_URL,
            query,
            page
        );

        let res = Request::get(&url).send()?;
        let html = Node::new(&res.body);

        let mut entries = Vec::new();
        let novel_nodes = html.select(".novel-list.chapters > li.novel-item")?;
        
        for novel_node in novel_nodes {
            let a_node = novel_node.select("a")?.into_iter().next();
            if let Some(a) = a_node {
                let key = a.attr("href")?.unwrap_or_default().replace("/book/", "");
                let title = a.attr("title")?.unwrap_or_default();
                
                let img_node = novel_node.select("img")?.into_iter().next();
                let cover = img_node.and_then(|img| img.attr("src").ok().flatten()).map(|src| format!("{}/{}", BASE_URL, src));

                entries.push(Novel {
                    key,
                    title,
                    authors: None,
                    artist: None,
                    description: None,
                    tags: None,
                    cover,
                    url: None,
                    status: NovelStatus::Unknown,
                    content_rating: ContentRating::Safe,
                    nsfw: 0,
                    chapters: None,
                });
            }
        }

        let has_next_page = html.select(".pagination li.page-item:last-child")?
            .into_iter()
            .next()
            .map(|node| node.attr("class").map(|c| !c.unwrap_or_default().contains("disabled")).unwrap_or(false))
            .unwrap_or(false);

        Ok(PageResult {
            entries,
            has_next_page,
        })
    }

    fn get_novel_update(novel: Novel, needs_details: bool, needs_chapters: bool) -> Result<Novel> {
        let mut updated_novel = novel.clone();

        if needs_details {
            let url = format!("{}/book/{}", BASE_URL, novel.key);
            let res = Request::get(&url).send()?;
            let html = Node::new(&res.body);

            let cover_node = html.select(".cover img")?.into_iter().next();
            if let Some(n) = cover_node {
                if let Some(src) = n.attr("src")? {
                    updated_novel.cover = Some(src);
                }
            }

            let title_node = html.select(".main-head .novel-title")?.into_iter().next();
            if let Some(n) = title_node {
                updated_novel.title = n.text()?;
            }

            let author_node = html.select(".author a")?.into_iter().next();
            if let Some(n) = author_node {
                if let Some(title) = n.attr("title")? {
                    updated_novel.authors = Some(vec![title]);
                }
            }

            let desc_nodes = html.select("#info .content p")?;
            let mut desc_parts = Vec::new();
            for n in desc_nodes {
                desc_parts.push(n.text()?);
            }
            if !desc_parts.is_empty() {
                updated_novel.description = Some(desc_parts.join("\n\n"));
            }

            let tag_nodes = html.select(".categories ul li a")?;
            let mut tags = Vec::new();
            for n in tag_nodes {
                tags.push(n.text()?);
            }
            if !tags.is_empty() {
                updated_novel.tags = Some(tags);
            }

            updated_novel.status = NovelStatus::Ongoing;
            updated_novel.content_rating = ContentRating::Safe;
            updated_novel.url = Some(url);
        }

        if needs_chapters {
            let mut page = 1;
            let mut all_chapters = Vec::new();

            loop {
                let url = format!("{}/book/{}/chapters?page={}", BASE_URL, novel.key, page);
                let res = Request::get(&url).send()?;
                let html = Node::new(&res.body);
                let chapter_nodes = html.select(".chapter-list > li")?;

                if chapter_nodes.is_empty() {
                    break;
                }

                for node in chapter_nodes {
                    let a_node = node.select("a")?.into_iter().next();
                    if let Some(a) = a_node {
                        let title = a.attr("title")?.unwrap_or_default();
                        let key = a.attr("href")?.map(|h| h.split('/').last().unwrap_or_default().to_string()).unwrap_or_default();
                        
                        let chap_no_node = node.select(".chapter-no")?.into_iter().next();
                        let chapter_number = match chap_no_node {
                            Some(n) => n.text()?.trim().parse::<f32>().ok(),
                            None => None,
                        };
                        
                        all_chapters.push(Chapter {
                            key,
                            title: Some(title.split(':').nth(1).unwrap_or("").trim().to_string()),
                            volume: None,
                            chapter: chapter_number,
                            date_updated: None, 
                            scanlator: None,
                            url: a.attr("href")?,
                            lang: None,
                            paywalled: None,
                        });
                    }
                }

                let has_next_page = html.select(".pagination li.page-item:last-child")?
                    .into_iter()
                    .next()
                    .map(|node| node.attr("class").map(|c| !c.unwrap_or_default().contains("disabled")).unwrap_or(false))
                    .unwrap_or(false);

                if !has_next_page {
                    break;
                }
                
                page += 1;
            }

            updated_novel.chapters = Some(all_chapters);
        }

        Ok(updated_novel)
    }

    fn get_chapter_content(novel: Novel, chapter: Chapter) -> Result<Vec<Page>> {
        let url = format!("{}/book/{}/{}", BASE_URL, novel.key, chapter.key);
        let res = Request::get(&url).send()?;
        let html = Node::new(&res.body);

        let mut content_list = Vec::new();
        let p_nodes = html.select("#content p")?;

        for (index, node) in p_nodes.into_iter().enumerate() {
            let content = node.text()?;
            
            if content.starts_with('[') && content.ends_with(']') {
                let quote = content[1..content.len()-1].to_string();
                content_list.push(Page {
                    index: index as i32,
                    content: PageContent::Text(quote),
                    has_description: false,
                    description: None,
                    headers: None,
                });
            } else if content == "***" {
                content_list.push(Page {
                    index: index as i32,
                    content: PageContent::Text("---".to_string()), 
                    has_description: false,
                    description: None,
                    headers: None,
                });
            } else {
                content_list.push(Page {
                    index: index as i32,
                    content: PageContent::Text(content),
                    has_description: false,
                    description: None,
                    headers: None,
                });
            }
        }

        let current_len = content_list.len() as i32;
        content_list.push(Page {
            index: current_len,
            content: PageContent::Text("---".to_string()),
            has_description: false,
            description: None,
            headers: None,
        });

        let review_link = format!("LINK: [click here for chapter reviews.]({})", url);
        content_list.push(Page {
            index: current_len + 1,
            content: PageContent::Text(review_link),
            has_description: false,
            description: None,
            headers: None,
        });

        Ok(content_list)
    }
}

export_novel_plugin!(NovelFire);
