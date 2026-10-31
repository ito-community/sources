//! A series page: its details and its complete chapter list.
//!
//! The page lists every chapter in its markup (the "Show more" button only reveals what is
//! already there), each with the site's own lock mark for early-access chapters. The page's data
//! additionally carries each chapter's position number, which names a chapter such as
//! "Prologue" that has no number in its name.

use crate::dom::{Query, attr, non_empty, own_text, text};
use crate::flight::{Classified, Flight};
use crate::images;
use crate::links::{self, Link};
use crate::models::{is_status, rating_of, severity, status_of};
use crate::numbers;
use crate::time;
use ito_rs::models::manga::{ContentRating, Status};
use ito_rs::models::{Chapter, ContentKind};
use ito_rs::{Error, Result};
use scraper::{ElementRef, Html};
use serde_json::{Map, Value};
use std::collections::HashMap;

/// Everything a series page tells.
pub struct Series {
    pub title: String,
    pub cover: Option<String>,
    pub description: Option<String>,
    pub authors: Option<Vec<String>>,
    pub tags: Option<Vec<String>>,
    pub status: Status,
    pub rating: ContentRating,
    pub chapters: Vec<Chapter>,
}

fn layout(what: &str) -> Error {
    Error::Host(format!(
        "Temple Scan changed its series page ({what}); the source needs an update"
    ))
}

/// Whether the page is the site's "page not found" page.
pub fn is_not_found(doc: &Html) -> bool {
    doc.query("h1")
        .map(|headings| headings.iter().any(|heading| text(heading) == "404"))
        .unwrap_or(false)
}

/// Reads the page of series `slug`.
pub fn parse(doc: &Html, slug: &str) -> Result<Series> {
    let Some(heading) = doc.first("h1#series-title")? else {
        return Err(if is_not_found(doc) {
            Error::NotFound("Temple Scan has no such series".into())
        } else {
            layout("title")
        });
    };
    let title = non_empty(text(&heading)).ok_or_else(|| layout("title"))?;

    let cover = doc
        .first("meta[property='og:image']")?
        .and_then(|meta| attr(&meta, "content"))
        .and_then(images::locate);

    let mut paragraphs = Vec::new();
    if let Some(synopsis) = doc.first("#series-synopsis-text")? {
        for paragraph in synopsis.query("p")? {
            paragraphs.extend(non_empty(text(&paragraph)));
        }
        if paragraphs.is_empty() {
            paragraphs.extend(non_empty(text(&synopsis)));
        }
    }
    let description = (!paragraphs.is_empty()).then(|| paragraphs.join("\n\n"));

    let mut authors = Vec::new();
    for entry in doc.query("dl > div")? {
        let (Some(term), Some(detail)) = (entry.first("dt")?, entry.first("dd")?) else {
            continue;
        };
        if text(&term).eq_ignore_ascii_case("author") {
            authors = credits(&text(&detail));
        }
    }

    let mut status = Status::Unknown;
    for stat in doc.query("ul[aria-label='Series stats'] > li")? {
        let label = text(&stat);
        if is_status(&label) {
            status = status_of(&label);
            break;
        }
    }

    let mut tags = Vec::new();
    for genre in doc.query("ul[aria-label='Genres'] > li")? {
        tags.extend(non_empty(text(&genre)));
    }
    let rating = tags
        .iter()
        .map(|tag| rating_of(tag))
        .max_by_key(severity)
        .unwrap_or(ContentRating::Safe);

    Ok(Series {
        title,
        cover,
        description,
        authors: (!authors.is_empty()).then_some(authors),
        tags: (!tags.is_empty()).then_some(tags),
        status,
        rating,
        chapters: chapters(doc, slug)?,
    })
}

/// Splits a credit line such as `A, B | C and D` into names. Separators inside parentheses stay
/// part of the name.
pub fn credits(line: &str) -> Vec<String> {
    let mut names: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut depth = 0_u32;
    let chars: Vec<char> = line.chars().collect();
    let mut index = 0;
    while index < chars.len() {
        let c = chars[index];
        match c {
            '(' | '[' => depth += 1,
            ')' | ']' => depth = depth.saturating_sub(1),
            _ => {}
        }
        let separator = depth == 0
            && (matches!(c, ',' | '|' | '/' | '&' | ';')
                || (c == ' ' && and_follows(&chars, index)));
        if separator {
            if c == ' ' {
                index += 4;
            }
            push_name(&mut names, &mut current);
        } else {
            current.push(c);
        }
        index += 1;
    }
    push_name(&mut names, &mut current);
    names
}

/// Whether the characters from `index` spell a space, "and", and another space.
fn and_follows(chars: &[char], index: usize) -> bool {
    let word: String = chars.iter().skip(index).take(5).collect();
    word.eq_ignore_ascii_case(" and ")
}

fn push_name(names: &mut Vec<String>, current: &mut String) {
    let name = current.split_whitespace().collect::<Vec<_>>().join(" ");
    current.clear();
    if !name.is_empty() && !names.contains(&name) {
        names.push(name);
    }
}

/// What the page's data says about one chapter.
#[derive(Debug, Default, Clone, Copy)]
pub struct Facts {
    pub position: Option<f64>,
}

/// Per-chapter facts by chapter slug, from the page's data. The markup is authoritative, so
/// anything unreadable here only means those facts are not used.
fn facts(doc: &Html) -> HashMap<String, Facts> {
    let Ok(flight) = Flight::from_document(doc) else {
        return HashMap::new();
    };
    let Some(props) = flight.find(&["seriesData"]) else {
        return HashMap::new();
    };
    let Some(Value::Object(series)) = props.get("seriesData") else {
        return HashMap::new();
    };
    let seasons = series
        .values()
        .find_map(|value| value.as_array())
        .map(Vec::as_slice)
        .unwrap_or_default();
    let mut facts = HashMap::new();
    for season in seasons.iter().filter_map(Value::as_object) {
        let Some(Value::Array(chapters)) = season.values().find(|value| value.is_array()) else {
            continue;
        };
        let records: Vec<&Map<String, Value>> =
            chapters.iter().filter_map(Value::as_object).collect();
        let known = ["index", "chapter_title", "chapter_thumbnail", "created_at"];
        let Some(fields) = Classified::of(&records, &known) else {
            continue;
        };
        for record in records {
            let Some(slug) = record.get(&fields.slug).and_then(Value::as_str) else {
                continue;
            };
            let position = match record.get("index") {
                Some(Value::String(text)) => text.trim().parse::<f64>().ok(),
                Some(Value::Number(number)) => number.as_f64(),
                _ => None,
            };
            facts.insert(
                slug.to_owned(),
                Facts {
                    position: position.filter(|p| p.is_finite()),
                },
            );
        }
    }
    facts
}

/// Labels some chapter anchors carry besides the chapter's own name and subtitle.
fn is_badge(label: &str) -> bool {
    ["new", "premium", "up next", "read"]
        .iter()
        .any(|badge| label.eq_ignore_ascii_case(badge))
}

fn chapters(doc: &Html, series: &str) -> Result<Vec<Chapter>> {
    let items = doc.query("#chapter-list > li")?;
    if items.is_empty() && doc.first("#chapters-title")?.is_none() {
        return Err(layout("chapter list"));
    }
    let facts = facts(doc);
    let mut chapters: Vec<Chapter> = Vec::with_capacity(items.len());
    let mut listed = 0_usize;
    for item in &items {
        let Some(anchor) = item.first("a[href]")? else {
            continue;
        };
        let Some(Link::Chapter {
            series: owner,
            chapter: slug,
        }) = attr(&anchor, "href").and_then(links::parse)
        else {
            continue;
        };
        if owner != series {
            continue;
        }
        listed += 1;
        if chapters.iter().any(|chapter| chapter.key == slug) {
            // A repeated slug is the same page; the site shows it twice.
            continue;
        }
        let known = facts.get(&slug).copied().unwrap_or_default();
        chapters.push(chapter_from_anchor(&anchor, series, slug, &known)?);
    }
    match declared_total(doc)? {
        Some(total) if total != listed => Err(Error::Host(format!(
            "Temple Scan's chapter list is incomplete ({listed} of {total} chapters found); the source needs an update"
        ))),
        None if !items.is_empty() => Err(layout("chapter count")),
        _ => Ok(chapters),
    }
}

/// Builds a chapter from the anchor that links to it. The anchor holds the chapter's name, an
/// optional subtitle, its date, and the site's labels: "Premium" marks an early-access chapter.
pub fn chapter_from_anchor(
    anchor: &ElementRef<'_>,
    series: &str,
    slug: String,
    facts: &Facts,
) -> Result<Chapter> {
    let mut labels = Vec::new();
    let mut premium = false;
    for span in anchor.query("span")? {
        let label = own_text(&span);
        if label.eq_ignore_ascii_case("premium") {
            premium = true;
        }
        let hidden = span.value().classes().any(|class| class == "sr-only");
        // The date's own spans (an abbreviation and a spoken form) are not part of the name.
        let in_date = span
            .ancestors()
            .filter_map(ElementRef::wrap)
            .any(|node| node.value().name() == "time");
        if !label.is_empty() && !is_badge(&label) && !hidden && !in_date {
            labels.push(label);
        }
    }
    let mut labels = labels.into_iter();
    let name = labels.next().unwrap_or_else(|| slug.clone());
    let subtitle = labels.next();
    let number =
        numbers::from_name(&name).or_else(|| facts.position.map(|position| position as f32));
    let date = anchor
        .first("time[datetime]")?
        .and_then(|time| attr(&time, "datetime"))
        .and_then(time::parse_iso);
    let title = match subtitle {
        Some(subtitle) => format!("{name}: {subtitle}"),
        None => name,
    };
    Ok(Chapter {
        url: Some(links::chapter_url(series, &slug)),
        key: slug,
        title: Some(title),
        volume: None,
        chapter: number,
        date_updated: date,
        scanlator: None,
        lang: Some("en".into()),
        // Only the site's own early-access marks lock a chapter; nothing is assumed otherwise.
        paywalled: premium.then_some(true),
        kind: ContentKind::Images,
    })
}

/// The "N total" the page prints beside the heading of its chapter list.
fn declared_total(doc: &Html) -> Result<Option<usize>> {
    let Some(label) = doc.first("#chapters-title + span")? else {
        return Ok(None);
    };
    let digits: String = text(&label).chars().filter(char::is_ascii_digit).collect();
    Ok(digits.parse().ok())
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::dom;
    use crate::flight::script;

    pub(crate) struct Row<'a> {
        pub slug: &'a str,
        pub name: &'a str,
        pub subtitle: Option<&'a str>,
        pub date: &'a str,
        pub premium: bool,
    }

    pub(crate) fn row<'a>(slug: &'a str, name: &'a str, date: &'a str) -> Row<'a> {
        Row {
            slug,
            name,
            subtitle: None,
            date,
            premium: false,
        }
    }

    pub(crate) fn list_item(series: &str, row: &Row<'_>) -> String {
        let subtitle = row
            .subtitle
            .map(|subtitle| format!("<span class='truncate'>{subtitle}</span>"))
            .unwrap_or_default();
        let premium = if row.premium {
            "<span class='new'>New</span><span class='amber'><svg></svg>Premium</span>"
        } else {
            ""
        };
        format!(
            "<li><a href='/comic/{series}/{slug}' id='chapter-link-{slug}'><div class='thumb'><img src='https://media.templetoons.com/file/b/thumbnails/x.jpg'></div><div class='info'><span class='truncate'>{name}</span>{subtitle}<span class='meta'>{premium}<time datetime='{date}'>3 days ago</time></span></div></a></li>",
            slug = row.slug,
            name = row.name,
            date = row.date,
        )
    }

    pub(crate) fn page(series: &str, rows: &[Row<'_>], extra: &str) -> String {
        let items: String = rows.iter().map(|row| list_item(series, row)).collect();
        format!(
            "<html><head><meta property='og:image' content='https://media.templetoons.com/file/b/covers/c.webp'></head><body><main>\
             <h1 id='series-title'>  Evil Husband </h1>\
             <dl><div><dt>Author</dt><dd>Saha, Bomi (artist, color) | Lee and Kim</dd></div><div><dt>Studio</dt><dd>Ridi</dd></div></dl>\
             <ul aria-label='Series stats'><li>1.4M views</li><li>{n} chapters</li><li>Completed</li><li>Updated 3 days ago</li></ul>\
             <ul aria-label='Genres'><li>Romance</li><li>+18</li><li>Josei</li></ul>\
             <div id='series-synopsis-text'><div><p>First paragraph.</p><p>  Second\n paragraph. </p><p> </p></div></div>\
             <h2 id='chapters-title'>Chapters</h2><span>{n} total</span>\
             <ul id='chapter-list'>{items}</ul>{extra}</main></body></html>",
            n = rows.len()
        )
    }

    fn series(rows: &[Row<'_>], extra: &str) -> Series {
        parse(
            &dom::parse(&page("evil-husband", rows, extra)),
            "evil-husband",
        )
        .expect("series")
    }

    #[test]
    fn details_come_from_the_page() {
        let series = series(&[row("chapter-1", "Chapter 1", "2025-01-01T00:00:00Z")], "");
        assert_eq!(series.title, "Evil Husband");
        assert_eq!(
            series.cover.as_deref(),
            Some("https://media.templetoons.com/file/b/covers/c.webp")
        );
        assert_eq!(
            series.description.as_deref(),
            Some("First paragraph.\n\nSecond paragraph.")
        );
        assert_eq!(
            series.authors.as_deref(),
            Some(
                &[
                    "Saha".to_owned(),
                    "Bomi (artist, color)".to_owned(),
                    "Lee".to_owned(),
                    "Kim".to_owned()
                ][..]
            )
        );
        assert_eq!(
            series.tags.as_deref(),
            Some(&["Romance".to_owned(), "+18".to_owned(), "Josei".to_owned()][..])
        );
        assert!(matches!(series.status, Status::Completed));
        assert_eq!(
            severity(&series.rating),
            2,
            "the +18 genre marks an adult series"
        );
    }

    #[test]
    fn credit_lines_split_on_separators_outside_brackets() {
        assert_eq!(
            credits("Gyeja (계자), Bammandeuk"),
            ["Gyeja (계자)", "Bammandeuk"]
        );
        assert_eq!(credits("Hyeon Minye and Bullye"), ["Hyeon Minye", "Bullye"]);
        assert_eq!(credits("Seom On-hwa / Hase"), ["Seom On-hwa", "Hase"]);
        assert_eq!(credits("Kim Duyoo | Hamji"), ["Kim Duyoo", "Hamji"]);
        assert_eq!(
            credits("Ondo, YP, ZIG, Blue N"),
            ["Ondo", "YP", "ZIG", "Blue N"]
        );
        assert_eq!(credits("Zigozi9 Mune Ki"), ["Zigozi9 Mune Ki"]);
        assert_eq!(
            credits("Anderson"),
            ["Anderson"],
            "a name containing 'and' is one name"
        );
        assert_eq!(credits("A, A,  "), ["A"]);
        assert!(credits("  ").is_empty());
    }

    #[test]
    fn chapters_keep_site_order_numbers_dates_and_titles() {
        let mut half = row("chapter-86-5", "Chapter 86.5", "2025-02-01T00:00:00Z");
        half.subtitle = Some("Season 2 Afterword");
        let rows = [
            row("chapter-87", "Chapter 87", "2025-03-01T00:00:00Z"),
            half,
            row("chapter-86", "chapter 86", "2025-01-01T00:00:00Z"),
            row("chapter-0", "Chapter 0", "2024-12-01T00:00:00Z"),
        ];
        let chapters = series(&rows, "").chapters;
        let keys: Vec<_> = chapters.iter().map(|c| c.key.as_str()).collect();
        assert_eq!(
            keys,
            ["chapter-87", "chapter-86-5", "chapter-86", "chapter-0"]
        );
        let numbers: Vec<_> = chapters.iter().map(|c| c.chapter).collect();
        assert_eq!(numbers, [Some(87.0), Some(86.5), Some(86.0), Some(0.0)]);
        assert_eq!(
            chapters[1].title.as_deref(),
            Some("Chapter 86.5: Season 2 Afterword")
        );
        assert_eq!(chapters[2].title.as_deref(), Some("chapter 86"));
        assert_eq!(chapters[0].date_updated, Some(1_740_787_200.0));
        assert_eq!(
            chapters[0].url.as_deref(),
            Some("https://templetoons.com/comic/evil-husband/chapter-87")
        );
        assert!(
            chapters
                .iter()
                .all(|c| c.kind == ContentKind::Images && c.lang.as_deref() == Some("en"))
        );
    }

    #[test]
    fn odd_chapter_slugs_are_keys_as_they_are() {
        let rows = [
            row("103", "Chapter 103", "2025-01-02T00:00:00Z"),
            row("95958-chapter-1", "Chapter 1", "2025-01-01T00:00:00Z"),
        ];
        let chapters = series(&rows, "").chapters;
        assert_eq!(chapters[0].key, "103");
        assert_eq!(chapters[1].key, "95958-chapter-1");
        assert_eq!(
            chapters[1].url.as_deref(),
            Some("https://templetoons.com/comic/evil-husband/95958-chapter-1")
        );
    }

    #[test]
    fn early_access_chapters_are_marked_locked_and_the_rest_are_not() {
        let mut locked = row("chapter-52", "Chapter 52", "2026-10-30T04:46:31.593Z");
        locked.premium = true;
        let rows = [
            locked,
            row("chapter-51", "Chapter 51", "2026-10-23T00:28:32.455Z"),
        ];
        let chapters = series(&rows, "").chapters;
        assert_eq!(chapters[0].paywalled, Some(true));
        assert_eq!(chapters[1].paywalled, None);
        // The marker is a label, not a word in a chapter's own name.
        let mut named = row("chapter-1", "Premium Edition 1", "2025-01-01T00:00:00Z");
        named.subtitle = Some("Read me");
        let chapters = series(&[named], "").chapters;
        assert_eq!(chapters[0].paywalled, None);
        assert_eq!(
            chapters[0].title.as_deref(),
            Some("Premium Edition 1: Read me")
        );
    }

    #[test]
    fn the_pages_data_names_unnumbered_chapters() {
        // Field names are renamed tokens in the site's data; the plain ones are fixed.
        let row_data = "9:[\"$\",\"$L29\",null,{\"seriesData\":{\"ts1\":\"evil-husband\",\"ts2\":[{\"index\":0,\"season_name\":\"All\",\"ts3\":[\
            {\"index\":\"2\",\"ts4\":\"Chapter 2\",\"chapter_title\":null,\"chapter_thumbnail\":null,\"ts5\":\"chapter-2\",\"ts6\":10,\"created_at\":\"2025-01-02T00:00:00Z\"},\
            {\"index\":\"1\",\"ts4\":\"Chapter 1\",\"chapter_title\":null,\"chapter_thumbnail\":null,\"ts5\":\"chapter-1\",\"ts6\":0,\"created_at\":\"2025-01-01T00:00:00Z\"},\
            {\"index\":\"0\",\"ts4\":\"Prologue\",\"chapter_title\":null,\"chapter_thumbnail\":null,\"ts5\":\"prologue\",\"ts6\":0,\"created_at\":\"2024-12-01T00:00:00Z\"}]}]}}]\n";
        let rows = [
            row("chapter-2", "Chapter 2", "2025-01-02T00:00:00Z"),
            row("chapter-1", "Chapter 1", "2025-01-01T00:00:00Z"),
            row("prologue", "Prologue", "2024-12-01T00:00:00Z"),
        ];
        let chapters = series(&rows, &script(&[row_data])).chapters;
        assert_eq!(chapters[2].chapter, Some(0.0), "Prologue is chapter zero");
        assert_eq!(
            chapters[0].paywalled, None,
            "a price is not a lock; only the site's label marks early access"
        );
        assert_eq!(chapters[1].paywalled, None);
        // Without the data there is nothing to say about the unnumbered chapter.
        let chapters = series(&rows, "").chapters;
        assert_eq!(chapters[2].chapter, None);
    }

    #[test]
    fn a_name_with_a_number_is_never_overridden_by_a_position() {
        let row_data = "9:{\"seriesData\":{\"ts1\":\"s\",\"ts2\":[{\"index\":0,\"ts3\":[\
            {\"index\":\"56.4\",\"ts4\":\"Chapter 56.5\",\"chapter_title\":null,\"chapter_thumbnail\":null,\"ts5\":\"chapter-56-5\",\"ts6\":0,\"created_at\":\"2025-01-02T00:00:00Z\"}]}]}}\n";
        let rows = [row("chapter-56-5", "Chapter 56.5", "2025-01-02T00:00:00Z")];
        assert_eq!(
            series(&rows, &script(&[row_data])).chapters[0].chapter,
            Some(56.5)
        );
    }

    #[test]
    fn repeated_slugs_are_one_chapter_and_foreign_links_are_ignored() {
        let rows = [
            row("chapter-2", "Chapter 2", "2025-01-02T00:00:00Z"),
            row("chapter-2", "Chapter 2", "2025-01-02T00:00:00Z"),
            row("chapter-1", "Chapter 1", "2025-01-01T00:00:00Z"),
        ];
        let html = page("evil-husband", &rows, "").replace(
            "<li><a href='/comic/evil-husband/chapter-1'",
            "<li><a href='/comic/other/chapter-1'",
        );
        // The other series' link is skipped, so the list is two short of the three printed.
        let error = parse(&dom::parse(&html), "evil-husband")
            .err()
            .expect("incomplete");
        assert!(matches!(error, Error::Host(message) if message.contains("incomplete")));
        // Without the foreign link, the repeated slug collapses and still matches the total.
        let rows = [
            row("chapter-2", "Chapter 2", "2025-01-02T00:00:00Z"),
            row("chapter-2", "Chapter 2", "2025-01-02T00:00:00Z"),
        ];
        let chapters = series(&rows, "").chapters;
        assert_eq!(chapters.len(), 1);
    }

    #[test]
    fn a_list_shorter_than_its_total_is_an_error_not_a_partial_list() {
        let rows = [
            row("chapter-2", "Chapter 2", "2025-01-02T00:00:00Z"),
            row("chapter-1", "Chapter 1", "2025-01-01T00:00:00Z"),
        ];
        let html = page("evil-husband", &rows, "").replace("2 total", "1,116 total");
        assert!(matches!(
            parse(&dom::parse(&html), "evil-husband"),
            Err(Error::Host(_))
        ));
    }

    #[test]
    fn a_list_whose_total_cannot_be_read_is_a_layout_error() {
        let rows = [row("chapter-2", "Chapter 2", "2025-01-02T00:00:00Z")];
        let html = page("evil-husband", &rows, "").replace("1 total", "several total");
        assert!(matches!(
            parse(&dom::parse(&html), "evil-husband"),
            Err(Error::Host(message)) if message.contains("changed its series page")
        ));
    }

    #[test]
    fn a_series_without_chapters_is_empty_but_a_missing_list_is_an_error() {
        let html = page("evil-husband", &[], "");
        assert!(
            parse(&dom::parse(&html), "evil-husband")
                .expect("series")
                .chapters
                .is_empty()
        );
        let html = page("evil-husband", &[], "")
            .replace(
                "<h2 id='chapters-title'>Chapters</h2><span>0 total</span>",
                "",
            )
            .replace("<ul id='chapter-list'></ul>", "");
        assert!(matches!(
            parse(&dom::parse(&html), "evil-husband"),
            Err(Error::Host(_))
        ));
    }

    #[test]
    fn missing_pages_and_changed_pages_are_told_apart() {
        let missing = "<html><body><h1>404</h1><p>This page does not exist or is no longer available.</p></body></html>";
        assert!(is_not_found(&dom::parse(missing)));
        assert!(matches!(
            parse(&dom::parse(missing), "x"),
            Err(Error::NotFound(_))
        ));
        let changed = "<html><body><h1>Something else</h1></body></html>";
        assert!(!is_not_found(&dom::parse(changed)));
        assert!(matches!(
            parse(&dom::parse(changed), "x"),
            Err(Error::Host(_))
        ));
    }

    #[test]
    fn sparse_series_pages_still_parse() {
        let html = "<html><body><h1 id='series-title'>Bare</h1><h2 id='chapters-title'>Chapters</h2><ul id='chapter-list'></ul></body></html>";
        let series = parse(&dom::parse(html), "bare").expect("series");
        assert_eq!(series.title, "Bare");
        assert!(
            series.description.is_none()
                && series.authors.is_none()
                && series.tags.is_none()
                && series.cover.is_none()
        );
        assert!(matches!(series.status, Status::Unknown));
        assert_eq!(severity(&series.rating), 0);
    }
}
