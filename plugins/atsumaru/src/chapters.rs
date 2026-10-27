//! Chapter list policy: hidden scanlators and optional collapsing of alternative versions.
//!
//! Collapsing only hides other groups' copies of a chapter number. Chapters without a number
//! and a group's own chapters sharing a number are always kept, so nothing is lost when a
//! site numbers extras or splits a chapter. Chapter ids are never rewritten.

use std::cmp::{Ordering, Reverse};
use std::collections::HashMap;

use crate::models::{ChapterItem, Scanlator};
use crate::settings::{Alternatives, ChapterPreferences};

/// Applies the chapter preferences and orders the list by descending chapter number.
pub fn select(
    mut chapters: Vec<ChapterItem>,
    groups: &[Scanlator],
    preferences: &ChapterPreferences,
) -> Vec<ChapterItem> {
    let groups = Groups { groups };
    if !preferences.hidden_scanlators.is_empty() {
        chapters.retain(|chapter| {
            groups
                .position(chapter, &preferences.hidden_scanlators)
                .is_none()
        });
    }
    if preferences.alternatives != Alternatives::ShowAll {
        chapters = collapse(chapters, &groups, preferences);
    }
    // Stable, so the site's order is kept among equal numbers.
    chapters.sort_by(|a, b| match (a.number, b.number) {
        (Some(a), Some(b)) => (b + 0.0).total_cmp(&(a + 0.0)),
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (None, None) => Ordering::Equal,
    });
    chapters
}

struct Groups<'a> {
    groups: &'a [Scanlator],
}

impl Groups<'_> {
    fn get(&self, chapter: &ChapterItem) -> Option<&Scanlator> {
        let id = chapter.scanlation_manga_id.as_deref()?;
        self.groups.iter().find(|group| group.id == id)
    }

    /// Where the chapter's group appears in a list of group names or ids.
    fn position(&self, chapter: &ChapterItem, names: &[String]) -> Option<usize> {
        let id = chapter.scanlation_manga_id.as_deref()?;
        let name = self.get(chapter).map(|group| group.name.as_str());
        names.iter().position(|entry| {
            entry == id || name.is_some_and(|name| name.eq_ignore_ascii_case(entry))
        })
    }
}

fn collapse(
    chapters: Vec<ChapterItem>,
    groups: &Groups,
    preferences: &ChapterPreferences,
) -> Vec<ChapterItem> {
    // The best version of each chapter number.
    let mut best: HashMap<u64, usize> = HashMap::new();
    for (index, chapter) in chapters.iter().enumerate() {
        let Some(number) = chapter.number else {
            continue;
        };
        best.entry(number_key(number))
            .and_modify(|current| {
                if compare(chapter, &chapters[*current], groups, preferences).is_lt() {
                    *current = index;
                }
            })
            .or_insert(index);
    }
    let winners: HashMap<u64, Option<String>> = best
        .into_iter()
        .map(|(key, index)| (key, chapters[index].scanlation_manga_id.clone()))
        .collect();
    chapters
        .into_iter()
        .filter(|chapter| {
            chapter.number.is_none_or(|number| {
                winners.get(&number_key(number)) == Some(&chapter.scanlation_manga_id)
            })
        })
        .collect()
}

/// Orders versions of one chapter number, best first. Total, so the result doesn't depend
/// on the order the site lists chapters in.
fn compare(
    a: &ChapterItem,
    b: &ChapterItem,
    groups: &Groups,
    preferences: &ChapterPreferences,
) -> Ordering {
    let preferred = |chapter: &ChapterItem| {
        groups
            .position(chapter, &preferences.preferred_scanlators)
            .unwrap_or(usize::MAX)
    };
    let score = |chapter: &ChapterItem| {
        groups
            .get(chapter)
            .and_then(|group| group.score)
            .unwrap_or(f64::NEG_INFINITY)
    };
    let uploaded = |chapter: &ChapterItem| chapter.created_at.unwrap_or(i64::MIN);
    let pages = |chapter: &ChapterItem| chapter.page_count.unwrap_or(0);

    preferred(a)
        .cmp(&preferred(b))
        .then_with(|| match preferences.alternatives {
            Alternatives::HighestVoted => score(b).total_cmp(&score(a)),
            Alternatives::NewestUpload => Ordering::Equal,
            Alternatives::MostPages => pages(b).cmp(&pages(a)),
            Alternatives::ShowAll => Ordering::Equal,
        })
        .then_with(|| Reverse(uploaded(a)).cmp(&Reverse(uploaded(b))))
        .then_with(|| a.id.cmp(&b.id))
}

/// Groups `0.0` with `-0.0`; other values compare by their exact bits.
fn number_key(number: f64) -> u64 {
    (number + 0.0).to_bits()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chapter(
        id: &str,
        group: &str,
        number: Option<f64>,
        created_at: i64,
        pages: u32,
    ) -> ChapterItem {
        ChapterItem {
            id: id.into(),
            scanlation_manga_id: Some(group.into()),
            number,
            title: None,
            created_at: Some(created_at),
            page_count: Some(pages),
        }
    }

    fn group(id: &str, name: &str, score: f64) -> Scanlator {
        Scanlator {
            id: id.into(),
            name: name.into(),
            score: Some(score),
        }
    }

    fn preferences(alternatives: Alternatives) -> ChapterPreferences {
        ChapterPreferences {
            hidden_scanlators: Vec::new(),
            preferred_scanlators: Vec::new(),
            alternatives,
        }
    }

    fn ids(chapters: &[ChapterItem]) -> Vec<&str> {
        chapters.iter().map(|chapter| chapter.id.as_str()).collect()
    }

    fn sample() -> Vec<ChapterItem> {
        vec![
            chapter("a1", "a", Some(1.0), 10, 20),
            chapter("b1", "b", Some(1.0), 30, 25),
            chapter("a2", "a", Some(2.0), 40, 18),
            chapter("a2-extra", "a", Some(2.0), 41, 5),
            chapter("b2", "b", Some(2.0), 35, 30),
            chapter("b1.5", "b", Some(1.5), 32, 12),
            chapter("special", "b", None, 50, 3),
            chapter("a0", "a", Some(-0.0), 1, 2),
            chapter("b0", "b", Some(0.0), 2, 2),
        ]
    }

    #[test]
    fn showing_all_versions_only_orders_by_number() {
        let groups = [group("a", "Alpha", 1.0), group("b", "Beta", 2.0)];
        let result = select(sample(), &groups, &preferences(Alternatives::ShowAll));
        assert_eq!(
            ids(&result),
            [
                "a2", "a2-extra", "b2", "b1.5", "a1", "b1", "a0", "b0", "special"
            ]
        );
    }

    #[test]
    fn collapsing_keeps_the_winning_group_with_its_own_duplicates() {
        let groups = [group("a", "Alpha", 9.0), group("b", "Beta", 2.0)];
        let result = select(sample(), &groups, &preferences(Alternatives::HighestVoted));
        assert_eq!(
            ids(&result),
            ["a2", "a2-extra", "b1.5", "a1", "a0", "special"]
        );
    }

    #[test]
    fn collapsing_does_not_depend_on_site_order() {
        let groups = [group("a", "Alpha", 1.0), group("b", "Beta", 1.0)];
        let mode = preferences(Alternatives::MostPages);
        let forward = select(sample(), &groups, &mode);
        let mut reversed_input = sample();
        reversed_input.reverse();
        let reversed = select(reversed_input, &groups, &mode);
        assert_eq!(ids(&forward), ["b2", "b1.5", "b1", "b0", "special"]);
        // Equal page counts for 0 fall back to the newest upload.
        assert_eq!(ids(&forward), ids(&reversed));
    }

    #[test]
    fn preferred_groups_win_before_the_selected_measure() {
        let groups = [group("a", "Alpha", 1.0), group("b", "Beta", 9.0)];
        let mut mode = preferences(Alternatives::NewestUpload);
        mode.preferred_scanlators = vec!["alpha".into()];
        let result = select(sample(), &groups, &mode);
        assert_eq!(
            ids(&result),
            ["a2", "a2-extra", "b1.5", "a1", "a0", "special"]
        );
    }

    #[test]
    fn hidden_groups_match_by_name_or_id() {
        let groups = [group("a", "Alpha", 1.0), group("b", "Beta", 9.0)];
        let mut mode = preferences(Alternatives::ShowAll);
        mode.hidden_scanlators = vec!["BETA".into()];
        assert_eq!(
            ids(&select(sample(), &groups, &mode)),
            ["a2", "a2-extra", "a1", "a0"]
        );
        mode.hidden_scanlators = vec!["a".into()];
        assert_eq!(
            ids(&select(sample(), &groups, &mode)),
            ["b2", "b1.5", "b1", "b0", "special"]
        );
    }
}
