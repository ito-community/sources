use std::collections::HashMap;

use crate::models::ComixChapter;

fn is_official_like(ch: &ComixChapter) -> bool {
    ch.scanlation_group_id == Some(10702) || ch.is_official
}

/// Prefers official uploads, then the higher vote count. The site DTO has no numeric update
/// timestamp, so equal candidates keep the one already stored.
fn is_better(new_ch: &ComixChapter, cur: &ComixChapter) -> bool {
    let official_new = is_official_like(new_ch);
    let official_cur = is_official_like(cur);

    if official_new != official_cur {
        return official_new;
    }

    new_ch.votes > cur.votes
}

pub fn dedup_insert(map: &mut HashMap<String, ComixChapter>, ch: ComixChapter) {
    let key = ch.number.to_string();
    match map.get(&key) {
        None => {
            map.insert(key, ch);
        }
        Some(current) => {
            if is_better(&ch, current) {
                map.insert(key, ch);
            }
        }
    }
}

pub fn urlencode(value: &str) -> String {
    urlencoding::encode(value).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::ScanlationGroup;

    fn chapter(votes: i32, is_official: bool, group: Option<i32>) -> ComixChapter {
        ComixChapter {
            chapter_id: votes,
            scanlation_group_id: group,
            number: 1.0,
            name: None,
            votes,
            is_official,
            scanlation_group: Some(ScanlationGroup {
                name: "group".into(),
            }),
        }
    }

    #[test]
    fn official_upload_replaces_a_community_upload_regardless_of_votes() {
        let mut map = HashMap::new();
        dedup_insert(&mut map, chapter(50, false, Some(1)));
        dedup_insert(&mut map, chapter(0, true, Some(2)));
        assert_eq!(map["1"].chapter_id, 0);
    }

    #[test]
    fn higher_votes_replace_an_equal_status_upload() {
        let mut map = HashMap::new();
        dedup_insert(&mut map, chapter(3, false, Some(1)));
        dedup_insert(&mut map, chapter(9, false, Some(2)));
        dedup_insert(&mut map, chapter(4, false, Some(3)));
        assert_eq!(map["1"].chapter_id, 9);
    }

    #[test]
    fn equal_candidates_keep_the_stored_upload() {
        let mut map = HashMap::new();
        dedup_insert(&mut map, chapter(5, false, Some(1)));
        dedup_insert(&mut map, chapter(5, false, Some(2)));
        assert_eq!(map["1"].chapter_id, 5);
        assert_eq!(map["1"].scanlation_group_id, Some(1));
    }
}
