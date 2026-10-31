//! The complete catalog and how it is browsed.
//!
//! The site's `/comics` page does not ask the server for pages: it receives every series at
//! once, then filters by status, sorts, and pages in the browser (15 at a time, with a "Load
//! more" button growing the visible slice). The same complete list is what this module reads and
//! applies the same status filter and orderings to, so a browse here shows what the site shows.

use crate::flight::{Classified, Flight};
use crate::links;
use crate::models::Listed;
use crate::time;
use ito_rs::{Error, Result};
use scraper::Html;
use serde_json::{Map, Value};
use std::cmp::Ordering;

/// Entries per page of a browse.
pub const PAGE_SIZE: usize = 20;

/// Fields the site leaves unrenamed in each series record.
const KNOWN: [&str; 7] = [
    "thumbnail",
    "badge",
    "status",
    "created_at",
    "alternative_names",
    "update_chapter",
    "total_views",
];

/// One series of the catalog.
#[derive(Debug, Clone, PartialEq)]
pub struct Item {
    pub slug: String,
    pub title: String,
    pub cover: Option<String>,
    pub badge: Option<String>,
    pub status: String,
    pub created: Option<f64>,
    pub updated: Option<f64>,
    pub views: u64,
}

impl Item {
    pub fn listed(&self) -> Listed {
        Listed {
            slug: self.slug.clone(),
            title: self.title.clone(),
            cover: self.cover.clone(),
            badge: self.badge.clone(),
            status: Some(self.status.clone()),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Order {
    Updated,
    Created,
    Title,
    Views,
}

impl Order {
    pub const DEFAULT: Order = Order::Updated;

    pub fn from_id(id: &str) -> Option<Order> {
        match id {
            "update_chapter" => Some(Order::Updated),
            "created_at" => Some(Order::Created),
            "title" => Some(Order::Title),
            "total_views" => Some(Order::Views),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Direction {
    Descending,
    Ascending,
}

impl Direction {
    pub fn from_id(id: &str) -> Option<Direction> {
        match id {
            "desc" => Some(Direction::Descending),
            "asc" => Some(Direction::Ascending),
            _ => None,
        }
    }
}

/// What to show from the catalog.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Browse {
    /// Only series with exactly this status; `None` shows every status.
    pub status: Option<String>,
    pub order: Order,
    pub direction: Direction,
}

impl Default for Browse {
    fn default() -> Self {
        Browse {
            status: None,
            order: Order::DEFAULT,
            direction: Direction::Descending,
        }
    }
}

fn layout() -> Error {
    Error::Host("Temple Scan changed how it lists its comics; the source needs an update".into())
}

/// Reads the complete catalog out of the `/comics` page.
pub fn parse(doc: &Html) -> Result<Vec<Item>> {
    let flight = Flight::from_document(doc)?;
    let props = flight.find(&["list", "initialPage"]).ok_or_else(layout)?;
    let Some(Value::Array(list)) = props.get("list") else {
        return Err(layout());
    };
    let records: Vec<&Map<String, Value>> = list.iter().filter_map(Value::as_object).collect();
    if records.is_empty() || records.len() != list.len() {
        return Err(layout());
    }
    let fields = Classified::of(&records, &KNOWN).ok_or_else(layout)?;
    let mut items: Vec<Item> = Vec::with_capacity(records.len());
    for record in records {
        let Some(slug) = record.get(&fields.slug).and_then(Value::as_str) else {
            return Err(layout());
        };
        if !links::valid_slug(slug) || items.iter().any(|item| item.slug == slug) {
            continue;
        }
        let title = record
            .get(&fields.text)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|title| !title.is_empty())
            .unwrap_or(slug);
        let text = |key: &str| {
            record
                .get(key)
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_owned)
        };
        let date = |key: &str| text(key).and_then(|value| time::parse_iso(&value));
        items.push(Item {
            slug: slug.to_owned(),
            title: title.to_owned(),
            cover: text("thumbnail"),
            badge: text("badge"),
            status: text("status").unwrap_or_default(),
            created: date("created_at"),
            updated: date("update_chapter"),
            views: views(record.get("total_views")),
        });
    }
    if items.is_empty() {
        return Err(layout());
    }
    Ok(items)
}

fn views(value: Option<&Value>) -> u64 {
    match value {
        Some(Value::Number(number)) => number
            .as_u64()
            .or_else(|| number.as_f64().filter(|v| *v >= 0.0).map(|v| v as u64))
            .unwrap_or(0),
        _ => 0,
    }
}

/// The items `browse` selects, in its order.
pub fn select<'a>(items: &'a [Item], browse: &Browse) -> Vec<&'a Item> {
    let mut chosen: Vec<&Item> = items
        .iter()
        .filter(|item| {
            browse
                .status
                .as_deref()
                .is_none_or(|status| item.status == status)
        })
        .collect();
    chosen.sort_by(|a, b| {
        let ordering = compare(a, b, browse.order).then_with(|| a.slug.cmp(&b.slug));
        match browse.direction {
            Direction::Ascending => ordering,
            Direction::Descending => ordering.reverse(),
        }
    });
    chosen
}

/// Ascending order on one key. A missing date is the oldest possible, as on the site.
fn compare(a: &Item, b: &Item, order: Order) -> Ordering {
    match order {
        Order::Updated => date_order(a.updated, b.updated),
        Order::Created => date_order(a.created, b.created),
        Order::Views => a.views.cmp(&b.views),
        Order::Title => a.title.to_lowercase().cmp(&b.title.to_lowercase()),
    }
}

fn date_order(a: Option<f64>, b: Option<f64>) -> Ordering {
    a.unwrap_or(f64::NEG_INFINITY)
        .total_cmp(&b.unwrap_or(f64::NEG_INFINITY))
}

/// One page of a browse: the entries, and whether another page follows. Pages count from one;
/// anything lower is the first. A page past the end is empty.
pub fn page(items: &[Item], browse: &Browse, page: i32) -> (Vec<Listed>, bool) {
    let chosen = select(items, browse);
    let index = usize::try_from(page.max(1) - 1).unwrap_or(0);
    let start = index.saturating_mul(PAGE_SIZE);
    let entries = chosen
        .iter()
        .skip(start)
        .take(PAGE_SIZE)
        .map(|item| item.listed())
        .collect();
    (entries, chosen.len() > start.saturating_add(PAGE_SIZE))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::dom;
    use crate::flight::script;

    pub(crate) fn record(
        title: &str,
        slug: &str,
        status: &str,
        created: &str,
        updated: Option<&str>,
        views: u64,
    ) -> String {
        let updated = updated.map_or("null".to_owned(), |value| format!("\"{value}\""));
        format!(
            r#"{{"tkn1":{title:?},"tkn2":{slug:?},"thumbnail":"https://media.templetoons.com/file/b/covers/{slug}.webp","badge":"Manhwa +18","status":{status:?},"created_at":"{created}","alternative_names":"","update_chapter":{updated},"total_views":{views},"tkn3":[{{"tkn4":"Chapter 1","tkn5":"chapter-1","created_at":"{created}"}}]}}"#
        )
    }

    pub(crate) fn page_html(records: &[String]) -> String {
        let row = format!(
            "c:[[\"$\",\"$L24\",\"1\",{{\"list\":[{}],\"initialPage\":1}}]]\n",
            records.join(",")
        );
        format!("<html><body>{}</body></html>", script(&[&row]))
    }

    fn sample() -> Vec<Item> {
        let html = page_html(&[
            record(
                " Zebra Stripes",
                "zebra-stripes",
                "Ongoing",
                "2026-01-01T00:00:00Z",
                Some("2026-10-01T00:00:00Z"),
                50,
            ),
            record(
                "apple",
                "apple",
                "Completed",
                "2025-01-01T00:00:00Z",
                Some("2026-09-01T00:00:00Z"),
                900,
            ),
            record(
                "Mango",
                "mango",
                "Ongoing",
                "2026-06-01T00:00:00Z",
                None,
                10,
            ),
            record(
                "Banana",
                "banana",
                "Dropped",
                "2024-01-01T00:00:00Z",
                Some("2026-10-01T00:00:00Z"),
                900,
            ),
            record(
                "Cherry",
                "cherry-",
                "Ongoing",
                "2026-03-01T00:00:00Z",
                Some("2026-05-01T00:00:00Z"),
                5,
            ),
        ]);
        parse(&dom::parse(&html)).expect("catalog")
    }

    fn slugs(entries: &[Listed]) -> Vec<&str> {
        entries.iter().map(|entry| entry.slug.as_str()).collect()
    }

    fn order(order: Order, direction: Direction) -> Vec<String> {
        select(
            &sample(),
            &Browse {
                status: None,
                order,
                direction,
            },
        )
        .into_iter()
        .map(|item| item.slug.clone())
        .collect()
    }

    #[test]
    fn the_catalog_is_read_from_the_page_whatever_the_renamed_tokens() {
        let items = sample();
        assert_eq!(items.len(), 5);
        let zebra = &items[0];
        assert_eq!(zebra.title, "Zebra Stripes", "titles are trimmed");
        assert_eq!(zebra.slug, "zebra-stripes");
        assert_eq!(zebra.status, "Ongoing");
        assert_eq!(zebra.views, 50);
        assert!(zebra.updated.is_some() && zebra.created.is_some());
        assert_eq!(items[2].updated, None, "a series never updated has no date");
        assert_eq!(items[4].slug, "cherry-", "slugs keep a trailing hyphen");
        let listed = zebra.listed();
        assert_eq!(listed.badge.as_deref(), Some("Manhwa +18"));
        assert_eq!(listed.status.as_deref(), Some("Ongoing"));
    }

    #[test]
    fn default_browsing_is_newest_update_first_across_every_status() {
        let (entries, next) = page(&sample(), &Browse::default(), 1);
        // Equal update times fall back to the slug, so the order never depends on input order.
        assert_eq!(
            slugs(&entries),
            ["zebra-stripes", "banana", "apple", "cherry-", "mango"]
        );
        assert!(!next);
    }

    #[test]
    fn every_order_and_direction_is_a_total_order() {
        assert_eq!(
            order(Order::Created, Direction::Descending),
            ["mango", "cherry-", "zebra-stripes", "apple", "banana"]
        );
        assert_eq!(
            order(Order::Created, Direction::Ascending),
            ["banana", "apple", "zebra-stripes", "cherry-", "mango"]
        );
        assert_eq!(
            order(Order::Views, Direction::Descending),
            ["banana", "apple", "zebra-stripes", "mango", "cherry-"]
        );
        assert_eq!(
            order(Order::Views, Direction::Ascending),
            ["cherry-", "mango", "zebra-stripes", "apple", "banana"]
        );
        assert_eq!(
            order(Order::Title, Direction::Ascending),
            ["apple", "banana", "cherry-", "mango", "zebra-stripes"]
        );
        assert_eq!(
            order(Order::Title, Direction::Descending),
            ["zebra-stripes", "mango", "cherry-", "banana", "apple"]
        );
        assert_eq!(
            order(Order::Updated, Direction::Ascending),
            ["mango", "cherry-", "apple", "banana", "zebra-stripes"]
        );
    }

    #[test]
    fn a_status_selects_exactly_that_status() {
        let items = sample();
        let only = |status: &str| {
            let browse = Browse {
                status: Some(status.into()),
                ..Browse::default()
            };
            select(&items, &browse)
                .into_iter()
                .map(|item| item.slug.as_str())
                .collect::<Vec<_>>()
        };
        assert_eq!(only("Ongoing"), ["zebra-stripes", "cherry-", "mango"]);
        assert_eq!(only("Completed"), ["apple"]);
        assert_eq!(only("Dropped"), ["banana"]);
        assert!(
            only("Hiatus").is_empty(),
            "a status nobody has is an empty list"
        );
        assert!(
            only("ongoing").is_empty(),
            "statuses are the site's exact words"
        );
    }

    fn many(count: usize) -> Vec<Item> {
        (0..count)
            .map(|n| Item {
                slug: format!("s{n:03}"),
                title: format!("Title {n:03}"),
                cover: None,
                badge: None,
                status: "Ongoing".into(),
                created: Some(n as f64),
                updated: Some(n as f64),
                views: n as u64,
            })
            .collect()
    }

    #[test]
    fn pages_are_one_based_and_know_when_the_last_one_is_reached() {
        let items = many(45);
        let browse = Browse {
            order: Order::Title,
            direction: Direction::Ascending,
            ..Browse::default()
        };
        let first = page(&items, &browse, 1);
        assert_eq!((first.0.len(), first.1), (20, true));
        assert_eq!(first.0[0].slug, "s000");
        let second = page(&items, &browse, 2);
        assert_eq!((second.0.len(), second.1), (20, true));
        assert_eq!(second.0[0].slug, "s020");
        let third = page(&items, &browse, 3);
        assert_eq!((third.0.len(), third.1), (5, false));
        assert_eq!(third.0[4].slug, "s044");
        let beyond = page(&items, &browse, 4);
        assert!(beyond.0.is_empty() && !beyond.1);
        // Pages below one mean the first.
        assert_eq!(slugs(&page(&items, &browse, 0).0), slugs(&first.0));
        assert_eq!(slugs(&page(&items, &browse, -7).0), slugs(&first.0));
        assert!(page(&items, &browse, i32::MAX).0.is_empty());
    }

    #[test]
    fn an_exact_multiple_of_the_page_size_has_no_phantom_next_page() {
        let items = many(40);
        let (entries, next) = page(&items, &Browse::default(), 2);
        assert_eq!(entries.len(), 20);
        assert!(!next);
        let (entries, next) = page(&many(0), &Browse::default(), 1);
        assert!(entries.is_empty() && !next);
    }

    #[test]
    fn a_catalog_that_is_not_where_or_what_it_should_be_is_an_error() {
        // No list at all.
        let none = "<html><body><script>self.__next_f.push([1,\"1:{\\\"a\\\":1}\\n\"])</script></body></html>";
        assert!(matches!(parse(&dom::parse(none)), Err(Error::Host(_))));
        // An empty list is not a catalog.
        let empty = page_html(&[]);
        assert!(matches!(parse(&dom::parse(&empty)), Err(Error::Host(_))));
        // Records that cannot be told apart.
        let row = "c:[{\"list\":[{\"a\":\"x\",\"b\":\"y\"}],\"initialPage\":1}]\n";
        let html = format!("<body>{}</body>", script(&[row]));
        assert!(matches!(parse(&dom::parse(&html)), Err(Error::Host(_))));
        // A list entry that is not a record.
        let row = "c:[{\"list\":[1,2],\"initialPage\":1}]\n";
        let html = format!("<body>{}</body>", script(&[row]));
        assert!(matches!(parse(&dom::parse(&html)), Err(Error::Host(_))));
    }

    #[test]
    fn duplicate_and_unusable_slugs_are_dropped_not_fatal() {
        let html = page_html(&[
            record("One", "one", "Ongoing", "2026-01-01T00:00:00Z", None, 1),
            record(
                "One again",
                "one",
                "Ongoing",
                "2026-01-01T00:00:00Z",
                None,
                2,
            ),
            record("Slash", "a/b", "Ongoing", "2026-01-01T00:00:00Z", None, 3),
            record("Two", "two", "Ongoing", "not a date", None, 4),
        ]);
        let items = parse(&dom::parse(&html)).expect("catalog");
        let slugs: Vec<_> = items.iter().map(|item| item.slug.as_str()).collect();
        assert_eq!(slugs, ["one", "two"]);
        assert_eq!(items[0].title, "One");
        assert_eq!(items[1].created, None);
    }

    #[test]
    fn order_ids_are_stable_identities() {
        assert_eq!(Order::from_id("update_chapter"), Some(Order::Updated));
        assert_eq!(Order::from_id("created_at"), Some(Order::Created));
        assert_eq!(Order::from_id("title"), Some(Order::Title));
        assert_eq!(Order::from_id("total_views"), Some(Order::Views));
        assert_eq!(Order::from_id("popular"), None);
        assert_eq!(Direction::from_id("desc"), Some(Direction::Descending));
        assert_eq!(Direction::from_id("asc"), Some(Direction::Ascending));
        assert_eq!(Direction::from_id("up"), None);
    }
}
