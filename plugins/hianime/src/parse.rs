//! Parsers for HiAnime's server-rendered pages: catalog cards, the details facts, episode
//! rows, the watch-page stream config, and the keys used to address anime and episodes.
//!
//! Selectors follow the behavior spec. The unit-test fixtures are synthetic markup written
//! from those selectors, not copies of site pages.

use crate::{BASE_URL, markup};
use base64::{
    Engine, alphabet,
    engine::{DecodePaddingMode, GeneralPurpose, GeneralPurposeConfig},
};
use ito_rs::{
    Error, Result,
    models::anime::{Anime, ContentRating, Episode, Status},
};

/// Details-page facts. Absent fields stay `None` or empty so the caller keeps existing data.
#[derive(Debug)]
pub struct Details {
    pub title: Option<String>,
    pub cover: Option<String>,
    pub description: Option<String>,
    /// `None` unless the page shows a status this plugin recognizes.
    pub status: Option<Status>,
    /// The show type (for example `TV`) followed by its genres.
    pub tags: Vec<String>,
    pub studios: Vec<String>,
}

/// Builds an anime with HiAnime's detail URL and key `key` (a `/details/...` path).
pub fn anime(key: String, title: String, cover: Option<String>) -> Anime {
    let url = Some(format!("{BASE_URL}{key}"));
    Anime {
        key,
        title,
        studios: None,
        description: None,
        tags: None,
        cover,
        url,
        status: Status::Unknown,
        content_rating: ContentRating::Safe,
        nsfw: 0,
        episodes: None,
        seasons: None,
    }
}

/// Parses the catalog cards (`article.hk-tile`) of a listing or search page, in page order.
/// Cards without a link to a HiAnime details page are skipped.
pub fn cards(html: &str) -> Vec<Anime> {
    let mut anime = Vec::new();
    let mut cursor = html;
    while let Some(card) = markup::next_element(cursor, "article", |tag| tag.has_class("hk-tile")) {
        cursor = card.after;
        anime.extend(card_anime(card.inner));
    }
    anime
}

fn card_anime(card: &str) -> Option<Anime> {
    let name = markup::next_element(card, "h3", |tag| tag.has_class("hk-tile__name"))?;
    let link = markup::next_element(name.inner, "a", |_| true)?;
    let key = site_path(&link.tag.attr("href")?)?;
    if !is_details_path(&key) {
        return None;
    }
    let title = markup::next_element(name.inner, "span", |tag| tag.has_class("hk-dual"))
        .and_then(|span| span.tag.attr("data-en"))
        .and_then(non_empty)
        .or_else(|| {
            link.tag
                .attr("title")
                .and_then(|title| non_empty(strip_meta(&title)))
        })
        .or_else(|| non_empty(markup::text(link.inner)))?;
    let cover = markup::next_element(card, "img", |tag| {
        tag.attr("src")
            .is_some_and(|src| src.contains("/images/poster/"))
    })
    .or_else(|| markup::next_element(card, "img", |tag| tag.attr("src").is_some()))
    .and_then(|image| image.tag.attr("src"))
    .map(|src| absolute_url(&src));
    Some(anime(key, title, cover))
}

/// Returns true when a listing or search page links to a next page.
///
/// A `<link rel="next">` counts, and so does an `aria-label="Next"` link that is not disabled.
pub fn has_next_page(html: &str) -> bool {
    markup::tags(html).any(|tag| {
        let rel_next = tag.attr("rel").is_some_and(|rel| {
            rel.split_ascii_whitespace()
                .any(|token| token.eq_ignore_ascii_case("next"))
        });
        let enabled_next_link = tag.name.eq_ignore_ascii_case("a")
            && tag.attr("aria-label").as_deref() == Some("Next")
            && !tag.has_class("disabled")
            && tag.attr("aria-disabled").as_deref() != Some("true");
        (tag.name.eq_ignore_ascii_case("link") && rel_next) || enabled_next_link
    })
}

/// Returns the N of an "N found" result heading, or `None` when the page shows no such count.
pub fn result_count(html: &str) -> Option<u32> {
    let text = markup::visible_text(html);
    let words: Vec<&str> = text.split_whitespace().collect();
    words.windows(2).find_map(|pair| match pair {
        [count, "found"] => count.replace(',', "").parse().ok(),
        _ => None,
    })
}

/// Returns true when the page carries the empty-search state: an element whose class list
/// includes `hk-empty`.
pub fn has_empty_state(html: &str) -> bool {
    markup::tags(html).any(|tag| tag.has_class("hk-empty"))
}

/// Parses the details page: title, poster, synopsis, status, show type, genres, and studios.
pub fn details(html: &str) -> Details {
    let facts = facts(html);

    let title =
        markup::next_element(html, "h1", |tag| tag.has_class("hk-title__h")).and_then(|heading| {
            heading
                .tag
                .attr("data-en")
                .and_then(non_empty)
                .or_else(|| non_empty(markup::text(heading.inner)))
        });
    let cover = markup::next_element(html, "*", |tag| tag.has_class("hk-title__poster"))
        .and_then(|poster| {
            markup::next_element(poster.inner, "img", |tag| tag.attr("src").is_some())
        })
        .and_then(|image| image.tag.attr("src"))
        .map(|src| absolute_url(&src));
    let description = markup::next_element(html, "*", |tag| tag.has_class("hk-clamp"))
        .and_then(|clamp| non_empty(markup::text(clamp.inner)));
    let status = fact(&facts, "Status").and_then(|value| status_from_text(&markup::text(value)));
    let studios = fact(&facts, "Studio")
        .map(|value| link_texts(value, None))
        .unwrap_or_default();

    let kind = show_type(html);
    let genres = markup::next_element(html, "*", |tag| tag.has_class("hk-facts__genres"))
        .map(|block| link_texts(block.inner, Some("hk-chip")))
        .unwrap_or_default();
    let tags: Vec<String> = kind.into_iter().chain(genres).collect();

    Details {
        title,
        cover,
        description,
        status,
        tags,
        studios,
    }
}

/// Parses the episode rows (`section#hk-episodes a.hk-ep`) in page order.
///
/// A page without the episode section is one of two things. An upcoming show, whose Status is
/// "Not aired yet", has no episodes yet and yields an empty list. A movie, whose show type is
/// "Movie", yields its single watch link as the episode (see `movie_episode`). Any other page
/// without the section is an error. An empty section is an empty list.
pub fn episodes(html: &str) -> Result<Vec<Episode>> {
    let Some(section) = markup::next_element(html, "section", |tag| {
        tag.attr("id").as_deref() == Some("hk-episodes")
    }) else {
        if is_not_yet_aired(html) {
            return Ok(Vec::new());
        }
        if is_movie_page(html) {
            return movie_episode(html);
        }
        return Err(Error::Html(
            "HiAnime details page has no episode list".to_string(),
        ));
    };

    let mut episodes = Vec::new();
    let mut cursor = section.inner;
    while let Some(link) = markup::next_element(cursor, "a", |tag| tag.has_class("hk-ep")) {
        cursor = link.after;
        let Some(href) = link.tag.attr("href") else {
            continue;
        };
        let Some(key) = site_path(&href) else {
            continue;
        };
        if !is_watch_path(&key) {
            continue;
        }
        episodes.push(Episode {
            key,
            title: link.tag.attr("data-name").and_then(non_empty),
            episode: link
                .tag
                .attr("data-ep")
                .and_then(|number| number.trim().parse::<f32>().ok()),
            date_updated: None,
            url: Some(absolute_url(&href)),
            lang: None,
            paywalled: None,
        });
    }
    Ok(episodes)
}

/// Builds the one episode of a movie details page. A movie has no episode list, so its page
/// links its single watch page. Zero or several distinct movie watch links are an error.
fn movie_episode(html: &str) -> Result<Vec<Episode>> {
    let mut keys: Vec<String> = Vec::new();
    for tag in markup::tags(html).filter(|tag| tag.name.eq_ignore_ascii_case("a")) {
        let Some(key) = tag.attr("href").and_then(|href| site_path(&href)) else {
            continue;
        };
        if is_movie_watch_path(&key) && !keys.contains(&key) {
            keys.push(key);
        }
    }
    if keys.len() != 1 {
        return Err(Error::Html(
            "HiAnime details page has no episode list".to_string(),
        ));
    }
    let key = keys.swap_remove(0);
    Ok(vec![Episode {
        url: Some(absolute_url(&key)),
        key,
        title: None,
        episode: Some(1.0),
        date_updated: None,
        lang: None,
        paywalled: None,
    }])
}

/// Reads the episode's stream token from the watch page's `article.hk-watch`.
///
/// `data-hk-watch` is single-quoted, HTML-escaped JSON. Decoding the attribute once leaves
/// the JSON text, which holds the `token` field.
pub fn watch_token(html: &str) -> Result<String> {
    let article = markup::next_element(html, "article", |tag| tag.has_class("hk-watch"))
        .ok_or_else(|| Error::Html("HiAnime watch page has no player config".to_string()))?;
    let raw = article
        .tag
        .attr("data-hk-watch")
        .ok_or_else(|| Error::Html("HiAnime player config is missing".to_string()))?;
    let config: serde_json::Value = serde_json::from_str(&raw)
        .map_err(|_| Error::Html("HiAnime player config is not JSON".to_string()))?;
    config
        .get("token")
        .and_then(serde_json::Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| Error::Html("HiAnime player config has no token".to_string()))
}

/// Decodes the MegaPlay episode id from a stream token.
///
/// The token is base64url of `<episodeId>:<hash>`. Returns `None` for a token with
/// characters outside the base64url alphabet or a prefix that is not all digits.
pub fn episode_id_from_token(token: &str) -> Option<String> {
    let well_formed = token
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'='));
    if !well_formed {
        return None;
    }
    let engine = GeneralPurpose::new(
        &alphabet::URL_SAFE,
        GeneralPurposeConfig::new().with_decode_padding_mode(DecodePaddingMode::Indifferent),
    );
    let bytes = engine.decode(token).ok()?;
    let text = std::str::from_utf8(&bytes).ok()?;
    let (id, _hash) = text.split_once(':')?;
    (!id.is_empty() && id.bytes().all(|byte| byte.is_ascii_digit())).then(|| id.to_string())
}

/// Maps the details "Status" text to an Ito status. Text this plugin does not recognize gives
/// `None`, so the caller keeps the stored status.
pub fn status_from_text(text: &str) -> Option<Status> {
    let lower = text.to_ascii_lowercase();
    if lower.contains("not aired") {
        // ito-rs has no not-yet-released status, so Unknown is the closest existing variant.
        Some(Status::Unknown)
    } else if lower.contains("airing") {
        Some(Status::Ongoing)
    } else if lower.contains("finished") || lower.contains("completed") {
        Some(Status::Completed)
    } else {
        None
    }
}

/// Returns the anime key for a HiAnime details or watch URL on hianime.ms.
///
/// `/details/<slug>-<id>` maps to itself. `/watch-<slug>-episode-<n>-<id>` maps to the
/// details key `/details/<slug>-<id>`.
pub fn anime_key_from_url(url: &str) -> Option<String> {
    let rest = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))?;
    let (host, path) = rest.split_once('/').unwrap_or((rest, ""));
    if !is_site_host(host) {
        return None;
    }
    let path = path.split(['?', '#']).next().unwrap_or_default();

    if let Some(slug) = path.strip_prefix("details/") {
        let key = format!("/details/{}", slug.trim_end_matches('/'));
        return is_details_path(&key).then_some(key);
    }
    let watch = path.strip_prefix("watch-")?;
    let (show, tail) = watch.split_once("-episode-")?;
    let (_number, short_id) = tail.rsplit_once('-')?;
    let key = format!("/details/{show}-{short_id}");
    is_details_path(&key).then_some(key)
}

/// Returns true for a details key such as `/details/one-piece-1a877`.
pub fn is_details_path(key: &str) -> bool {
    key.strip_prefix("/details/").is_some_and(is_slug)
}

/// Returns true for a watch key such as `/watch-one-piece-episode-1-1a877`.
pub fn is_watch_path(key: &str) -> bool {
    key.strip_prefix("/watch-").is_some_and(is_slug)
}

/// Returns true for a movie watch key such as `/watch-your-name-7k2m9`: a watch key with no
/// `-episode-<n>` part.
pub fn is_movie_watch_path(key: &str) -> bool {
    is_watch_path(key) && !key.contains("-episode-")
}

/// Turns an absolute, protocol-relative, or site-relative link into an absolute URL.
pub fn absolute_url(href: &str) -> String {
    if let Some(rest) = href.strip_prefix("//") {
        return format!("https://{rest}");
    }
    if href.starts_with("https://") || href.starts_with("http://") {
        return href.to_string();
    }
    format!("{BASE_URL}/{}", href.trim_start_matches('/'))
}

/// Returns the site path of a link: no scheme, host, query, or fragment, with a leading `/`.
///
/// Relative links are site links. Absolute and protocol-relative links count only when they
/// point at HiAnime. A link to any other host gives `None`.
pub fn site_path(href: &str) -> Option<String> {
    let (host, path) = if let Some(rest) = href.strip_prefix("//") {
        split_host(rest)
    } else if let Some((scheme, rest)) = href
        .split_once("://")
        .filter(|(scheme, _)| !scheme.contains(['/', '?', '#']))
    {
        if !(scheme.eq_ignore_ascii_case("https") || scheme.eq_ignore_ascii_case("http")) {
            return None;
        }
        split_host(rest)
    } else {
        ("", href)
    };
    if !host.is_empty() && !is_site_host(host) {
        return None;
    }
    let path = path.split(['?', '#']).next().unwrap_or_default();
    Some(format!("/{}", path.trim_start_matches('/')))
}

/// Splits `host` from the path, query, or fragment that follows it. The scheme and `//` must
/// already be removed.
fn split_host(rest: &str) -> (&str, &str) {
    match rest.find(['/', '?', '#']) {
        Some(at) => (&rest[..at], &rest[at..]),
        None => (rest, ""),
    }
}

/// Returns true for the HiAnime site host, with or without `www.`.
fn is_site_host(host: &str) -> bool {
    host.eq_ignore_ascii_case("hianime.ms") || host.eq_ignore_ascii_case("www.hianime.ms")
}

/// Returns the `(label, definition markup)` pairs of every `dl.hk-facts` list.
fn facts(html: &str) -> Vec<(String, &str)> {
    let mut facts = Vec::new();
    let mut cursor = html;
    while let Some(list) = markup::next_element(cursor, "dl", |tag| tag.has_class("hk-facts")) {
        cursor = list.after;
        let mut row_cursor = list.inner;
        while let Some(term) = markup::next_element(row_cursor, "dt", |_| true) {
            let Some(definition) = markup::next_element(term.after, "dd", |_| true) else {
                break;
            };
            facts.push((markup::text(term.inner), definition.inner));
            row_cursor = definition.after;
        }
    }
    facts
}

fn fact<'a>(facts: &[(String, &'a str)], label: &str) -> Option<&'a str> {
    facts
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case(label))
        .map(|(_, value)| *value)
}

/// Returns the text of each `a` in `block`, optionally only those with `required_class`.
fn link_texts(block: &str, required_class: Option<&str>) -> Vec<String> {
    let mut texts = Vec::new();
    let mut cursor = block;
    while let Some(link) = markup::next_element(cursor, "a", |tag| {
        required_class.is_none_or(|class| tag.has_class(class))
    }) {
        cursor = link.after;
        texts.extend(non_empty(markup::text(link.inner)));
    }
    texts
}

fn non_empty(value: String) -> Option<String> {
    let trimmed = value.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

/// Returns the show type from the details page's kick line, for example `TV` or `Movie`.
fn show_type(html: &str) -> Option<String> {
    markup::next_element(html, "*", |tag| tag.has_class("hk-title__kick"))
        .and_then(|kick| markup::next_element(kick.inner, "*", |tag| tag.has_class("hk-tag--line")))
        .and_then(|line| non_empty(markup::text(line.inner)))
}

fn is_movie_page(html: &str) -> bool {
    show_type(html).is_some_and(|kind| kind.eq_ignore_ascii_case("Movie"))
}

fn is_not_yet_aired(html: &str) -> bool {
    let facts = facts(html);
    fact(&facts, "Status")
        .is_some_and(|value| markup::text(value).eq_ignore_ascii_case("Not aired yet"))
}

/// Removes a trailing " (TYPE, YEAR)" from a card title attribute.
fn strip_meta(title: &str) -> String {
    match title.rsplit_once(" (") {
        Some((name, meta)) if meta.ends_with(')') => name.to_string(),
        _ => title.to_string(),
    }
}

fn is_slug(value: &str) -> bool {
    !value.is_empty()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;

    const CARDS: &str = r#"
<main>
<article class="hk-tile" data-pf-card data-pf-ref="s6808">
  <a class="hk-tile__poster" href="/details/sample-show-abcde"><img src="/images/poster/sample-show.webp" alt=""></a>
  <h3 class="hk-tile__name"><a href="https://hianime.ms/details/sample-show-abcde?ref=list" title="Sample Show (TV, 2020)"><span class="hk-dual" data-en="Sample Show" data-jp="Sample JP"></span></a></h3>
  <p class="hk-tile__meta"><span>TV</span><span>2020</span></p>
</article>
<article class="hk-tile" data-pf-card>
  <img src="/images/poster/no-link.webp" alt="">
  <p>no detail link</p>
</article>
</main>"#;

    #[test]
    fn cards_parse_key_title_and_cover_and_skip_cards_without_a_detail_link() {
        let parsed = cards(CARDS);
        assert_eq!(parsed.len(), 1);
        let show = &parsed[0];
        assert_eq!(show.key, "/details/sample-show-abcde");
        assert_eq!(show.title, "Sample Show");
        assert_eq!(
            show.cover.as_deref(),
            Some("https://hianime.ms/images/poster/sample-show.webp")
        );
        assert_eq!(
            show.url.as_deref(),
            Some("https://hianime.ms/details/sample-show-abcde")
        );
    }

    #[test]
    fn card_title_falls_back_to_the_link_title_without_its_metadata() {
        let html = r#"<article class="hk-tile"><h3 class="hk-tile__name"><a href="/details/fallback-title-1x2y3" title="Fallback Title (Movie, 2001)">x</a></h3></article>"#;
        let parsed = cards(html);
        assert_eq!(parsed[0].title, "Fallback Title");
        assert!(parsed[0].cover.is_none());
    }

    #[test]
    fn card_links_accept_protocol_relative_hianime_hrefs_and_drop_other_hosts() {
        let html = r#"<article class="hk-tile"><h3 class="hk-tile__name"><a href="//www.hianime.ms/details/relative-one-abcde" title="Relative One (TV, 2020)">x</a></h3></article><article class="hk-tile"><h3 class="hk-tile__name"><a href="https://example.com/details/elsewhere-abcde" title="Elsewhere (TV, 2020)">x</a></h3></article>"#;
        let parsed = cards(html);
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].key, "/details/relative-one-abcde");
    }

    #[test]
    fn next_page_follows_rel_next_and_an_enabled_next_link_only() {
        let rel_next =
            r#"<head><link rel="prev" href="/x?page=1"><link rel="next" href="/x?page=3"></head>"#;
        let disabled =
            r##"<nav><a class="page-link disabled" aria-label="Next" href="#">Next</a></nav>"##;
        let enabled = r#"<nav><a class="page-link" aria-label="Next" href="/search?q=a&amp;page=2">Next</a></nav>"#;
        let prev_only = r#"<head><link rel="prev" href="/x"></head>"#;
        assert!(has_next_page(rel_next));
        assert!(!has_next_page(disabled));
        assert!(has_next_page(enabled));
        assert!(!has_next_page(prev_only));
    }

    #[test]
    fn result_count_reads_the_n_found_heading() {
        assert_eq!(
            result_count("<h1>Search</h1><p><span>58</span> found</p>"),
            Some(58)
        );
        assert_eq!(result_count("<p>1,204 found</p>"), Some(1204));
        assert_eq!(result_count("<p>0 found</p>"), Some(0));
        assert_eq!(result_count("<p>no anime found</p>"), None);
    }

    const DETAILS: &str = r#"
<section class="hk-title">
  <div class="hk-title__poster"><img src="/images/poster/sample-show.webp" alt=""></div>
  <div class="hk-title__kick"><span class="hk-tag--line">TV</span></div>
  <h1 class="hk-title__h" data-en="Sample Show" data-jp="Sample JP">Sample Show Page</h1>
  <div class="hk-clamp" itemprop="description">A &amp; B story.</div>
  <dl class="hk-facts">
    <dt>Status</dt><dd>Airing now</dd>
    <dt>Studio</dt><dd><a href="/studio/sample">Sample Studio</a></dd>
  </dl>
  <div class="hk-facts__genres"><a class="hk-chip" href="/genre/action" title="Action anime">Action</a><a class="hk-chip" href="/genre/drama">Drama</a></div>
</section>"#;

    #[test]
    fn details_read_title_cover_synopsis_status_type_genres_and_studio() {
        let parsed = details(DETAILS);
        assert_eq!(parsed.title.as_deref(), Some("Sample Show"));
        assert_eq!(
            parsed.cover.as_deref(),
            Some("https://hianime.ms/images/poster/sample-show.webp")
        );
        assert_eq!(parsed.description.as_deref(), Some("A & B story."));
        assert!(matches!(parsed.status, Some(Status::Ongoing)));
        assert_eq!(parsed.tags, vec!["TV", "Action", "Drama"]);
        assert_eq!(parsed.studios, vec!["Sample Studio"]);
    }

    #[test]
    fn a_details_page_without_fields_reports_nothing_so_callers_keep_existing_data() {
        let parsed = details("<main><h1>Nothing here</h1></main>");
        assert!(parsed.title.is_none());
        assert!(parsed.description.is_none());
        assert!(parsed.status.is_none());
        assert!(parsed.tags.is_empty());
        assert!(parsed.studios.is_empty());
    }

    #[test]
    fn status_maps_airing_and_finished_text_and_leaves_other_text_unmapped() {
        assert!(matches!(
            status_from_text("Airing now"),
            Some(Status::Ongoing)
        ));
        assert!(matches!(
            status_from_text("Finished"),
            Some(Status::Completed)
        ));
        assert!(matches!(
            status_from_text("Completed"),
            Some(Status::Completed)
        ));
        assert!(status_from_text("Not yet aired").is_none());
        assert!(status_from_text("").is_none());
    }

    const EPISODES: &str = r#"
<section id="hk-episodes" class="hk-eps">
  <div class="hk-eps__range" data-range="1-100"><button data-hk-range="1">1-100</button></div>
  <a class="hk-ep" href="/watch-sample-show-episode-1-abcde" data-ep="1" data-name="The Start" title="Episode 1: The Start"><span class="hk-ep__num">1</span></a>
  <a class="hk-ep" href="/watch-sample-show-episode-2-abcde" data-ep="2" data-name="Second"><span class="hk-ep__dub">Dub</span></a>
  <a class="hk-ep" href="/elsewhere" data-ep="3"></a>
</section>
<a class="hk-ep" href="/watch-outside-episode-9-zzzzz" data-ep="9"></a>"#;

    #[test]
    fn episodes_read_watch_key_number_and_title_from_the_episode_section_only() {
        let parsed = episodes(EPISODES).expect("episode section");
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].key, "/watch-sample-show-episode-1-abcde");
        assert_eq!(parsed[0].episode, Some(1.0));
        assert_eq!(parsed[0].title.as_deref(), Some("The Start"));
        assert_eq!(
            parsed[0].url.as_deref(),
            Some("https://hianime.ms/watch-sample-show-episode-1-abcde")
        );
        assert_eq!(parsed[1].episode, Some(2.0));
        assert_eq!(parsed[1].title.as_deref(), Some("Second"));
    }

    #[test]
    fn episodes_without_the_episode_section_are_an_error_not_an_empty_list() {
        let html =
            r#"<main><a class="hk-ep" href="/watch-x-episode-1-abcde" data-ep="1"></a></main>"#;
        assert!(matches!(episodes(html), Err(Error::Html(_))));
    }

    #[test]
    fn watch_token_reads_the_single_quoted_escaped_player_config() {
        let token = URL_SAFE_NO_PAD.encode("2142:doe");
        let html = concat!(
            "<article class=\"hk-watch\" data-hk-watch='{&quot;ep&quot;:1,",
            "&quot;token&quot;:&quot;TOKEN&quot;,&quot;lang&quot;:&quot;sub&quot;}'></article>"
        )
        .replace("TOKEN", &token);
        assert_eq!(watch_token(&html).expect("token"), token);
    }

    #[test]
    fn watch_page_without_a_config_is_an_error() {
        assert!(watch_token("<article class=\"hk-watch\"></article>").is_err());
    }

    #[test]
    fn episode_id_is_the_numeric_prefix_of_the_token() {
        let token = URL_SAFE_NO_PAD.encode("2142:doe");
        assert_eq!(episode_id_from_token(&token).as_deref(), Some("2142"));
        assert_eq!(
            episode_id_from_token(&URL_SAFE_NO_PAD.encode("doe:2142")),
            None
        );
        assert_eq!(episode_id_from_token("MjE0%2"), None);
    }

    #[test]
    fn anime_keys_come_from_details_and_watch_urls_on_the_site_only() {
        assert_eq!(
            anime_key_from_url("https://hianime.ms/details/one-piece-1a877?x=1").as_deref(),
            Some("/details/one-piece-1a877")
        );
        assert_eq!(
            anime_key_from_url("https://www.hianime.ms/watch-one-piece-episode-1-1a877").as_deref(),
            Some("/details/one-piece-1a877")
        );
        assert_eq!(
            anime_key_from_url("https://example.com/details/one-piece-1a877"),
            None
        );
        assert_eq!(anime_key_from_url("https://hianime.ms/search?q=one"), None);
        assert_eq!(
            anime_key_from_url("https://hianime.ms/watch-one-piece-episode-1"),
            None
        );
    }

    #[test]
    fn site_paths_keep_only_hianime_links_and_strip_query_and_fragment() {
        assert_eq!(
            site_path("https://hianime.ms/details/a-b?ref=1#x").as_deref(),
            Some("/details/a-b")
        );
        assert_eq!(
            site_path("//www.hianime.ms/details/a-b").as_deref(),
            Some("/details/a-b")
        );
        assert_eq!(
            site_path("/watch-a-episode-1-b").as_deref(),
            Some("/watch-a-episode-1-b")
        );
        assert_eq!(site_path("https://example.com/details/a-b"), None);
        assert_eq!(site_path("//example.com/details/a-b"), None);
        assert_eq!(site_path("https://hianime.ms.example/details/a-b"), None);
        assert_eq!(site_path("ftp://hianime.ms/details/a-b"), None);
    }

    #[test]
    fn absolute_urls_resolve_protocol_relative_and_site_relative_links() {
        assert_eq!(
            absolute_url("/images/poster/a.webp"),
            "https://hianime.ms/images/poster/a.webp"
        );
        assert_eq!(
            absolute_url("//hianime.ms/images/poster/a.webp"),
            "https://hianime.ms/images/poster/a.webp"
        );
    }

    const MOVIE: &str = r#"<section class="hk-title"><div class="hk-title__kick"><span class="hk-tag--line">Movie</span></div><h1 class="hk-title__h" data-en="Your Name">Your Name</h1><a class="hk-btn" href="https://hianime.ms/watch-your-name-7k2m9">Watch</a></section>"#;

    #[test]
    fn a_movie_details_page_yields_one_episode_from_its_watch_link() {
        let parsed = episodes(MOVIE).expect("movie episode");
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].key, "/watch-your-name-7k2m9");
        assert_eq!(parsed[0].episode, Some(1.0));
        assert!(parsed[0].title.is_none());
        assert_eq!(
            parsed[0].url.as_deref(),
            Some("https://hianime.ms/watch-your-name-7k2m9")
        );
    }

    #[test]
    fn a_details_page_with_several_bare_watch_links_and_no_episode_section_is_an_error() {
        let html = r#"<main><a href="/watch-your-name-7k2m9">Watch</a><a href="https://hianime.ms/watch-other-show-q3w4e">Other</a><a href="/watch-your-name-7k2m9?ref=2">Again</a></main>"#;
        assert!(matches!(episodes(html), Err(Error::Html(_))));
    }

    const UPCOMING: &str = r#"<section class="hk-title"><div class="hk-title__kick"><span class="hk-tag--line">TV</span></div><h1 class="hk-title__h" data-en="Upcoming Show">Upcoming Show</h1><dl class="hk-facts"><dt>Status</dt><dd>Not aired yet</dd></dl><span class="hk-tag">Not aired yet</span></section>"#;

    #[test]
    fn an_upcoming_show_without_an_episode_section_has_no_episodes() {
        assert!(episodes(UPCOMING).expect("upcoming show").is_empty());
    }

    #[test]
    fn an_upcoming_show_maps_its_status_to_unknown() {
        assert!(matches!(details(UPCOMING).status, Some(Status::Unknown)));
    }

    #[test]
    fn a_series_page_missing_its_episode_section_is_an_error_despite_one_bare_watch_link() {
        let html = r#"<section class="hk-title"><div class="hk-title__kick"><span class="hk-tag--line">TV</span></div><a href="/watch-other-show-q3w4e">Other</a></section>"#;
        assert!(matches!(episodes(html), Err(Error::Html(_))));
    }

    #[test]
    fn result_count_ignores_script_and_style_bodies() {
        let html =
            "<style>.x{}</style><script>var s = '5 found';</script><h1><span>58</span> found</h1>";
        assert_eq!(result_count(html), Some(58));
        assert_eq!(result_count("<script>3 found</script>"), None);
    }
}
