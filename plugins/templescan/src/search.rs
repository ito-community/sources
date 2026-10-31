//! Text search through the site's own search endpoint.
//!
//! The site's search box asks `/api/search?q=<text>&page=<n>&limit=<n>` and shows
//! `{ total, page, limit, projects: [...] }`. Searching needs at least two characters, a page
//! holds at most 20 results, and the endpoint takes no filters or ordering.

use crate::models::Listed;
use ito_rs::{Error, Result};
use serde::Deserialize;

/// Results the site returns per page at most.
pub const LIMIT: u64 = 20;
const MIN_CHARS: usize = 2;
const MAX_CHARS: usize = 200;

/// The text to search for, as the site's own box would send it: trimmed, at most 200
/// characters, and at least two. Shorter text is not searched.
pub fn normalize(query: &str) -> Option<String> {
    let trimmed: String = query.trim().chars().take(MAX_CHARS).collect();
    let trimmed = trimmed.trim_end().to_owned();
    (trimmed.chars().count() >= MIN_CHARS).then_some(trimmed)
}

/// The site path that asks for one page of results for already normalised text.
pub fn path(query: &str, page: i32) -> String {
    format!(
        "/api/search?q={}&page={}&limit={LIMIT}",
        crate::url::encode(query),
        page.max(1)
    )
}

/// One page of results.
#[derive(Debug, PartialEq)]
pub struct Results {
    pub entries: Vec<Listed>,
    pub has_next_page: bool,
}

#[derive(Deserialize)]
struct Reply {
    total: u64,
    #[serde(default)]
    limit: Option<u64>,
    #[serde(default)]
    projects: Vec<Project>,
}

#[derive(Deserialize)]
struct Project {
    title: String,
    series_slug: String,
    #[serde(default)]
    thumbnail: Option<String>,
    #[serde(default)]
    badge: Option<String>,
    #[serde(default)]
    status: Option<String>,
}

/// Reads a successful search reply for `page`.
///
/// Novels are left out: the site lists them in search, but a novel chapter's reader carries no
/// text or images, so this source (which serves comics) cannot open them. They still count in
/// the site's total, which only makes the last page short.
pub fn parse(body: &str, page: i32) -> Result<Results> {
    let reply: Reply = serde_json::from_str(body).map_err(|_| {
        Error::Host("Temple Scan sent a search reply the source does not understand".into())
    })?;
    let limit = reply.limit.filter(|limit| *limit > 0).unwrap_or(LIMIT);
    let shown = u64::try_from(page.max(1))
        .unwrap_or(1)
        .saturating_mul(limit);
    let entries = reply
        .projects
        .into_iter()
        .filter(|project| {
            !project
                .badge
                .as_deref()
                .is_some_and(|badge| badge.trim().eq_ignore_ascii_case("novel"))
                && crate::links::valid_slug(&project.series_slug)
        })
        .map(|project| Listed {
            title: Some(project.title.trim())
                .filter(|title| !title.is_empty())
                .unwrap_or(&project.series_slug)
                .to_owned(),
            slug: project.series_slug,
            cover: project.thumbnail.filter(|cover| !cover.trim().is_empty()),
            badge: project.badge.filter(|badge| !badge.trim().is_empty()),
            status: project.status.filter(|status| !status.trim().is_empty()),
        })
        .collect();
    Ok(Results {
        entries,
        has_next_page: reply.total > shown,
    })
}

/// The error for a search reply that is not a success.
pub fn failure(status: u16) -> Error {
    match status {
        401 => Error::NeedsLogin(
            "Temple Scan refused the search. Sign in to the site or finish the verification it asks for, then try again."
                .into(),
        ),
        404 => Error::NotFound("Temple Scan's search was not found".into()),
        429 => Error::Server("Temple Scan is limiting searches; try again in a moment".into()),
        other => Error::Server(format!("Temple Scan search answered with HTTP {other}")),
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub(crate) fn body(total: u64, limit: u64, count: usize, extra: &str) -> String {
        let projects: Vec<String> = (0..count)
            .map(|n| {
                format!(
                    r#"{{"title":" Title {n} ","series_slug":"slug-{n}","thumbnail":"https://media.templetoons.com/file/b/covers/{n}.webp","badge":"Manhwa +18","status":"Ongoing","alternative_names":"x"}}"#
                )
            })
            .collect();
        format!(
            r#"{{"total":{total},"page":1,"limit":{limit},"projects":[{}{extra}]}}"#,
            projects.join(",")
        )
    }

    #[test]
    fn queries_are_trimmed_capped_and_need_two_characters() {
        assert_eq!(normalize("  orchid  ").as_deref(), Some("orchid"));
        assert_eq!(normalize("한글").as_deref(), Some("한글"));
        assert_eq!(normalize("a"), None);
        assert_eq!(normalize("   a  "), None);
        assert_eq!(normalize(""), None);
        assert_eq!(normalize("ab").as_deref(), Some("ab"));
        let long = "x".repeat(500);
        assert_eq!(normalize(&long).map(|q| q.chars().count()), Some(200));
    }

    #[test]
    fn the_path_carries_encoded_text_a_one_based_page_and_the_site_limit() {
        assert_eq!(path("orchid", 1), "/api/search?q=orchid&page=1&limit=20");
        assert_eq!(
            path("a&b=c d#e", 3),
            "/api/search?q=a%26b%3Dc%20d%23e&page=3&limit=20"
        );
        assert_eq!(path("한", 0), "/api/search?q=%ED%95%9C&page=1&limit=20");
        assert_eq!(path("x y", -4), "/api/search?q=x%20y&page=1&limit=20");
    }

    #[test]
    fn a_page_of_results_maps_to_titles_and_knows_whether_more_follow() {
        let reply = parse(&body(45, 20, 20, ""), 1).expect("results");
        assert_eq!(reply.entries.len(), 20);
        assert!(reply.has_next_page);
        assert_eq!(reply.entries[0].title, "Title 0", "titles are trimmed");
        assert_eq!(reply.entries[0].slug, "slug-0");
        assert_eq!(reply.entries[0].badge.as_deref(), Some("Manhwa +18"));
        let second = parse(&body(45, 20, 20, ""), 2).expect("results");
        assert!(second.has_next_page);
        let last = parse(&body(45, 20, 5, ""), 3).expect("results");
        assert!(!last.has_next_page);
        // An exact multiple has no empty page after it.
        assert!(
            !parse(&body(40, 20, 20, ""), 2)
                .expect("results")
                .has_next_page
        );
        // The site's effective limit decides, whatever was asked.
        assert!(
            parse(&body(25, 10, 10, ""), 2)
                .expect("results")
                .has_next_page
        );
        assert!(
            !parse(&body(25, 10, 5, ""), 3)
                .expect("results")
                .has_next_page
        );
    }

    #[test]
    fn no_matches_is_the_sites_short_reply() {
        let reply = parse(r#"{"total":0,"projects":[]}"#, 1).expect("results");
        assert!(reply.entries.is_empty() && !reply.has_next_page);
    }

    #[test]
    fn novels_and_unusable_entries_are_left_out() {
        let novel = r#",{"title":"Wild Night (novel)","series_slug":"wild-night-novel","thumbnail":"https://media.templetoons.com/file/b/covers/n.webp","badge":"Novel","status":"Ongoing"}"#;
        let broken = r#",{"title":"Bad","series_slug":"a/b","badge":"Manhwa"}"#;
        let blank =
            r#",{"title":"  ","series_slug":"blank-title","thumbnail":"","badge":"","status":""}"#;
        let reply = parse(&body(4, 20, 1, &format!("{novel}{broken}{blank}")), 1).expect("results");
        let slugs: Vec<_> = reply.entries.iter().map(|e| e.slug.as_str()).collect();
        assert_eq!(slugs, ["slug-0", "blank-title"]);
        let blank = &reply.entries[1];
        assert_eq!(
            blank.title, "blank-title",
            "a title the site left blank falls back to the slug"
        );
        assert_eq!(
            (
                blank.cover.as_deref(),
                blank.badge.as_deref(),
                blank.status.as_deref()
            ),
            (None, None, None)
        );
    }

    #[test]
    fn replies_that_are_not_search_replies_are_errors() {
        for text in [
            "",
            "<html>challenge</html>",
            "[]",
            r#"{"projects":[]}"#,
            r#"{"total":"many","projects":[]}"#,
            r#"{"total":1,"projects":[{"title":"x"}]}"#,
        ] {
            assert!(matches!(parse(text, 1), Err(Error::Host(_))), "{text:?}");
        }
    }

    #[test]
    fn failing_statuses_have_their_own_errors() {
        assert!(matches!(failure(403), Error::Server(message) if message.contains("403")));
        assert!(matches!(failure(401), Error::NeedsLogin(_)));
        assert!(matches!(failure(404), Error::NotFound(_)));
        assert!(matches!(failure(429), Error::Server(_)));
        assert!(matches!(failure(500), Error::Server(message) if message.contains("500")));
    }
}
