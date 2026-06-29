use std::collections::HashMap;

use crate::models::ComixChapter;

fn is_official_like(ch: &ComixChapter) -> bool {
	ch.scanlation_group_id == Some(10702) || ch.is_official
}

fn is_better(new_ch: &ComixChapter, cur: &ComixChapter) -> bool {
	let official_new = is_official_like(new_ch);
	let official_cur = is_official_like(cur);

	if official_new && !official_cur {
		return true;
	}
	if !official_new && official_cur {
		return false;
	}

	if new_ch.votes > cur.votes {
		return true;
	}
	if new_ch.votes < cur.votes {
		return false;
	}

	new_ch.updated_at > cur.updated_at
}

pub fn dedup_insert(map: &mut HashMap<String, ComixChapter>, ch: ComixChapter) {
	let key = ch.number.to_string();
	match map.get(&key) {
		None => {
			map.insert(key, ch.clone());
		}
		Some(current) => {
			if is_better(&ch, current) {
				map.insert(key, ch.clone());
			}
		}
	}
}

pub fn urlencode(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char);
            }
            _ => {
                out.push_str(&format!("%{:02X}", b));
            }
        }
    }
    out
}
