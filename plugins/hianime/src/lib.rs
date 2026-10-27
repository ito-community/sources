//! HiAnime (hianime.ms) anime provider.
//!
//! Catalog, search, details, and episodes come from server-rendered pages. Streams come
//! from the Ryu server only: the `/stream/<token>/<lang>` hop resolves to MegaPlay's `s-2`
//! embed, and that is the sole source for both sub and dub. Blocks and challenges fail the
//! call instead of returning an empty result.

use ito_rs::{
    Error, Result, export_anime_plugin,
    models::{
        FilterItem, HomeComponent, HomeComponentValue, HomeLayout, LinkValue, Listing,
        anime::{Anime, Episode, PageResult, Video},
    },
    provider::AnimeProvider,
};

mod http;
mod markup;
mod megaplay;
mod parse;

use megaplay::Lang;

pub struct HiAnimeProvider;

const BASE_URL: &str = "https://hianime.ms";
const SITE_REFERER: &str = "https://hianime.ms/";

/// Listing paths served as `/{path}?page=N`. The listing id is the path itself.
const LISTING_PATHS: [&str; 3] = ["most-popular", "top-airing", "updates"];

impl HiAnimeProvider {
    fn fetch_html(url: &str) -> Result<String> {
        let response = http::fetch(url, &[])?;
        Ok(String::from_utf8_lossy(&response.body).into_owned())
    }

    fn page_of(url: &str) -> Result<PageResult> {
        let html = Self::fetch_html(url)?;
        Ok(PageResult {
            entries: parse::cards(&html),
            has_next_page: parse::has_next_page(&html),
        })
    }

    /// Builds one home row from the first page of a listing. An empty listing gives no row.
    fn listing_section(path: &str, title: &str) -> Result<Option<HomeComponent>> {
        let page = Self::page_of(&format!("{BASE_URL}/{path}?page=1"))?;
        if page.entries.is_empty() {
            return Ok(None);
        }
        Ok(Some(HomeComponent {
            title: Some(title.to_string()),
            subtitle: None,
            value: HomeComponentValue::AnimeScroller(
                page.entries,
                Some(Listing {
                    id: path.to_string(),
                    name: title.to_string(),
                    kind: 0,
                }),
            ),
        }))
    }

    /// Resolves one language through the stream hop and MegaPlay.
    fn language_videos(token: &str, episode_id: &str, lang: Lang) -> Result<Vec<Video>> {
        let hop_url = format!("{BASE_URL}/stream/{token}/{}", lang.as_str());
        let hop = http::fetch(&hop_url, &[("Referer", SITE_REFERER)])?;
        let embed = megaplay::embed_url(episode_id, lang);
        let mut videos = megaplay::extract(&embed, &String::from_utf8_lossy(&hop.body), lang)?;
        for video in &mut videos {
            video.quality = format!("Auto {} Ryu", lang.label());
        }
        Ok(videos)
    }
}

impl AnimeProvider for HiAnimeProvider {
    fn get_home() -> Result<HomeLayout> {
        let mut components = Vec::new();
        components.extend(Self::listing_section("most-popular", "Most Popular")?);
        for (path, title) in [("top-airing", "Top Airing"), ("updates", "New Episodes")] {
            match Self::listing_section(path, title) {
                Ok(section) => components.extend(section),
                Err(error) if http::is_blocked(&error) => return Err(error),
                Err(error) => {
                    ito_rs::host::print(&format!("HiAnime: {title} row skipped: {error}"));
                }
            }
        }
        Ok(HomeLayout { components })
    }

    fn get_anime_list(listing: Listing, page: i32) -> Result<PageResult> {
        let path = LISTING_PATHS
            .into_iter()
            .find(|path| *path == listing.id)
            .ok_or(Error::Unsupported)?;
        Self::page_of(&format!("{BASE_URL}/{path}?page={page}"))
    }

    fn get_search_anime_list(
        query: &str,
        page: i32,
        _filters: Vec<FilterItem>,
    ) -> Result<PageResult> {
        Self::page_of(&format!(
            "{BASE_URL}/search?q={}&page={page}",
            http::encode_query(query)
        ))
    }

    fn get_anime_update(
        mut anime: Anime,
        needs_details: bool,
        needs_episodes: bool,
    ) -> Result<Anime> {
        if !needs_details && !needs_episodes {
            return Ok(anime);
        }
        if !parse::is_details_path(&anime.key) {
            return Err(Error::Unsupported);
        }
        let html = Self::fetch_html(&format!("{BASE_URL}{}", anime.key))?;

        if needs_details {
            let details = parse::details(&html);
            if let Some(title) = details.title {
                anime.title = title;
            }
            if let Some(cover) = details.cover {
                anime.cover = Some(cover);
            }
            anime.description = details.description;
            anime.status = details.status;
            anime.tags = (!details.tags.is_empty()).then_some(details.tags);
            anime.studios = (!details.studios.is_empty()).then_some(details.studios);
            anime.url = Some(format!("{BASE_URL}{}", anime.key));
        }
        if needs_episodes {
            anime.episodes = Some(parse::episodes(&html));
        }
        Ok(anime)
    }

    fn get_video_list(_anime: Anime, episode: Episode) -> Result<Vec<Video>> {
        if !parse::is_watch_path(&episode.key) {
            return Err(Error::Unsupported);
        }
        let html = Self::fetch_html(&format!("{BASE_URL}{}", episode.key))?;
        let token = parse::watch_token(&html)?;
        let episode_id = parse::episode_id_from_token(&token)
            .ok_or_else(|| Error::Html("HiAnime stream token is malformed".to_string()))?;

        let mut videos = Vec::new();
        for lang in Lang::ALL {
            match Self::language_videos(&token, &episode_id, lang) {
                Ok(found) => videos.extend(found),
                Err(error) if http::is_blocked(&error) => return Err(error),
                Err(_) => {
                    ito_rs::host::print(&format!("HiAnime: {} stream unavailable", lang.label()));
                }
            }
        }
        if videos.is_empty() {
            return Err(Error::Net(
                "HiAnime has no playable sub or dub stream for this episode".to_string(),
            ));
        }
        Ok(videos)
    }

    fn handle_url(url: &str) -> Result<LinkValue> {
        let key = parse::anime_key_from_url(url).ok_or(Error::Unsupported)?;
        Ok(LinkValue::Anime(parse::anime(key, String::new(), None)))
    }
}

export_anime_plugin!(HiAnimeProvider);
