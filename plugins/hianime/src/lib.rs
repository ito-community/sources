//! HiAnime (hianime.ms) anime provider.
//!
//! Catalog, search, details, and episodes come from server-rendered pages. Streams come
//! from the Ryu server only: the `/stream/<token>/<lang>` hop resolves to MegaPlay's `s-2`
//! embed, and that is the sole source for both sub and dub.
//!
//! A page that lacks the structure a call needs is an error, never an empty result. A block
//! or a challenge on the watch page, the stream hop, or a stream fails the call, and nothing
//! retries around it. A blocked caption track is the one exception: it drops only that track.

use ito_rs::{
    Error, export_anime_plugin,
    models::{
        FilterItem, HomeComponent, HomeComponentValue, HomeLayout, LinkValue, Listing,
        anime::{Anime, Episode, PageResult, Video},
    },
    provider::AnimeProvider,
};

#[cfg(test)]
mod fake;
mod http;
mod markup;
mod megaplay;
mod parse;

use http::{Failure, HostNet, Transport, Upstream};
use megaplay::{CaptionCache, Lang};

pub struct HiAnimeProvider;

const BASE_URL: &str = "https://hianime.ms";
const SITE_REFERER: &str = "https://hianime.ms/";

/// Listing paths served as `/{path}?page=N`. The listing id is the path itself.
const LISTING_PATHS: [&str; 3] = ["most-popular", "top-airing", "updates"];

/// Fetches a site page and returns its body as text.
fn fetch_html<N: Transport>(net: &N, url: &str) -> Result<String, Failure> {
    let response = http::get(net, Upstream::Site, url, &[])?;
    Ok(String::from_utf8_lossy(&response.body).into_owned())
}

/// Builds a listing page from its cards. A listing with no cards is always an error, since
/// every HiAnime listing has content, so a markup change never reads as an empty list.
fn listing_page(html: &str) -> ito_rs::Result<PageResult> {
    let entries = parse::cards(html);
    if entries.is_empty() {
        return Err(Error::Html(
            "HiAnime listing has no result cards".to_string(),
        ));
    }
    Ok(PageResult {
        entries,
        has_next_page: parse::has_next_page(html),
    })
}

/// Builds a search page from its cards. Zero cards is an empty page only when the page shows the
/// empty-search state (`hk-empty`) or reports zero results with an "N found" heading. Any other
/// empty page is an error.
fn search_page(html: &str) -> ito_rs::Result<PageResult> {
    let entries = parse::cards(html);
    let empty_state = parse::has_empty_state(html) || parse::result_count(html) == Some(0);
    if entries.is_empty() && !empty_state {
        return Err(Error::Html(
            "HiAnime search page has no result cards".to_string(),
        ));
    }
    if entries.is_empty() {
        return Ok(PageResult {
            entries,
            has_next_page: false,
        });
    }
    Ok(PageResult {
        entries,
        has_next_page: parse::has_next_page(html),
    })
}

/// Builds one home row from the first page of a listing.
fn listing_row<N: Transport>(net: &N, path: &str, title: &str) -> Result<HomeComponent, Failure> {
    let html = fetch_html(net, &format!("{BASE_URL}/{path}?page=1"))?;
    let page = listing_page(&html)?;
    Ok(HomeComponent {
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
    })
}

/// Builds the home layout.
///
/// Most Popular is the row every HiAnime home page carries, so when it cannot be read the whole
/// home fails, and the layout always has at least that row. Top Airing and New Episodes are
/// extras: a failure there is logged and the row is skipped. A block still fails the call.
fn home<N: Transport>(net: &N) -> Result<HomeLayout, Failure> {
    let mut components = vec![listing_row(net, "most-popular", "Most Popular")?];
    for (path, title) in [("top-airing", "Top Airing"), ("updates", "New Episodes")] {
        match listing_row(net, path, title) {
            Ok(row) => components.push(row),
            Err(blocked @ Failure::Blocked { .. }) => return Err(blocked),
            Err(Failure::Other(_)) => {
                http::note(&format!("HiAnime: {title} row skipped"));
            }
        }
    }
    Ok(HomeLayout { components })
}

fn anime_list<N: Transport>(net: &N, listing: &Listing, page: i32) -> Result<PageResult, Failure> {
    let path = LISTING_PATHS
        .into_iter()
        .find(|path| *path == listing.id)
        .ok_or(Error::Unsupported)?;
    let html = fetch_html(net, &format!("{BASE_URL}/{path}?page={page}"))?;
    Ok(listing_page(&html)?)
}

fn search<N: Transport>(net: &N, query: &str, page: i32) -> Result<PageResult, Failure> {
    let html = fetch_html(
        net,
        &format!(
            "{BASE_URL}/search?q={}&page={page}",
            http::encode_query(query)
        ),
    )?;
    Ok(search_page(&html)?)
}

/// Fills in the requested parts of an anime from its details page.
///
/// A details refresh overwrites only the fields the page shows. A field the page omits keeps
/// its stored value, so a page that changed shape cannot erase the library entry.
fn anime_update<N: Transport>(
    net: &N,
    mut anime: Anime,
    needs_details: bool,
    needs_episodes: bool,
) -> Result<Anime, Failure> {
    if !needs_details && !needs_episodes {
        return Ok(anime);
    }
    if !parse::is_details_path(&anime.key) {
        return Err(Error::Unsupported.into());
    }
    let html = fetch_html(net, &format!("{BASE_URL}{}", anime.key))?;

    if needs_details {
        let details = parse::details(&html);
        if let Some(title) = details.title {
            anime.title = title;
        }
        if let Some(cover) = details.cover {
            anime.cover = Some(cover);
        }
        if let Some(description) = details.description {
            anime.description = Some(description);
        }
        if let Some(status) = details.status {
            anime.status = status;
        }
        if !details.tags.is_empty() {
            anime.tags = Some(details.tags);
        }
        if !details.studios.is_empty() {
            anime.studios = Some(details.studios);
        }
        anime.url = Some(format!("{BASE_URL}{}", anime.key));
    }
    if needs_episodes {
        anime.episodes = Some(parse::episodes(&html)?);
    }
    Ok(anime)
}

/// Resolves one language through the stream hop and MegaPlay.
///
/// The hop is classed by the URL requested (hianime.ms), even when the host follows its
/// redirect to MegaPlay. Blocks on the MegaPlay steps name MegaPlay.
fn language_videos<N: Transport>(
    net: &N,
    token: &str,
    episode_id: &str,
    lang: Lang,
    captions: &mut CaptionCache,
) -> Result<Vec<Video>, Failure> {
    let hop_url = format!("{BASE_URL}/stream/{token}/{}", lang.as_str());
    let hop = http::get(net, Upstream::Site, &hop_url, &[("Referer", SITE_REFERER)])?;
    let embed = megaplay::embed_url(episode_id, lang);
    let quality = format!("Auto {} Ryu", lang.label());
    megaplay::extract(
        net,
        &embed,
        &String::from_utf8_lossy(&hop.body),
        lang,
        &quality,
        captions,
    )
}

fn video_list<N: Transport>(net: &N, episode: &Episode) -> Result<Vec<Video>, Failure> {
    if !parse::is_watch_path(&episode.key) {
        return Err(Error::Unsupported.into());
    }
    let html = fetch_html(net, &format!("{BASE_URL}{}", episode.key))?;
    let token = parse::watch_token(&html)?;
    let episode_id = parse::episode_id_from_token(&token)
        .ok_or_else(|| Error::Html("HiAnime stream token is malformed".to_string()))?;

    let mut videos = Vec::new();
    // Sub and dub share their caption tracks, so the cache lives for the whole call.
    let mut captions = CaptionCache::new();
    for lang in Lang::ALL {
        match language_videos(net, &token, &episode_id, lang, &mut captions) {
            Ok(found) => videos.extend(found),
            Err(blocked @ Failure::Blocked { .. }) => return Err(blocked),
            // One language failing does not hide the other, and the call succeeds with what
            // loaded. Only a call where neither language loads fails, below. The log line
            // names the language and never the URL or the error.
            Err(Failure::Other(_)) => {
                http::note(&format!("HiAnime: {} stream unavailable", lang.label()));
            }
        }
    }
    if videos.is_empty() {
        return Err(Error::Net(
            "HiAnime has no playable sub or dub stream for this episode".to_string(),
        )
        .into());
    }
    Ok(videos)
}

impl AnimeProvider for HiAnimeProvider {
    fn get_home() -> ito_rs::Result<HomeLayout> {
        home(&HostNet).map_err(Error::from)
    }

    fn get_anime_list(listing: Listing, page: i32) -> ito_rs::Result<PageResult> {
        anime_list(&HostNet, &listing, page).map_err(Error::from)
    }

    fn get_search_anime_list(
        query: &str,
        page: i32,
        _filters: Vec<FilterItem>,
    ) -> ito_rs::Result<PageResult> {
        search(&HostNet, query, page).map_err(Error::from)
    }

    fn get_anime_update(
        anime: Anime,
        needs_details: bool,
        needs_episodes: bool,
    ) -> ito_rs::Result<Anime> {
        anime_update(&HostNet, anime, needs_details, needs_episodes).map_err(Error::from)
    }

    fn get_video_list(_anime: Anime, episode: Episode) -> ito_rs::Result<Vec<Video>> {
        video_list(&HostNet, &episode).map_err(Error::from)
    }

    fn handle_url(url: &str) -> ito_rs::Result<LinkValue> {
        let key = parse::anime_key_from_url(url).ok_or(Error::Unsupported)?;
        Ok(LinkValue::Anime(parse::anime(key, String::new(), None)))
    }
}

export_anime_plugin!(HiAnimeProvider);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fake::Canned;
    use crate::http::USER_AGENT;
    use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
    use ito_rs::models::anime::Status;
    use serde_json::json;

    const SHOW_KEY: &str = "/details/show-one-abcde";
    const EPISODE_KEY: &str = "/watch-show-one-episode-1-abcde";
    const CAPTION: &str = "https://megaplay.buzz/cc/en.vtt";
    const EMBED_HTML: &str = r#"<html><div id="megaplay-player" data-id="2222"></div></html>"#;
    const SOURCES_SUB: &str = "https://megaplay.buzz/stream/getSources?id=2222&type=sub";
    const SOURCES_DUB: &str = "https://megaplay.buzz/stream/getSources?id=2222&type=dub";

    fn token() -> String {
        URL_SAFE_NO_PAD.encode("2142:doe")
    }

    fn hop(lang: &str) -> String {
        format!("{BASE_URL}/stream/{}/{lang}", token())
    }

    fn episode() -> Episode {
        Episode {
            key: EPISODE_KEY.to_string(),
            title: None,
            episode: Some(1.0),
            date_updated: None,
            url: None,
            lang: None,
            paywalled: None,
        }
    }

    fn card_page(slug: &str) -> String {
        format!(
            r#"<main><article class="hk-tile"><h3 class="hk-tile__name"><a href="/details/{slug}" title="{slug} (TV, 2020)">x</a></h3></article></main>"#
        )
    }

    fn watch_page() -> String {
        format!(
            "<article class=\"hk-watch\" data-hk-watch='{{&quot;token&quot;:&quot;{}&quot;}}'></article>",
            token()
        )
    }

    fn sources(tracks: serde_json::Value) -> String {
        json!({ "sources": { "file": "https://cdn.example/master.m3u8" }, "tracks": tracks })
            .to_string()
    }

    /// A network where the episode resolves for both languages, with `tracks` as its captions.
    fn ryu_net(tracks: serde_json::Value) -> Canned {
        Canned::new()
            .on(&format!("{BASE_URL}{EPISODE_KEY}"), 200, watch_page())
            .on(&hop("sub"), 200, EMBED_HTML)
            .on(&hop("dub"), 200, EMBED_HTML)
            .on(SOURCES_SUB, 200, sources(tracks.clone()))
            .on(SOURCES_DUB, 200, sources(tracks))
            .on(CAPTION, 200, "WEBVTT\n\n00:00.000 --> 00:01.000\nHi\n")
    }

    fn english_track() -> serde_json::Value {
        json!([{ "kind": "captions", "file": CAPTION, "label": "English" }])
    }

    #[test]
    fn video_list_returns_sub_and_dub_with_quality_labels_and_playback_headers() {
        let net = ryu_net(english_track());
        let videos = video_list(&net, &episode()).expect("videos");
        let qualities: Vec<&str> = videos.iter().map(|video| video.quality.as_str()).collect();
        assert_eq!(qualities, ["Auto SUB Ryu", "Auto DUB Ryu"]);
        for video in &videos {
            let headers = video.headers.as_ref().expect("playback headers");
            assert_eq!(
                headers.get("User-Agent").map(String::as_str),
                Some(USER_AGENT)
            );
            assert_eq!(
                headers.get("Origin").map(String::as_str),
                Some("https://megaplay.buzz")
            );
            let subtitles = video.subtitles.as_deref().expect("subtitles");
            assert_eq!(subtitles.last().map(|s| s.language.as_str()), Some("Off"));
        }
    }

    #[test]
    fn every_request_carries_the_one_user_agent() {
        let net = ryu_net(json!([]));
        video_list(&net, &episode()).expect("videos");
        let sent = net.requests();
        assert!(sent.len() > 3, "expected the full Ryu chain");
        for request in sent {
            assert_eq!(
                request.header("User-Agent"),
                Some(USER_AGENT),
                "request to {}",
                request.url
            );
        }
    }

    #[test]
    fn a_caption_shared_by_sub_and_dub_is_fetched_once() {
        let net = ryu_net(english_track());
        video_list(&net, &episode()).expect("videos");
        assert_eq!(net.sent_to(CAPTION), 1);
    }

    #[test]
    fn a_language_that_fails_does_not_hide_the_other() {
        let net = Canned::new()
            .on(&format!("{BASE_URL}{EPISODE_KEY}"), 200, watch_page())
            .on(&hop("sub"), 200, EMBED_HTML)
            .on(SOURCES_SUB, 200, sources(json!([])));
        let videos = video_list(&net, &episode()).expect("sub still plays");
        assert_eq!(videos.len(), 1);
        assert_eq!(videos[0].quality, "Auto SUB Ryu");
    }

    #[test]
    fn video_list_fails_when_neither_language_loads() {
        let net = Canned::new().on(&format!("{BASE_URL}{EPISODE_KEY}"), 200, watch_page());
        let Err(failure) = video_list(&net, &episode()) else {
            panic!("expected no playable stream");
        };
        assert!(matches!(failure, Failure::Other(Error::Net(_))));
    }

    #[test]
    fn a_megaplay_block_fails_the_call_and_names_megaplay() {
        let net = Canned::new()
            .on(&format!("{BASE_URL}{EPISODE_KEY}"), 200, watch_page())
            .on(&hop("sub"), 200, EMBED_HTML)
            .on(SOURCES_SUB, 403, "");
        let Err(failure) = video_list(&net, &episode()) else {
            panic!("expected a block");
        };
        assert!(matches!(
            failure,
            Failure::Blocked {
                upstream: Upstream::MegaPlay,
                status: 403
            }
        ));
    }

    #[test]
    fn a_challenge_page_on_a_success_status_blocks_the_watch_page() {
        let net = Canned::new().on(
            &format!("{BASE_URL}{EPISODE_KEY}"),
            200,
            "<html>window._cf_chl_opt = {};</html>",
        );
        let Err(failure) = video_list(&net, &episode()) else {
            panic!("expected a block");
        };
        assert!(matches!(
            failure,
            Failure::Blocked {
                upstream: Upstream::Site,
                status: 200
            }
        ));
    }

    #[test]
    fn home_fails_when_most_popular_has_no_cards() {
        let net = Canned::new().on(
            &format!("{BASE_URL}/most-popular?page=1"),
            200,
            "<main></main>",
        );
        let Err(failure) = home(&net) else {
            panic!("expected a failed home");
        };
        assert!(matches!(failure, Failure::Other(Error::Html(_))));
    }

    #[test]
    fn home_skips_an_extra_row_that_fails_and_keeps_the_others() {
        let net = Canned::new()
            .on(
                &format!("{BASE_URL}/most-popular?page=1"),
                200,
                card_page("most-one"),
            )
            .on(
                &format!("{BASE_URL}/updates?page=1"),
                200,
                card_page("updated-one"),
            );
        let layout = home(&net).expect("home");
        let titles: Vec<&str> = layout
            .components
            .iter()
            .filter_map(|component| component.title.as_deref())
            .collect();
        assert_eq!(titles, ["Most Popular", "New Episodes"]);
    }

    #[test]
    fn a_block_in_an_extra_row_fails_the_home() {
        let net = Canned::new()
            .on(
                &format!("{BASE_URL}/most-popular?page=1"),
                200,
                card_page("most-one"),
            )
            .on(&format!("{BASE_URL}/top-airing?page=1"), 403, "");
        let Err(failure) = home(&net) else {
            panic!("expected a block");
        };
        assert!(matches!(
            failure,
            Failure::Blocked {
                upstream: Upstream::Site,
                status: 403
            }
        ));
    }

    #[test]
    fn listing_page_without_cards_is_an_error_not_an_empty_list() {
        let net = Canned::new().on(
            &format!("{BASE_URL}/most-popular?page=2"),
            200,
            "<main></main>",
        );
        let listing = Listing {
            id: "most-popular".to_string(),
            name: "Most Popular".to_string(),
            kind: 0,
        };
        let Err(failure) = anime_list(&net, &listing, 2) else {
            panic!("expected an error");
        };
        assert!(matches!(failure, Failure::Other(Error::Html(_))));
    }

    #[test]
    fn a_search_page_with_the_empty_state_is_an_empty_page() {
        let net = Canned::new().on(
            &format!("{BASE_URL}/search?q=zzzz&page=1"),
            200,
            r#"<main><div class="hk-empty"><h1>Nothing found for "zzzz"</h1></div></main>"#,
        );
        let page = search(&net, "zzzz", 1).expect("empty page");
        assert!(page.entries.is_empty());
        assert!(!page.has_next_page);
    }

    #[test]
    fn search_that_reports_zero_results_is_an_empty_page() {
        let net = Canned::new().on(
            &format!("{BASE_URL}/search?q=nothing&page=1"),
            200,
            "<h1><span>0</span> found</h1>",
        );
        let page = search(&net, "nothing", 1).expect("empty page");
        assert!(page.entries.is_empty());
        assert!(!page.has_next_page);
    }

    #[test]
    fn search_that_reports_results_but_shows_no_cards_is_an_error() {
        let net = Canned::new().on(
            &format!("{BASE_URL}/search?q=one+piece&page=1"),
            200,
            "<h1><span>58</span> found</h1>",
        );
        let Err(failure) = search(&net, "one piece", 1) else {
            panic!("expected an error");
        };
        assert!(matches!(failure, Failure::Other(Error::Html(_))));
    }

    #[test]
    fn details_refresh_keeps_the_fields_the_page_omits() {
        let net = Canned::new().on(
            &format!("{BASE_URL}{SHOW_KEY}"),
            200,
            "<main><h1>Changed layout</h1><section id=\"hk-episodes\"></section></main>",
        );
        let mut anime = parse::anime(SHOW_KEY.to_string(), "Show One".to_string(), None);
        anime.description = Some("Stored synopsis".to_string());
        anime.status = Status::Ongoing;
        let anime = anime_update(&net, anime, true, false).expect("update");
        assert_eq!(anime.description.as_deref(), Some("Stored synopsis"));
        assert!(matches!(anime.status, Status::Ongoing));
        assert_eq!(anime.title, "Show One");
    }

    #[test]
    fn episodes_update_fails_when_the_episode_section_is_missing() {
        let net = Canned::new().on(
            &format!("{BASE_URL}{SHOW_KEY}"),
            200,
            "<main><h1>Changed layout</h1></main>",
        );
        let anime = parse::anime(SHOW_KEY.to_string(), "Show One".to_string(), None);
        let Err(failure) = anime_update(&net, anime, false, true) else {
            panic!("expected an error");
        };
        assert!(matches!(failure, Failure::Other(Error::Html(_))));
    }

    #[test]
    fn a_stream_token_that_is_not_base64url_fails_the_call() {
        let html = "<article class=\"hk-watch\" data-hk-watch='{&quot;token&quot;:&quot;bad%token&quot;}'></article>";
        let net = Canned::new().on(&format!("{BASE_URL}{EPISODE_KEY}"), 200, html);
        let Err(failure) = video_list(&net, &episode()) else {
            panic!("expected an error");
        };
        assert!(matches!(failure, Failure::Other(Error::Html(_))));
    }

    #[test]
    fn a_movie_anime_update_with_needs_episodes_returns_one_episode() {
        let key = "/details/your-name-7k2m9";
        let movie_watch = "/watch-your-name-7k2m9";
        let net = Canned::new().on(
            &format!("{BASE_URL}{key}"),
            200,
            format!(r#"<main><div class="hk-title__kick"><span class="hk-tag--line">Movie</span></div><a href="{movie_watch}">Watch</a></main>"#),
        );
        let anime = parse::anime(key.to_string(), "Your Name".to_string(), None);
        let anime = anime_update(&net, anime, false, true).expect("update");
        let episodes = anime.episodes.expect("episodes");
        assert_eq!(episodes.len(), 1);
        assert_eq!(episodes[0].key, movie_watch);
        assert_eq!(episodes[0].episode, Some(1.0));
    }

    #[test]
    fn an_upcoming_show_updates_with_an_empty_episode_list() {
        let net = Canned::new().on(
            &format!("{BASE_URL}{SHOW_KEY}"),
            200,
            r#"<main><dl class="hk-facts"><dt>Status</dt><dd>Not aired yet</dd></dl></main>"#,
        );
        let anime = parse::anime(SHOW_KEY.to_string(), "Show One".to_string(), None);
        let anime = anime_update(&net, anime, true, true).expect("update");
        assert_eq!(anime.episodes.map(|episodes| episodes.len()), Some(0));
        assert!(matches!(anime.status, Status::Unknown));
    }
}
