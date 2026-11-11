//! Chapter parsing for the series page's Astro chapter island, and the chapter-number rules shared
//! with the latest-updates parser on the home page.

use std::collections::{HashMap, HashSet};

use ito_rs::catalog::{MAX_ABS_NUMBER, MAX_KEY_BYTES, MAX_TITLE_BYTES};
use ito_rs::models::{CatalogCompleteness, CatalogScope, Chapter, ChapterCatalog, ContentKind};
use serde_json::Value;

use crate::helpers::get_chapter_url;

/// What the chapter list covers. With `show_locked` off, paywalled uploads are removed, so the
/// list is a filtered view of what the site shows and is marked `Partial`. With it on, the list
/// is not checked against the site's own total, so its completeness stays unknown.
pub fn list_catalog(show_locked: bool) -> Option<ChapterCatalog> {
    (!show_locked).then_some(ChapterCatalog {
        scope: CatalogScope::Visible,
        completeness: CatalogCompleteness::Partial,
    })
}

/// Keeps a chapter number the SDK accepts. A number outside its limits is dropped and the
/// chapter keeps its other fields.
pub fn chapter_number(value: f64) -> Option<f64> {
    (value.is_finite() && value.abs() <= MAX_ABS_NUMBER).then_some(value)
}

/// The key a numbered chapter has always been published under: the number formatted as an `f32`.
/// Changing this formatting would change the key of existing downloads and reading progress.
fn numbered_key(number: f64) -> String {
    (number as f32).to_string()
}

/// One chapter object of the island, read before keys are assigned.
struct Row<'a> {
    /// The number the SDK accepts, if the site gives one.
    number: Option<f64>,
    /// The site's URL slug for this upload, when non-empty.
    slug: Option<&'a str>,
    title: Option<&'a str>,
    locked: bool,
}

fn read_row(obj: &Value) -> Row<'_> {
    // Index through `Value` (not `Map`) so a missing field yields Null instead of panicking.
    Row {
        number: obj["number"][1].as_f64().and_then(chapter_number),
        slug: obj["slug"][1].as_str().filter(|slug| !slug.is_empty()),
        title: obj["title"][1].as_str(),
        locked: obj["is_premium"][1].as_bool().unwrap_or_default(),
    }
}

/// Whether a slug may serve as a fallback key. Pages are fetched from `/chapter/<key>`, so the key
/// must be a path-safe token. It must start with a letter, so it can never look like a number key,
/// and it must not parse as a number (which also rules out `inf` and `nan`).
fn fallback_key(slug: &str) -> bool {
    slug.len() <= MAX_KEY_BYTES
        && slug.starts_with(|c: char| c.is_ascii_alphabetic())
        && slug
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        && slug.parse::<f64>().is_err()
}

/// Assigns each row its key, or `None` when the row is not listed. The key of a chapter never
/// depends on `show_locked`, but which uploads compete for a key does:
///
/// 1. Numbered chapters: the first listed upload of each number owns the number key. Hidden locked
///    uploads are not listed when `show_locked` is off, so they never block a free upload of the same
///    number. Later uploads of that number are not listed, because each number URL serves one upload.
/// 2. Unnumbered chapters: the slug is the key, when the slug is unique among every chapter in the
///    raw list (hidden ones included), passes [`fallback_key`], and is not already a key.
///
/// The site's numeric chapter id is never a key: `/chapter/<id>` does not serve the chapter.
fn assign_keys(rows: &[Row], show_locked: bool) -> Vec<Option<String>> {
    let mut slug_uses: HashMap<&str, usize> = HashMap::new();
    for slug in rows.iter().filter_map(|row| row.slug) {
        *slug_uses.entry(slug).or_default() += 1;
    }

    let listed = |row: &Row| show_locked || !row.locked;
    let mut used: HashSet<String> = HashSet::new();
    let mut keys = vec![None; rows.len()];

    for (index, row) in rows.iter().enumerate() {
        if !listed(row) {
            continue;
        }
        if let Some(number) = row.number {
            let key = numbered_key(number);
            if used.insert(key.clone()) {
                keys[index] = Some(key);
            }
        }
    }

    for (index, row) in rows.iter().enumerate() {
        if !listed(row) || row.number.is_some() {
            continue;
        }
        let Some(slug) = row.slug else { continue };
        if slug_uses.get(slug) == Some(&1) && fallback_key(slug) && used.insert(slug.to_string()) {
            keys[index] = Some(slug.to_string());
        }
    }

    keys
}

/// Maps the `chapters` array of the chapter-list island to chapters.
///
/// Each value is Astro-encoded as a `[type, value]` pair, so fields are read at index 1. Keys are
/// described on [`assign_keys`]. A chapter with neither a usable number nor a usable slug is
/// skipped.
pub fn parse_chapter_list(
    chapters_arr: &[Value],
    manga_key: &str,
    show_locked: bool,
) -> Vec<Chapter> {
    let rows: Vec<Row> = chapters_arr
        .iter()
        .map(|entry| &entry[1])
        .filter(|obj| obj.is_object())
        .map(read_row)
        .collect();
    let keys = assign_keys(&rows, show_locked);

    let mut chapters = Vec::new();
    for (row, key) in rows.iter().zip(keys) {
        let Some(key) = key else { continue };

        let url = get_chapter_url(&key, manga_key);
        chapters.push(Chapter {
            title: row
                .title
                .filter(|title| title.len() <= MAX_TITLE_BYTES)
                .map(str::to_string),
            chapter: row.number,
            url: Some(url),
            lang: Some("en".to_string()),
            paywalled: Some(row.locked),
            ..Chapter::new(key, ContentKind::Images)
        });
    }
    chapters
}

#[cfg(test)]
mod tests {
    use super::*;
    use ito_rs::catalog::validate_chapters;
    use serde_json::json;

    fn keys(chapters: &[Chapter]) -> Vec<&str> {
        chapters.iter().map(|c| c.key.as_str()).collect()
    }

    fn captured() -> Vec<Value> {
        serde_json::from_str(include_str!("fixtures/chapter_list_captured.json"))
            .expect("fixture parses")
    }

    /// Six chapters copied from a saved series page's chapter-list island, trimmed to the
    /// fields the parser reads. Keys must match the ABI 2 release exactly.
    #[test]
    fn captured_series_page_keys_are_unchanged() {
        let chapters = parse_chapter_list(&captured(), "cyberpunk-chivalry", true);
        assert_eq!(keys(&chapters), ["6", "5", "4", "3", "2", "1"]);
        assert_eq!(
            chapters[0].url.as_deref(),
            Some("https://asurascans.com/comics/cyberpunk-chivalry/chapter/6")
        );
        assert_eq!(chapters[0].chapter, Some(6.0));
        validate_chapters(&chapters).expect("captured list passes the SDK contract");
    }

    /// Representative (not captured) entries: fractional numbers keep their published keys.
    #[test]
    fn fractional_numbers_keep_their_published_keys() {
        let arr = vec![
            json!([0, {"id": [0, 700001], "number": [0, 10.1], "title": [0, "Ten and a tenth"], "is_premium": [0, false]}]),
            json!([0, {"id": [0, 700002], "number": [0, 2.5], "title": [0, "Two and a half"], "is_premium": [0, false]}]),
        ];
        let chapters = parse_chapter_list(&arr, "series", true);
        assert_eq!(keys(&chapters), ["10.1", "2.5"]);
        assert_eq!(chapters[0].chapter, Some(10.1));
        assert_eq!(chapters[1].chapter, Some(2.5));
        validate_chapters(&chapters).expect("fractional list passes the SDK contract");
    }

    /// Representative (not captured) entries: an unnumbered chapter is keyed by its slug, which is
    /// the form the chapter page URL uses.
    #[test]
    fn unnumbered_chapters_are_keyed_by_their_slug() {
        let arr = vec![
            json!([0, {"id": [0, 900001], "slug": [0, "chapter-prologue"], "number": [0, null], "title": [0, "Prologue"], "is_premium": [0, false]}]),
            json!([0, {"id": [0, 900002], "slug": [0, "chapter-interlude"], "number": [0, null], "title": [0, "Interlude"], "is_premium": [0, false]}]),
        ];
        let chapters = parse_chapter_list(&arr, "series", true);
        assert_eq!(keys(&chapters), ["chapter-prologue", "chapter-interlude"]);
        assert_eq!(chapters[0].chapter, None);
        assert_eq!(
            chapters[0].url.as_deref(),
            Some("https://asurascans.com/comics/series/chapter/chapter-prologue")
        );
        validate_chapters(&chapters).expect("unnumbered list passes the SDK contract");
    }

    /// Representative (not captured) entries: an unnumbered chapter with no usable slug has no key
    /// and is skipped. Its numeric id is never used, and neither is a slug that looks like a number.
    #[test]
    fn chapter_without_number_or_usable_slug_is_skipped() {
        let arr = vec![
            json!([0, {"id": [0, 900003], "number": [0, null], "title": [0, "No slug"], "is_premium": [0, false]}]),
            json!([0, {"id": [0, 900004], "slug": [0, ""], "number": [0, null], "is_premium": [0, false]}]),
            json!([0, {"id": [0, 900005], "slug": [0, "150"], "number": [0, null], "is_premium": [0, false]}]),
        ];
        assert!(parse_chapter_list(&arr, "series", true).is_empty());
    }

    /// Representative (not captured) entries: a numeric slug or id never takes chapter 150's key.
    /// Numbered keys are assigned first, so the real chapter 150 keeps `150`.
    #[test]
    fn number_looking_fallbacks_never_take_a_numbered_key() {
        let arr = vec![
            json!([0, {"id": [0, 150], "slug": [0, "150"], "number": [0, null], "title": [0, "Side story"], "is_premium": [0, false]}]),
            json!([0, {"id": [0, 9000], "slug": [0, "chapter-150"], "number": [0, 150], "is_premium": [0, false]}]),
            json!([0, {"id": [0, 151], "slug": [0, "1.5e2"], "number": [0, null], "is_premium": [0, false]}]),
        ];
        let chapters = parse_chapter_list(&arr, "series", true);
        assert_eq!(keys(&chapters), ["150"]);
        assert_eq!(chapters[0].chapter, Some(150.0));
        validate_chapters(&chapters).expect("list passes the SDK contract");
    }

    /// Representative (not captured) entries: a slug is a fallback only when no other chapter in the
    /// list uses it, so two unnumbered chapters that share a slug are both skipped.
    #[test]
    fn shared_slug_is_not_a_key() {
        let arr = vec![
            json!([0, {"id": [0, 900006], "slug": [0, "chapter-extra"], "number": [0, null], "is_premium": [0, false]}]),
            json!([0, {"id": [0, 900007], "slug": [0, "chapter-extra"], "number": [0, null], "is_premium": [0, false]}]),
        ];
        assert!(parse_chapter_list(&arr, "series", true).is_empty());
    }

    /// Representative (not captured) entries: two uploads of one chapter number. The first listed
    /// upload keeps the number key and the later uploads are dropped, since the number URL serves
    /// only one upload.
    #[test]
    fn repeated_number_keeps_the_first_upload_and_drops_the_rest() {
        let arr = vec![
            json!([0, {"id": [0, 900010], "slug": [0, "chapter-6"], "number": [0, 6], "title": [0, "First upload"], "is_premium": [0, false]}]),
            json!([0, {"id": [0, 900011], "slug": [0, "chapter-6-alt"], "number": [0, 6], "title": [0, "Second upload"], "is_premium": [0, false]}]),
            json!([0, {"id": [0, 900012], "slug": [0, "chapter-6-again"], "number": [0, 6], "is_premium": [0, false]}]),
        ];
        let chapters = parse_chapter_list(&arr, "series", true);
        assert_eq!(keys(&chapters), ["6"]);
        assert_eq!(chapters[0].title.as_deref(), Some("First upload"));
        validate_chapters(&chapters).expect("repeated number list passes the SDK contract");
    }

    /// Keys do not depend on `show_locked`, but which upload is listed does. With locked chapters
    /// shown, the locked first upload of number 6 is listed. With them hidden, the free later upload
    /// is listed under the same key `"6"` and the same URL.
    #[test]
    fn keys_do_not_depend_on_show_locked() {
        let arr = vec![
            json!([0, {"id": [0, 900020], "slug": [0, "chapter-6"], "number": [0, 6], "title": [0, "Locked upload"], "is_premium": [0, true]}]),
            json!([0, {"id": [0, 900021], "slug": [0, "chapter-6-free"], "number": [0, 6], "title": [0, "Free upload"], "is_premium": [0, false]}]),
            json!([0, {"id": [0, 900022], "slug": [0, "chapter-extra"], "number": [0, null], "is_premium": [0, false]}]),
        ];
        let shown = parse_chapter_list(&arr, "series", true);
        assert_eq!(keys(&shown), ["6", "chapter-extra"]);
        assert_eq!(shown[0].title.as_deref(), Some("Locked upload"));
        assert_eq!(shown[0].paywalled, Some(true));

        let free = parse_chapter_list(&arr, "series", false);
        assert_eq!(keys(&free), ["6", "chapter-extra"]);
        assert_eq!(free[0].title.as_deref(), Some("Free upload"));
        assert_eq!(free[0].paywalled, Some(false));
        assert_eq!(free[0].url, shown[0].url);
        validate_chapters(&free).expect("filtered list passes the SDK contract");
    }

    /// A hidden locked first upload of number 6 plus a free later upload of number 6: with locked
    /// chapters hidden, the free upload is listed with key `"6"`.
    #[test]
    fn hidden_locked_first_upload_leaves_free_later_upload_with_number_key() {
        let arr = vec![
            json!([0, {"id": [0, 900060], "slug": [0, "chapter-6"], "number": [0, 6], "title": [0, "Locked"], "is_premium": [0, true]}]),
            json!([0, {"id": [0, 900061], "slug": [0, "chapter-6-free"], "number": [0, 6], "title": [0, "Free"], "is_premium": [0, false]}]),
        ];
        let chapters = parse_chapter_list(&arr, "series", false);
        assert_eq!(keys(&chapters), ["6"]);
        assert_eq!(chapters[0].title.as_deref(), Some("Free"));
        assert_eq!(
            chapters[0].url.as_deref(),
            Some("https://asurascans.com/comics/series/chapter/6")
        );
        validate_chapters(&chapters).expect("filtered list passes the SDK contract");
    }

    /// Uniqueness of a slug is checked over the raw list, hidden chapters included. A hidden locked
    /// chapter with the same slug as a free one therefore blocks the free one's fallback key.
    #[test]
    fn hidden_locked_chapter_still_blocks_a_shared_slug() {
        let arr = vec![
            json!([0, {"id": [0, 900070], "slug": [0, "chapter-extra"], "number": [0, null], "is_premium": [0, true]}]),
            json!([0, {"id": [0, 900071], "slug": [0, "chapter-extra"], "number": [0, null], "is_premium": [0, false]}]),
        ];
        assert!(parse_chapter_list(&arr, "series", false).is_empty());
        assert!(parse_chapter_list(&arr, "series", true).is_empty());
    }

    /// Representative (not captured) entries: an out-of-range number is dropped and the chapter is
    /// keyed by its slug; an over-long title is dropped and the chapter is kept.
    #[test]
    fn out_of_range_values_are_dropped_not_the_chapter() {
        let long_title = "a".repeat(MAX_TITLE_BYTES + 1);
        let arr = vec![
            json!([0, {"id": [0, 900030], "slug": [0, "chapter-special"], "number": [0, 1e10], "is_premium": [0, false]}]),
            json!([0, {"id": [0, 900031], "slug": [0, "chapter-7"], "number": [0, 7], "title": [0, long_title], "is_premium": [0, false]}]),
        ];
        let chapters = parse_chapter_list(&arr, "series", true);
        assert_eq!(keys(&chapters), ["chapter-special", "7"]);
        assert_eq!(chapters[0].chapter, None);
        assert_eq!(chapters[1].title, None);
        validate_chapters(&chapters).expect("guarded list passes the SDK contract");
    }

    #[test]
    fn locked_chapters_are_removed_only_when_the_setting_is_off() {
        let arr = vec![
            json!([0, {"id": [0, 900040], "slug": [0, "chapter-1"], "number": [0, 1], "is_premium": [0, true]}]),
            json!([0, {"id": [0, 900041], "slug": [0, "chapter-2"], "number": [0, 2], "is_premium": [0, false]}]),
        ];
        assert_eq!(keys(&parse_chapter_list(&arr, "series", true)), ["1", "2"]);
        assert_eq!(keys(&parse_chapter_list(&arr, "series", false)), ["2"]);
    }

    #[test]
    fn catalog_marks_only_filtered_lists_as_partial() {
        assert_eq!(list_catalog(true), None);
        assert_eq!(
            list_catalog(false),
            Some(ChapterCatalog {
                scope: CatalogScope::Visible,
                completeness: CatalogCompleteness::Partial,
            })
        );
    }

    /// The captured list plus every representative case together still form one valid list.
    #[test]
    fn combined_list_passes_the_sdk_contract() {
        let mut arr = captured();
        arr.push(json!([0, {"id": [0, 900050], "slug": [0, "chapter-extra"], "number": [0, null], "is_premium": [0, false]}]));
        arr.push(json!([0, {"id": [0, 900051], "slug": [0, "chapter-6-alt"], "number": [0, 6], "is_premium": [0, false]}]));
        arr.push(json!([0, {"id": [0, 900052], "slug": [0, "chapter-10-1"], "number": [0, 10.1], "is_premium": [0, false]}]));
        let chapters = parse_chapter_list(&arr, "series", true);
        assert_eq!(
            keys(&chapters),
            ["6", "5", "4", "3", "2", "1", "chapter-extra", "10.1"]
        );
        validate_chapters(&chapters).expect("combined list passes the SDK contract");
    }
}
