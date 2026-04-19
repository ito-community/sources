use ito_rs::{
    export_anime_plugin,
    models::{
        anime::{Anime, ContentRating, Episode, PageResult, Status, Video, Season},
        FilterItem, HomeLayout, Listing, LinkValue, HomeComponent, HomeComponentValue, AnimeWithEpisode,
    },
    provider::AnimeProvider,
    Result,
    net::Request,
    html::Node,
    host,
};
use std::collections::HashMap;

pub struct AnimeKaiProvider;

const BASE_URL: &str = "https://anikai.to";

impl AnimeKaiProvider {
    fn encode_token(text: &str) -> Option<String> {
        let url = format!("https://enc-dec.app/api/enc-kai?text={}", Self::url_encode(text));
        host::print(&format!("AnimeKai: Encoding token via {}", url));
        let res = Request::get(&url).send();
        match res {
            Ok(res) => {
                if res.status == 200 {
                    if let Ok(json) = serde_json::from_slice::<serde_json::Value>(&res.body) {
                        if json.get("status").and_then(|s| s.as_i64()) == Some(200) {
                            return json.get("result").and_then(|r| r.as_str()).map(|s| s.to_string());
                        } else {
                            host::print(&format!("AnimeKai: Encode API status error: {:?}", json.get("status")));
                        }
                    }
                } else {
                    host::print(&format!("AnimeKai: Encode API status: {}", res.status));
                }
            }
            Err(e) => {
                host::print(&format!("AnimeKai: Encode API request failed: {}", e));
            }
        }
        None
    }

    fn decode_kai(text: &str) -> Option<serde_json::Value> {
        let mut map = HashMap::new();
        map.insert("text", text);
        let body = serde_json::to_vec(&map).unwrap_or_default();

        let res = Request::post("https://enc-dec.app/api/dec-kai")
            .header("Content-Type", "application/json")
            .body(&body)
            .send().ok()?;

        if res.status == 200 {
            if let Ok(json) = serde_json::from_slice::<serde_json::Value>(&res.body) {
                if json.get("status").and_then(|s| s.as_i64()) == Some(200) {
                    return json.get("result").cloned();
                }
            }
        }
        None
    }
    
    fn decode_mega(text: &str, user_agent: Option<&str>) -> Option<serde_json::Value> {
        let mut map = HashMap::new();
        map.insert("text", text);
        let default_ua = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36";
        map.insert("agent", user_agent.unwrap_or(default_ua));
        let body = serde_json::to_vec(&map).unwrap_or_default();

        let res = Request::post("https://enc-dec.app/api/dec-mega")
            .header("Content-Type", "application/json")
            .body(&body)
            .send().ok()?;

        if res.status == 200 {
            if let Ok(json) = serde_json::from_slice::<serde_json::Value>(&res.body) {
                if json.get("status").and_then(|s| s.as_i64()) == Some(200) {
                    if let Some(result) = json.get("result") {
                        if result.is_string() {
                            let result_str = result.as_str().unwrap();
                            if let Ok(parsed_result) = serde_json::from_str::<serde_json::Value>(result_str) {
                                return Some(parsed_result);
                            }
                        }
                        return Some(result.clone());
                    }
                }
            }
        }
        None
    }

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
                    use std::fmt::Write;
                    write!(&mut encoded, "%{:02X}", b).unwrap();
                }
            }
        }
        encoded
    }

    fn parse_anime_item(item: &Node) -> Result<Anime> {
        let href = item.attr("href")?.unwrap_or_default();
        let slug = if href.starts_with("/watch/") {
            href.replace("/watch/", "").split('#').next().unwrap_or_default().to_string()
        } else {
            href.split('#').next().unwrap_or_default().to_string()
        };

        let title_node = item.select(".title")?.into_iter().next()
            .or(item.select(".detail .title")?.into_iter().next());
        
        let title = if let Some(node) = title_node {
            node.text()?.trim().to_string()
        } else {
            item.attr("title")?.unwrap_or_default()
        };

        let mut cover = item.select("img")?.first().and_then(|img| img.attr("data-src").ok().flatten())
            .or(item.select("img")?.first().and_then(|img| img.attr("src").ok().flatten()));

        if cover.is_none() {
            if let Some(style) = item.attr("style")? {
                if let Some(start) = style.find("url(") {
                    let rest = &style[start + 4..];
                    if let Some(end) = rest.find(')') {
                        cover = Some(rest[..end].trim_matches(|c| c == '\'' || c == '"').to_string());
                    }
                }
            }
        }

        Ok(Anime {
            key: slug.clone(),
            title,
            studios: None,
            description: None,
            tags: None,
            cover,
            url: Some(format!("{}/watch/{}", BASE_URL, slug)),
            status: Status::Unknown,
            content_rating: ContentRating::Safe,
            nsfw: 0,
            episodes: None,
            seasons: None,
        })
    }
}

impl AnimeProvider for AnimeKaiProvider {
    fn get_home_stream() -> Result<bool> {
        Ok(false)
    }

    fn get_home() -> Result<HomeLayout> {
        let res = Request::get(format!("{}/home", BASE_URL)).send()?;
        if res.status != 200 {
            return Ok(HomeLayout { components: vec![] });
        }

        let document = Node::new(&res.body);
        let mut components = Vec::new();

        // Featured (Big Scroller)
        if let Ok(slides) = document.select("section#featured .swiper-slide") {
            let mut entries = Vec::new();
            for slide in slides {
                if let Some(link) = slide.select(".detail a.watch-btn")?.into_iter().next() {
                    let mut anime = Self::parse_anime_item(&link)?;
                    if let Some(title_node) = slide.select(".detail p.title")?.into_iter().next() {
                        anime.title = title_node.text()?;
                    }
                    if let Some(desc_node) = slide.select(".detail p.desc")?.into_iter().next() {
                        anime.description = Some(desc_node.text()?);
                    }
                    if anime.cover.is_none() {
                        if let Some(style) = slide.attr("style")? {
                            if let Some(start) = style.find("url(") {
                                let rest = &style[start + 4..];
                                if let Some(end) = rest.find(')') {
                                    anime.cover = Some(rest[..end].trim_matches(|c| c == '\'' || c == '"').to_string());
                                }
                            }
                        }
                    }
                    entries.push(anime);
                }
            }
            if !entries.is_empty() {
                components.push(HomeComponent {
                    title: None,
                    subtitle: None,
                    value: HomeComponentValue::AnimeBigScroller(entries, Some(10.0)),
                });
            }
        }

        // Latest Updates
        if let Ok(nodes) = document.select("section#latest-updates .aitem") {
            let mut entries = Vec::new();
            for node in nodes {
                let anime = Self::parse_anime_item(&node)?;
                let ep_num = node.select(".info .sub")?.first()
                    .or(node.select(".info .dub")?.first())
                    .and_then(|e| e.text().ok())
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
            if !entries.is_empty() {
                components.push(HomeComponent {
                    title: Some("Latest Updates".to_string()),
                    subtitle: None,
                    value: HomeComponentValue::AnimeEpisodeList(None, entries, None),
                });
            }
        }

        // Top Trending
        if let Ok(nodes) = document.select("section#trending-anime .aitem") {
            let mut entries = Vec::new();
            for node in nodes {
                let anime = Self::parse_anime_item(&node)?;
                entries.push(anime);
            }
            if !entries.is_empty() {
                components.push(HomeComponent {
                    title: Some("Top Trending".to_string()),
                    subtitle: None,
                    value: HomeComponentValue::AnimeScroller(entries, None),
                });
            }
        }

        Ok(HomeLayout { components })
    }

    fn get_anime_list(_listing: Listing, _page: i32) -> Result<PageResult> {
        Ok(PageResult {
            entries: Vec::new(),
            has_next_page: false,
        })
    }

    fn get_search_anime_list(query: String, _page: i32, _filters: Vec<FilterItem>) -> Result<PageResult> {
        let search_url = format!(
            "{}/ajax/anime/search?keyword={}",
            BASE_URL,
            Self::url_encode(&query)
        );

        let res = Request::get(&search_url)
            .header("X-Requested-With", "XMLHttpRequest")
            .header("Referer", &format!("{}/", BASE_URL))
            .send()?;

        if res.status != 200 {
            return Ok(PageResult {
                entries: Vec::new(),
                has_next_page: false,
            });
        }

        let mut entries = Vec::new();
        if let Ok(json) = serde_json::from_slice::<serde_json::Value>(&res.body) {
            if let Some(html) = json.get("result").and_then(|r| r.get("html")).and_then(|h| h.as_str()) {
                let document = Node::new(html.as_bytes());
                let items = document.select("a.aitem")?;
                for item in items {
                    entries.push(Self::parse_anime_item(&item)?);
                }
            }
        }

        Ok(PageResult {
            entries,
            has_next_page: false,
        })
    }

    fn get_anime_update(mut anime: Anime, needs_details: bool, needs_episodes: bool) -> Result<Anime> {
        let url = format!("{}/watch/{}", BASE_URL, anime.key);
        let res = Request::get(&url)
            .header("Referer", &format!("{}/", BASE_URL))
            .send()?;

        if res.status != 200 {
            return Ok(anime);
        }

        let body_str = String::from_utf8_lossy(&res.body);
        let document = Node::new(&res.body);

        if needs_details {
            if let Some(node) = document.select("h1.title")?.first() {
                anime.title = node.text()?;
            }
            if let Some(node) = document.select(".poster img")?.first() {
                anime.cover = node.attr("src")?;
            }
            if let Some(node) = document.select(".desc")?.first() {
                anime.description = Some(node.text()?);
            }

            // Status
            for node in document.select(".detail > div > div")? {
                let text = node.text()?;
                if text.contains("Status:") {
                    let s = node.select("span")?.first().map(|e| e.text()).transpose()?.unwrap_or_default().to_lowercase();
                    anime.status = match s.trim() {
                        "ongoing" => Status::Ongoing,
                        "completed" => Status::Completed,
                        "hiatus" => Status::Hiatus,
                        "cancelled" => Status::Cancelled,
                        _ => Status::Unknown,
                    };
                }
                if text.contains("Studios:") {
                    let mut studios = Vec::new();
                    for a in node.select("span a")? {
                        studios.push(a.text()?);
                    }
                    if ! studios.is_empty() {
                        anime.studios = Some(studios);
                    }
                }
                if text.contains("Genres:") {
                    let mut tags = Vec::new();
                    for a in node.select("span a")? {
                        tags.push(a.text()?);
                    }
                    if !tags.is_empty() {
                        anime.tags = Some(tags);
                    }
                }
            }

            // Seasons
            let mut seasons = Vec::new();
            for node in document.select("section#seasons .swiper-slide.aitem")? {
                if let Some(link) = node.select("a.poster")?.first() {
                    let href = link.attr("href")?.unwrap_or_default();
                    let key = href.replace("/watch/", "");
                    let title = node.select(".detail span")?.first().map(|e| e.text()).transpose()?.unwrap_or_default();
                    let is_current = node.attr("class")?.map(|c| c.contains("active")).unwrap_or_default();
                    seasons.push(Season {
                        key,
                        title,
                        is_current,
                    });
                }
            }
            if !seasons.is_empty() {
                anime.seasons = Some(seasons);
            }

            anime.url = Some(url);
        }

        if needs_episodes {
            let mut ani_id = String::new();
            if let Some(start) = body_str.find(r#"id="syncData""#) {
                if let Some(end) = body_str[start..].find("</script>") {
                    let script_content = &body_str[start..start + end];
                    if let Some(json_start) = script_content.find('>') {
                        let json_str = script_content[json_start + 1..].trim();
                        if let Ok(json) = serde_json::from_str::<serde_json::Value>(json_str) {
                            if let Some(id) = json.get("anime_id").and_then(|v| v.as_str()) {
                                ani_id = id.to_string();
                            }
                        }
                    }
                }
            }

            if !ani_id.is_empty() {
                if let Some(encoded) = Self::encode_token(&ani_id) {
                    let ep_list_url = format!("{}/ajax/episodes/list?ani_id={}&_={}", BASE_URL, ani_id, encoded);
                    let ep_res = Request::get(&ep_list_url)
                        .header("X-Requested-With", "XMLHttpRequest")
                        .header("Referer", &format!("{}/", BASE_URL))
                        .send()?;
                    
                    if ep_res.status == 200 {
                        if let Ok(json) = serde_json::from_slice::<serde_json::Value>(&ep_res.body) {
                            if let Some(html) = json.get("result").and_then(|r| r.as_str()) {
                                let ep_doc = Node::new(html.as_bytes());
                                let mut episodes = Vec::new();
                                for node in ep_doc.select("a")? {
                                    let ep_num = node.attr("num")?.unwrap_or_default();
                                    let ep_token = node.attr("token")?.unwrap_or_default();
                                    if ep_token.is_empty() { continue; }
                                    let title = node.select("span")?.first().map(|e| e.text()).transpose()?.unwrap_or_default();

                                    episodes.push(Episode {
                                        key: ep_token,
                                        title: if title.is_empty() { Some(format!("Episode {}", ep_num)) } else { Some(title) },
                                        episode: ep_num.parse::<f32>().ok(),
                                        date_updated: None,
                                        url: None,
                                        lang: None,
                                        paywalled: None,
                                    });
                                }
                                if !episodes.is_empty() {
                                    anime.episodes = Some(episodes);
                                }
                            }
                        }
                    }
                }
            }
        }

        Ok(anime)
    }

    fn get_video_list(_anime: Anime, episode: Episode) -> Result<Vec<Video>> {
        let ep_token = episode.key;
        let mut videos = Vec::new();

        if let Some(encoded_token) = Self::encode_token(&ep_token) {
            let servers_url = format!(
                "{}/ajax/links/list?token={}&_={}",
                BASE_URL, ep_token, encoded_token
            );
            
            let res = Request::get(&servers_url)
                .header("X-Requested-With", "XMLHttpRequest")
                .header("Referer", &format!("{}/", BASE_URL))
                .send()?;

            if res.status == 200 {
                if let Ok(json) = serde_json::from_slice::<serde_json::Value>(&res.body) {
                    if let Some(html) = json.get("result").and_then(|r| r.as_str()) {
                        let document = Node::new(html.as_bytes());
                        let groups = document.select(".server-items")?;

                        for group in groups {
                            let lang = group.attr("data-id")?.unwrap_or_else(|| "sub".to_string());
                            let servers = group.select(".server")?;

                            for server in servers {
                                let link_id = server.attr("data-lid")?.unwrap_or_default();
                                let server_name = server.text()?.trim().to_string();

                                if !link_id.is_empty() {
                                    if let Some(encoded_link) = Self::encode_token(&link_id) {
                                        let view_url = format!(
                                            "{}/ajax/links/view?id={}&_={}",
                                            BASE_URL, link_id, encoded_link
                                        );
                                        let view_res = Request::get(&view_url)
                                            .header("X-Requested-With", "XMLHttpRequest")
                                            .header("Referer", &format!("{}/", BASE_URL))
                                            .send()?;

                                        if view_res.status == 200 {
                                            if let Ok(view_json) = serde_json::from_slice::<serde_json::Value>(&view_res.body) {
                                                if let Some(encrypted_res) = view_json.get("result").and_then(|r| r.as_str()) {
                                                    let mut embed_data = Self::decode_kai(encrypted_res);
                                                    if embed_data.is_none() {
                                                        let ua = view_res.headers.get("X-Used-User-Agent").map(|s| s.as_str());
                                                        embed_data = Self::decode_mega(encrypted_res, ua);
                                                    }
                                                    
                                                    if let Some(data) = embed_data {
                                                        if let Some(embed_url) = data.get("url").and_then(|u| u.as_str()) {
                                                            let quality_label = format!("{} ({})", server_name, lang);

                                                            if embed_url.contains("/e/") || embed_url.contains("megacloud") || embed_url.contains("rapid-cloud") {
                                                                if let Some((playlist_url, subtitles)) = megacloud::extract(embed_url) {
                                                                    // Skip URLs containing megaup.cc because the CDN nodes (ap9.megaup.cc) have broken SSL certs on iOS.
                                                                    // These are duplicates of working lab27core.site entries.
                                                                    if !playlist_url.contains(".megaup.cc") {
                                                                        let mut headers = HashMap::new();
                                                                        headers.insert("User-Agent".to_string(), "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.0 Safari/605.1.15".to_string());

                                                                        videos.push(Video {
                                                                            url: playlist_url,
                                                                            quality: quality_label,
                                                                            headers: Some(headers),
                                                                            audio_tracks: None,
                                                                            subtitles,
                                                                        });
                                                                    } else {
                                                                        ito_rs::host::print(&format!("[DEBUG-MEGACLOUD] Skipping megaup.cc CDN (iOS TLS broken): {}", playlist_url));
                                                                    }
                                                                } else {
                                                                    let mut fallback_success = false;
                                                                    let video_id = embed_url.trim_end_matches('/').split('/').last().unwrap_or_default();
                                                                    let embed_base = if embed_url.contains("/e/") {
                                                                        embed_url.split("/e/").next().unwrap_or_default()
                                                                    } else {
                                                                        embed_url.rsplitn(2, '/').last().unwrap_or_default()
                                                                    };
                                                                    let media_url = format!("{}/media/{}", embed_base, video_id);
                                                                    ito_rs::host::print(&format!("[DEBUG-MEDIA] Fetching media endpoint: {}", media_url));

                                                                    if let Ok(m_res) = Request::get(&media_url)
                                                                        .header("Referer", embed_url)
                                                                        .header("User-Agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36")
                                                                        .send() {
                                                                        ito_rs::host::print(&format!("[DEBUG-MEDIA] Media endpoint status: {}", m_res.status));
                                                                        if m_res.status == 200 {
                                                                            if let Ok(media_json) = serde_json::from_slice::<serde_json::Value>(&m_res.body) {
                                                                                if let Some(encrypted_media) = media_json.get("result").and_then(|r| r.as_str()) {
                                                                                    ito_rs::host::print(&format!("[DEBUG-MEDIA] Got encrypted media payload (len {})", encrypted_media.len()));
                                                                                    
                                                                                    let mut user_agent = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36".to_string();
                                                                                    if let Some(ua) = m_res.headers.get("X-Used-User-Agent").or_else(|| m_res.headers.get("x-used-user-agent")) {
                                                                                        user_agent = ua.clone();
                                                                                    }

                                                                                    if let Some(dec_data) = Self::decode_mega(encrypted_media, Some(&user_agent)) {
                                                                                        if let Some(sources) = dec_data.get("sources").and_then(|s| s.as_array()) {
                                                                                            if let Some(first_source) = sources.first() {
                                                                                                if let Some(dec_url) = first_source.get("file").and_then(|f| f.as_str()) {
                                                                                                    ito_rs::host::print(&format!("AnimeKai: Megacloud failed, but /media/ fallback worked: {}", dec_url));

                                                                                                    // Skip broken CDN nodes
                                                                                                    if !dec_url.contains(".megaup.cc") {
                                                                                                        let mut final_subtitles = Vec::new();
                                                                                                        if let Some(tracks) = dec_data.get("tracks").and_then(|t| t.as_array()) {
                                                                                                            for track in tracks {
                                                                                                                let file = track.get("file").and_then(|f| f.as_str()).unwrap_or("");
                                                                                                                let label = track.get("label").and_then(|l| l.as_str()).unwrap_or("");
                                                                                                                let kind = track.get("kind").and_then(|k| k.as_str()).unwrap_or("");

                                                                                                                if kind == "captions" || kind == "subtitles" {
                                                                                                                    final_subtitles.push(ito_rs::models::anime::Subtitle {
                                                                                                                        url: file.to_string(),
                                                                                                                        language: label.to_string(),
                                                                                                                        format: "vtt".to_string(),
                                                                                                                        is_hardsub: false,
                                                                                                                    });
                                                                                                                }
                                                                                                            }
                                                                                                        }

                                                                                                        let mut media_headers = HashMap::new();
                                                                                                        media_headers.insert("User-Agent".to_string(), "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.0 Safari/605.1.15".to_string());

                                                                                                        videos.push(Video {
                                                                                                            url: dec_url.to_string(),
                                                                                                            quality: quality_label.clone(),
                                                                                                            headers: Some(media_headers),
                                                                                                            audio_tracks: None,
                                                                                                            subtitles: if final_subtitles.is_empty() { None } else { Some(final_subtitles) },
                                                                                                        });
                                                                                                        fallback_success = true;
                                                                                                    } else {
                                                                                                        ito_rs::host::print(&format!("[DEBUG-MEDIA] Skipping fallback megaup.cc CDN (iOS TLS broken): {}", dec_url));
                                                                                                    }
                                                                                                }
                                                                                            }
                                                                                        }
                                                                                    }
                                                                                }
                                                                            }
                                                                        }
                                                                    }

                                                                    if !fallback_success {
                                                                        ito_rs::host::print("AnimeKai: All extractions failed, adding embed URL as final fallback");
                                                                        videos.push(Video {
                                                                            url: embed_url.to_string(),
                                                                            quality: quality_label,
                                                                            headers: None,
                                                                            audio_tracks: None,
                                                                            subtitles: None,
                                                                        });
                                                                    }
                                                                }
                                                            } else {
                                                                videos.push(Video {
                                                                    url: embed_url.to_string(),
                                                                    quality: quality_label,
                                                                    headers: None,
                                                                    audio_tracks: None,
                                                                    subtitles: None,
                                                                });
                                                            }
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        Ok(videos)
    }

    fn handle_url(url: String) -> Result<LinkValue> {
        if url.contains("/watch/") {
            let key = url.split("/watch/").last().unwrap_or_default().split('#').next().unwrap_or_default().to_string();
            Ok(LinkValue::Anime(Anime {
                key,
                title: String::new(),
                studios: None,
                description: None,
                tags: None,
                cover: None,
                url: Some(url),
                status: Status::Unknown,
                content_rating: ContentRating::Safe,
                nsfw: 0,
                episodes: None,
                seasons: None,
            }))
        } else {
            Err(ito_rs::Error::Unsupported)
        }
    }
}

// MARK: - MegaCloud Crypto Module
mod megacloud {
    use ito_rs::net::Request;
    use std::collections::HashMap;

    pub fn extract(embed_url: &str) -> Option<(String, Option<Vec<ito_rs::models::anime::Subtitle>>)> {
        ito_rs::host::print(&format!("[DEBUG-MEGACLOUD] Extracting from: {}", embed_url));
        let res = Request::get(embed_url)
            .header("Referer", "https://anikai.to/")
            .send().ok()?;
        
        ito_rs::host::print(&format!("[DEBUG-MEGACLOUD] Embed page fetch status: {}", res.status));
        let html = String::from_utf8_lossy(&res.body).to_string();
        let nonce = extract_nonce(&html);
        ito_rs::host::print(&format!("[DEBUG-MEGACLOUD] Extracted nonce: {:?}", nonce));
        let nonce = nonce?;

        let parts: Vec<&str> = embed_url.split('/').collect();
        let domain = parts.get(2)?;
        let base_path = parts[3..parts.len() - 1].join("/");
        let last_part = parts.last()?;
        let xrax = last_part.split('?').next()?;
        
        let sources_url = format!("https://{}/{}/getSources?id={}&_k={}", domain, base_path, xrax, nonce);
        ito_rs::host::print(&format!("[DEBUG-MEGACLOUD] getSources URL: {}", sources_url));
        
        let sources_res = Request::get(&sources_url)
            .header("Referer", "https://anikai.to/")
            .header("X-Requested-With", "XMLHttpRequest")
            .send().ok()?;

        ito_rs::host::print(&format!("[DEBUG-MEGACLOUD] getSources fetch status: {}", sources_res.status));
        let json: serde_json::Value = serde_json::from_slice(&sources_res.body).ok()?;

        // CHECK IF IT RETURNED HTML WITH ANOTHER PAGE DATA
        if let Some(status) = json.get("status").and_then(|s| s.as_i64()) {
            if status == 200 {
                 if let Some(result_html) = json.get("result").and_then(|r| r.as_str()) {
                     ito_rs::host::print("[DEBUG-MEGACLOUD] getSources returned HTML, checking for inner __PAGE_DATA");
                     if let Some(inner_nonce) = extract_nonce(result_html) {
                         ito_rs::host::print(&format!("[DEBUG-MEGACLOUD] Found inner nonce: {}", inner_nonce));
                         
                         let mut user_agent = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36".to_string();
                         if let Some(ua) = sources_res.headers.get("X-Used-User-Agent").or_else(|| sources_res.headers.get("x-used-user-agent")) {
                             user_agent = ua.clone();
                         }

                         let mut payload_map = std::collections::HashMap::new();
                         payload_map.insert("text", inner_nonce.clone());
                         payload_map.insert("agent", user_agent);
                         let body = serde_json::to_vec(&payload_map).unwrap_or_default();

                         let dec_mega_res = Request::post("https://enc-dec.app/api/dec-mega")
                            .header("Content-Type", "application/json")
                            .body(&body)
                            .send().ok()?;
                         
                         ito_rs::host::print(&format!("[DEBUG-MEGACLOUD] dec-mega API status: {}", dec_mega_res.status));
                         if dec_mega_res.status == 200 {
                             if let Ok(mega_json) = serde_json::from_slice::<serde_json::Value>(&dec_mega_res.body) {
                                 ito_rs::host::print(&format!("[DEBUG-MEGACLOUD] dec-mega response: {}", mega_json));
                                 if mega_json.get("status").and_then(|s| s.as_i64()) == Some(200) {
                                     if let Some(mega_result) = mega_json.get("result") {
                                          let final_json = if mega_result.is_string() {
                                              serde_json::from_str::<serde_json::Value>(mega_result.as_str().unwrap()).unwrap_or(mega_result.clone())
                                          } else {
                                              mega_result.clone()
                                          };
                                          
                                          return extract_from_json(&final_json);
                                     }
                                 } else {
                                     ito_rs::host::print(&format!("[DEBUG-MEGACLOUD] dec-mega API error: {:?}", mega_json));
                                 }
                             }
                         }
                     }
                 }
            }
        }

        extract_from_json(&json)
    }

    fn extract_from_json(json: &serde_json::Value) -> Option<(String, Option<Vec<ito_rs::models::anime::Subtitle>>)> {
        let mut final_sources = None;

        if let Some(is_encrypted) = json.get("encrypted") {
            let encrypted = is_encrypted.as_bool().unwrap_or(is_encrypted.as_i64().map(|v| v != 0).unwrap_or(false));
            ito_rs::host::print(&format!("[DEBUG-MEGACLOUD] Encrypted: {}", encrypted));
            if encrypted {
                if let Some(encrypted_base64) = json.get("sources").and_then(|s| s.as_str()) {
                    let keys_url = "https://raw.githubusercontent.com/yogesh-hacker/MegacloudKeys/refs/heads/main/keys.json";
                    let keys_res = Request::get(keys_url).send().ok()?;
                    if let Ok(keys_json) = serde_json::from_slice::<serde_json::Value>(&keys_res.body) {
                        if let Some(vidstr_key) = keys_json.get("vidstr").and_then(|k| k.as_str()) {
                            let decrypted = decrypt(encrypted_base64, "", vidstr_key);
                            if let Ok(parsed_decrypted) = serde_json::from_str::<serde_json::Value>(&decrypted) {
                                final_sources = parsed_decrypted.as_array().cloned();
                            }
                        }
                    }
                }
            } else {
                final_sources = json.get("sources").and_then(|s| s.as_array().cloned());
            }
        }
        
        let mut final_subtitles = Vec::new();
        if let Some(tracks) = json.get("tracks").and_then(|t| t.as_array()) {
            for track in tracks {
                let file = track.get("file").and_then(|f| f.as_str()).unwrap_or("");
                let label = track.get("label").and_then(|l| l.as_str()).unwrap_or("");
                let kind = track.get("kind").and_then(|k| k.as_str()).unwrap_or("");

                if kind == "captions" || kind == "subtitles" {
                    final_subtitles.push(ito_rs::models::anime::Subtitle {
                        url: file.to_string(),
                        language: label.to_string(),
                        format: "vtt".to_string(),
                        is_hardsub: false,
                    });
                }
            }
        }

        if let Some(sources) = final_sources {
            if let Some(first_source) = sources.first() {
                if let Some(file) = first_source.get("file").and_then(|f| f.as_str()) {
                    ito_rs::host::print(&format!("[DEBUG-MEGACLOUD] Found source file: {}", file));
                    let subs = if final_subtitles.is_empty() { None } else { Some(final_subtitles) };
                    return Some((file.to_string(), subs));
                }
            }
        }
        
        None
    }

    fn extract_nonce(html: &str) -> Option<String> {
        if let Some(start) = html.find("window.__PAGE_DATA=\"") {
            let offset = "window.__PAGE_DATA=\"".len();
            let rest = &html[start + offset..];
            if let Some(end) = rest.find('"') {
                return Some(rest[..end].to_string());
            }
        }
        
        let mut potentials = Vec::new();
        let mut current = String::new();
        
        for c in html.chars() {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                current.push(c);
            } else {
                if !current.is_empty() {
                    potentials.push(current);
                    current = String::new();
                }
            }
        }
        if !current.is_empty() {
            potentials.push(current);
        }
        
        let keywords = ["addEventListener", "DOMContentLoaded", "getElementById", "querySelector", "function", "document", "window", "console"];
        
        for p in &potentials {
            if p.len() >= 48 && !keywords.iter().any(|&k| p.contains(k)) {
                return Some(p.clone());
            }
        }
        
        let parts16: Vec<String> = potentials.into_iter().filter(|p| p.len() == 16 && !keywords.iter().any(|&k| p.contains(k))).collect();
        if parts16.len() >= 3 {
            return Some(format!("{}{}{}", parts16[0], parts16[1], parts16[2]));
        }
        
        None
    }

    fn keygen(megacloud_key: &str, client_key: &str) -> String {
        let temp_key = format!("{}{}", megacloud_key, client_key);
        let mut hash_val: i64 = 0;
        
        for c in temp_key.chars() {
            let asc = (c as u32 & 0x7F) as i64;
            hash_val = asc.wrapping_add(hash_val.wrapping_mul(31)).wrapping_add(hash_val << 7).wrapping_sub(hash_val);
        }
        
        let l_hash = hash_val.abs() % 0x7FFF_FFFF_FFFF_FFFF;
        
        let temp_key_xor: Vec<char> = temp_key.chars().map(|c| {
            let v = (c as u32 & 0x7F) ^ 247;
            std::char::from_u32(v).unwrap_or(c)
        }).collect();
        
        let pivot = ((l_hash % temp_key_xor.len() as i64) as usize) + 5;
        
        let mut rotated_key_str = Vec::new();
        if pivot < temp_key_xor.len() {
            rotated_key_str.extend_from_slice(&temp_key_xor[pivot..]);
            rotated_key_str.extend_from_slice(&temp_key_xor[..pivot]);
        } else {
            rotated_key_str.extend_from_slice(&temp_key_xor);
        }
        
        let leaf_arr: Vec<char> = client_key.chars().rev().collect();
        
        let mut return_key = String::new();
        let max_len = rotated_key_str.len().max(leaf_arr.len());
        for i in 0..max_len {
            if i < rotated_key_str.len() { return_key.push(rotated_key_str[i]); }
            if i < leaf_arr.len() { return_key.push(leaf_arr[i]); }
        }
        
        let limit = 96 + (l_hash % 33) as usize;
        let return_key: String = return_key.chars().take(limit).collect();
        
        return_key.chars().map(|c| {
            let v = c as u32 & 0x7F;
            std::char::from_u32((v % 95) + 32).unwrap_or(c)
        }).collect()
    }

    fn columnar_cipher(src: &str, ikey: &str) -> String {
        let col_count = ikey.chars().count();
        if col_count == 0 { return src.to_string(); }
        let src_chars: Vec<char> = src.chars().collect();
        let row_count = (src_chars.len() + col_count - 1) / col_count;
        
        let mut grid = vec![vec![' '; col_count]; row_count];
        let mut sorted_map: Vec<(char, usize)> = ikey.chars().enumerate().map(|(i, c)| (c, i)).collect();
        sorted_map.sort_by_key(|k| k.0);
        
        let mut src_idx = 0;
        for &(_, orig_col) in &sorted_map {
            for row in 0..row_count {
                if src_idx < src_chars.len() {
                    grid[row][orig_col] = src_chars[src_idx];
                    src_idx += 1;
                }
            }
        }
        
        let mut result = String::with_capacity(src_chars.len());
        for row in 0..row_count {
            for col in 0..col_count {
                result.push(grid[row][col]);
            }
        }
        result
    }

    fn seed_shuffle(array: &[char], ikey: &str) -> Vec<char> {
        let mut hash_val: i64 = 0;
        for c in ikey.chars() {
            let v = (c as u32 & 0x7F) as i64;
            hash_val = (hash_val.wrapping_mul(31).wrapping_add(v)) & 0xFFFF_FFFF;
        }
        let mut shuffle_num = hash_val;
        
        let mut pseudo_rand = |max: usize| -> usize {
            shuffle_num = (shuffle_num.wrapping_mul(1_103_515_245).wrapping_add(12345)) & 0x7FFF_FFFF;
            (shuffle_num % max as i64) as usize
        };
        
        let mut result = array.to_vec();
        if result.len() <= 1 { return result; }
        for i in (1..result.len()).rev() {
            let swap_idx = pseudo_rand(i + 1);
            result.swap(i, swap_idx);
        }
        result
    }

    fn decrypt(src: &str, client_key: &str, megacloud_key: &str) -> String {
        use base64::{engine::general_purpose, Engine as _};

        let layers = 3;
        let gen_key = keygen(megacloud_key, client_key);
        
        let dec_data = match general_purpose::STANDARD.decode(src) {
            Ok(data) => data,
            Err(_) => return src.to_string(),
        };
        
        let mut dec_str = match String::from_utf8(dec_data) {
            Ok(s) => s,
            Err(_) => return src.to_string(),
        };

        let char_array: Vec<char> = (32..=126).filter_map(|i| std::char::from_u32(i)).collect();

        for iteration in (1..=layers).rev() {
            let layer_key = format!("{}{}", gen_key, iteration);
            let mut hash_val: i64 = 0;
            for c in layer_key.chars() {
                let v = (c as u32 & 0x7F) as i64;
                hash_val = (hash_val.wrapping_mul(31).wrapping_add(v)) & 0xFFFF_FFFF;
            }
            
            let mut seed = hash_val;
            let mut seed_rand = |max: usize| -> usize {
                seed = (seed.wrapping_mul(1_103_515_245).wrapping_add(12345)) & 0x7FFF_FFFF;
                (seed % max as i64) as usize
            };
            
            let mut dec_arr: Vec<char> = dec_str.chars().collect();
            for i in 0..dec_arr.len() {
                if let Some(c_idx) = char_array.iter().position(|&x| x == dec_arr[i]) {
                    let random_shift = seed_rand(95) as isize;
                    let mut new_idx = (c_idx as isize - random_shift) % 95;
                    if new_idx < 0 {
                        new_idx += 95;
                    }
                    dec_arr[i] = char_array[new_idx as usize];
                }
            }
            
            dec_str = dec_arr.into_iter().collect();
            dec_str = columnar_cipher(&dec_str, &layer_key);
            
            let sub_values = seed_shuffle(&char_array, &layer_key);
            let mut char_map = std::collections::HashMap::new();
            for i in 0..sub_values.len() {
                char_map.insert(sub_values[i], char_array[i]);
            }
            
            dec_str = dec_str.chars().map(|c| *char_map.get(&c).unwrap_or(&c)).collect();
        }

        if dec_str.chars().count() >= 4 {
            let prefix: String = dec_str.chars().take(4).collect();
            if let Ok(len) = prefix.parse::<usize>() {
                let payload: String = dec_str.chars().skip(4).take(len).collect();
                return payload;
            }
        }

        dec_str
    }
}

export_anime_plugin!(AnimeKaiProvider);
