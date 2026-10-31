//! What a title looks like in listings, and how the site's labels map to Ito's.

use crate::images;
use crate::links;
use ito_rs::models::manga::{ContentRating, Manga, Status, Viewer};

/// A title as a listing shows it, whichever page or data it came from.
#[derive(Debug, Clone, PartialEq)]
pub struct Listed {
    pub slug: String,
    pub title: String,
    pub cover: Option<String>,
    /// The site's label such as `Manhwa +18`: a kind, and an age mark for adult series.
    pub badge: Option<String>,
    pub status: Option<String>,
}

impl Listed {
    pub fn into_manga(self) -> Manga {
        let rating = self.badge.as_deref().map_or(ContentRating::Safe, rating_of);
        Manga {
            url: Some(links::series_url(&self.slug)),
            key: self.slug,
            title: self.title,
            authors: None,
            artist: None,
            description: None,
            tags: None,
            cover: self.cover.as_deref().and_then(images::locate),
            status: self.status.as_deref().map_or(Status::Unknown, status_of),
            nsfw: nsfw_flag(&rating),
            content_rating: rating,
            // Every series is a vertical strip.
            viewer: Viewer::Webtoon,
            chapters: None,
        }
    }
}

/// The site's status words. `Dropped` is a series the group stopped, which Ito calls cancelled.
pub fn status_of(label: &str) -> Status {
    match label.trim().to_ascii_lowercase().as_str() {
        "ongoing" => Status::Ongoing,
        "completed" => Status::Completed,
        "hiatus" => Status::Hiatus,
        "canceled" | "cancelled" | "dropped" => Status::Cancelled,
        _ => Status::Unknown,
    }
}

/// Whether `label` is one of the words the site uses for a series' status.
pub fn is_status(label: &str) -> bool {
    !matches!(status_of(label), Status::Unknown)
}

/// The age a label such as `Manhwa +18` or `+15` marks, if it marks one.
pub fn age_mark(label: &str) -> Option<u32> {
    let mut rest = label;
    while let Some(at) = rest.find('+') {
        rest = &rest[at + 1..];
        let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
        if let Ok(age) = digits.parse() {
            return Some(age);
        }
    }
    None
}

/// The rating an age mark stands for: adult marks are explicit, a teen mark is suggestive.
pub fn rating_of(label: &str) -> ContentRating {
    match age_mark(label) {
        Some(age) if age >= 18 => ContentRating::Nsfw,
        Some(age) if age >= 15 => ContentRating::Suggestive,
        _ => ContentRating::Safe,
    }
}

pub fn nsfw_flag(rating: &ContentRating) -> i32 {
    i32::from(matches!(rating, ContentRating::Nsfw))
}

/// Orders ratings from least to most explicit, so a detail page can raise a rating but never
/// lower what a listing already showed.
pub fn severity(rating: &ContentRating) -> u8 {
    match rating {
        ContentRating::Safe => 0,
        ContentRating::Suggestive => 1,
        ContentRating::Nsfw => 2,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn listed(badge: Option<&str>, status: Option<&str>) -> Listed {
        Listed {
            slug: "evil-husband".into(),
            title: "Evil Husband".into(),
            cover: Some("https://media.templetoons.com/file/b/covers/c.webp".into()),
            badge: badge.map(Into::into),
            status: status.map(Into::into),
        }
    }

    #[test]
    fn age_marks_decide_the_rating() {
        for (badge, expected) in [
            ("Manhwa +18", ContentRating::Nsfw),
            ("One Shoot +18", ContentRating::Nsfw),
            ("Manhwa +18  Bl", ContentRating::Nsfw),
            ("Manhwa BL +18", ContentRating::Nsfw),
            ("+21", ContentRating::Nsfw),
            ("Manhwa +15", ContentRating::Suggestive),
            ("Manhwa", ContentRating::Safe),
            ("", ContentRating::Safe),
            ("Novel", ContentRating::Safe),
            ("C++ Manhwa", ContentRating::Safe),
            ("Manhwa +", ContentRating::Safe),
        ] {
            assert_eq!(
                severity(&rating_of(badge)),
                severity(&expected),
                "{badge:?}"
            );
        }
    }

    #[test]
    fn an_age_mark_after_a_stray_plus_is_still_found() {
        assert_eq!(age_mark("a+b +18"), Some(18));
        assert_eq!(age_mark("+15 +18"), Some(15));
        assert_eq!(age_mark("none"), None);
    }

    #[test]
    fn site_status_words_map_to_ito_statuses() {
        assert!(matches!(status_of("Ongoing"), Status::Ongoing));
        assert!(matches!(status_of(" completed "), Status::Completed));
        assert!(matches!(status_of("Hiatus"), Status::Hiatus));
        assert!(matches!(status_of("Canceled"), Status::Cancelled));
        assert!(matches!(status_of("Dropped"), Status::Cancelled));
        assert!(matches!(status_of("Season 2"), Status::Unknown));
        assert!(is_status("Dropped"));
        assert!(!is_status("Updated 3 days ago"));
    }

    #[test]
    fn a_listed_title_becomes_an_entry_with_a_stable_key_and_the_sites_rating() {
        let entry = listed(Some("Manhwa +18"), Some("Ongoing")).into_manga();
        assert_eq!(entry.key, "evil-husband");
        assert_eq!(
            entry.url.as_deref(),
            Some("https://templetoons.com/comic/evil-husband")
        );
        assert!(matches!(entry.status, Status::Ongoing));
        assert_eq!(severity(&entry.content_rating), 2);
        assert_eq!(entry.nsfw, 1);
        assert!(matches!(entry.viewer, Viewer::Webtoon));
        assert!(entry.chapters.is_none());
        let safe = listed(None, None).into_manga();
        assert_eq!((severity(&safe.content_rating), safe.nsfw), (0, 0));
        assert!(matches!(safe.status, Status::Unknown));
    }

    #[test]
    fn covers_off_the_site_are_left_out() {
        let mut item = listed(None, None);
        item.cover = Some("https://example.com/c.jpg".into());
        assert_eq!(item.into_manga().cover, None);
    }
}
