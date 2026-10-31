//! The home page: new series, trending series and the latest chapter updates.
//!
//! The page mixes its own sections with sponsored cards styled like series. Only addresses that
//! are series pages of this site make a card; sponsored cards (marked as such, and linking off
//! the site) are not part of any section.

use crate::detail::{Facts, chapter_from_anchor};
use crate::dom::{Query, attr, non_empty, text};
use crate::images;
use crate::links::{self, Link};
use crate::models::Listed;
use ito_rs::models::Chapter;
use ito_rs::{Error, Result};
use scraper::{ElementRef, Html};

/// A series with the chapter its card announces.
pub struct Update {
    pub listed: Listed,
    pub chapter: Chapter,
}

#[derive(Default)]
pub struct Home {
    pub new_series: Vec<Listed>,
    pub trending: Vec<Listed>,
    pub latest: Vec<Update>,
}

/// Reads the sections of the home page. Each section stands alone, so one the site changes does
/// not take the others with it; a page with none of them is an error, not an empty home.
pub fn parse(doc: &Html) -> Result<Home> {
    let home = Home {
        new_series: new_series(doc)?,
        trending: trending(doc)?,
        latest: latest(doc)?,
    };
    if home.new_series.is_empty() && home.trending.is_empty() && home.latest.is_empty() {
        return Err(Error::Host(
            "Temple Scan changed its home page; the source needs an update".into(),
        ));
    }
    Ok(home)
}

fn section_of<'a>(heading: &ElementRef<'a>) -> Option<ElementRef<'a>> {
    heading
        .ancestors()
        .filter_map(ElementRef::wrap)
        .find(|node| node.value().name() == "section")
}

fn new_series(doc: &Html) -> Result<Vec<Listed>> {
    let Some(heading) = doc
        .query("h2")?
        .into_iter()
        .find(|heading| text(heading).eq_ignore_ascii_case("new series"))
    else {
        return Ok(Vec::new());
    };
    let Some(section) = section_of(&heading) else {
        return Ok(Vec::new());
    };
    let mut cards = Vec::new();
    for anchor in section.query("a[href]")? {
        // The slider repeats its slides to loop; the hero banner links carry no cover.
        if anchor.first("img")?.is_some() {
            cards.extend(card(&anchor, "span[class*='absolute']")?);
        }
    }
    Ok(unique(cards))
}

fn trending(doc: &Html) -> Result<Vec<Listed>> {
    let Some(section) = doc.first("section[aria-labelledby='trending-title']")? else {
        return Ok(Vec::new());
    };
    let mut cards = Vec::new();
    for anchor in section.query("ol > li > a[href]")? {
        cards.extend(card(&anchor, "span[class*='rounded-full']")?);
    }
    Ok(unique(cards))
}

fn latest(doc: &Html) -> Result<Vec<Update>> {
    let Some(heading) = doc.first("#latest-updates-title")? else {
        return Ok(Vec::new());
    };
    let Some(section) = section_of(&heading) else {
        return Ok(Vec::new());
    };
    let mut updates: Vec<Update> = Vec::new();
    for article in section.query("article")? {
        if attr(&article, "data-sponsored").is_some() {
            continue;
        }
        let Some(cover_link) = article.first("a[href]")? else {
            continue;
        };
        // The card's title is a heading next to the cover, not part of the cover's link.
        let title = article
            .first("h3 a[href]")?
            .map(|title| text(&title))
            .and_then(non_empty);
        let Some(listed) = card_with(&cover_link, "span[class*='absolute']", title)? else {
            continue;
        };
        let mut announced = None;
        for anchor in article.query("ul a[href]")? {
            if let Some(Link::Chapter { series, chapter }) =
                attr(&anchor, "href").and_then(links::parse)
                && series == listed.slug
            {
                announced = Some(chapter_from_anchor(
                    &anchor,
                    &series,
                    chapter,
                    &Facts::default(),
                )?);
                break;
            }
        }
        let Some(chapter) = announced else {
            continue;
        };
        if updates
            .iter()
            .all(|update| update.listed.slug != listed.slug)
        {
            updates.push(Update { listed, chapter });
        }
    }
    Ok(updates)
}

/// The title card `anchor` makes: a series address on the site, a title and, when present, a
/// cover and the site's label. Anything else (an advertisement, a banner) is not a card.
fn card(anchor: &ElementRef<'_>, badge: &str) -> Result<Option<Listed>> {
    let title = match anchor.first("h3")? {
        Some(heading) => non_empty(text(&heading)),
        None => match anchor.first("span[class*='line-clamp']")? {
            Some(label) => non_empty(text(&label)),
            None => None,
        },
    };
    card_with(anchor, badge, title)
}

/// Like [`card`], for a card whose title is found beside its link rather than inside it.
fn card_with(
    anchor: &ElementRef<'_>,
    badge: &str,
    title: Option<String>,
) -> Result<Option<Listed>> {
    let Some(Link::Series(slug)) = attr(anchor, "href").and_then(links::parse) else {
        return Ok(None);
    };
    let Some(title) = title else {
        return Ok(None);
    };
    let cover = anchor
        .first("img")?
        .and_then(|image| attr(&image, "src").or_else(|| attr(&image, "data-src")))
        .and_then(images::locate);
    let mut label = None;
    for span in anchor.query(badge)? {
        let candidate = text(&span);
        if !candidate.is_empty()
            && !["new", "sponsored"]
                .iter()
                .any(|word| candidate.eq_ignore_ascii_case(word))
        {
            label = Some(candidate);
            break;
        }
    }
    Ok(Some(Listed {
        slug,
        title,
        cover,
        badge: label,
        status: None,
    }))
}

fn unique(cards: Vec<Listed>) -> Vec<Listed> {
    let mut seen: Vec<Listed> = Vec::with_capacity(cards.len());
    for card in cards {
        if seen.iter().all(|kept| kept.slug != card.slug) {
            seen.push(card);
        }
    }
    seen
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::dom;
    use crate::models::severity;

    pub(crate) fn slide(slug: &str, title: &str, badge: &str) -> String {
        format!(
            "<div class='swiper-slide'><a href='/comic/{slug}'><img src='https://media.templetoons.com/file/b/covers/{slug}.webp' alt=''><div><h3>{title}</h3></div><span class='absolute top-2 right-2'>{badge}</span></a></div>"
        )
    }

    pub(crate) fn ranked(slug: &str, title: &str, badge: &str) -> String {
        format!(
            "<li><a href='/comic/{slug}'><span><span aria-hidden='true'>1</span><span class='sr-only'>Rank 1:</span><span><img src='https://media.templetoons.com/file/b/covers/{slug}.webp'></span></span><span><span class='line-clamp-2 h-10'>{title}</span><span><span class='hidden truncate rounded-full border'>{badge}</span><span>17K<span class='sr-only'>views</span></span></span></span></a></li>"
        )
    }

    pub(crate) fn update_card(
        slug: &str,
        title: &str,
        badge: &str,
        chapters: &[(&str, &str, &str, bool)],
    ) -> String {
        let rows: String = chapters
            .iter()
            .map(|(chapter, name, date, premium)| {
                let lock = if *premium {
                    "<span class='inline-flex'><svg></svg><span class='sr-only'>Premium</span></span>"
                } else {
                    ""
                };
                format!(
                    "<li><a href='/comic/{slug}/{chapter}'><span class='flex'>{lock}<span class='truncate'><span class='sr-only'>{title},</span>{name}</span></span><time datetime='{date}'><span aria-hidden='true'>1d</span><span class='sr-only'>2 days ago</span></time></a></li>"
                )
            })
            .collect();
        format!(
            "<article class='group/card'><a href='/comic/{slug}' tabindex='-1'><img src='https://media.templetoons.com/file/b/covers/{slug}.webp'><span class='absolute left-1.5 top-1.5'>New</span><span class='absolute bottom-1.5 right-1.5'>{badge}</span></a><h3><a href='/comic/{slug}'>{title}</a></h3><ul>{rows}</ul></article>"
        )
    }

    const SPONSORED: &str = "<article class='group/card' data-sponsored=''><a href='https://ads.example/4/1' rel='noopener noreferrer sponsored' target='_blank'><img src='/promo/card.webp'><span>Sponsored</span></a><h3><a href='https://ads.example/4/1'>Talk to a bot</a></h3></article>";

    pub(crate) fn home_page() -> String {
        format!(
            "<html><body><main>\
             <section><div class='hero-swiper'><div class='swiper-slide'><a href='/comic/hero-only'>Hero title</a></div></div></section>\
             <section><h2>New Series</h2><div class='swiper'>{}{}{}</div></section>\
             <section aria-labelledby='trending-title'><h2 id='trending-title'>Trending</h2><ol>{}{}</ol></section>\
             <section><h2 id='latest-updates-title'>Latest updates</h2><div>{}{}{}{}</div></section>\
             </main></body></html>",
            slide(
                "a-suckers-love-story",
                "A Sucker's Love Story",
                "One Shot +18"
            ),
            slide("switch-boyfriend", "Switch Boyfriend", "Manhwa +18"),
            slide(
                "a-suckers-love-story",
                "A Sucker's Love Story",
                "One Shot +18"
            ),
            ranked(
                "tears-on-a-withered-flower",
                "Tears on a Withered Flower",
                "Manhwa +18"
            ),
            ranked("survival-instinct", "Survival Instinct", "Manhwa"),
            SPONSORED,
            update_card(
                "until-he-becomes-an-orchid",
                "Until He Becomes An Orchid",
                "Manhwa +18",
                &[
                    (
                        "chapter-29",
                        "Chapter 29",
                        "2026-10-28T19:49:51.695Z",
                        false
                    ),
                    (
                        "chapter-28",
                        "Chapter 28",
                        "2026-10-21T16:12:33.852Z",
                        false
                    ),
                ]
            ),
            update_card(
                "evil-husband",
                "Evil Husband",
                "Manhwa +15",
                &[
                    ("chapter-52", "Chapter 52", "2026-10-30T04:46:31.593Z", true),
                    (
                        "chapter-51",
                        "Chapter 51",
                        "2026-10-23T00:28:32.455Z",
                        false
                    ),
                ]
            ),
            update_card("no-chapters", "No Chapters", "Manhwa", &[]),
        )
    }

    #[test]
    fn new_series_come_from_their_own_section_without_repeats_or_banners() {
        let home = parse(&dom::parse(&home_page())).expect("home");
        let slugs: Vec<_> = home.new_series.iter().map(|s| s.slug.as_str()).collect();
        assert_eq!(slugs, ["a-suckers-love-story", "switch-boyfriend"]);
        let first = &home.new_series[0];
        assert_eq!(first.title, "A Sucker's Love Story");
        assert_eq!(first.badge.as_deref(), Some("One Shot +18"));
        assert_eq!(
            first.cover.as_deref(),
            Some("https://media.templetoons.com/file/b/covers/a-suckers-love-story.webp")
        );
    }

    #[test]
    fn trending_is_the_ranked_list_in_rank_order() {
        let home = parse(&dom::parse(&home_page())).expect("home");
        let titles: Vec<_> = home.trending.iter().map(|s| s.title.as_str()).collect();
        assert_eq!(titles, ["Tears on a Withered Flower", "Survival Instinct"]);
        assert_eq!(home.trending[0].badge.as_deref(), Some("Manhwa +18"));
        assert_eq!(home.trending[1].badge.as_deref(), Some("Manhwa"));
    }

    #[test]
    fn latest_updates_announce_each_series_newest_chapter_and_skip_sponsored_cards() {
        let home = parse(&dom::parse(&home_page())).expect("home");
        let slugs: Vec<_> = home.latest.iter().map(|u| u.listed.slug.as_str()).collect();
        assert_eq!(slugs, ["until-he-becomes-an-orchid", "evil-husband"]);
        let orchid = &home.latest[0];
        assert_eq!(orchid.listed.title, "Until He Becomes An Orchid");
        assert_eq!(
            orchid.listed.badge.as_deref(),
            Some("Manhwa +18"),
            "the New label is not the badge"
        );
        assert_eq!(orchid.chapter.key, "chapter-29");
        assert_eq!(
            orchid.chapter.title.as_deref(),
            Some("Chapter 29"),
            "the series name read to screen readers is not the chapter"
        );
        assert_eq!(orchid.chapter.chapter, Some(29.0));
        assert_eq!(orchid.chapter.paywalled, None);
        assert!(orchid.chapter.date_updated.is_some());
        let evil = &home.latest[1];
        assert_eq!(evil.chapter.key, "chapter-52");
        assert_eq!(
            evil.chapter.paywalled,
            Some(true),
            "the newest chapter is early access"
        );
        assert_eq!(
            severity(&crate::models::rating_of(
                evil.listed.badge.as_deref().unwrap_or("")
            )),
            1
        );
    }

    #[test]
    fn a_section_the_site_changed_does_not_hide_the_others() {
        let html = home_page().replace("<h2 id='latest-updates-title'>", "<h2>");
        let home = parse(&dom::parse(&html)).expect("home");
        assert!(home.latest.is_empty());
        assert!(!home.new_series.is_empty() && !home.trending.is_empty());
    }

    #[test]
    fn a_page_with_no_known_section_is_an_error_not_an_empty_home() {
        let html = "<html><body><main><h2>Something new</h2></main></body></html>";
        assert!(matches!(parse(&dom::parse(html)), Err(Error::Host(_))));
    }
}
