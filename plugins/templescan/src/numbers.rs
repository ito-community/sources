//! Chapter numbers from the names the site gives chapters.
//!
//! The site reads the number out of a chapter's name, preferring the number that follows a
//! "chapter" style word and otherwise the first number anywhere, and only then falls back to the
//! chapter's position. Zero and fractional chapters are real numbers: "Chapter 0" is chapter 0,
//! and "Chapter 86.5" sits between 86 and 87.

/// Words that introduce a number, in the order the site tries them at one position.
const WORDS: [&str; 5] = ["chapter", "chap", "ch", "episode", "ep"];

/// The number in a chapter name: the first number after a chapter word, else the first number.
pub fn from_name(name: &str) -> Option<f32> {
    let lower = name.to_ascii_lowercase();
    let bytes = lower.as_bytes();
    for start in 0..bytes.len() {
        if !lower.is_char_boundary(start) {
            continue;
        }
        for word in WORDS {
            if lower[start..].starts_with(word)
                && let Some(number) = number_after(&lower[start + word.len()..])
            {
                return number.parse().ok();
            }
        }
    }
    (0..bytes.len())
        .filter(|start| lower.is_char_boundary(*start))
        .find_map(|start| number_at(&lower[start..]))
        .and_then(|number| number.parse().ok())
}

/// The number following `.`, whitespace and `#` that may separate a chapter word from it.
fn number_after(rest: &str) -> Option<&str> {
    let rest = rest.strip_prefix('.').unwrap_or(rest);
    let rest = rest.trim_start();
    let rest = rest.strip_prefix('#').unwrap_or(rest);
    number_at(rest.trim_start())
}

/// A decimal number at the start of `text`: digits, optionally a point and more digits.
fn number_at(text: &str) -> Option<&str> {
    let whole = text.bytes().take_while(u8::is_ascii_digit).count();
    if whole == 0 {
        return None;
    }
    let fraction = text[whole..]
        .strip_prefix('.')
        .map(|rest| rest.bytes().take_while(u8::is_ascii_digit).count())
        .unwrap_or(0);
    let end = if fraction > 0 {
        whole + 1 + fraction
    } else {
        whole
    };
    Some(&text[..end])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_and_decorated_chapter_names() {
        assert_eq!(from_name("Chapter 116"), Some(116.0));
        assert_eq!(from_name("chapter 58"), Some(58.0));
        assert_eq!(from_name("CHAPTER 7"), Some(7.0));
        assert_eq!(from_name("Chapter 41 - End Of Season 1"), Some(41.0));
        assert_eq!(from_name("Chapter 41.5 Testimonials"), Some(41.5));
        assert_eq!(from_name("Ch.12"), Some(12.0));
        assert_eq!(from_name("Ch #12"), Some(12.0));
        assert_eq!(from_name("Episode 3"), Some(3.0));
        assert_eq!(from_name("Ep. 4"), Some(4.0));
        assert_eq!(from_name("chapter12"), Some(12.0));
    }

    #[test]
    fn zero_and_fractional_chapters_are_numbers() {
        assert_eq!(from_name("Chapter 0"), Some(0.0));
        assert_eq!(from_name("Chapter 00"), Some(0.0));
        assert_eq!(from_name("Chapter 86.5"), Some(86.5));
        assert_eq!(from_name("Chapter 0.5"), Some(0.5));
        assert_eq!(
            from_name("Chapter 10."),
            Some(10.0),
            "a trailing point is punctuation"
        );
        assert_eq!(from_name("Chapter 007"), Some(7.0));
    }

    #[test]
    fn the_word_beats_an_earlier_number() {
        assert_eq!(from_name("Season 2 Chapter 9"), Some(9.0));
        assert_eq!(
            from_name("Side Story 3"),
            Some(3.0),
            "no word: the first number"
        );
        assert_eq!(from_name("2nd Special"), Some(2.0));
        assert_eq!(from_name("Prologue"), None);
        assert_eq!(from_name(""), None);
        assert_eq!(from_name("Chapters"), None);
    }

    #[test]
    fn unicode_names_do_not_split_characters() {
        assert_eq!(from_name("제 1화 Chapter 5"), Some(5.0));
        assert_eq!(from_name("외전"), None);
        assert_eq!(from_name("Ünïcode Chapter 12"), Some(12.0));
    }
}
