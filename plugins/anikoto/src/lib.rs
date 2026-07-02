use ito_rs::{
    export_anime_plugin,
    models::{
        anime::{Anime, ContentRating, Episode, PageResult, Status, Video},
        FilterItem, HomeLayout, LinkValue, Listing, HomeComponent, HomeComponentValue, AnimeWithEpisode
    },
    provider::AnimeProvider,
    Result, Error,
    net::Request,
    html::Node,
    webview::Webview,
};

mod megaplay;

pub struct AnikotoProvider;

const BASE_URL: &str = "https://anikototv.to";

impl AnikotoProvider {
    fn url_encode(text: &str) -> String {
        let mut encoded = String::new();
        for b in text.as_bytes() {
            match *b {
                b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                    encoded.push(*b as char);
                }
                b' ' => {
                    encoded.push('+');
                }
                _ => {
                    encoded.push_str(&format!("%{:02X}", b));
                }
            }
        }
        encoded
    }

    fn parse_anime_item(node: &Node) -> Result<Anime> {
        let mut href = node.attr("href")?.unwrap_or_default();
        if href.is_empty() {
            if let Ok(links) = node.select("a") {
                if let Some(first_link) = links.first() {
                    href = first_link.attr("href")?.unwrap_or_default();
                }
            }
        }

        let mut url = href.clone();
        if !url.starts_with("http") {
            url = format!("{}{}", BASE_URL, url);
        }

        let mut key = href.clone();
        if key.contains("/watch/") {
            key = key.split("/watch/").last().unwrap_or_default().to_string();
            // Try to extract just the slug by removing /ep-
            if let Some(slug) = key.split("/ep-").next() {
                key = slug.to_string();
            }
        }

        let mut title = String::new();
        if let Ok(name_nodes) = node.select(".name") {
            if let Some(name_node) = name_nodes.first() {
                title = name_node.text().unwrap_or_default().trim().to_string();
            }
        }
        if title.is_empty() {
            if let Ok(imgs) = node.select("img") {
                if let Some(img) = imgs.first() {
                    title = img.attr("alt").unwrap_or_default().unwrap_or_default();
                }
            }
        }

        let mut cover = None;
        if let Ok(imgs) = node.select("img") {
            if let Some(img) = imgs.first() {
                cover = img.attr("src").unwrap_or_default();
            }
        }

        Ok(Anime {
            key,
            title,
            studios: None,
            description: None,
            tags: None,
            cover,
            url: Some(url),
            status: Status::Unknown,
            content_rating: ContentRating::Safe,
            nsfw: 0,
            episodes: None,
            seasons: None,
        })
    }
}

impl AnimeProvider for AnikotoProvider {
    fn get_home() -> Result<HomeLayout> {
        let res = Request::get(format!("{}/home", BASE_URL)).send()?;
        if res.status != 200 {
            return Ok(HomeLayout { components: vec![] });
        }

        let document = Node::new(&res.body);
        let mut components = Vec::new();

        if let Ok(nodes) = document.select("#recent-update .item") {
            let mut entries = Vec::new();
            for node in nodes {
                if let Ok(anime) = Self::parse_anime_item(&node) {
                    let ep_num = node.select(".ep-status.sub span")
                        .ok()
                        .and_then(|n| n.first().map(|s| s.text().unwrap_or_default()))
                        .and_then(|t| t.trim().parse::<f32>().ok());
                        
                    entries.push(AnimeWithEpisode {
                        anime,
                        episode: Episode {
                            key: String::new(),
                            title: None,
                            episode: ep_num,
                            date_updated: None,
                            url: None,
                            lang: None,
                            paywalled: None,
                        },
                    });
                }
            }
            if !entries.is_empty() {
                components.push(HomeComponent {
                    title: Some("Latest Episode".to_string()),
                    subtitle: None,
                    value: HomeComponentValue::AnimeEpisodeList(None, entries, None),
                });
            }
        }

        if let Ok(nodes) = document.select(".scaff.side.items .item") {
            let mut entries = Vec::new();
            for node in nodes {
                if let Ok(anime) = Self::parse_anime_item(&node) {
                    entries.push(anime);
                }
            }
            if !entries.is_empty() {
                components.push(HomeComponent {
                    title: Some("Top anime".to_string()),
                    subtitle: None,
                    value: HomeComponentValue::AnimeScroller(entries, None),
                });
            }
        }

        Ok(HomeLayout { components })
    }

    fn get_anime_list(_listing: Listing, _page: i32) -> Result<PageResult> {
        Ok(PageResult { entries: vec![], has_next_page: false })
    }

    fn get_search_anime_list(query: &str, page: i32, _filters: Vec<FilterItem>) -> Result<PageResult> {
        let url = format!("{}/filter?keyword={}&page={}", BASE_URL, Self::url_encode(query), page);
        let res = Request::get(&url).send()?;
        if res.status != 200 {
            return Ok(PageResult { entries: vec![], has_next_page: false });
        }

        let document = Node::new(&res.body);
        let mut entries = Vec::new();
        if let Ok(items) = document.select("#list-items .item") {
            for item in items {
                if let Ok(anime) = Self::parse_anime_item(&item) {
                    entries.push(anime);
                }
            }
        }

        let has_next_page = !entries.is_empty();

        Ok(PageResult { entries, has_next_page })
    }

    fn get_anime_update(mut anime: Anime, needs_details: bool, needs_episodes: bool) -> Result<Anime> {
        let mut fetch_url = format!("{}/watch/{}", BASE_URL, anime.key);
        // Fallback to ep-1 if /watch/slug fails or if anime.url specifically contains ep-1
        if let Some(ref u) = anime.url {
            if u.contains("/ep-") {
                fetch_url = u.clone();
            }
        }
        
        ito_rs::host::print(&format!("Fetching details from: {}", fetch_url));
        
        let mut res = Webview::load_url(&fetch_url);
        if res.is_err() {
            let fallback_url = format!("{}/watch/{}/ep-1", BASE_URL, anime.key);
            ito_rs::host::print(&format!("Failed loading url, trying fallback: {}", fallback_url));
            res = Webview::load_url(&fallback_url);
            if res.is_err() {
                ito_rs::host::print("Fallback failed too.");
                return Ok(anime);
            }
        }
        
        let response = res.unwrap();
        let document = Node::new(response.html.as_bytes());
        
        if needs_details {
            if let Ok(nodes) = document.select("h1.title") {
                if let Some(node) = nodes.first() {
                    anime.title = node.text().unwrap_or_default().trim().to_string();
                    ito_rs::host::print(&format!("Found title: {}", anime.title));
                }
            }
            if let Ok(nodes) = document.select(".binfo .poster img") {
                if let Some(node) = nodes.first() {
                    anime.cover = node.attr("src").unwrap_or_default();
                }
            }
            if let Ok(nodes) = document.select(".synopsis .content") {
                if let Some(node) = nodes.first() {
                    anime.description = Some(node.text().unwrap_or_default().trim().to_string());
                    ito_rs::host::print("Found description.");
                }
            }
            
            // Genres
            if let Ok(nodes) = document.select(".bmeta .meta a") {
                let mut tags = Vec::new();
                for node in nodes {
                    if let Some(href) = node.attr("href").unwrap_or_default() {
                        if href.contains("/genre/") {
                            tags.push(node.text().unwrap_or_default().trim().to_string());
                        } else if href.contains("/status/") {
                            let text = node.text().unwrap_or_default().trim().to_lowercase();
                            anime.status = if text.contains("airing") {
                                Status::Ongoing
                            } else if text.contains("completed") {
                                Status::Completed
                            } else {
                                Status::Unknown
                            };
                        }
                    }
                }
                if !tags.is_empty() {
                    anime.tags = Some(tags);
                }
            }
            
            anime.url = Some(fetch_url);
        }

        if needs_episodes {
            if let Ok(nodes) = document.select("#w-episodes a") {
                if nodes.is_empty() {
                    ito_rs::host::print("WARNING: #w-episodes a matched 0 nodes!");
                    if let Ok(ep_nodes) = document.select("#w-episodes") {
                        if let Some(ep_node) = ep_nodes.first() {
                            ito_rs::host::print(&format!("HTML inside #w-episodes: {}", ep_node.html().unwrap_or_default()));
                        } else {
                            ito_rs::host::print("WARNING: #w-episodes also not found! Server might have changed HTML completely.");
                        }
                    }
                }
                
                let mut episodes = Vec::new();
                for node in nodes {
                    let mut ep_num_str = node.attr("data-num").unwrap_or_default().unwrap_or_default();
                    if ep_num_str.is_empty() {
                        ep_num_str = node.attr("data-slug").unwrap_or_default().unwrap_or_default();
                    }
                    
                    let href = node.attr("href").unwrap_or_default().unwrap_or_default();
                    if ep_num_str.is_empty() && href.contains("/ep-") {
                        if let Some(slug) = href.split("/ep-").last() {
                            ep_num_str = slug.to_string();
                        }
                    }

                    let mal_id = node.attr("data-mal").unwrap_or_default().unwrap_or_default();
                    
                    if ep_num_str.is_empty() {
                        ito_rs::host::print(&format!("Skipping node without ep_num: href='{}'", href));
                        continue;
                    }
                    
                    let ep_num = ep_num_str.parse::<f32>().ok();
                    let data_ids = node.attr("data-ids").unwrap_or_default().unwrap_or_default();
                    let key = if !data_ids.is_empty() {
                        data_ids
                    } else {
                        format!("{}:{}", mal_id, ep_num_str)
                    };
                    
                    let title = node.select(".d-title")
                        .ok()
                        .and_then(|n| n.first().map(|s| s.text().unwrap_or_default()))
                        .unwrap_or_else(|| format!("Episode {}", ep_num_str));

                    episodes.push(Episode {
                        key,
                        title: Some(title.trim().to_string()),
                        episode: ep_num,
                        date_updated: None,
                        url: Some(href),
                        lang: None,
                        paywalled: None,
                    });
                }
                ito_rs::host::print(&format!("Found {} episodes.", episodes.len()));
                if episodes.is_empty() {
                    ito_rs::host::print("WARNING: Nodes were found but all were skipped.");
                }
                anime.episodes = Some(episodes);
            } else {
                ito_rs::host::print("No episodes found in details page.");
            }
        }

        Ok(anime)
    }

    fn get_video_list(_anime: Anime, episode: Episode) -> Result<Vec<Video>> {
        let mut videos = Vec::new();
        
        let data_ids = episode.key;
        if data_ids.is_empty() {
            ito_rs::host::print("Video list: Episode key (data-ids) is empty");
            return Err(Error::Unsupported);
        }

        let list_url = format!("{}/ajax/server/list?servers={}", BASE_URL, data_ids);
        ito_rs::host::print(&format!("Video list: Fetching server list: {}", list_url));
        
        let list_res = Request::get(&list_url)
            .header("X-Requested-With", "XMLHttpRequest")
            .send()?;
            
        if list_res.status != 200 {
            ito_rs::host::print(&format!("Video list: Server list returned status {}", list_res.status));
            return Err(Error::Unsupported);
        }
        
        let list_json: serde_json::Value = serde_json::from_slice(&list_res.body).map_err(|_| Error::Unsupported)?;
        let html_str = list_json.get("result").and_then(|r| r.as_str()).unwrap_or_default();
        
        let document = Node::new(html_str.as_bytes());
        
        if let Ok(type_nodes) = document.select(".servers .type") {
            for type_node in type_nodes {
                let data_type = type_node.attr("data-type").unwrap_or_default().unwrap_or_default();
                let type_label = match data_type.to_lowercase().as_str() {
                    "sub" => "SUB",
                    "hsub" => "Hard SUB",
                    "dub" => "DUB",
                    _ => &data_type,
                };
                
                if let Ok(server_nodes) = type_node.select("li[data-link-id]") {
                    for node in server_nodes {
                        let link_id = node.attr("data-link-id").unwrap_or_default().unwrap_or_default();
                        if link_id.is_empty() {
                            continue;
                        }

                        let server_name = node.text().unwrap_or_default().trim().to_string();
                        
                        let ajax_url = format!("{}/ajax/server?get={}", BASE_URL, link_id);
                        if let Ok(ajax_res) = Request::get(&ajax_url)
                            .header("X-Requested-With", "XMLHttpRequest")
                            .send() 
                        {
                            if ajax_res.status == 200 {
                                if let Ok(json) = serde_json::from_slice::<serde_json::Value>(&ajax_res.body) {
                                    if let Some(embed_url) = json.get("result").and_then(|r| r.get("url")).and_then(|u| u.as_str()) {
                                        ito_rs::host::print(&format!("Video list: Found embed_url: {}", embed_url));
                                        if let Some(mut extracted_videos) = megaplay::extract(embed_url, BASE_URL) {
                                            for v in &mut extracted_videos {
                                                v.quality = format!("{} {} {}", v.quality, type_label, server_name);
                                            }
                                            videos.extend(extracted_videos);
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        } else {
            ito_rs::host::print("Video list: Failed to find .servers .type");
        }

        Ok(videos)
    }

    fn handle_url(url: &str) -> Result<LinkValue> {
        if url.contains("/watch/") {
            let key = url.split("/watch/").last().unwrap_or_default().split('/').next().unwrap_or_default().to_string();
            Ok(LinkValue::Anime(Anime {
                key,
                title: String::new(),
                studios: None,
                description: None,
                tags: None,
                cover: None,
                url: Some(url.to_string()),
                status: Status::Unknown,
                content_rating: ContentRating::Safe,
                nsfw: 0,
                episodes: None,
                seasons: None,
            }))
        } else {
            Err(Error::Unsupported)
        }
    }
}

export_anime_plugin!(AnikotoProvider);